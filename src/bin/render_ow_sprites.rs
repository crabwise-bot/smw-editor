//! Headless mock screenshot of the world-editor overworld sprite tool.
//!
//! egui can't render headless, so this composes an honest mock of the new
//! "Sprites" tool: the map is the real emulator render of submap 0, the
//! marker positions / numbers / visibility states come from the real ROM
//! tables via `smwe_rom::overworld::sprites` (the same code the UI uses),
//! and the two custom sprites go through the real insert →
//! `write_custom_table` → re-parse round trip. Only the panel chrome
//! (button borders, checkbox squares) is drawn rather than real egui
//! widgets.
//!
//! ```sh
//! cargo run --bin render_ow_sprites -- --rom=/path/to/smw.smc --out=docs/screenshots/ow-sprites.png
//! ```

use std::sync::Arc;

use ab_glyph::{Font, FontRef, Glyph, Point, PxScale, ScaleFont};
use image::{ImageBuffer, Rgb, RgbImage};
use smw_editor::render_util::{fill_rect, rect_border, render_tile};
use smwe_emu::{emu::CheckedMem, rom::Rom as EmuRom, Cpu};
use smwe_rom::overworld::{
    sprites::{self, CustomOwSprite, CustomSpriteTable, VanillaOwSprites},
    SUBMAP_NAMES,
};

const MONO_CANDIDATES: &[&str] = &["/usr/share/fonts/truetype/dejavu/DejaVuSansMono.ttf"];
const SANS_CANDIDATES: &[&str] = &["/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf"];
const SANS_BOLD_CANDIDATES: &[&str] = &["/usr/share/fonts/truetype/dejavu/DejaVuSans-Bold.ttf"];

const VRAM_L1_TILEMAP_BASE: usize = 0x2000 * 2;
const VRAM_L2_TILEMAP_BASE: usize = 0x3000 * 2;
const OW_COLS: u32 = 64;
const OW_ROWS: u32 = 64;

fn load_font(candidates: &[&str]) -> anyhow::Result<FontRef<'static>> {
    for p in candidates {
        if let Ok(data) = std::fs::read(p) {
            let leaked: &'static [u8] = Box::leak(data.into_boxed_slice());
            return FontRef::try_from_slice(leaked).map_err(|e| anyhow::anyhow!("{p}: {e}"));
        }
    }
    anyhow::bail!("no font file found; tried {candidates:?}")
}

/// Draw one line of text; returns the advance width in px.
fn draw_text(img: &mut RgbImage, font: &FontRef, text: &str, x: i32, y: i32, px: f32, color: Rgb<u8>) -> i32 {
    let scaled = font.as_scaled(PxScale::from(px));
    let mut caret_x = x as f32;
    let baseline = y as f32 + scaled.ascent();
    let mut prev = None;
    for ch in text.chars() {
        let id = font.glyph_id(ch);
        if let Some(p) = prev {
            caret_x += scaled.kern(p, id);
        }
        let glyph = Glyph { id, scale: PxScale::from(px), position: Point { x: caret_x, y: baseline } };
        if let Some(o) = scaled.outline_glyph(glyph) {
            let bb = o.px_bounds();
            o.draw(|gx, gy, v| {
                let px_x = bb.min.x as i32 + gx as i32;
                let px_y = bb.min.y as i32 + gy as i32;
                if px_x >= 0 && px_y >= 0 {
                    let (px_x, px_y) = (px_x as u32, px_y as u32);
                    if px_x < img.width() && px_y < img.height() {
                        let d = img.get_pixel(px_x, px_y).0;
                        let s = color.0;
                        let a = (v * 255.0) as u16;
                        let inv = 255 - a;
                        img.put_pixel(
                            px_x,
                            px_y,
                            Rgb([
                                ((s[0] as u16 * a + d[0] as u16 * inv) / 255) as u8,
                                ((s[1] as u16 * a + d[1] as u16 * inv) / 255) as u8,
                                ((s[2] as u16 * a + d[2] as u16 * inv) / 255) as u8,
                            ]),
                        );
                    }
                }
            });
        }
        caret_x += scaled.h_advance(id);
        prev = Some(id);
    }
    (caret_x - x as f32) as i32
}

