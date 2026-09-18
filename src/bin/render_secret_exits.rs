//! Headless mock screenshot of the world-editor Secret Exits 2/3 window.
//!
//! egui can't render headless, so this composes an honest mock: the map is
//! the real emulator render of submap 0, and the dialog contents (level
//! selector, direction checkboxes, level list) come from a real
//! `SecretExitSettings` that goes through the actual
//! `write_secret_exits` → `parse_secret_exits` ROM round trip on a scratch
//! copy of the real ROM. Only the window chrome (title bar, button borders,
//! checkbox squares) is drawn rather than real egui widgets.
//!
//! ```sh
//! cargo run --bin render_secret_exits -- --rom=/path/to/smw.smc --out=docs/screenshots/ow-secret-exits.png
//! ```

use std::sync::Arc;

use ab_glyph::{Font, FontRef, Glyph, Point, PxScale, ScaleFont};
use image::{ImageBuffer, Rgb, RgbImage};
use smw_editor::render_util::{fill_rect, rect_border, render_tile};
use smwe_emu::{emu::CheckedMem, rom::Rom as EmuRom, Cpu};
use smwe_rom::overworld::secret_exits::{
    self,
    SecretExitEntry,
    SecretExitSettings,
    DIR_DOWN,
    DIR_LEFT,
    DIR_RIGHT,
    DIR_UP,
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

/// Draw one direction-checkbox row; returns the new y.
fn dir_row(
    img: &mut RgbImage, sans: &FontRef, mono: &FontRef, label: &str, mask: u8, x: u32, y: u32, ink: Rgb<u8>,
    gray: Rgb<u8>,
) -> u32 {
    let mut y = y;
    draw_text(img, sans, label, x as i32, y as i32, 13.0, ink);
    y += 24;
    let mut cx = x + 12;
    for (bit, name) in [(DIR_UP, "Up"), (DIR_DOWN, "Down"), (DIR_LEFT, "Left"), (DIR_RIGHT, "Right")] {
        let on = mask & bit != 0;
        let glyph = if on { "\u{2611}" } else { "\u{2610}" };
        draw_text(img, sans, glyph, cx as i32, y as i32, 14.0, if on { ink } else { gray });
        draw_text(img, mono, name, (cx + 22) as i32, (y + 1) as i32, 12.0, if on { ink } else { gray });
        cx += 92;
    }
    y + 26
}

fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let output = args.iter().find_map(|a| a.strip_prefix("--out=")).unwrap_or("docs/screenshots/ow-secret-exits.png");
    let rom_path = args
        .iter()
        .find_map(|a| a.strip_prefix("--rom="))
        .or_else(|| args.iter().skip(1).find(|a| !a.starts_with("--")).map(|a| a.as_str()))
        .unwrap_or("smw.smc");

    let mono = load_font(MONO_CANDIDATES)?;
    let sans = load_font(SANS_CANDIDATES)?;
    let sans_bold = load_font(SANS_BOLD_CANDIDATES)?;

    // ── Real settings, same code the UI uses ───────────────────────────
    let raw = std::fs::read(rom_path)?;
    let rom_bytes: Vec<u8> = if raw.len() % 0x400 == 0x200 { raw[0x200..].to_vec() } else { raw };
    anyhow::ensure!(
        secret_exits::parse_secret_exits(&rom_bytes, 0).entries.is_empty(),
        "vanilla ROM should have no block"
    );

    let mut settings = SecretExitSettings::default();
    settings.set(SecretExitEntry { level: 0x101, exit2: DIR_UP | DIR_RIGHT, exit3: DIR_LEFT });
    settings.set(SecretExitEntry { level: 0x007, exit2: DIR_DOWN, exit3: DIR_UP | DIR_LEFT });
    let mut scratch = rom_bytes.clone();
    secret_exits::write_secret_exits(&settings, &mut scratch, 0)?;
    let settings = secret_exits::parse_secret_exits(&scratch, 0);
    anyhow::ensure!(settings.entries.len() == 2, "round trip lost entries");
    anyhow::ensure!(
        settings.get(0x101) == Some(SecretExitEntry { level: 0x101, exit2: DIR_UP | DIR_RIGHT, exit3: DIR_LEFT })
    );

    // ── Real emulator render of submap 0 ───────────────────────────────
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
    let map = ImageBuffer::<Rgb<u8>, _>::from_raw(512, 512, pixels).expect("image buffer");

    // ── Compose: dialog mock + map + caption ───────────────────────────
    let (pw, mw, gap) = (400u32, 512u32, 24u32);
    let (w, h) = (pw + gap + mw + 48, 800);
    let mut img = RgbImage::new(w, h);
    let bg = Rgb([0xF2, 0xF2, 0xF2]);
    let ink = Rgb([0x1A, 0x1A, 0x1A]);
    let gray = Rgb([0x66, 0x66, 0x66]);
    let accent = Rgb([0x1F, 0x6F, 0xC2]);
    for p in img.pixels_mut() {
        *p = bg;
    }
    fill_rect(&mut img, 0, 0, w, 52, Rgb([0x2B, 0x2B, 0x2B]));
    draw_text(
        &mut img,
        &sans_bold,
        "World Editor \u{2014} Secret Exits 2/3 window (headless mock; map + settings are real ROM output)",
        24,
        15,
        16.0,
        Rgb([0xFF, 0xFF, 0xFF]),
    );

    // Dialog window chrome.
    let (wx, wy, ww) = (24u32, 72u32, pw);
    fill_rect(&mut img, wx, wy, ww, 560, Rgb([0xFF, 0xFF, 0xFF]));
    rect_border(&mut img, wx, wy, ww, 560, Rgb([0x99, 0x99, 0x99]));
    fill_rect(&mut img, wx, wy, ww, 34, Rgb([0xE8, 0xE8, 0xE8]));
    draw_text(
        &mut img,
        &sans_bold,
        "Secret Exits 2/3 \u{2014} direction to enable",
        (wx + 12) as i32,
        (wy + 8) as i32,
        14.0,
        ink,
    );
    draw_text(&mut img, &sans, "\u{2715}", (wx + ww - 26) as i32, (wy + 8) as i32, 14.0, gray);

    let mut y = wy + 48;
    let ix = wx + 16;
    draw_text(&mut img, &sans, "Movement directions granted on the overworld when", ix as i32, y as i32, 12.0, gray);
    y += 16;
    draw_text(
        &mut img,
        &sans,
        "the player clears a level through Secret Exit 2 or 3.",
        ix as i32,
        y as i32,
        12.0,
        gray,
    );
    y += 30;

    // Level selector (mock slider).
    draw_text(&mut img, &sans_bold, "Level:", ix as i32, (y + 2) as i32, 13.0, ink);
    fill_rect(&mut img, ix + 52, y, 220, 22, Rgb([0xF2, 0xF2, 0xF2]));
    rect_border(&mut img, ix + 52, y, 220, 22, Rgb([0x99, 0x99, 0x99]));
    fill_rect(&mut img, ix + 200, y + 2, 12, 18, accent); // slider knob
    draw_text(&mut img, &mono, "$101", (ix + 284) as i32, (y + 3) as i32, 13.0, ink);
    y += 40;

    // Direction checkboxes for the selected level (real values).
    let entry = settings.get(0x101).expect("level $101 entry");
    y = dir_row(&mut img, &sans, &mono, "Directions to enable on Secret Exit 2:", entry.exit2, ix, y, ink, gray);
    y = dir_row(&mut img, &sans, &mono, "Directions to enable on Secret Exit 3:", entry.exit3, ix, y, ink, gray);

    // Level list (real entries).
    draw_text(&mut img, &sans_bold, "Levels with settings:", ix as i32, y as i32, 13.0, ink);
    y += 26;
    for e in &settings.entries {
        let selected = e.level == 0x101;
        if selected {
            fill_rect(&mut img, ix, y - 3, ww - 32, 22, Rgb([0xFF, 0xF3, 0xD6]));
        }
        draw_text(
            &mut img,
            &mono,
            &format!("Level ${:03X} \u{2014} exit 2: {:#04X}, exit 3: {:#04X}", e.level, e.exit2, e.exit3),
            (ix + 6) as i32,
            y as i32,
            11.5,
            ink,
        );
        y += 24;
    }
    y += 8;

    // Notes.
    for line in [
        "Goal tapes: level editor, sprite $7B (Goal Point), Extra bits = 2/3.",
        "Stock-ROM note: real Secret Exit 2/3 behavior needs LM v3.00's ASM;",
        "the settings above are stored faithfully regardless.",
    ] {
        draw_text(&mut img, &sans, line, ix as i32, y as i32, 11.0, gray);
        y += 17;
    }

    // Map.
    let map_x = pw + gap + 24;
    for (px, py, p) in map.enumerate_pixels() {
        img.put_pixel(map_x + px, 64 + py, *p);
    }
    rect_border(&mut img, map_x, 64, 512, 512, Rgb([0x99, 0x99, 0x99]));

    // Caption: real storage facts.
    draw_text(
        &mut img,
        &sans,
        "editor-owned SMWSEXIT RATS block (real write \u{2192} re-parse round trip; 2 levels stored)",
        24,
        (h - 56) as i32,
        12.0,
        gray,
    );
    draw_text(
        &mut img,
        &sans,
        "direction bits = vanilla OWLevelTileSettings convention ($01 right, $02 left, $04 down, $08 up)",
        24,
        (h - 34) as i32,
        12.0,
        gray,
    );

    img.save(output)?;
    println!("wrote {output}");
    Ok(())
}
