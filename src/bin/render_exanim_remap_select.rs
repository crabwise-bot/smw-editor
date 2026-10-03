//! Headless screenshot of the ExAnimation "Remap" + "8x8 Select" feature
//! (LM v3.32 parity).
//!
//! egui can't render headless, so this composes an honest mock of the
//! "ExAnimated Frames" dialog with the new buttons: the tile browser is the
//! REAL VRAM atlas (level 0x105 through the real emulator path, decoded with
//! the exact `Tile::from_4bpp` + CGRAM-row coloring the dialog's `atlas()`
//! uses), the slot/destination values are a REAL `ExAnimation` frame, the
//! highlighted 8x8-Select target is a real `SelectTarget`-style position, and
//! the Remap window's "Remapped N reference(s)" line is the REAL result of
//! running `smwe_rom::exanimation::remap_addresses` on the demo data.
//! Everything else (window chrome, buttons, labels) is a composed mock of
//! the egui layout — labeled as such in the caption.
//!
//! ```sh
//! cargo run --bin render_exanim_remap_select -- --out=docs/screenshots/exanim-remap-8x8select.png --rom=smw.smc
//! ```

use std::sync::Arc;

use ab_glyph::{Font, FontRef, Glyph, Point, PxScale, ScaleFont};
use image::{Rgb, RgbImage};
use smw_editor::render_util::read_color;
use smwe_emu::{emu::CheckedMem, rom::Rom as EmuRom, Cpu};
use smwe_rom::{
    exanimation::{remap_addresses, ExAnimFrame, ExAnimFrameKind, ExAnimTrigger, ExAnimation},
    graphics::gfx_file::Tile,
};

const SANS_CANDIDATES: &[&str] = &["/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf"];
const SANS_BOLD_CANDIDATES: &[&str] = &["/usr/share/fonts/truetype/dejavu/DejaVuSans-Bold.ttf"];

fn load_font(candidates: &[&str]) -> anyhow::Result<FontRef<'static>> {
    for p in candidates {
        if let Ok(data) = std::fs::read(p) {
            let leaked: &'static [u8] = Box::leak(data.into_boxed_slice());
            return FontRef::try_from_slice(leaked).map_err(|e| anyhow::anyhow!("{p}: {e}"));
        }
    }
    anyhow::bail!("no font file found; tried {candidates:?}")
}

fn draw_text(img: &mut RgbImage, font: &FontRef, text: &str, x: i32, y: i32, px: f32, color: Rgb<u8>) {
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
}

fn text_w(font: &FontRef, text: &str, px: f32) -> f32 {
    let scaled = font.as_scaled(PxScale::from(px));
    let mut w = 0.0f32;
    let mut prev = None;
    for ch in text.chars() {
        let id = font.glyph_id(ch);
        if let Some(p) = prev {
            w += scaled.kern(p, id);
        }
        w += scaled.h_advance(id);
        prev = Some(id);
    }
    w
}

fn fill(img: &mut RgbImage, x: u32, y: u32, w: u32, h: u32, c: Rgb<u8>) {
    for yy in y..(y + h).min(img.height()) {
        for xx in x..(x + w).min(img.width()) {
            img.put_pixel(xx, yy, c);
        }
    }
}

fn rect_outline(img: &mut RgbImage, x: u32, y: u32, w: u32, h: u32, t: u32, color: Rgb<u8>) {
    for d in 0..t {
        for xx in x..x + w {
            for &yy in &[y + d, y + h - 1 - d] {
                if xx < img.width() && yy < img.height() {
                    img.put_pixel(xx, yy, color);
                }
            }
        }
        for yy in y..y + h {
            for &xx in &[x + d, x + w - 1 - d] {
                if xx < img.width() && yy < img.height() {
                    img.put_pixel(xx, yy, color);
                }
            }
        }
    }
}

/// Mock button: flat fill, 1px border, centered label. Returns the rect.
fn button(img: &mut RgbImage, font: &FontRef, x: u32, y: u32, label: &str, selected: bool) -> (u32, u32, u32, u32) {
    let (bg, fg) =
        if selected { (Rgb([70, 110, 160]), Rgb([255, 255, 255])) } else { (Rgb([58, 58, 58]), Rgb([230, 230, 230])) };
    let w = (text_w(font, label, 15.0) as u32 + 24).max(64);
    let h = 28u32;
    fill(img, x, y, w, h, bg);
    rect_outline(img, x, y, w, h, 1, Rgb([120, 120, 120]));
    let tw = text_w(font, label, 15.0) as i32;
    draw_text(img, font, label, x as i32 + (w as i32 - tw) / 2, y as i32 + 6, 15.0, fg);
    (x, y, w, h)
}

