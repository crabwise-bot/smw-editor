//! Headless screenshot of the 8x8 tile editor feature.
//!
//! egui can't render headless, so this composes an honest mock of the new
//! "8x8 Tile Editor" window: the real GFX-file selector state, the real
//! palette-colored tile grid (every tile of the file rendered through the
//! exact `tile_rgba8`/`cgram_palette_row` functions the UI uses, copied
//! verbatim below), a real pixel-editor pane, and a real pixel edit applied
//! through the same re-encode path `apply_tile_editor_pixels` uses
//! (`GfxFile::to_raw_bytes`), with the byte diff asserted. The Map16
//! double-click handoff caption shows real computed values: an actual Map16
//! block's sub-tile mapped to its source GFX file via the level's
//! ObjectTileset and OBJECTGFXLIST, exactly like `vram_tile_to_gfx_source`.
//!
//! ```sh
//! cargo run --bin render_8x8_editor -- --out=docs/screenshots/8x8-tile-editor.png --rom=smw.smc
//! ```

use std::sync::Arc;

use ab_glyph::{Font, FontRef, Glyph, Point, PxScale, ScaleFont};
use image::{Rgb, RgbImage};
use smwe_emu::{emu::CheckedMem, rom::Rom as EmuRom, Cpu};
use smwe_rom::{
    graphics::gfx_file::{Tile, TileFormat},
    objects::map16::Tile8x8,
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

/// Verbatim copy of the UI helpers
/// (`src/ui/editor_prototypes/level_editor/tile_editor.rs`): decode one
/// 16-color CGRAM palette row (SNES BGR555).
fn cgram_palette_row(cgram: &[u8], row: usize) -> [[u8; 3]; 16] {
    let mut out = [[0u8; 3]; 16];
    for i in 0..16usize {
        let off = row * 32 + i * 2;
        if off + 1 < cgram.len() {
            let c = cgram[off] as u16 | ((cgram[off + 1] as u16) << 8);
            out[i] = [((c & 0x1F) << 3) as u8, (((c >> 5) & 0x1F) << 3) as u8, (((c >> 10) & 0x1F) << 3) as u8];
        }
    }
    out
}

/// Verbatim copy of the UI helper: render one 8x8 tile's color indices into an
/// 8x8 RGB buffer using the given palette.
fn tile_rgb8(tile: &Tile, palette: &[[u8; 3]; 16], out: &mut [u8]) {
    for py in 0..8usize {
        for px in 0..8usize {
            let idx = tile.color_indices.get(py * 8 + px).copied().unwrap_or(0);
            let c = palette[(idx as usize).min(15)];
            let off = (py * 8 + px) * 3;
            out[off] = c[0];
            out[off + 1] = c[1];
            out[off + 2] = c[2];
        }
    }
}

fn blit_rgb(img: &mut RgbImage, rgb: &[u8], x0: u32, y0: u32, scale: u32) {
    for (i, pix) in rgb.chunks(3).enumerate() {
        let (sx, sy) = ((i % 8) as u32, (i / 8) as u32);
        for dy in 0..scale {
            for dx in 0..scale {
                img.put_pixel(x0 + sx * scale + dx, y0 + sy * scale + dy, Rgb([pix[0], pix[1], pix[2]]));
            }
        }
    }
}

fn rect_outline(img: &mut RgbImage, x0: u32, y0: u32, w: u32, h: u32, t: u32, color: Rgb<u8>) {
    for d in 0..t {
        for x in x0..x0 + w {
            for &(yy, xx) in &[(y0 + d, x), (y0 + h - 1 - d, x)] {
                if xx < img.width() && yy < img.height() {
                    img.put_pixel(xx, yy, color);
                }
            }
        }
        for y in y0..y0 + h {
            for &(xx, yy) in &[(x0 + d, y), (x0 + w - 1 - d, y)] {
                if xx < img.width() && yy < img.height() {
                    img.put_pixel(xx, yy, color);
                }
            }
        }
    }
}

fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let output = args.iter().find_map(|a| a.strip_prefix("--out=")).unwrap_or("docs/screenshots/8x8-tile-editor.png");
    let rom_path = args
        .iter()
        .find_map(|a| a.strip_prefix("--rom="))
        .or_else(|| args.iter().skip(1).find(|a| !a.starts_with("--")).map(|a| a.as_str()))
        .unwrap_or("smw.smc");

    let sans = load_font(SANS_CANDIDATES)?;
    let sans_bold = load_font(SANS_BOLD_CANDIDATES)?;

    // Real ROM + emulator state, exactly like the editor's level load.
    let rom_bytes = std::fs::read(rom_path)?;
    let mut emu_rom = EmuRom::new(rom_bytes.clone());
    emu_rom.load_symbols(include_str!("../../symbols/SMW_U.sym"));
    let mut cpu = Cpu::new(CheckedMem::new(Arc::new(emu_rom)));
    smwe_emu::emu::decompress_sublevel(&mut cpu, 0x105);
    let cgram = cpu.mem.cgram.clone();
    let tileset = cpu.mem.wram[0x1931] as usize; // ObjectTileset
    assert!(tileset < 26, "level 0x105 tileset {tileset} out of OBJECTGFXLIST range");

    let rom = smwe_rom::SmwRom::from_file(rom_path)?;
    // FG1 file of the loaded level's tileset — the file the tile grid shows.
    let fg1 = rom.gfx.object_gfx_list.gfx_file_for_object_tile(Tile8x8(0x0000), tileset);
    let file = &rom.gfx.files[fg1];
    let n_tiles = file.tiles.len();
    assert!(n_tiles >= 0x80, "upload slot holds 0x80 tiles");

    // Most common non-zero palette among FG page-0 blocks: the palette row
    // the grid is colored with (what the UI defaults to after a handoff).
    let page = smwe_rom::map16_file::export_page(&rom, smwe_rom::map16_file::PAGE_FG0, 0)?;
    let blocks = smwe_rom::map16_file::parse_page(&page)?;
    let mut pal_counts = [0usize; 8];
    for b in blocks.iter().take(0x100) {
        for w in [b.upper_left.0, b.lower_left.0, b.upper_right.0, b.lower_right.0] {
            let p = ((w >> 10) & 7) as usize;
            if p != 0 {
                pal_counts[p] += 1;
            }
        }
    }
    let mut pal_row = 0usize;
    for (i, &c) in pal_counts.iter().enumerate() {
        if c > pal_counts[pal_row] {
            pal_row = i;
        }
    }
    let palette = cgram_palette_row(&cgram, pal_row);

    // Real edit: paint 5 pixels on a real tile through the same re-encode
    // path the UI's Apply button uses, and assert the byte diff is confined
    // to that tile.
    let sel: usize = 0x23;
    let paint: &[(usize, usize, u8)] = &[(2, 1, 6), (3, 1, 6), (2, 2, 6), (5, 5, 7), (5, 6, 7)];
    let mut tiles: Vec<Tile> = file.tiles.clone();
    let mut px = tiles[sel].color_indices.to_vec();
    for &(x, y, c) in paint {
        px[y * 8 + x] = c;
    }
    tiles[sel].color_indices = px.into_boxed_slice();
    let before = file.to_raw_bytes();
    let after = smwe_rom::graphics::gfx_file::GfxFile { tile_format: file.tile_format, tiles }.to_raw_bytes();
    assert_eq!(before.len(), after.len());
    let diffs: Vec<usize> =
        before.iter().zip(after.iter()).enumerate().filter(|(_, (a, b))| a != b).map(|(i, _)| i).collect();
    assert!(!diffs.is_empty(), "paint must change bytes");
    let tile_size = match file.tile_format {
        TileFormat::Tile2bpp => 16,
        TileFormat::Tile3bpp | TileFormat::Tile3bppMode7 => 24,
        TileFormat::Tile4bpp => 32,
        TileFormat::Tile8bpp => 64,
    };
    assert!(diffs.iter().all(|&i| i / tile_size == sel), "byte diff must be confined to tile {sel:#04X}");
    let edited_tile = &smwe_rom::graphics::gfx_file::GfxFile {
        tile_format: file.tile_format,
        tiles:       {
            let mut t = file.tiles.clone();
            let mut px = t[sel].color_indices.to_vec();
            for &(x, y, c) in paint {
                px[y * 8 + x] = c;
            }
            t[sel].color_indices = px.into_boxed_slice();
            t
        },
    }
    .tiles[sel];

    // Real handoff values: Map16 block 0x125's upper-left sub-tile.
    let blk125 = &blocks[0x25];
    let t0 = blk125.upper_left.0;
    let tile_num = (t0 & 0x3FF) as usize;
    let tile_pal = ((t0 >> 10) & 7) as usize;
    let slot = tile_num / 0x80;
    let src_file = rom.gfx.object_gfx_list.gfx_file_for_object_tile(Tile8x8(t0), tileset);
    let src_tile = tile_num % 0x80;

    // ── Compose the mock window ──────────────────────────────────────────
    const W: u32 = 920;
    const H: u32 = 640;
    let bg = Rgb([30, 30, 38]);
    let mut img = RgbImage::from_pixel(W, H, bg);
    // Title bar
    for y in 0..44u32 {
        for x in 0..W {
            img.put_pixel(x, y, Rgb([42, 42, 54]));
        }
    }
    draw_text(&mut img, &sans_bold, "8x8 Tile Editor", 16, 8, 22.0, Rgb([235, 235, 245]));
    draw_text(
        &mut img,
        &sans,
        &format!("GFX file: {fg1:02X}   Palette: {pal_row}"),
        16,
        56,
        16.0,
        Rgb([200, 200, 210]),
    );
    draw_text(
        &mut img,
        &sans,
        &format!("Format: {}  •  {n_tiles} tiles", file.tile_format),
        330,
        56,
        16.0,
        Rgb([160, 160, 175]),
    );
    draw_text(
        &mut img,
        &sans,
        &format!(
            "Double-clicked Map16 block 0x125 upper-left (tile {tile_num:#05X}, pal {tile_pal}) → GFX file {src_file:02X} tile {src_tile:#04X}"
        ),
        16,
        84,
        14.0,
        Rgb([150, 200, 150]),
    );

    // Tile grid: 16 cols at 2x.
    const GX: u32 = 24;
    const GY: u32 = 120;
    const GS: u32 = 2;
    let rows = n_tiles.div_ceil(16);
    let mut tile_rgb = [0u8; 8 * 8 * 3];
    for (i, tile) in file.tiles.iter().enumerate() {
        tile_rgb8(tile, &palette, &mut tile_rgb);
        blit_rgb(&mut img, &tile_rgb, GX + (i % 16) as u32 * 16, GY + (i / 16) as u32 * 16, GS);
    }
    // Selection highlight on the edited tile.
    rect_outline(&mut img, GX + (sel % 16) as u32 * 16, GY + (sel / 16) as u32 * 16, 16, 16, 2, Rgb([255, 220, 60]));

    // Right pane: pixel editor for the edited tile.
    const PX0: u32 = 340;
    const PY0: u32 = 120;
    const CELL: u32 = 22;
    draw_text(
        &mut img,
        &sans_bold,
        &format!("Tile {sel:#04X} of GFX file {fg1:02X}"),
        PX0 as i32,
        PY0 as i32,
        17.0,
        Rgb([235, 235, 245]),
    );
    let ey = PY0 + 34;
    for py in 0..8usize {
        for px in 0..8usize {
            let idx = edited_tile.color_indices[py * 8 + px];
            let (x0, y0) = (PX0 + px as u32 * CELL, ey + py as u32 * CELL);
            if idx == 0 {
                let shade = if (px + py) % 2 == 0 { 46u8 } else { 74u8 };
                for y in y0..y0 + CELL {
                    for x in x0..x0 + CELL {
                        img.put_pixel(x, y, Rgb([shade, shade, shade]));
                    }
                }
            } else {
                let c = palette[(idx as usize).min(15)];
                for y in y0..y0 + CELL {
                    for x in x0..x0 + CELL {
                        img.put_pixel(x, y, Rgb(c));
                    }
                }
            }
            rect_outline(&mut img, x0, y0, CELL, CELL, 1, Rgb([10, 10, 12]));
        }
    }
    // Highlight the 5 painted pixels.
    for &(x, y, _) in paint {
        rect_outline(&mut img, PX0 + x as u32 * CELL, ey + y as u32 * CELL, CELL, CELL, 2, Rgb([255, 80, 80]));
    }

    // Palette swatches.
    let sy = ey + 8 * CELL + 18;
    draw_text(&mut img, &sans, "Paint color:", PX0 as i32, sy as i32, 15.0, Rgb([200, 200, 210]));
    for i in 0..8usize {
        let (x0, y0) = (PX0 + i as u32 * 30, sy + 26);
        let c = palette[i];
        for y in y0..y0 + 24 {
            for x in x0..x0 + 24 {
                img.put_pixel(x, y, Rgb(c));
            }
        }
        if i == 6 {
            rect_outline(&mut img, x0, y0, 24, 24, 2, Rgb([255, 255, 255]));
        }
    }

    // Apply button mock + status.
    let by = sy + 70;
    for y in by..by + 34 {
        for x in PX0..PX0 + 170 {
            img.put_pixel(x, y, Rgb([58, 110, 180]));
        }
    }
    rect_outline(&mut img, PX0, by, 170, 34, 1, Rgb([120, 170, 230]));
    draw_text(&mut img, &sans, "Apply pixel edits", PX0 as i32 + 18, by as i32 + 7, 15.0, Rgb([240, 245, 255]));
    draw_text(
        &mut img,
        &sans,
        &format!("{} pixels painted on tile {sel:#04X} — Apply stages it for save", paint.len()),
        PX0 as i32,
        by as i32 + 44,
        14.0,
        Rgb([220, 170, 80]),
    );
    draw_text(
        &mut img,
        &sans,
        "Left-drag paints • right-click picks up a color",
        PX0 as i32,
        by as i32 + 68,
        13.0,
        Rgb([150, 150, 165]),
    );
    let _ = slot;
    let _ = rows;

    img.save(output)?;
    println!("wrote {output} (level 0x105 tileset {tileset}, FG1 file {fg1:02X}, palette row {pal_row})");
    Ok(())
}
