//! Headless screenshot of the "Change Layer 3 Settings" dialog.
//!
//! egui can't render headless, so this composes an honest mock of the new
//! dialog: the real setting descriptions from `smwe_rom::layer3`
//! (resolved from the vanilla Layer3TilemapSettings/Layer3Ptr tables for
//! level 0x127's tileset), the real WYSIWYG stripe preview rendered from the
//! vanilla Tide stripe image (`$059549`) through the emulator's VRAM/CGRAM,
//! and the per-level GFX bypass control.
//!
//! ```sh
//! cargo run --bin render_layer3 -- --out=docs/screenshots/layer3-settings.png --rom=smw.smc
//! ```

use std::sync::Arc;

use ab_glyph::{Font, FontRef, Glyph, Point, PxScale, ScaleFont};
use image::{Rgb, RgbImage};
use smwe_emu::{emu::CheckedMem, rom::Rom as EmuRom, Cpu};
use smwe_rom::layer3;

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
                    let (px_x, py_y) = (px_x as u32, px_y as u32);
                    if px_x < img.width() && py_y < img.height() {
                        let bg = img.get_pixel(px_x, py_y);
                        let a = (v * 255.0) as u8;
                        let r = ((color[0] as u16 * a as u16 + bg[0] as u16 * (255 - a as u16)) / 255) as u8;
                        let g = ((color[1] as u16 * a as u16 + bg[1] as u16 * (255 - a as u16)) / 255) as u8;
                        let b = ((color[2] as u16 * a as u16 + bg[2] as u16 * (255 - a as u16)) / 255) as u8;
                        img.put_pixel(px_x, py_y, Rgb([r, g, b]));
                    }
                }
            });
        }
        caret_x += scaled.h_advance(id);
        prev = Some(id);
    }
}

fn fill_rect(img: &mut RgbImage, x: u32, y: u32, w: u32, h: u32, c: Rgb<u8>) {
    for yy in y..(y + h).min(img.height()) {
        for xx in x..(x + w).min(img.width()) {
            img.put_pixel(xx, yy, c);
        }
    }
}