fn field(img: &mut RgbImage, font: &FontRef, x: u32, y: u32, label: &str, value: &str) -> u32 {
    draw_text(img, font, label, x as i32, y as i32 + 5, 15.0, Rgb([200, 200, 200]));
    let lx = x + text_w(font, label, 15.0) as u32 + 10;
    let w = (text_w(font, value, 15.0) as u32 + 20).max(70);
    fill(img, lx, y, w, 26, Rgb([30, 30, 30]));
    rect_outline(img, lx, y, w, 26, 1, Rgb([110, 110, 110]));
    draw_text(img, font, value, lx as i32 + 8, y as i32 + 5, 15.0, Rgb([255, 200, 90]));
    lx + w
}

fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let output =
        args.iter().find_map(|a| a.strip_prefix("--out=")).unwrap_or("docs/screenshots/exanim-remap-8x8select.png");
    let rom_path = args.iter().find_map(|a| a.strip_prefix("--rom=")).unwrap_or("smw.smc");

    let sans = load_font(SANS_CANDIDATES)?;
    let bold = load_font(SANS_BOLD_CANDIDATES)?;

    // Real ROM + emulator state, exactly like the editor's level load: this
    // is the "clean post-load VRAM snapshot" the dialog's tile browser
    // decodes from.
    let raw = std::fs::read(rom_path)?;
    let rom_bytes = if raw.len() % 0x400 == 0x200 { raw[0x200..].to_vec() } else { raw };
    let mut emu_rom = EmuRom::new(rom_bytes);
    emu_rom.load_symbols(include_str!("../../symbols/SMW_U.sym"));
    let mut cpu = Cpu::new(CheckedMem::new(Arc::new(emu_rom)));
    smwe_emu::emu::decompress_sublevel(&mut cpu, 0x105);
    smwe_emu::emu::fetch_anim_frame(&mut cpu);
    let vram = cpu.mem.vram.clone();
    let cgram = cpu.mem.cgram.clone();

    // Most colorful CGRAM row for the atlas (the dialog defaults to row 0
    // and lets the user switch; we label the row we use).
    let mut best_row = 0usize;
    let mut best_n = 0usize;
    for row in 0..8usize {
        let n = (0..16usize)
            .filter(|&i| {
                let c = read_color(&cgram, row * 16 + i);
                c != [0, 0, 0]
            })
            .count();
        if n > best_n {
            best_n = n;
            best_row = row;
        }
    }

    // Real demo animation frame (the values the dialog would show).
    let frame = ExAnimFrame {
        kind:            ExAnimFrameKind::Line8x8,
        dest:            0x1000,
        speed:           4,
        trigger:         ExAnimTrigger::Always,
        frames:          2,
        units_per_frame: 4,
        payload:         vec![0x2000, 0x2010, 0x2000, 0x2020, 0x2030, 0x2040, 0x2050, 0x2060],
    };
    // The Remap status line is the REAL result of remap_addresses on a copy.
    let mut remap_demo = ExAnimation { frames: vec![frame.clone()], disable_original: false };
    let remapped = remap_addresses(&mut remap_demo, 0x2000, 0x4000, true, true);
    assert_eq!(remapped, 2);

    // The double-click reveal target: slot (step 0, tile 1) = $2010 →
    // VRAM tile 513 (col 1, row 8 of the 64-wide atlas).
    let reveal_tile = 0x2010usize / 16;

    // ── Canvas ──────────────────────────────────────────────────────────
    const W: u32 = 1500;
    const H: u32 = 800;
    let mut img = RgbImage::new(W, H);
    fill(&mut img, 0, 0, W, H, Rgb([24, 24, 24]));

    // Window chrome.
    fill(&mut img, 12, 12, W - 24, H - 24, Rgb([38, 38, 38]));
    rect_outline(&mut img, 12, 12, W - 24, H - 24, 1, Rgb([90, 90, 90]));
    fill(&mut img, 12, 12, W - 24, 34, Rgb([52, 52, 52]));
    draw_text(&mut img, &bold, "ExAnimated Frames", 26, 20, 17.0, Rgb([235, 235, 235]));

    // Tabs.
    draw_text(&mut img, &sans, "Level 105", 26, 58, 15.0, Rgb([255, 255, 255]));
    fill(&mut img, 20, 76, 92, 3, Rgb([90, 140, 200]));
    draw_text(&mut img, &sans, "Global (all levels)", 130, 58, 15.0, Rgb([150, 150, 150]));

    // Left: frame list.
    draw_text(&mut img, &bold, "Frames", 26, 92, 15.0, Rgb([220, 220, 220]));
    fill(&mut img, 20, 114, 260, 120, Rgb([30, 30, 30]));
    rect_outline(&mut img, 20, 114, 260, 120, 1, Rgb([80, 80, 80]));
    fill(&mut img, 22, 116, 256, 26, Rgb([70, 110, 160]));
    draw_text(&mut img, &sans, "#0 Line 8x8 @ $1000 · 2f", 28, 121, 14.0, Rgb([255, 255, 255]));
    draw_text(&mut img, &sans, "#1 Palette rotate @ $0020 · 4f", 28, 147, 14.0, Rgb([200, 200, 200]));
    button(&mut img, &sans, 20, 244, "+ Add", false);
    button(&mut img, &sans, 100, 244, "− Del", false);

    // Right: frame editor.
    let ex = 310u32;
    let mut y = 92u32;
    let mut x = field(&mut img, &sans, ex, y, "Type", "Line 8x8 ▾");
    field(&mut img, &sans, x + 18, y, "Trigger", "Always ▾");
    y += 40;
    x = field(&mut img, &sans, ex, y, "VRAM dest", "$1000");
    // The ◎ target button, shown because 8x8 Select is on.
    let (bx, by, bw, bh) = button(&mut img, &sans, x + 12, y - 1, "◎", true);
    let _ = (bx, by, bw, bh);
    x = field(&mut img, &sans, x + 96, y, "Speed (ticks/step)", "4");
    y += 40;
    x = field(&mut img, &sans, ex, y, "Steps", "2");
    field(&mut img, &sans, x + 18, y, "Tiles/step", "4");
    y += 44;

    draw_text(
        &mut img,
        &sans,
        "Source tiles per step (click a slot, then a tile below):",
        ex as i32,
        y as i32,
        15.0,
        Rgb([210, 210, 210]),
    );
    y += 28;
    // The two new LM v3.32 buttons.
    let (rx, ry, rw, _) = button(&mut img, &sans, ex, y, "Remap…", false);
    let (sx, _, sw, _) = button(&mut img, &sans, rx + rw + 12, y, "■ 8x8 Select", true);
    let _ = (ry, sx, sw);
    y += 36;
    draw_text(
        &mut img,
        &sans,
        "8x8 Select → filling step 0 tile 1; click a tile below (advances automatically),",
        ex as i32,
        y as i32,
        14.0,
        Rgb([140, 200, 255]),
    );
    y += 20;
    draw_text(&mut img, &sans, "or click any slot to retarget.", ex as i32, y as i32, 14.0, Rgb([140, 200, 255]));
    y += 28;

    // Slot rows — real values from the demo frame; the 8x8-Select target
    // (step 0, tile 1 = $2010) is highlighted.
    for step in 0..2usize {
        draw_text(&mut img, &sans, &format!("step {step:3}:"), ex as i32, y as i32 + 5, 14.0, Rgb([190, 190, 190]));
        let mut sx2 = ex + 76;
        for unit in 0..4usize {
            let v = frame.payload[step * 4 + unit];
            let targeted = step == 0 && unit == 1;
            let label = format!("${v:04X}");
            let w = (text_w(&sans, &label, 14.0) as u32 + 18).max(52);
            fill(&mut img, sx2, y, w, 24, if targeted { Rgb([70, 110, 160]) } else { Rgb([52, 52, 52]) });
            rect_outline(&mut img, sx2, y, w, 24, 1, Rgb([120, 120, 120]));
            draw_text(&mut img, &sans, &label, sx2 as i32 + 9, y as i32 + 5, 14.0, Rgb([235, 235, 235]));
            sx2 += w + 8;
        }
        y += 32;
    }
    y += 6;
    draw_text(
        &mut img,
        &sans,
        &format!("Tile browser (view VRAM)    palette row [{best_row}]"),
        ex as i32,
        y as i32,
        15.0,
        Rgb([210, 210, 210]),
    );
    y += 26;

    // Real VRAM atlas, 1x, exactly like the dialog's atlas() (64 cols).
    const ACOLS: usize = 64;
    let tile_count = vram.len() / 32;
    let rows = tile_count.div_ceil(ACOLS);
    let (aw, ah) = (ACOLS * 8, rows * 8);
    let mut pal = [[0u8; 3]; 16];
    for i in 0..16usize {
        pal[i] = read_color(&cgram, best_row * 16 + i);
    }
    for t in 0..tile_count {
        let bytes = &vram[t * 32..(t + 1) * 32];
        let Ok((_, tile)) = Tile::from_4bpp(bytes) else { continue };
        let (tx, ty) = ((t % ACOLS) * 8, (t / ACOLS) * 8);
        for (pi, &ci) in tile.color_indices.iter().enumerate() {
            let c = pal[(ci & 0xF) as usize];
            img.put_pixel(ex + tx as u32 + (pi % 8) as u32, y + ty as u32 + (pi / 8) as u32, Rgb(c));
        }
    }
    rect_outline(&mut img, ex, y, aw as u32, ah as u32, 1, Rgb([90, 90, 90]));
    // Double-click reveal flash on tile 513 (the $2010 slot's tile).
    let rcx = ex + (reveal_tile % ACOLS) as u32 * 8;
    let rcy = y + (reveal_tile / ACOLS) as u32 * 8;
    rect_outline(&mut img, rcx.saturating_sub(2), rcy.saturating_sub(2), 12, 12, 2, Rgb([255, 235, 59]));
    // Cursor arrow near the revealed tile.
    let (cx, cy) = (rcx + 18, rcy + 2);
    for i in 0..14u32 {
        img.put_pixel(cx + i / 2, cy + i, Rgb([255, 255, 255]));
        img.put_pixel(cx + i / 3, cy + i, Rgb([255, 255, 255]));
    }

    // Floating Remap window (mock of the egui window).
    let wx = 900u32;
    let wy = 380u32;
    let ww = 560u32;
    let wh = 300u32;
    fill(&mut img, wx, wy, ww, wh, Rgb([44, 44, 44]));
    rect_outline(&mut img, wx, wy, ww, wh, 1, Rgb([110, 110, 110]));
    fill(&mut img, wx, wy, ww, 32, Rgb([58, 58, 58]));
    draw_text(&mut img, &bold, "Remap ExAnimation tiles", wx as i32 + 14, wy as i32 + 8, 16.0, Rgb([235, 235, 235]));
    let mut wyy = wy + 44;
    draw_text(
        &mut img,
        &sans,
        "After moving tiles around in VRAM, re-point every frame",
        wx as i32 + 14,
        wyy as i32,
        14.0,
        Rgb([200, 200, 200]),
    );
    wyy += 20;
    draw_text(
        &mut img,
        &sans,
        "source tile and destination at the new addresses.",
        wx as i32 + 14,
        wyy as i32,
        14.0,
        Rgb([200, 200, 200]),
    );
    wyy += 32;
    let mut wxx = field(&mut img, &sans, wx + 14, wyy, "Old VRAM address", "$2000");
    field(&mut img, &sans, wxx + 16, wyy, "New VRAM address", "$4000");
    let _ = wxx;
    wyy += 40;
    draw_text(&mut img, &sans, "☑ Frame source tiles", wx as i32 + 14, wyy as i32, 15.0, Rgb([220, 220, 220]));
    wyy += 28;
    draw_text(&mut img, &sans, "☑ Destinations", wx as i32 + 14, wyy as i32, 15.0, Rgb([220, 220, 220]));
    wyy += 34;
    let (ax, _, aw2, _) = button(&mut img, &sans, wx + 14, wyy, "Apply remap", false);
    button(&mut img, &sans, ax + aw2 + 12, wyy, "Cancel", false);
    wyy += 38;
    draw_text(
        &mut img,
        &sans,
        &format!("Remapped {remapped} reference(s)."),
        wx as i32 + 14,
        wyy as i32,
        14.0,
        Rgb([140, 220, 140]),
    );

    // Caption (inside the window, below the atlas).
    draw_text(
        &mut img,
        &sans,
        "Composed mock of the egui dialog (egui can't render headless). Tile browser = real VRAM atlas from level 0x105;",
        20,
        (H - 52) as i32,
        13.0,
        Rgb([130, 130, 130]),
    );
    draw_text(
        &mut img,
        &sans,
        "slot values, reveal tile, and the Remap count are real computed values.",
        20,
        (H - 34) as i32,
        13.0,
        Rgb([130, 130, 130]),
    );

    img.save(output)?;
    println!("wrote {output} (atlas {tile_count} tiles, palette row {best_row}, remapped {remapped})");
    Ok(())
}
