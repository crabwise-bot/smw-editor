//! Shared raster + level-render helpers for the `src/bin/render_*` screenshot tools.
//!
//! These were copy-pasted across a dozen binaries; the bodies here are the
//! canonical versions extracted verbatim so behavior is identical.

use image::{Rgb, RgbImage};
use smwe_emu::Cpu;

// ── RgbImage drawing ─────────────────────────────────────────────────────────

/// Fill a rectangle on an `RgbImage`, clipped to the image bounds.
pub fn fill_rect(img: &mut RgbImage, x: u32, y: u32, w: u32, h: u32, c: Rgb<u8>) {
    for yy in y..(y + h).min(img.height()) {
        for xx in x..(x + w).min(img.width()) {
            img.put_pixel(xx, yy, c);
        }
    }
}

/// 1px border rectangle on an `RgbImage`.
pub fn rect_border(img: &mut RgbImage, x: u32, y: u32, w: u32, h: u32, c: Rgb<u8>) {
    fill_rect(img, x, y, w, 1, c);
    fill_rect(img, x, y + h - 1, w, 1, c);
    fill_rect(img, x, y, 1, h, c);
    fill_rect(img, x + w - 1, y, 1, h, c);
}

// ── raw RGB-buffer drawing (alpha-blended) ───────────────────────────────────

/// Alpha-blend one RGBA pixel into a raw RGB buffer.
pub fn blend(buf: &mut [u8], idx: usize, rgba: [u8; 4]) {
    let a = rgba[3] as u32;
    for c in 0..3 {
        let dst = buf[idx + c] as u32;
        buf[idx + c] = ((dst * (255 - a) + rgba[c] as u32 * a) / 255) as u8;
    }
}

/// Fill a rectangle in a raw RGB buffer (`stride` = row width in px), alpha-blended.
pub fn fill_rect_raw(buf: &mut [u8], stride: u32, x: u32, y: u32, w: u32, h: u32, rgba: [u8; 4]) {
    for yy in y..y + h {
        for xx in x..x + w {
            let idx = ((yy * stride + xx) * 3) as usize;
            if idx + 2 < buf.len() {
                blend(buf, idx, rgba);
            }
        }
    }
}

/// Stroke a rectangle outline in a raw RGB buffer, `t` px thick.
pub fn stroke_rect(buf: &mut [u8], stride: u32, x: u32, y: u32, w: u32, h: u32, rgb: [u8; 3], t: u32) {
    let c = [rgb[0], rgb[1], rgb[2], 255];
    for i in 0..t {
        hline(buf, stride, x, y + i, w, c);
        hline(buf, stride, x, y + h - 1 - i, w, c);
        vline(buf, stride, x + i, y, h, c);
        vline(buf, stride, x + w - 1 - i, y, h, c);
    }
}

/// Horizontal line in a raw RGB buffer, alpha-blended.
pub fn hline(buf: &mut [u8], stride: u32, x: u32, y: u32, w: u32, rgba: [u8; 4]) {
    for xx in x..x + w {
        let idx = ((y * stride + xx) * 3) as usize;
        if idx + 2 < buf.len() {
            blend(buf, idx, rgba);
        }
    }
}

/// Vertical line in a raw RGB buffer, alpha-blended.
pub fn vline(buf: &mut [u8], stride: u32, x: u32, y: u32, h: u32, rgba: [u8; 4]) {
    for yy in y..y + h {
        let idx = ((yy * stride + x) * 3) as usize;
        if idx + 2 < buf.len() {
            blend(buf, idx, rgba);
        }
    }
}

// ── level rendering (from the composed VRAM tilemaps) ────────────────────────

