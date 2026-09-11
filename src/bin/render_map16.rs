//! Headless screenshot of the Map16 page import/export feature.
//!
//! egui can't render headless, so this composes an honest mock of the Map16
//! block editor's new import/export section: the real page/tileset selectors
//! and Export/Import buttons the PR adds, plus a status-bar line showing the
//! real result string the export path produces. Both tile atlases are real
//! program output — the left atlas renders the 256 tiles of FG page 0
//! (tileset 0) exported from the vanilla ROM with the exact
//! `smwe_rom::map16_file::export_page` function the UI calls, rasterized
//! through the same emulator VRAM/CGRAM path the editor's own tile picker
//! uses (`render_sub_tile`, copied verbatim below). The right strip
//! re-renders after a real import round trip into a scratch ROM copy (tiles
//! 0x30-0x3F palette-shifted before import), visibly proving the import
//! wrote the ROM: the re-export is byte-identical to the modified page.
//!
//! ```sh
//! cargo run --bin render_map16 -- --out=docs/screenshots/map16-import-export.png --rom=smw.smc
//! ```

use std::sync::Arc;

use ab_glyph::{Font, FontRef, Glyph, Point, PxScale, ScaleFont};
use image::{Rgb, RgbImage};
use smwe_emu::{emu::CheckedMem, rom::Rom as EmuRom, Cpu};
use smwe_rom::map16_file::{self, MAP16_PAGE_TILES};

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

fn fill_rect(img: &mut RgbImage, x: u32, y: u32, w: u32, h: u32, c: Rgb<u8>) {
    for yy in y..(y + h).min(img.height()) {
        for xx in x..(x + w).min(img.width()) {
            img.put_pixel(xx, yy, c);
        }
    }
}

fn rect_border(img: &mut RgbImage, x: u32, y: u32, w: u32, h: u32, c: Rgb<u8>) {
    fill_rect(img, x, y, w, 1, c);
    fill_rect(img, x, y + h - 1, w, 1, c);
    fill_rect(img, x, y, 1, h, c);
    fill_rect(img, x + w - 1, y, 1, h, c);
}

