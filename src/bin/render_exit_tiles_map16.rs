// Headless screenshot for the Lunar Magic v3.70 "Exit Enabled Tiles in the
// Map16 editor" view: renders the full FG Map16 grid (16x32 blocks, the same
// emulator VRAM/CGRAM path the editor's own tile picker uses) and paints the
// same pink markers the Map16 browser grid draws, computed from the ROM's
// acts-like table through smwe_rom::block_behavior::is_exit_enabled.
//   --rom=PATH --level=0x105 --out=docs/screenshots/exit-tiles-map16.png
// The level only supplies VRAM/CGRAM and its level mode (0x09C is only
// exit-enabled in level mode 0x01, same as the in-editor view).
use std::{collections::HashMap, sync::Arc};

use anyhow::Context;
use smw_editor::{
    level_png_export::level_geom_of,
    render_util::{fill_rect_raw, stroke_rect},
};
use smwe_emu::{emu::CheckedMem, rom::Rom as EmuRom, Cpu};
use smwe_rom::{block_behavior::is_exit_enabled, map16_expanded::act_as_of};

const COLS: usize = 16;
const ROWS: usize = 32; // FG blocks 0x000-0x1FF
const BLOCK_PX: u32 = 16;
const SCALE: u32 = 2;
const CELL: u32 = BLOCK_PX * SCALE; // 32
const ATLAS_W: u32 = COLS as u32 * CELL; // 512
const ATLAS_H: u32 = ROWS as u32 * CELL; // 1024

fn arg(name: &str, default: &str) -> String {
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        if a == name {
            return args.next().unwrap_or_else(|| default.to_string());
        }
    }
    default.to_string()
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
            let off = ((py_abs as usize) * stride + px_abs as usize) * 3;
            if off + 2 < pixels.len() {
                pixels[off] = r;
                pixels[off + 1] = g;
                pixels[off + 2] = b;
            }
        }
    }
}

fn main() -> anyhow::Result<()> {
    let rom_path = arg("--rom", "smw.smc");
    let out_path = arg("--out", "docs/screenshots/exit-tiles-map16.png");
    let level: u16 = u16::from_str_radix(arg("--level", "0x105").trim_start_matches("0x"), 16).unwrap_or(0x105);

    let raw = std::fs::read(&rom_path).with_context(|| format!("read {rom_path}"))?;
    let rom_bytes: Vec<u8> = if raw.len() % 0x400 == 0x200 { raw[0x200..].to_vec() } else { raw };
    let acts: HashMap<u16, u16> = smwe_rom::map16_expanded::read_acts_table(&rom_bytes, 0).unwrap_or_default();

    let mut emu_rom = EmuRom::new(rom_bytes.clone());
    emu_rom.load_symbols(include_str!("../../symbols/SMW_U.sym"));
    let mut cpu = Cpu::new(CheckedMem::new(Arc::new(emu_rom)));
    smwe_emu::emu::decompress_sublevel(&mut cpu, level);
    let g = level_geom_of(&mut cpu);
    eprintln!("level {level:#05X}: mode {:#04X}", g.level_mode);

    let vram = cpu.mem.vram.clone();
    let cgram = cpu.mem.cgram.clone();

    // Map16Common block pointers (same as TilePicker::rebuild).
    let map16_bank = cpu.mem.cart.resolve("Map16Common").unwrap_or(0) & 0xFF0000;
    let mut block_ptrs = [0u32; 512];
    for (block_id, block_ptr) in block_ptrs.iter_mut().enumerate() {
        let ptr_lo = 0x0FBE + block_id * 2;
        if ptr_lo + 1 < 0x10000 {
            *block_ptr = cpu.mem.load_u16(ptr_lo as u32) as u32 + map16_bank;
        }
    }

    // ── Render the FG atlas ──
    let mut pixels = vec![0u8; (ATLAS_W * ATLAS_H * 3) as usize];
    // Dark backdrop (unassigned blocks stay dark, like the editor texture).
    for p in pixels.chunks_exact_mut(3) {
        p[0] = 24;
        p[1] = 24;
        p[2] = 28;
    }
    for block_id in 0..512usize {
        let block_ptr = block_ptrs[block_id];
        if block_ptr == map16_bank {
            continue;
        }
        let col = (block_id % COLS) as u32;
        let row = (block_id / COLS) as u32;
        let x0 = col * CELL;
        let y0 = row * CELL;
        let sub_offsets = [(0u32, 0u32), (0, 8), (8, 0), (8, 8)];
        for (sub_i, (sx, sy)) in sub_offsets.into_iter().enumerate() {
            let lo = cpu.mem.cart.read(block_ptr + (sub_i as u32) * 2).unwrap_or(0);
            let hi = cpu.mem.cart.read(block_ptr + (sub_i as u32) * 2 + 1).unwrap_or(0);
            let t = lo as u16 | ((hi as u16) << 8);
            // Render at 1x into a scratch block, then upscale.
            let mut small = vec![0u8; (BLOCK_PX * BLOCK_PX * 3) as usize];
            render_sub_tile(&vram, &cgram, t, sx, sy, &mut small, BLOCK_PX as usize);
            for sy2 in 0..BLOCK_PX {
                for sx2 in 0..BLOCK_PX {
                    let s = ((sy2 * BLOCK_PX + sx2) * 3) as usize;
                    for dy in 0..SCALE {
                        for dx in 0..SCALE {
                            let d = (((y0 + sy2 * SCALE + dy) * ATLAS_W + (x0 + sx2 * SCALE + dx)) * 3) as usize;
                            pixels[d..d + 3].copy_from_slice(&small[s..s + 3]);
                        }
                    }
                }
            }
        }
    }

    // ── Paint the markers (same pink as the in-editor overlay) ──
    let mut marked = Vec::new();
    for block_id in 0..0x200u16 {
        if is_exit_enabled(act_as_of(&acts, block_id), g.level_mode) {
            marked.push(block_id);
            let col = (block_id as u32 % COLS as u32) * CELL;
            let row = (block_id as u32 / COLS as u32) * CELL;
            fill_rect_raw(&mut pixels, ATLAS_W, col, row, CELL, CELL, [255, 110, 180, 70]);
            stroke_rect(&mut pixels, ATLAS_W, col, row, CELL, CELL, [255, 110, 180], 2);
        }
    }
    eprintln!("marked {} blocks: {marked:04X?}", marked.len());

    let img = image::RgbImage::from_raw(ATLAS_W, ATLAS_H, pixels).expect("atlas size");
    img.save(&out_path)?;
    eprintln!("wrote {out_path} ({ATLAS_W}x{ATLAS_H})");
    Ok(())
}