fn main() -> anyhow::Result<()> {
    let mut out = String::from("docs/screenshots/layer3-settings.png");
    let mut rom_path = String::from("smw.smc");
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--out" => out = args.next().unwrap(),
            "--rom" => rom_path = args.next().unwrap(),
            _ => {}
        }
    }

    let font = load_font(SANS_CANDIDATES)?;
    let bold = load_font(SANS_BOLD_CANDIDATES)?;
    let rom_bytes_raw = std::fs::read(&rom_path)?;
    let rom_bytes = if rom_bytes_raw.len() % 0x400 == 0x200 { rom_bytes_raw[0x200..].to_vec() } else { rom_bytes_raw };

    // Level 0x127: tileset 0, Layer 3 setting 1 (rising tide).
    let level = 0x127u16;
    let rom = smwe_rom::SmwRom::from_file(&rom_path)?;
    let lvl = &rom.levels[level as usize];
    let tileset = lvl.primary_header.fg_bg_gfx();
    let setting = lvl.secondary_header.layer3();

    // Emulator for VRAM/CGRAM.
    let mut emu_rom = EmuRom::new(rom_bytes.clone());
    emu_rom.load_symbols(include_str!("../../symbols/SMW_U.sym"));
    let mut cpu = Cpu::new(CheckedMem::new(Arc::new(emu_rom)));
    smwe_emu::emu::decompress_sublevel(&mut cpu, level);
    let vram = cpu.mem.vram.clone();
    let cgram = cpu.mem.cgram.clone();

    // Resolve and render the stripe preview.
    let info = layer3::resolve_layer3(&rom_bytes, tileset, setting).unwrap();
    let grid = layer3::layer3_tilemap(&rom_bytes, info.stripe_snes).unwrap();
    // Render to RGB using the same logic as the dialog (4bpp, CGRAM).
    let (y0, y1) = {
        let mut min = 64usize;
        let mut max = 0usize;
        for (y, row) in grid.iter().enumerate() {
            if row.iter().any(|&w| w != layer3::LAYER3_EMPTY_TILE_WORD) {
                min = min.min(y);
                max = max.max(y);
            }
        }
        (min.saturating_sub(1), (max + 2).min(64))
    };
    let pw = 64 * 8;
    let ph = (y1 - y0) * 8;
    let mut preview = RgbImage::new(pw as u32, ph as u32);
    // Checkerboard for transparency.
    for y in 0..ph {
        for x in 0..pw {
            let c = if (x / 8 + y / 8) % 2 == 0 { 40 } else { 55 };
            preview.put_pixel(x as u32, y as u32, Rgb([c, c, c]));
        }
    }
    let cword = |i: usize| -> u16 {
        let o = 2 * i;
        u16::from_le_bytes([cgram[o], cgram[o + 1]])
    };
    for (ry, row) in grid.iter().enumerate().skip(y0).take(y1 - y0) {
        for (x, &word) in row.iter().enumerate() {
            if word == layer3::LAYER3_EMPTY_TILE_WORD {
                continue;
            }
            let tile = (word & 0x3FF) as usize;
            let pal = ((word >> 10) & 7) as usize;
            for py in 0..8 {
                for px in 0..8 {
                    let bit = 7 - px;
                    let base = tile * 32;
                    let b0 = (vram[base + py * 2] >> bit) & 1;
                    let b1 = (vram[base + py * 2 + 1] >> bit) & 1;
                    let b2 = (vram[base + 16 + py * 2] >> bit) & 1;
                    let b3 = (vram[base + 16 + py * 2 + 1] >> bit) & 1;
                    let idx = (b0 | (b1 << 1) | (b2 << 2) | (b3 << 3)) as usize;
                    if idx == 0 {
                        continue;
                    }
                    let cw = cword(pal * 16 + idx);
                    let r = (((cw & 0x1F) * 255 + 15) / 31) as u8;
                    let g = ((((cw >> 5) & 0x1F) * 255 + 15) / 31) as u8;
                    let b = ((((cw >> 10) & 0x1F) * 255 + 15) / 31) as u8;
                    preview.put_pixel((x * 8 + px) as u32, ((ry - y0) * 8 + py) as u32, Rgb([r, g, b]));
                }
            }
        }
    }

    // Compose the dialog mock.
    let w = 700u32;
    let h = 620u32;
    let mut img = RgbImage::new(w, h);
    fill_rect(&mut img, 0, 0, w, h, Rgb([32, 32, 36])); // window bg
    fill_rect(&mut img, 0, 0, w, 28, Rgb([48, 48, 54])); // title bar
    draw_text(&mut img, &bold, "Change Layer 3 Settings", 12, 6, 14.0, Rgb([230, 230, 230]));

    let mut y = 44;
    let tx = Rgb([210, 210, 210]);
    draw_text(&mut img, &font, &format!("Object tileset: {tileset}  (level {level:#05X})"), 12, y, 13.0, tx);
    y += 24;
    draw_text(&mut img, &bold, "Layer 3 setting (secondary header bits 7-6):", 12, y, 13.0, tx);
    y += 20;
    for s in 0..=3u8 {
        let desc = layer3::describe_layer3_setting(&rom_bytes, tileset, s);
        let sel = s == setting;
        // Radio circle
        let cx = 20;
        let cy = y + 7;
        for dy in -6..=6 {
            for dx in -6..=6 {
                if dx * dx + dy * dy <= 36 {
                    img.put_pixel((cx + dx) as u32, (cy + dy) as u32, Rgb([180, 180, 180]));
                }
            }
        }
        if sel {
            for dy in -3..=3 {
                for dx in -3..=3 {
                    if dx * dx + dy * dy <= 9 {
                        img.put_pixel((cx + dx) as u32, (cy + dy) as u32, Rgb([80, 160, 255]));
                    }
                }
            }
        }
        draw_text(&mut img, &font, &desc, 32, y, 12.0, if sel { Rgb([255, 255, 255]) } else { Rgb([160, 160, 160]) });
        y += 20;
    }
    y += 8;
    draw_text(&mut img, &bold, "WYSIWYG preview (vanilla stripe image):", 12, y, 13.0, tx);
    y += 20;
    // Scale preview to fit width 512 max.
    let scale = (512.0 / pw as f32).min(1.0);
    let dw = (pw as f32 * scale) as u32;
    let dh = (ph as f32 * scale) as u32;
    for dy in 0..dh {
        for dx in 0..dw {
            let sx = (dx as f32 / scale) as u32;
            let sy = (dy as f32 / scale) as u32;
            img.put_pixel(12 + dx, (y as u32) + dy, *preview.get_pixel(sx.min(pw as u32 - 1), sy.min(ph as u32 - 1)));
        }
    }
    y += dh as i32 + 12;
    draw_text(&mut img, &bold, "Layer 3 GFX bypass (per-level override):", 12, y, 13.0, tx);
    y += 20;
    // ComboBox mock
    fill_rect(&mut img, 12, y as u32, 280, 24, Rgb([48, 48, 54]));
    draw_text(&mut img, &font, "None (level's own GFX)", 18, y + 4, 12.0, tx);
    y += 32;
    let note = "The bypass changes the WYSIWYG preview. Applying it in-game on a vanilla ROM needs Lunar Magic's";
    let note2 = "closed-source \"Layer 3 GFX and tilemap bypass\" ASM hack; vanilla SMW always renders Layer 3";
    let note3 = "from the level's own GFX files.";
    draw_text(&mut img, &font, note, 12, y, 11.0, Rgb([160, 160, 160]));
    y += 16;
    draw_text(&mut img, &font, note2, 12, y, 11.0, Rgb([160, 160, 160]));
    y += 16;
    draw_text(&mut img, &font, note3, 12, y, 11.0, Rgb([160, 160, 160]));

    // Crop to content.
    let h2 = (y + 24).min(h as i32) as u32;
    let cropped = image::imageops::crop_imm(&img, 0, 0, w, h2).to_image();
    cropped.save(&out)?;
    println!("wrote {out} ({w}x{h2})");
    Ok(())
}
