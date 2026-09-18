//! Headless screenshot of the ExGFX Manager feature.
//!
//! egui can't render headless, so this composes an honest mock of the new
//! "ExGFX Manager" window: a real ExGFX file (index 0x80) is inserted into an
//! in-memory expanded copy of the real ROM via the real
//! `smwe_rom::exgfx::ExGfxData` insert path, a real Super GFX Bypass record
//! points level 0x105's FG1 slot at it, and the mock lists the real file
//! (real tile count, real "levels using" cross-reference). The tile strip at
//! the bottom renders the file's real first 16 tiles through the level's real
//! FG CGRAM palette row, exactly as the 8x8 tile editor colors them.
//!
//! ```sh
//! cargo run --bin render_exgfx_manager -- --out=docs/screenshots/exgfx-manager.png --rom=smw.smc
//! ```

use std::sync::Arc;

use ab_glyph::{Font, FontRef, Point, PxScale, ScaleFont};
use image::{Rgb, RgbImage};
use smwe_emu::{emu::CheckedMem, rom::Rom as EmuRom, Cpu};
use smwe_rom::{
    exgfx::{BypassData, ExGfxData, EXGFX_FILE_BYTES},
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

/// CGRAM palette row as RGB triples (copied verbatim from the 8x8 tile editor).
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

/// Render one 8x8 tile's color indices through a palette row into RGB.
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
    let output = args.iter().find_map(|a| a.strip_prefix("--out=")).unwrap_or("docs/screenshots/exgfx-manager.png");
    let rom_path = args
        .iter()
        .find_map(|a| a.strip_prefix("--rom="))
        .or_else(|| args.iter().skip(1).find(|a| !a.starts_with("--")).map(|a| a.as_str()))
        .unwrap_or("smw.smc");

    let sans = load_font(SANS_CANDIDATES)?;
    let sans_bold = load_font(SANS_BOLD_CANDIDATES)?;

    // Real ROM, expanded in memory (the real ROM is never modified).
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

    // Insert a real ExGFX file (deterministic pattern, not blank) and point
    // level 0x105's FG1 bypass slot at it — the real model paths.
    let raw: Vec<u8> = (0..EXGFX_FILE_BYTES).map(|i| (i & 0xFF) as u8).collect();
    let mut data = ExGfxData::parse(&scratch);
    data.insert_raw(0x80, raw).map_err(|e| anyhow::anyhow!("{e:?}"))?;
    data.write_to_rom(&mut scratch, header_offset).map_err(|e| anyhow::anyhow!("{e:?}"))?;
    let mut bypass = BypassData::parse(&scratch).unwrap_or_default();
    bypass.set_slot(0x105, 0, 0x80).map_err(|e| anyhow::anyhow!("{e:?}"))?;
    bypass.write_to_rom(&mut scratch, header_offset).map_err(|e| anyhow::anyhow!("{e:?}"))?;

    // Re-parse to prove the round-trip, like the editor does on load.
    let data = ExGfxData::parse(&scratch);
    let bypass = BypassData::parse(&scratch).map_err(|e| anyhow::anyhow!("{e:?}"))?;
    let file = data.files.get(&0x80).expect("ExGFX80 must parse back");
    assert_eq!(file.tiles.len(), 0x400, "a 32 KiB 4bpp file holds 1024 tiles");
    let levels_using: Vec<u16> =
        bypass.levels.iter().filter(|(_, slots)| slots.contains(&0x80)).map(|(&l, _)| l).collect();
    assert_eq!(levels_using, vec![0x105]);

    // Emulator CGRAM for the real FG palette row of level 0x105.
    let mut emu_rom = EmuRom::new(scratch[header_offset..].to_vec());
    emu_rom.load_symbols(include_str!("../../symbols/SMW_U.sym"));
    let mut cpu = Cpu::new(CheckedMem::new(Arc::new(emu_rom)));
    smwe_emu::emu::decompress_sublevel(&mut cpu, 0x105);
    let rom = smwe_rom::SmwRom::from_rom(
        smwe_rom::snes_utils::rom::Rom::new(scratch.clone()).map_err(|e| anyhow::anyhow!("{e:?}"))?,
    )
    .map_err(|e| anyhow::anyhow!("{e:?}"))?;
    let palette_fg = rom.levels[0x105].primary_header.palette_fg() as usize;
    let pal_fg = cgram_palette_row(&cpu.mem.cgram, palette_fg);

    // ── Compose the mock window ──────────────────────────────────────────
    const W: u32 = 680;
    const H: u32 = 560;
    let mut img = RgbImage::from_pixel(W, H, Rgb([30, 30, 38]));
    for y in 0..44u32 {
        for x in 0..W {
            img.put_pixel(x, y, Rgb([42, 42, 54]));
        }
    }
    draw_text(&mut img, &sans_bold, "ExGFX Manager", 16, 8, 22.0, Rgb([235, 235, 245]));
    draw_text(
        &mut img,
        &sans,
        "Extra graphics files (LM v1.10/v1.60). Inserted files live in ROM free space;",
        16,
        56,
        13.0,
        Rgb([200, 200, 210]),
    );
    draw_text(
        &mut img,
        &sans,
        "assign them to a level's FG/BG or sprite slots in Super GFX Bypass.",
        16,
        76,
        13.0,
        Rgb([200, 200, 210]),
    );

    let mut y = 108u32;
    // Column headers.
    draw_text(&mut img, &sans_bold, "File", 16, y as i32, 14.0, Rgb([160, 160, 175]));
    draw_text(&mut img, &sans_bold, "Tiles", 150, y as i32, 14.0, Rgb([160, 160, 175]));
    draw_text(&mut img, &sans_bold, "Levels using", 250, y as i32, 14.0, Rgb([160, 160, 175]));
    y += 30;
    // The real inserted file row.
    draw_text(&mut img, &sans, "ExGFX080", 16, y as i32, 15.0, Rgb([235, 220, 160]));
    draw_text(&mut img, &sans, &format!("{}", file.tiles.len()), 150, y as i32, 15.0, Rgb([200, 200, 210]));
    draw_text(&mut img, &sans, "105", 250, y as i32, 15.0, Rgb([200, 200, 210]));
    for (i, label) in ["Edit", "Extract", "Delete"].iter().enumerate() {
        let bx = 380 + i as u32 * 90;
        for yy in y..y + 24 {
            for xx in bx..bx + 78 {
                img.put_pixel(xx, yy, Rgb([58, 110, 180]));
            }
        }
        rect_outline(&mut img, bx, y, 78, 24, 1, Rgb([120, 170, 230]));
        draw_text(&mut img, &sans, label, bx as i32 + 10, y as i32 + 3, 13.0, Rgb([240, 245, 255]));
    }
    y += 44;

    draw_text(&mut img, &sans, "Insert ExGFX file…", 16, y as i32, 14.0, Rgb([200, 200, 210]));
    y += 30;
    draw_text(
        &mut img,
        &sans,
        "Insert 32768 bytes as ExGFX file index:  [ ExGFX080 ]   [ Insert ]  [ Cancel ]",
        16,
        y as i32,
        14.0,
        Rgb([200, 200, 210]),
    );
    y += 40;
    draw_text(
        &mut img,
        &sans,
        "In-game use needs Lunar Magic's ExGFX ASM hack — the editor authors and previews the data.",
        16,
        y as i32,
        12.0,
        Rgb([150, 150, 165]),
    );
    y += 34;

    // Real tiles of the inserted file, colored with the level's FG palette.
    draw_text(
        &mut img,
        &sans_bold,
        "ExGFX080 — first 16 tiles, level 0x105 FG palette row (same renderer as the 8x8 tile editor):",
        16,
        y as i32,
        13.0,
        Rgb([235, 235, 245]),
    );
    y += 28;
    let mut tile_rgb = [0u8; 8 * 8 * 3];
    for (i, tile) in file.tiles.iter().take(16).enumerate() {
        tile_rgb8(tile, &pal_fg, &mut tile_rgb);
        blit_rgb(&mut img, &tile_rgb, 16 + i as u32 * 40, y, 4);
    }

    img.save(output)?;
    println!("wrote {output} (ExGFX080: {} tiles, used by level 0x105 FG1)", file.tiles.len());
    Ok(())
}