/// Render one layer (`bg` = Layer 2/BG vs Layer 1) of a decompressed level
/// into a raw RGB `pixels` buffer (`width` px wide).
pub fn render_layer(cpu: &mut Cpu, bg: bool, width: u32, pixels: &mut [u8]) {
    let map16_bank = cpu.mem.cart.resolve("Map16Common").expect("Cannot resolve Map16Common") & 0xFF0000;
    // Resolve Map16 addresses on a scratch clone so the trampoline never
    // disturbs live CPU/WRAM state, and cache extended-block lookups.
    let mut scratch = cpu.clone();
    let map16_bg = smwe_emu::emu::lm_bg_map16_base(&mut scratch)
        .unwrap_or_else(|| cpu.mem.cart.resolve("Map16BGTiles").expect("Cannot resolve Map16BGTiles"));
    let mut ext_cache: std::collections::HashMap<u16, u32> = std::collections::HashMap::new();
    let vertical = cpu.mem.load_u8(0x5B) & if bg { 2 } else { 1 } != 0;
    let mode = cpu.mem.load_u8(0x1925);
    let renderer_table = cpu.mem.cart.resolve("CODE_058955").unwrap() + 9;
    let renderer = cpu.mem.load_u24(renderer_table + (mode as u32) * 3);
    let l2_renderers = [cpu.mem.cart.resolve("CODE_058B8D"), cpu.mem.cart.resolve("CODE_058C71")];
    let has_layer2 = l2_renderers.contains(&Some(renderer));

    let scr_len = match (vertical, has_layer2) {
        (false, false) => 0x20,
        (true, false) => 0x1C,
        (false, true) => 0x10,
        (true, true) => 0x0E,
    };
    let scr_size = if vertical { 16 * 32 } else { 16 * 27 };
    let (blocks_lo_addr, blocks_hi_addr) = match (bg, has_layer2) {
        (true, true) => {
            let offset = scr_len * scr_size;
            (0x7EC800 + offset, 0x7FC800 + offset)
        }
        (true, false) => (0x7EB900, 0x7EBD00),
        (false, _) => (0x7EC800, 0x7FC800),
    };
    let len = if has_layer2 { 256 * 27 } else { 512 * 27 };

    for idx in 0..len {
        let (block_x, block_y) = if vertical {
            let (screen, sidx) = (idx / (16 * 16), idx % (16 * 16));
            let (row, column) = (sidx / 16, sidx % 16);
            let (sub_y, sub_x) = (screen / 2, screen % 2);
            (column * 16 + sub_x * 256, row * 16 + sub_y * 256)
        } else {
            let (screen, sidx) = (idx / (16 * 27), idx % (16 * 27));
            let (row, column) = (sidx / 16, sidx % 16);
            (column * 16 + screen * 256, row * 16)
        };

        let idx_adj = if bg && !has_layer2 { idx % (16 * 27 * 2) } else { idx };
        let block_id = cpu.mem.load_u8(blocks_lo_addr + idx_adj) as u16
            | (((cpu.mem.load_u8(blocks_hi_addr + idx_adj) as u16) & 0x3F) << 8);
        if block_id == 0 {
            continue;
        }
        let block_ptr = if bg && !has_layer2 {
            block_id as u32 * 8 + map16_bg
        } else if block_id >= 0x200 {
            *ext_cache
                .entry(block_id)
                .or_insert_with(|| smwe_emu::emu::lm_ext_map16_data_addr(&mut scratch, block_id).unwrap_or(0))
        } else {
            cpu.mem.load_u16(0x0FBE + block_id as u32 * 2) as u32 + map16_bank
        };
        if block_ptr == 0 {
            continue;
        }

        for (sub, (off_x, off_y)) in (0..4).zip([(0u32, 0u32), (0, 8), (8, 0), (8, 8)]) {
            let t = cpu.mem.load_u16(block_ptr + sub * 2);
            render_bg_tile(&cpu.mem.vram, &cpu.mem.cgram, block_x + off_x, block_y + off_y, t, width, pixels);
        }
    }
}

/// Render one BG Map16 tile (`t` = tilemap word) at pixel `(x, y)`.
pub fn render_bg_tile(vram: &[u8], cgram: &[u8], x: u32, y: u32, t: u16, width: u32, pixels: &mut [u8]) {
    let tile = (t & 0x3FF) as usize;
    let pal = ((t >> 10) & 0x7) as usize;
    render_tile(vram, cgram, tile, pal, (t & 0x4000) != 0, (t & 0x8000) != 0, x, y, width, pixels);
}

/// Render one sprite/OAM tile (`t` = OAM tile word) at pixel `(x, y)`.
pub fn render_sp_tile(vram: &[u8], cgram: &[u8], x: u32, y: u32, t: u16, width: u32, pixels: &mut [u8]) {
    let tile = ((t & 0x1FF) + 0x600) as usize;
    let pal = (((t >> 9) & 0x7) + 8) as usize;
    render_tile(vram, cgram, tile, pal, (t & 0x4000) != 0, (t & 0x8000) != 0, x, y, width, pixels);
}

/// Render one 8x8 4bpp tile from VRAM with CGRAM palette at pixel `(x0, y0)`.
#[allow(clippy::too_many_arguments)]
pub fn render_tile(
    vram: &[u8], cgram: &[u8], tile_id: usize, palette: usize, flip_x: bool, flip_y: bool, x0: u32, y0: u32,
    width: u32, pixels: &mut [u8],
) {
    let tile_base = tile_id * 32;
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
            let rgb = read_color(cgram, palette * 16 + color_idx);
            let off = (((y0 + ty) * width + x0 + tx) * 3) as usize;
            if off + 2 < pixels.len() {
                pixels[off] = rgb[0];
                pixels[off + 1] = rgb[1];
                pixels[off + 2] = rgb[2];
            }
        }
    }
}

/// Decode one 15-bit CGRAM color to 8-bit RGB.
pub fn read_color(cgram: &[u8], idx: usize) -> [u8; 3] {
    let off = idx * 2;
    if off + 1 >= cgram.len() {
        return [0, 0, 0];
    }
    let lo = cgram[off] as u16;
    let hi = cgram[off + 1] as u16;
    let rgb = lo | (hi << 8);
    [((rgb & 0x1F) << 3) as u8, (((rgb >> 5) & 0x1F) << 3) as u8, (((rgb >> 10) & 0x1F) << 3) as u8]
}