fn draw_text(
    img: &mut RgbImage,
    font: &FontRef,
    text: &str,
    x: i32,
    y: i32,
    px: f32,
    color: Rgb<u8>,
) {
    let scaled = font.as_scaled(PxScale::from(px));
    let mut caret_x = x as f32;
    let baseline = y as f32 + scaled.ascent();
    let mut prev = None;
    for ch in text.chars() {
        let id = font.glyph_id(ch);
        if let Some(p) = prev {
            caret_x += scaled.kern(p, id);
        }
        let glyph = Glyph {
            id,
            scale: PxScale::from(px),
            position: Point { x: caret_x, y: baseline },
        };
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

/// Verbatim copy of the editor tile picker's sub-tile renderer
/// (`src/ui/editor_prototypes/level_editor/tile_picker.rs::render_sub_tile`):
/// decodes one 8x8 4bpp sub-tile from emulator VRAM with the CGRAM palette
/// the tile word selects, honoring flip bits. Transparent pixels are left
/// alone so the caller can pre-fill a background.
fn render_sub_tile(vram: &[u8], cgram: &[u8], t: u16, x0: u32, y0: u32, pixels: &mut [u8], stride: usize) {
    let tile_num = (t & 0x3FF) as usize;
    let pal = ((t >> 10) & 0x7) as usize;
    let flip_x = (t & 0x4000) != 0;
    let flip_y = (t & 0x8000) != 0;

    let tile_base = tile_num * 32;
    for ty in 0..8u32 {
        for tx in 0..8u32 {
            let px = if flip_x { 7 - tx } else { tx };
            let py = if flip_y { 7 - ty } else { ty };
            let row_off = tile_base + (py as usize) * 2;
            if row_off + 17 >= vram.len() {
                continue;
            }
            let b0 = vram[row_off];
            let b1 = vram[row_off + 1];
            let b2 = vram[row_off + 16];
            let b3 = vram[row_off + 17];
            let bit = 7 - px as usize;
            let color_idx =
                (((b0 >> bit) & 1) | (((b1 >> bit) & 1) << 1) | (((b2 >> bit) & 1) << 2) | (((b3 >> bit) & 1) << 3))
                    as usize;

            if color_idx == 0 {
                continue;
            }

            let pal_idx = pal * 16 + color_idx;
            let off_color = pal_idx * 2;
            if off_color + 1 >= cgram.len() {
                continue;
            }
            let lo = cgram[off_color] as u16;
            let hi = cgram[off_color + 1] as u16;
            let rgb = lo | (hi << 8);

            let r = ((rgb & 0x1F) << 3) as u8;
            let g = (((rgb >> 5) & 0x1F) << 3) as u8;
            let b = (((rgb >> 10) & 0x1F) << 3) as u8;

            let px_abs = x0 + tx;
            let py_abs = y0 + ty;
            let off = ((py_abs as usize) * stride + px_abs as usize) * 4;
            if off + 3 < pixels.len() {
                pixels[off] = r;
                pixels[off + 1] = g;
                pixels[off + 2] = b;
                pixels[off + 3] = 255;
            }
        }
    }
}

/// Render one 16x16 Map16 block (four tile words) into `pixels` at `scale`
/// with a checkerboard behind transparent pixels.
fn render_block(
    vram: &[u8],
    cgram: &[u8],
    words: &[u16; 4],
    x0: u32,
    y0: u32,
    scale: u32,
    pixels: &mut [u8],
    stride: u32,
) {
    // Checkerboard background for transparency.
    for y in 0..16 * scale {
        for x in 0..16 * scale {
            let checker = ((x / 4 + y / 4) % 2) == 0;
            let shade = if checker { 52u8 } else { 84u8 };
            let off = (((y0 + y) * stride + (x0 + x)) * 4) as usize;
            pixels[off] = shade;
            pixels[off + 1] = shade;
            pixels[off + 2] = shade;
            pixels[off + 3] = 255;
        }
    }
    // Render at 1x into a temp buffer, then upscale.
    let mut small = vec![0u8; 16 * 16 * 4];
    let quads = [(0u32, 0u32), (0, 8), (8, 0), (8, 8)];
    for (i, &(qx, qy)) in quads.iter().enumerate() {
        render_sub_tile(vram, cgram, words[i], qx, qy, &mut small, 16);
    }
    for sy in 0..16u32 {
        for sx in 0..16u32 {
            let s = ((sy * 16 + sx) * 4) as usize;
            // Skip fully-transparent temp pixels (checkerboard shows through).
            if small[s + 3] == 0 {
                continue;
            }
            for dy in 0..scale {
                for dx in 0..scale {
                    let d = (((y0 + sy * scale + dy) * stride + (x0 + sx * scale + dx)) * 4) as usize;
                    pixels[d..d + 4].copy_from_slice(&small[s..s + 4]);
                }
            }
        }
    }
}

fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let output = args
        .iter()
        .find_map(|a| a.strip_prefix("--out="))
        .unwrap_or("docs/screenshots/map16-import-export.png");
    let rom_path = args
        .iter()
        .find_map(|a| a.strip_prefix("--rom="))
        .or_else(|| args.iter().skip(1).find(|a| !a.starts_with("--")).map(|a| a.as_str()))
        .unwrap_or("smw.smc");

    let sans = load_font(SANS_CANDIDATES)?;
    let sans_bold = load_font(SANS_BOLD_CANDIDATES)?;

    // Emulator VRAM/CGRAM for level 0x105: the same source the editor's
    // tile picker renders from.
    let rom_bytes = std::fs::read(rom_path)?;
    let mut emu_rom = EmuRom::new(rom_bytes.clone());
    emu_rom.load_symbols(include_str!("../../symbols/SMW_U.sym"));
    let mut cpu = Cpu::new(CheckedMem::new(Arc::new(emu_rom)));
    smwe_emu::emu::decompress_sublevel(&mut cpu, 0x105);
    let vram = cpu.mem.vram.clone();
    let cgram = cpu.mem.cgram.clone();

    // Real data path 1: export FG page 0 (tileset 0) from the vanilla ROM.
    let rom = smwe_rom::SmwRom::from_file(rom_path)?;
    let tileset = 0usize;
    let page_data = map16_file::export_page(&rom, map16_file::PAGE_FG0, tileset)?;
    let blocks = map16_file::parse_page(&page_data)?;
    assert_eq!(blocks.len(), MAP16_PAGE_TILES);
    let status_export = format!(
        "Exported FG page 0 (tiles 000-0FF, tileset {tileset}) → map16-page00-ts{tileset}.bin ({} bytes)",
        page_data.len()
    );

    // Real data path 2: palette-shift tiles 0x30-0x3F, import into a scratch
    // ROM copy, re-export — the import round trip the tests assert.
    let mut modified = page_data.clone();
    for t in 0x30..0x40usize {
        for w in 0..4usize {
            let off = t * 8 + w * 2;
            let word = u16::from_le_bytes([modified[off], modified[off + 1]]);
            let word2 = (word & !(7 << 10)) | ((((word >> 10) + 2) % 8) << 10);
            modified[off..off + 2].copy_from_slice(&word2.to_le_bytes());
        }
    }
    let mut scratch = rom_bytes.clone();
    map16_file::import_page(&mut scratch, map16_file::PAGE_FG0, tileset, &modified, 0)?;
    let rom2 = smwe_rom::SmwRom::from_rom(smwe_rom::snes_utils::rom::Rom::new(scratch)?)?;
    let page_data2 = map16_file::export_page(&rom2, map16_file::PAGE_FG0, tileset)?;
    assert_eq!(page_data2, modified, "import round trip must be byte-identical");
    let blocks2 = map16_file::parse_page(&page_data2)?;
    let status_import =
        "Imported modified page → scratch ROM → re-export byte-identical ✓".to_string();

    // Render the full page atlas: 16×16 grid of 16x16 blocks at 2x.
    const SCALE: u32 = 2;
    let atlas_px = 16 * 16 * SCALE;
    let mut atlas: Vec<u8> = vec![0u8; (atlas_px * atlas_px * 4) as usize];
    for (i, block) in blocks.iter().enumerate() {
        let words =
            [block.upper_left.0, block.lower_left.0, block.upper_right.0, block.lower_right.0];
        render_block(
            &vram,
            &cgram,
            &words,
            (i % 16) as u32 * 16 * SCALE,
            (i / 16) as u32 * 16 * SCALE,
            SCALE,
            &mut atlas,
            atlas_px,
        );
    }

    // Compose the screenshot.
    let (w, h) = (1180u32, 900u32);
    let mut img = RgbImage::new(w, h);
    let bg = Rgb([0xF2, 0xF2, 0xF2]);
    let ink = Rgb([0x1A, 0x1A, 0x1A]);
    let gray = Rgb([0x66, 0x66, 0x66]);
    let dark = Rgb([0x2B, 0x2B, 0x2B]);
    let green = Rgb([0x1E, 0x7A, 0x1E]);
    for p in img.pixels_mut() {
        *p = bg;
    }
    fill_rect(&mut img, 0, 0, w, 52, dark);
    draw_text(
        &mut img,
        &sans_bold,
        "Map16 Block Editor — page import/export (headless mock; atlas is real ROM output)",
        24,
        15,
        18.0,
        Rgb([0xFF, 0xFF, 0xFF]),
    );

    // Left: control mock.
    let lx = 24u32;
    let mut y = 84u32;
    draw_text(&mut img, &sans_bold, "Page import/export", lx as i32, y as i32, 17.0, ink);
    y += 30;
    draw_text(
        &mut img,
        &sans,
        "Raw 0x800-byte page files are Lunar Magic Map16Page.bin compatible.",
        lx as i32,
        y as i32,
        13.0,
        gray,
    );
    y += 34;
    draw_text(&mut img, &sans, "Page: [FG page 0 (tiles 000-0FF) ▾]", lx as i32, y as i32, 14.0, ink);
    y += 28;
    draw_text(
        &mut img,
        &sans,
        "Tileset: [0: Normal ▾]  (this level uses tileset 0)",
        lx as i32,
        y as i32,
        14.0,
        ink,
    );
    y += 40;
    for (i, label) in ["Export page…", "Import…"].iter().enumerate() {
        let bx = lx + i as u32 * 150;
        fill_rect(&mut img, bx, y, 140, 32, Rgb([0xFF, 0xFF, 0xFF]));
        rect_border(&mut img, bx, y, 140, 32, Rgb([0x99, 0x99, 0x99]));
        draw_text(&mut img, &sans, label, (bx + 10) as i32, (y + 7) as i32, 14.0, ink);
    }
    y += 56;
    draw_text(&mut img, &sans_bold, "Status", lx as i32, y as i32, 15.0, ink);
    y += 28;
    fill_rect(&mut img, lx, y, 470, 62, Rgb([0xFF, 0xFF, 0xFF]));
    rect_border(&mut img, lx, y, 470, 62, Rgb([0x99, 0x99, 0x99]));
    draw_text(&mut img, &sans, &status_export, (lx + 8) as i32, (y + 8) as i32, 11.0, green);
    draw_text(&mut img, &sans, &status_import, (lx + 8) as i32, (y + 30) as i32, 11.0, green);
    y += 92;
    let hexline: String = page_data[..16].iter().map(|b| format!("{b:02x} ")).collect();
    draw_text(
        &mut img,
        &sans_bold,
        "map16-page00-ts0.bin — 2048 bytes, raw LM-compatible page:",
        lx as i32,
        y as i32,
        13.0,
        ink,
    );
    y += 26;
    draw_text(&mut img, &sans, &format!("first 16 bytes: {hexline}"), lx as i32, y as i32, 13.0, gray);

    // Right: the real atlas + before/after strip for the changed rows.
    let ax = 540u32;
    let mut ay = 84u32;
    draw_text(&mut img, &sans_bold, "Exported: FG page 0 from vanilla ROM", ax as i32, ay as i32, 15.0, ink);
    ay += 24;
    draw_text(
        &mut img,
        &sans,
        "real tiles via export_page → emulator VRAM/CGRAM (level 105)",
        ax as i32,
        ay as i32,
        12.0,
        gray,
    );
    ay += 26;
    for yy in 0..atlas_px {
        for xx in 0..atlas_px {
            let off = ((yy * atlas_px + xx) * 4) as usize;
            img.put_pixel(ax + xx, ay + yy, Rgb([atlas[off], atlas[off + 1], atlas[off + 2]]));
        }
    }
    rect_border(&mut img, ax, ay, atlas_px, atlas_px, Rgb([0x99, 0x99, 0x99]));
    ay += atlas_px + 16;
    draw_text(
        &mut img,
        &sans_bold,
        "Import proof — tiles 30-3F, before vs after import round trip:",
        ax as i32,
        ay as i32,
        14.0,
        ink,
    );
    ay += 26;
    // Zoomed strip: row 3 (tiles 0x30-0x3F) from the vanilla words vs the
    // palette-shifted imported words, stacked as before/after rows.
    let strip_scale = 2u32;
    let word_sets: Vec<Vec<[u16; 4]>> = [&blocks, &blocks2]
        .iter()
        .map(|bs| {
            bs.iter()
                .map(|b| [b.upper_left.0, b.lower_left.0, b.upper_right.0, b.lower_right.0])
                .collect()
        })
        .collect();
    for (k, words) in word_sets.iter().enumerate() {
        let label = if k == 0 { "before" } else { "after" };
        draw_text(&mut img, &sans, label, ax as i32, ay as i32, 12.0, gray);
        let row_y = ay + 20;
        for t in 0x30..0x40usize {
            let bx = ax + 52 + (t - 0x30) as u32 * 16 * strip_scale;
            let mut tile_px = vec![0u8; 16 * 16 * 4];
            render_block(&vram, &cgram, &words[t], 0, 0, 1, &mut tile_px, 16);
            for sy in 0..16u32 {
                for sx in 0..16u32 {
                    let s = ((sy * 16 + sx) * 4) as usize;
                    let (r, g, b) = (tile_px[s], tile_px[s + 1], tile_px[s + 2]);
                    for dy in 0..strip_scale {
                        for dx in 0..strip_scale {
                            img.put_pixel(
                                bx + sx * strip_scale + dx,
                                row_y + sy * strip_scale + dy,
                                Rgb([r, g, b]),
                            );
                        }
                    }
                }
            }
            rect_border(&mut img, bx, row_y, 16 * strip_scale, 16 * strip_scale, Rgb([0x99, 0x99, 0x99]));
        }
        ay = row_y + 16 * strip_scale + 14;
    }

    img.save(output)?;
    println!("wrote {output}");
    Ok(())
}
