//! Headless screenshot of the Super GFX Bypass feature.
//!
//! egui can't render headless, so this composes an honest mock of the new
//! "Super GFX Bypass" window: a real bypass record is installed into an
//! in-memory expanded copy of the real ROM via the real
//! `smwe_rom::exgfx::BypassData` path (FG1 -> ExGFX80, SP2 -> vanilla GFX0D,
//! everything else default), re-parsed, and each of the eight slot rows shows
//! the real stored value resolved through the real `slot_source_label`
//! helper. The tile strip at the bottom renders the real first tiles of the
//! FG1 slot's effective file (ExGFX80, palette-colored with the level's FG
//! row) next to what the slot would show by default — i.e. exactly what the
//! bypass Apply path uploads to VRAM.
//!
//! ```sh
//! cargo run --bin render_gfx_bypass -- --out=docs/screenshots/gfx-bypass.png --rom=smw.smc
//! ```

use std::sync::Arc;

use ab_glyph::{Font, FontRef, Point, PxScale, ScaleFont};
use image::{Rgb, RgbImage};
use smwe_emu::{emu::CheckedMem, rom::Rom as EmuRom, Cpu};
use smwe_rom::{
    exgfx::{slot_source_label, BypassData, ExGfxData, BYPASS_DEFAULT, BYPASS_SLOT_NAMES, EXGFX_FILE_BYTES},
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
        prev = Some(id);
        let glyph = id.with_scale_and_position(px, Point { x: caret_x, y: baseline });
        if let Some(out) = scaled.outline_glyph(glyph) {
            let bb = out.px_bounds();
            out.draw(|gx, gy, v| {
                let px_x = bb.min.x as i32 + gx as i32;
                let px_y = bb.min.y as i32 + gy as i32;
                if px_x >= 0 && px_y >= 0 && (px_x as u32) < img.width() && (px_y as u32) < img.height() {
                    let dst = img.get_pixel(px_x as u32, px_y as u32);
                    let a = v;
                    let r = (color[0] as f32 * a + dst[0] as f32 * (1.0 - a)) as u8;
                    let g = (color[1] as f32 * a + dst[1] as f32 * (1.0 - a)) as u8;
                    let b = (color[2] as f32 * a + dst[2] as f32 * (1.0 - a)) as u8;
                    img.put_pixel(px_x as u32, px_y as u32, Rgb([r, g, b]));
                }
            });
        }
        caret_x += scaled.h_advance(id);
    }
}

fn rect_outline(img: &mut RgbImage, x: u32, y: u32, w: u32, h: u32, t: u32, color: Rgb<u8>) {
    for yy in y..y + t {
        for xx in x..x + w {
            img.put_pixel(xx, yy, color);
        }
    }
    for yy in y + h - t..y + h {
        for xx in x..x + w {
            img.put_pixel(xx, yy, color);
        }
    }
    for yy in y..y + h {
        for xx in x..x + t {
            img.put_pixel(xx, yy, color);
        }
        for xx in x + w - t..x + w {
            img.put_pixel(xx, yy, color);
        }
    }
}

fn cgram_palette_row(cgram: &[u8], row: usize) -> [[u8; 3]; 16] {
    let mut out = [[0u8; 3]; 16];
    for i in 0..16 {
        let off = row * 32 + i * 2;
        let (lo, hi) = (cgram[off] as u16, cgram[off + 1] as u16);
        let w = lo | (hi << 8);
        let r5 = (w & 0x1F) as u8;
        let g5 = ((w >> 5) & 0x1F) as u8;
        let b5 = ((w >> 10) & 0x1F) as u8;
        out[i] = [(r5 << 3) | (r5 >> 2), (g5 << 3) | (g5 >> 2), (b5 << 3) | (b5 >> 2)];
    }
    out
}

fn tile_rgb8(tile: &Tile, pal: &[[u8; 3]; 16], out: &mut [u8; 8 * 8 * 3]) {
    for (i, &ci) in tile.color_indices.iter().take(64).enumerate() {
        let rgb = pal[(ci as usize).min(15)];
        out[i * 3] = rgb[0];
        out[i * 3 + 1] = rgb[1];
        out[i * 3 + 2] = rgb[2];
    }
}

