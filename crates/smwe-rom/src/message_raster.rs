//! True raster preview for message boxes: renders the 8×18 tile grid with the
//! real SMW message-font graphics.
//!
//! The font is GFX2A ("Message Box Letters", SNES $0BCB7B, 2bpp, 128 tiles),
//! decompressed from the ROM. Tile indices 0x00-0x7F map directly to GFX2A
//! tiles 0-127. The game draws these on Layer 3 (2bpp) with palette 6.
//!
//! Palette: the in-game message box is white text on a black background.
//! We render color 0 as black, color 1 as white, colors 2-3 as grays. The
//! exact CGRAM values come from the level's palette 6 at runtime; the glyph
//! shapes (the true SMW font) are what make this WYSIWYG.

use image::{Rgb, RgbImage};

use crate::font_map::message_cells;
use crate::graphics::gfx_file::{tile_format_of, GfxFile, TileFormat};
use crate::snes_utils::rom::Rom;

/// GFX file number for "Message Box Letters" (SMWDisX bank_08-0B.asm).
pub const MESSAGE_FONT_GFX_FILE: usize = 0x2A;

/// SNES address of the compressed GFX2A "Message Box Letters".
pub const MESSAGE_FONT_GFX_SNES: u32 = 0x0BCB7B;

/// Decompress the message font (GFX2A) from the ROM.
///
/// Returns 128 2bpp tiles. Tile N corresponds to message byte 0xN (bit 7
/// masked). Verified 2026-09-10: tile 0='A', tile 0x1A='!', tile 0x40='a',
//  etc., matching [`crate::font_map::FontMap::real`].
pub fn decompress_message_font(rom: &Rom) -> anyhow::Result<Vec<Box<[u8]>>> {
    let file_num = MESSAGE_FONT_GFX_FILE;
    let format = tile_format_of(file_num);
    if format != TileFormat::Tile2bpp {
        anyhow::bail!("GFX2A message font: expected 2bpp, found {format:?}");
    }
    let gfx = GfxFile::new(rom, file_num, false)?;
    if gfx.tiles.len() != 128 {
        anyhow::bail!("GFX2A message font: expected 128 tiles, found {}", gfx.tiles.len());
    }
    Ok(gfx.tiles.iter().map(|t| t.color_indices.clone()).collect())
}

/// Rasterize a message's 8×18 tile grid to an RGB image (144×64 pixels).
///
/// `cells` is the 8×18 tile-index grid from [`message_cells`]. `font` is the
/// 128 2bpp tiles from [`decompress_message_font`]. Each tile is 8×8 pixels;
/// the output is 18*8=144 wide, 8*8=64 tall.
pub fn rasterize_message(cells: [[u8; 18]; 8], font: &[Box<[u8]>]) -> RgbImage {
    const W: u32 = 18 * 8;
    const H: u32 = 8 * 8;
    let mut img = RgbImage::new(W, H);
    // White-on-black (canonical message-box look). Color 0 (transparent)
    // becomes the black background.
    let palette = [
        Rgb([0, 0, 0]),
        Rgb([255, 255, 255]),
        Rgb([128, 128, 128]),
        Rgb([192, 192, 192]),
    ];
    for (row, cells_row) in cells.iter().enumerate() {
        for (col, &tile_idx) in cells_row.iter().enumerate() {
            let tile = &font[(tile_idx & 0x7F) as usize % font.len()];
            for y in 0..8 {
                for x in 0..8 {
                    let c = tile[y * 8 + x] as usize;
                    let color = palette[c.min(3)];
                    img.put_pixel((col * 8 + x) as u32, (row * 8 + y) as u32, color);
                }
            }
        }
    }
    img
}

/// Convenience: decode `bytes` via [`message_cells`] and rasterize with
/// `font` from [`decompress_message_font`].
pub fn rasterize_message_bytes(bytes: &[u8], font: &[Box<[u8]>]) -> RgbImage {
    rasterize_message(message_cells(bytes), font)
}