fn put(img: &mut RgbImage, x: i32, y: i32, c: Rgb<u8>) {
    if x >= 0 && y >= 0 {
        let (x, y) = (x as u32, y as u32);
        if x < img.width() && y < img.height() {
            img.put_pixel(x, y, c);
        }
    }
}

/// Midpoint circle outline.
fn circle(img: &mut RgbImage, cx: i32, cy: i32, r: i32, c: Rgb<u8>) {
    let (mut x, mut y) = (r, 0);
    let mut err = 1 - r;
    while x >= y {
        for (px, py) in [(x, y), (y, x), (-x, y), (-y, x), (-x, -y), (-y, -x), (x, -y), (y, -x)] {
            put(img, cx + px, cy + py, c);
        }
        y += 1;
        if err < 0 {
            err += 2 * y + 1;
        } else {
            x -= 1;
            err += 2 * (y - x) + 1;
        }
    }
}

fn tilemap_vram_addr(base: usize, col: u32, row: u32) -> usize {
    let quadrant = ((row / 32) * 2) + (col / 32);
    let sub_row = row % 32;
    let sub_col = col % 32;
    let quadrant_offset = quadrant * 32 * 32 * 2;
    let idx = quadrant_offset + ((sub_row * 32 + sub_col) * 2);
    base + idx as usize
}

fn render_bg(vram: &[u8], tilemap_base: usize, cgram: &[u8], pixels: &mut [u8]) {
    for row in 0..OW_ROWS {
        for col in 0..OW_COLS {
            let addr = tilemap_vram_addr(tilemap_base, col, row);
            let t0 = vram[addr] as u16;
            let t1 = vram[addr + 1] as u16;
            render_tile(
                vram,
                cgram,
                (t0 | ((t1 & 3) << 8)) as usize,
                ((t1 >> 2) & 7) as usize,
                (t1 & 0x40) != 0,
                (t1 & 0x80) != 0,
                col * 8,
                row * 8,
                512,
                pixels,
            );
        }
    }
}

fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let output = args.iter().find_map(|a| a.strip_prefix("--out=")).unwrap_or("docs/screenshots/ow-sprites.png");
    let rom_path = args
        .iter()
        .find_map(|a| a.strip_prefix("--rom="))
        .or_else(|| args.iter().skip(1).find(|a| !a.starts_with("--")).map(|a| a.as_str()))
        .unwrap_or("smw.smc");

    let mono = load_font(MONO_CANDIDATES)?;
    let sans = load_font(SANS_CANDIDATES)?;
    let sans_bold = load_font(SANS_BOLD_CANDIDATES)?;

    // ── Real ROM data, same code the UI uses ──────────────────────────
    let raw = std::fs::read(rom_path)?;
    let rom_bytes: Vec<u8> = if raw.len() % 0x400 == 0x200 { raw[0x200..].to_vec() } else { raw };
    let vanilla = VanillaOwSprites::parse(&rom_bytes, 0)?;
    anyhow::ensure!(vanilla.sprites.len() == sprites::VANILLA_SPRITE_COUNT);

    // Insert two custom sprites and round-trip them through the real
    // write/parse path on a scratch copy (the vanilla ROM has none).
    let mut table = CustomSpriteTable::default();
    table.submaps[0].push(CustomOwSprite { number: 0x10, x: 20, y: 44, height: 3, extra: vec![0x2A] });
    table.submaps[0].push(CustomOwSprite { number: 0x2B, x: 40, y: 18, height: 0, extra: vec![0x01, 0x02] });
    let mut scratch = rom_bytes.clone();
    sprites::write_custom_table(&table, &mut scratch, 0)?;
    let table = sprites::parse_custom_table(&scratch, 0)?.expect("custom table round trip");
    anyhow::ensure!(table.submaps[0].len() == 2);
    anyhow::ensure!(table.submaps[0][0].extra == vec![0x2A]);

    // Which vanilla sprites does the game show on the main map? Ask the
    // real visibility logic.
    let mut markers: Vec<(i32, i32, u8, bool)> = Vec::new(); // (x, y, number, is_custom)
    for sprite in &vanilla.sprites {
        if vanilla.is_active_on(sprite.number, 0) {
            markers.push((sprite.x_px() as i32, sprite.y_px() as i32, sprite.number, false));
        }
    }
    for e in &table.submaps[0] {
        markers.push((e.x_px() as i32, e.y_px() as i32, e.number, true));
    }
    anyhow::ensure!(markers.iter().any(|m| m == &(56, 394, 0x07, false)), "smoke sprite missing: {markers:?}");

    // ── Real emulator render of submap 0 ──────────────────────────────
    let mut emu_rom = EmuRom::new(rom_bytes);
    emu_rom.load_symbols(include_str!("../../symbols/SMW_U.sym"));
    let mut cpu = Cpu::new(CheckedMem::new(Arc::new(emu_rom)));
    for addr in 0x1F02u32..=0x1F60 {
        cpu.mem.store_u8(addr, 0xFF); // all events active, like the editor preview
    }
    smwe_emu::emu::load_overworld(&mut cpu, 0);
    let mut pixels = vec![0u8; (512 * 512 * 3) as usize];
    render_bg(&cpu.mem.vram, VRAM_L2_TILEMAP_BASE, &cpu.mem.cgram, &mut pixels);
    render_bg(&cpu.mem.vram, VRAM_L1_TILEMAP_BASE, &cpu.mem.cgram, &mut pixels);
    let mut map = ImageBuffer::<Rgb<u8>, _>::from_raw(512, 512, pixels).expect("image buffer");

    // ── Sprite markers (same style as the UI: yellow = vanilla, cyan =
    // custom, white = selected) ───────────────────────────────────────
    let yellow = Rgb([0xFF, 0xD6, 0x40]);
    let cyan = Rgb([0x50, 0xDC, 0xFF]);
    let white = Rgb([0xFF, 0xFF, 0xFF]);
    let selected: Option<(i32, i32)> = Some((56, 394)); // Yoshi's House smoke
    for (x, y, number, is_custom) in &markers {
        if *x < -24 || *y < -24 || *x > 536 || *y > 536 {
            continue; // same culling as the UI overlay
        }
        let is_sel = selected == Some((*x, *y));
        let base = if *is_custom { cyan } else { yellow };
        let ring = if is_sel { white } else { base };
        circle(&mut map, *x, *y, 9, ring);
        if is_sel {
            circle(&mut map, *x, *y, 11, ring);
        }
        put(&mut map, *x, *y, base);
        draw_text(&mut map, &mono, &format!("{number:02X}"), x + 12, y - 7, 11.0, ring);
    }

    // ── Compose: panel mock + map + caption ───────────────────────────
    let (pw, mw, gap, cw) = (360u32, 512u32, 24u32, 0u32);
    let (w, h) = (pw + gap + mw + cw + 48, 800);
    let mut img = RgbImage::new(w, h);
    let bg = Rgb([0xF2, 0xF2, 0xF2]);
    let ink = Rgb([0x1A, 0x1A, 0x1A]);
    let gray = Rgb([0x66, 0x66, 0x66]);
    for p in img.pixels_mut() {
        *p = bg;
    }
    fill_rect(&mut img, 0, 0, w, 52, Rgb([0x2B, 0x2B, 0x2B]));
    draw_text(
        &mut img,
        &sans_bold,
        "World Editor \u{2014} Overworld sprite tool (headless mock; markers + data are real ROM output)",
        24,
        15,
        17.0,
        Rgb([0xFF, 0xFF, 0xFF]),
    );

    // Panel.
    let mut y = 72u32;
    draw_text(&mut img, &sans_bold, &format!("Sprites \u{2014} {}", SUBMAP_NAMES[0]), 24, y as i32, 17.0, ink);
    y += 30;
    draw_text(&mut img, &sans, "Custom sprites need a third-party runtime patch", 24, y as i32, 11.0, gray);
    y += 15;
    draw_text(&mut img, &sans, "to move in-game \u{2014} smw-editor stores the table.", 24, y as i32, 11.0, gray);
    y += 32;
    draw_text(&mut img, &sans_bold, "On this submap:", 24, y as i32, 13.0, ink);
    y += 26;
    for (x, sy, number, is_custom) in markers.iter().take(8) {
        let label = if *is_custom {
            format!("custom {number:02X} @ ({x}, {sy})")
        } else {
            format!("{number:02X} {} @ ({x}, {sy})", sprites::sprite_type_name(*number))
        };
        let is_sel = selected == Some((*x, *sy));
        if is_sel {
            fill_rect(&mut img, 24, y, pw - 24, 20, Rgb([0xFF, 0xF3, 0xD6]));
        }
        draw_text(&mut img, &mono, &label, 28, (y + 3) as i32, 11.0, ink);
        y += 22;
    }
    draw_text(
        &mut img,
        &sans,
        &format!("\u{2026} ({} total on this submap)", markers.len()),
        28,
        (y + 2) as i32,
        11.0,
        gray,
    );
    y += 30;
    fill_rect(&mut img, 24, y, 190, 30, Rgb([0xFF, 0xFF, 0xFF]));
    rect_border(&mut img, 24, y, 190, 30, Rgb([0x99, 0x99, 0x99]));
    draw_text(&mut img, &sans, "Insert custom sprite", 36, (y + 7) as i32, 13.0, ink);
    y += 44;

    // Selected-sprite editor (real values for the smoke sprite).
    draw_text(&mut img, &sans_bold, "Vanilla sprite \u{2014} slot 3", 24, y as i32, 14.0, ink);
    y += 28;
    let smoke = &vanilla.sprites[3];
    anyhow::ensure!(smoke.number == 0x07);
    draw_text(&mut img, &mono, &format!("Sprite: 07 {}", sprites::sprite_type_name(0x07)), 28, y as i32, 12.0, ink);
    y += 24;
    draw_text(&mut img, &mono, &format!("X {}   Y {}", smoke.x_px(), smoke.y_px()), 28, y as i32, 12.0, ink);
    y += 28;
    draw_text(&mut img, &sans_bold, "Visible on:", 24, y as i32, 13.0, ink);
    y += 24;
    for map_idx in 0..7u8 {
        let on = vanilla.is_active_on(0x07, map_idx);
        let label = if on { "\u{2611}" } else { "\u{2610}" };
        draw_text(&mut img, &sans, label, 28, y as i32, 13.0, ink);
        draw_text(&mut img, &sans, SUBMAP_NAMES[map_idx as usize], 52, y as i32, 12.0, if on { ink } else { gray });
        y += 20;
    }

    // Map.
    let map_x = pw + gap + 24;
    for (px, py, p) in map.enumerate_pixels() {
        img.put_pixel(map_x + px, 64 + py, *p);
    }
    rect_border(&mut img, map_x, 64, 512, 512, Rgb([0x99, 0x99, 0x99]));

    // Caption: real addresses + real counts.
    draw_text(
        &mut img,
        &sans,
        "13 vanilla slots @ $04F625 \u{00B7} visibility per sprite number @ $04F828 (set bit = hidden)",
        24,
        (h - 56) as i32,
        12.0,
        gray,
    );
    draw_text(
        &mut img,
        &sans,
        &format!(
            "custom table behind the $0EF55D pointer ({} custom here via real insert \u{2192} write \u{2192} re-parse)",
            table.submaps[0].len()
        ),
        24,
        (h - 34) as i32,
        12.0,
        gray,
    );

    img.save(output)?;
    println!("wrote {output} ({} markers)", markers.len());
    Ok(())
}