fn blit_rgb(img: &mut RgbImage, rgb: &[u8; 8 * 8 * 3], x: u32, y: u32, scale: u32) {
    for ty in 0..8u32 {
        for tx in 0..8u32 {
            let i = (ty * 8 + tx) as usize * 3;
            let c = Rgb([rgb[i], rgb[i + 1], rgb[i + 2]]);
            for dy in 0..scale {
                for dx in 0..scale {
                    img.put_pixel(x + tx * scale + dx, y + ty * scale + dy, c);
                }
            }
        }
    }
}

fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let output = args.iter().find_map(|a| a.strip_prefix("--out=")).unwrap_or("docs/screenshots/gfx-bypass.png");
    let rom_path = args
        .iter()
        .find_map(|a| a.strip_prefix("--rom="))
        .or_else(|| args.iter().skip(1).find(|a| !a.starts_with("--")).map(|a| a.as_str()))
        .unwrap_or("smw.smc");

    let sans = load_font(SANS_CANDIDATES)?;
    let sans_bold = load_font(SANS_BOLD_CANDIDATES)?;

    let rom_bytes = std::fs::read(rom_path)?;
    let header_offset = if rom_bytes.len() % 0x400 == 0x200 { 0x200 } else { 0 };
    let (smc_header, body) = rom_bytes.split_at(header_offset);
    let expanded = smwe_rom::rom_expansion::expand_rom(
        &smwe_rom::snes_utils::rom::Rom::new(body.to_vec()).map_err(|e| anyhow::anyhow!("{e:?}"))?,
        0x40_0000,
    )
    .map_err(|e| anyhow::anyhow!("{e:?}"))?;
    let mut scratch = smc_header.to_vec();
    scratch.extend_from_slice(expanded.bytes());

    // Real model: insert ExGFX80, then bypass FG1 -> ExGFX80, SP2 -> GFX0D.
    let raw: Vec<u8> = (0..EXGFX_FILE_BYTES).map(|i| (i & 0xFF) as u8).collect();
    let mut data = ExGfxData::parse(&scratch);
    data.insert_raw(0x80, raw).map_err(|e| anyhow::anyhow!("{e:?}"))?;
    data.write_to_rom(&mut scratch, header_offset).map_err(|e| anyhow::anyhow!("{e:?}"))?;
    let mut bypass = BypassData::parse(&scratch).unwrap_or_default();
    bypass.set_slot(0x105, 0, 0x80).map_err(|e| anyhow::anyhow!("{e:?}"))?;
    bypass.set_slot(0x105, 5, 0x0D).map_err(|e| anyhow::anyhow!("{e:?}"))?;
    bypass.write_to_rom(&mut scratch, header_offset).map_err(|e| anyhow::anyhow!("{e:?}"))?;

    let data = ExGfxData::parse(&scratch);
    let bypass = BypassData::parse(&scratch).map_err(|e| anyhow::anyhow!("{e:?}"))?;

    // Emulator CGRAM + the level's real tileset files for the "default" side.
    let mut emu_rom = EmuRom::new(scratch[header_offset..].to_vec());
    emu_rom.load_symbols(include_str!("../../symbols/SMW_U.sym"));
    let mut cpu = Cpu::new(CheckedMem::new(Arc::new(emu_rom)));
    smwe_emu::emu::decompress_sublevel(&mut cpu, 0x105);
    let rom = smwe_rom::SmwRom::from_rom(
        smwe_rom::snes_utils::rom::Rom::new(scratch.clone()).map_err(|e| anyhow::anyhow!("{e:?}"))?,
    )
    .map_err(|e| anyhow::anyhow!("{e:?}"))?;
    let level = &rom.levels[0x105];
    let palette_fg = level.primary_header.palette_fg() as usize;
    let pal_fg = cgram_palette_row(&cpu.mem.cgram, palette_fg);
    let fg_tileset = level.primary_header.fg_bg_gfx() as usize;
    let default_fg1 = rom.gfx.object_gfx_list.files_for_object_tileset(fg_tileset)[0];

    // ── Compose the mock window ──────────────────────────────────────────
    const W: u32 = 680;
    const H: u32 = 640;
    let mut img = RgbImage::from_pixel(W, H, Rgb([30, 30, 38]));
    for y in 0..44u32 {
        for x in 0..W {
            img.put_pixel(x, y, Rgb([42, 42, 54]));
        }
    }
    draw_text(&mut img, &sans_bold, "Super GFX Bypass", 16, 8, 22.0, Rgb([235, 235, 245]));
    draw_text(
        &mut img,
        &sans,
        "Per-level FG/BG and sprite GFX slot assignment (level 0x105).",
        16,
        56,
        13.0,
        Rgb([200, 200, 210]),
    );

    let mut y = 100u32;
    for slot in 0..8usize {
        let value = bypass.slot(0x105, slot).unwrap_or(BYPASS_DEFAULT);
        let label = slot_source_label(value);
        let name = BYPASS_SLOT_NAMES[slot];
        let name_color = if slot < 4 { Rgb([235, 220, 160]) } else { Rgb([160, 220, 235]) };
        draw_text(&mut img, &sans_bold, name, 28, y as i32, 15.0, name_color);
        // Mock combo box showing the real stored value.
        for yy in y..y + 26 {
            for xx in 120..400u32 {
                img.put_pixel(xx, yy, Rgb([52, 52, 64]));
            }
        }
        rect_outline(&mut img, 120, y, 280, 26, 1, Rgb([110, 110, 130]));
        let value_color = if value == BYPASS_DEFAULT { Rgb([160, 160, 175]) } else { Rgb([255, 220, 120]) };
        draw_text(&mut img, &sans, &label, 130, y as i32 + 3, 14.0, value_color);
        draw_text(&mut img, &sans, "▾", 378, y as i32 + 3, 14.0, Rgb([160, 160, 175]));
        y += 36;
    }

    for (i, label) in ["Apply", "Reset to defaults"].iter().enumerate() {
        let bx = 28 + i as u32 * 130;
        for yy in y..y + 28 {
            for xx in bx..bx + 118 {
                img.put_pixel(xx, yy, Rgb([58, 110, 180]));
            }
        }
        rect_outline(&mut img, bx, y, 118, 28, 1, Rgb([120, 170, 230]));
        draw_text(&mut img, &sans, label, bx as i32 + 12, y as i32 + 5, 14.0, Rgb([240, 245, 255]));
    }
    y += 48;
    draw_text(
        &mut img,
        &sans,
        "Vanilla slots upload through the game's real UploadGFXFile routine;",
        16,
        y as i32,
        12.0,
        Rgb([150, 150, 165]),
    );
    y += 20;
    draw_text(&mut img, &sans, "ExGFX slots copy 4bpp data directly.", 16, y as i32, 12.0, Rgb([150, 150, 165]));
    y += 34;

    // FG1: effective (ExGFX80) vs default (vanilla file) tiles, real data.
    draw_text(
        &mut img,
        &sans_bold,
        "FG1 slot — bypassed to ExGFX080 (left) vs tileset default (right), FG palette row:",
        16,
        y as i32,
        13.0,
        Rgb([235, 235, 245]),
    );
    y += 28;
    let mut tile_rgb = [0u8; 8 * 8 * 3];
    let ex_tiles = &data.files.get(&0x80).expect("ExGFX80").tiles;
    for (i, tile) in ex_tiles.iter().take(8).enumerate() {
        tile_rgb8(tile, &pal_fg, &mut tile_rgb);
        blit_rgb(&mut img, &tile_rgb, 16 + i as u32 * 40, y, 4);
    }
    let def_tiles = &rom.gfx.files[default_fg1].tiles;
    for (i, tile) in def_tiles.iter().take(8).enumerate() {
        tile_rgb8(tile, &pal_fg, &mut tile_rgb);
        blit_rgb(&mut img, &tile_rgb, 360 + i as u32 * 40, y, 4);
    }
    draw_text(&mut img, &sans, "ExGFX080", 16, y as i32 + 40, 12.0, Rgb([255, 220, 120]));
    draw_text(
        &mut img,
        &sans,
        &format!("default GFX{default_fg1:02X}"),
        360,
        y as i32 + 40,
        12.0,
        Rgb([160, 160, 175]),
    );

    img.save(output)?;
    println!("wrote {output} (level 0x105 FG1 -> ExGFX080, SP2 -> GFX0D, rest default)");
    Ok(())
}
