//! Headless screenshot for the LM v3.00 "Insert all GFX and ExGFX then
//! reload" toolbar button.
//!
//! Stages a real, highly visible GFX edit (solid bright tiles) in a scratch
//! ROM via the same LC_LZ2-compress + pointer-table path `save_to_rom`'s GFX
//! section uses, then runs the exact reload functions the toolbar button
//! triggers (`smwe_emu::emu::reload_level_graphics` /
//! `reload_overworld_graphics`) on a cart-swapped CPU and renders
//! before/after. Proves staged graphics appear live in both the level and
//! overworld views without a save.
//!
//!   --rom=PATH --out=docs/screenshots/insert-all-gfx-reload.png

use std::{path::Path, sync::Arc};

use anyhow::Context;
use image::{ImageBuffer, Rgb};
use smw_editor::{
    level_png_export::{render_level_png_from_cpu, LevelPngOptions},
    render_util::{read_color, render_tile},
};
use smwe_emu::{emu::CheckedMem, rom::Rom as EmuRom, Cpu};
use smwe_rom::{
    compression::lc_lz2,
    freespace::find_free_space,
    graphics::gfx_file::{GFX_POINTER_TABLE_BANK, GFX_POINTER_TABLE_HIGH, GFX_POINTER_TABLE_LOW},
    snes_utils::addr::{AddrPc, AddrSnes},
    SmwRom,
};

fn arg(name: &str, default: &str) -> String {
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        if a == name {
            return args.next().unwrap_or_else(|| default.to_string());
        }
    }
    default.to_string()
}

/// Paint tiles 0..8 of `file_num` solid bright (max color index) and insert
/// the re-encoded file into `rom_bytes`, replicating `save_to_rom`'s vanilla
/// GFX section (LC_LZ2 compress, write in place or repoint via the pointer
/// table + free space).
fn stage_visible_gfx_edit(rom: &SmwRom, rom_bytes: &mut [u8], file_num: usize) -> anyhow::Result<()> {
    let file = rom.gfx.files.get(file_num).with_context(|| format!("no GFX file {file_num:02X}"))?;
    let max_idx = match smwe_rom::graphics::gfx_file::tile_format_of(file_num) {
        smwe_rom::graphics::gfx_file::TileFormat::Tile4bpp => 15u8,
        _ => 7u8,
    };
    let mut tiles = file.tiles.clone();
    for tile in tiles.iter_mut().take(64) {
        for (i, px) in tile.color_indices.iter_mut().enumerate() {
            // Checkerboard of max-index and 0: unmistakable in the render.
            *px = if (i / 8 + i % 8) % 2 == 0 { max_idx } else { 0 };
        }
    }
    let raw = smwe_rom::graphics::gfx_file::GfxFile { tile_format: file.tile_format, tiles }.to_raw_bytes();

    let compressed = lc_lz2::compress(&raw);
    let pc_of = |snes: AddrSnes| -> anyhow::Result<usize> {
        Ok(AddrPc::try_from_lorom(snes).map_err(|e| anyhow::anyhow!("{e:?}"))?.as_index())
    };
    let low_pc = pc_of(GFX_POINTER_TABLE_LOW + file_num)?;
    let high_pc = pc_of(GFX_POINTER_TABLE_HIGH + file_num)?;
    let bank_pc = pc_of(GFX_POINTER_TABLE_BANK + file_num)?;
    let cur =
        AddrSnes((rom_bytes[low_pc] as u32) | ((rom_bytes[high_pc] as u32) << 8) | ((rom_bytes[bank_pc] as u32) << 16));
    let old_pc = pc_of(cur)?;
    let old_len = lc_lz2::decompress_with_len(&rom_bytes[old_pc..], false).map(|(_, len)| len).unwrap_or(0);
    if compressed.len() <= old_len {
        rom_bytes[old_pc..old_pc + compressed.len()].copy_from_slice(&compressed);
        if compressed.len() < old_len {
            rom_bytes[old_pc + compressed.len()..old_pc + old_len].fill(0xFF);
        }
    } else {
        let pc = find_free_space(rom_bytes, compressed.len(), 0x008000, 0)
            .with_context(|| format!("no free space for GFX file {file_num:02X}"))?;
        rom_bytes[old_pc..old_pc + old_len].fill(0xFF);
        let new_snes = AddrSnes::try_from_lorom(AddrPc(pc as u32)).map_err(|e| anyhow::anyhow!("{e:?}"))?.0;
        rom_bytes[low_pc] = (new_snes & 0xFF) as u8;
        rom_bytes[high_pc] = ((new_snes >> 8) & 0xFF) as u8;
        rom_bytes[bank_pc] = ((new_snes >> 16) & 0xFF) as u8;
        rom_bytes[pc..pc + compressed.len()].copy_from_slice(&compressed);
    }
    Ok(())
}

fn make_cpu(rom_bytes: &[u8]) -> Cpu {
    let mut emu_rom = EmuRom::new(rom_bytes.to_vec());
    emu_rom.load_symbols(include_str!("../../symbols/SMW_U.sym"));
    Cpu::new(CheckedMem::new(Arc::new(emu_rom)))
}

fn swap_cart(cpu: &mut Cpu, rom_bytes: &[u8]) {
    let mut emu_rom = EmuRom::new(rom_bytes.to_vec());
    emu_rom.load_symbols(include_str!("../../symbols/SMW_U.sym"));
    cpu.mem.cart = Arc::new(emu_rom);
}

/// Minimal overworld render: L2 then L1 tilemaps at 512x512, like
/// render_ow_submap's `render_bg`.
fn render_overworld(cpu: &Cpu) -> Vec<u8> {
    const L1_BASE: usize = 0x2000 * 2;
    const L2_BASE: usize = 0x3000 * 2;
    let mut pixels = vec![0u8; 512 * 512 * 3];
    {
        let backdrop = read_color(&cpu.mem.cgram, 0);
        for px in pixels.chunks_exact_mut(3) {
            px.copy_from_slice(&backdrop);
        }
    }
    for &(base, scroll) in &[(L2_BASE, (0i32, 0i32)), (L1_BASE, (0i32, 0i32))] {
        let _ = scroll;
        for row in 0..64u32 {
            for col in 0..64u32 {
                // 64x64 tilemap stored as 32x32 quadrants; simplified address:
                // matches render_ow_submap's tilemap_vram_addr for the main map.
                let q = (row / 32) * 2 + (col / 32);
                let sr = row % 32;
                let sc = col % 32;
                let addr = base + (q as usize) * 32 * 32 * 2 + ((sr * 32 + sc) as usize) * 2;
                if addr + 1 >= cpu.mem.vram.len() {
                    continue;
                }
                let t0 = cpu.mem.vram[addr] as u16;
                let t1 = cpu.mem.vram[addr + 1] as u16;
                let x = col * 8;
                let y = row * 8;
                if x >= 512 || y >= 512 {
                    continue;
                }
                render_tile(
                    &cpu.mem.vram,
                    &cpu.mem.cgram,
                    (t0 | ((t1 & 3) << 8)) as usize,
                    ((t1 >> 2) & 7) as usize,
                    (t1 & 0x40) != 0,
                    (t1 & 0x80) != 0,
                    x,
                    y,
                    512,
                    &mut pixels,
                );
            }
        }
    }
    pixels
}

fn main() -> anyhow::Result<()> {
    let rom_path = arg("--rom", "/home/hatch/workspace/smw-editor/smw.smc");
    let out_path = arg("--out", "docs/screenshots/insert-all-gfx-reload.png");

    let raw = std::fs::read(&rom_path).with_context(|| format!("read {rom_path}"))?;
    let rom_bytes: Vec<u8> = if raw.len() % 0x400 == 0x200 { raw[0x200..].to_vec() } else { raw };
    let rom = SmwRom::from_file(Path::new(&rom_path))?;

    // ── Level half: level 0x105, edit its FG1 file ──────────────────────────
    let level: u16 = 0x105;
    let fg_tileset = rom.levels[level as usize].primary_header.fg_bg_gfx() as usize;
    let fg1 = rom.gfx.object_gfx_list.files_for_object_tileset(fg_tileset)[0];
    eprintln!("level {level:03X}: fg tileset {fg_tileset}, FG1 = GFX file {fg1:02X}");

    let mut cpu = make_cpu(&rom_bytes);
    smwe_emu::emu::decompress_sublevel(&mut cpu, level);
    smwe_emu::emu::fetch_anim_frame(&mut cpu);
    let opts = LevelPngOptions::default();
    let before = render_level_png_from_cpu(&mut cpu, &opts)?;

    // Stage the edit + run the exact button path: cart swap, reload, anim.
    let mut scratch = rom_bytes.clone();
    stage_visible_gfx_edit(&rom, &mut scratch, fg1)?;
    swap_cart(&mut cpu, &scratch);
    smwe_emu::emu::reload_level_graphics(&mut cpu);
    smwe_emu::emu::fetch_anim_frame(&mut cpu);
    let after = render_level_png_from_cpu(&mut cpu, &opts)?;
    let changed_px = before.pixels.iter().zip(after.pixels.iter()).filter(|(a, b)| a != b).count() / 3;
    eprintln!("level render: {changed_px} pixels changed by the reload");

    // ── Overworld half: submap 0, edit its BG1 file (0x1C) ──────────────────
    let ow_file = 0x1Cu8;
    let mut ow = make_cpu(&rom_bytes);
    smwe_emu::emu::load_overworld(&mut ow, 0);
    let ow_before = render_overworld(&ow);

    let mut ow_scratch = rom_bytes.clone();
    stage_visible_gfx_edit(&rom, &mut ow_scratch, ow_file as usize)?;
    swap_cart(&mut ow, &ow_scratch);
    smwe_emu::emu::reload_overworld_graphics(&mut ow);
    let ow_after = render_overworld(&ow);
    let ow_changed = ow_before.iter().zip(ow_after.iter()).filter(|(a, b)| a != b).count() / 3;
    eprintln!("overworld render: {ow_changed} pixels changed by the reload");

    // ── Compose a labeled 2x2 grid (all panels cropped to 512x512) ──────────
    fn crop_512(pixels: &[u8], w: u32, h: u32) -> (Vec<u8>, u32, u32) {
        let (cw, ch) = (512.min(w), 512.min(h));
        let mut out = vec![0u8; (cw * ch * 3) as usize];
        for y in 0..ch {
            let src = ((y * w * 3) as usize)..(((y * w + cw) * 3) as usize);
            let dst = ((y * cw * 3) as usize)..(((y * cw + cw) * 3) as usize);
            out[dst].copy_from_slice(&pixels[src]);
        }
        (out, cw, ch)
    }
    let label_h = 24u32;
    let pw = 512u32;
    let ph = 512u32;
    let grid_w = pw * 2;
    let grid_h = (ph + label_h) * 2;
    let mut grid = ImageBuffer::<Rgb<u8>, _>::from_pixel(grid_w, grid_h, Rgb([24, 24, 28]));
    let labels = [
        format!("LEVEL {level:03X} BEFORE (STAGED GFX{fg1:02X} EDIT NOT VISIBLE)"),
        "LEVEL 105 AFTER: INSERT ALL GFX + RELOAD (LM V3.00)".to_owned(),
        "OVERWORLD BEFORE (STAGED GFX1C EDIT NOT VISIBLE)".to_owned(),
        "OVERWORLD AFTER: INSERT ALL GFX + RELOAD (LM V3.00)".to_owned(),
    ];
    let panels: Vec<Vec<u8>> = vec![before.pixels.clone(), after.pixels.clone(), ow_before.clone(), ow_after.clone()];
    let panel_sizes = [(before.width, before.height), (after.width, after.height), (512, 512), (512, 512)];
    for (i, (px, label)) in panels.iter().zip(labels.iter()).enumerate() {
        let (cx, cy) = ((i % 2) as u32 * pw, (i / 2) as u32 * (ph + label_h));
        for y in 0..label_h {
            for x in 0..pw {
                grid.put_pixel(cx + x, cy + y, Rgb([40, 44, 52]));
            }
        }
        draw_text(&mut grid, cx + 8, cy + 8, label, Rgb([220, 220, 230]));
        let (w0, h0) = panel_sizes[i];
        let (cropped, cw, ch) = crop_512(px, w0, h0);
        let img = ImageBuffer::<Rgb<u8>, _>::from_raw(cw, ch, cropped).context("panel buffer")?;
        image::imageops::overlay(&mut grid, &img, cx as i64, (cy + label_h) as i64);
        eprintln!("panel {i}: {label}");
    }
    grid.save(&out_path)?;
    println!("wrote {out_path}");
    Ok(())
}

/// Minimal 5x7 bitmap font for panel labels (uppercase, digits, basic
/// punctuation). Each glyph is 7 rows of 5 bits.
fn glyph_rows(c: char) -> [u8; 7] {
    match c {
        'A' => [0x0E, 0x11, 0x11, 0x1F, 0x11, 0x11, 0x11],
        'B' => [0x1E, 0x11, 0x11, 0x1E, 0x11, 0x11, 0x1E],
        'C' => [0x0E, 0x11, 0x10, 0x10, 0x10, 0x11, 0x0E],
        'D' => [0x1E, 0x11, 0x11, 0x11, 0x11, 0x11, 0x1E],
        'E' => [0x1F, 0x10, 0x10, 0x1E, 0x10, 0x10, 0x1F],
        'F' => [0x1F, 0x10, 0x10, 0x1E, 0x10, 0x10, 0x10],
        'G' => [0x0E, 0x11, 0x10, 0x17, 0x11, 0x11, 0x0F],
        'H' => [0x11, 0x11, 0x11, 0x1F, 0x11, 0x11, 0x11],
        'I' => [0x0E, 0x04, 0x04, 0x04, 0x04, 0x04, 0x0E],
        'K' => [0x11, 0x12, 0x14, 0x18, 0x14, 0x12, 0x11],
        'L' => [0x10, 0x10, 0x10, 0x10, 0x10, 0x10, 0x1F],
        'M' => [0x11, 0x1B, 0x15, 0x15, 0x11, 0x11, 0x11],
        'N' => [0x11, 0x19, 0x19, 0x15, 0x13, 0x13, 0x11],
        'O' => [0x0E, 0x11, 0x11, 0x11, 0x11, 0x11, 0x0E],
        'P' => [0x1E, 0x11, 0x11, 0x1E, 0x10, 0x10, 0x10],
        'R' => [0x1E, 0x11, 0x11, 0x1E, 0x14, 0x12, 0x11],
        'S' => [0x0F, 0x10, 0x10, 0x0E, 0x01, 0x01, 0x1E],
        'T' => [0x1F, 0x04, 0x04, 0x04, 0x04, 0x04, 0x04],
        'U' => [0x11, 0x11, 0x11, 0x11, 0x11, 0x11, 0x0E],
        'V' => [0x11, 0x11, 0x11, 0x11, 0x11, 0x0A, 0x04],
        'W' => [0x11, 0x11, 0x11, 0x15, 0x15, 0x1B, 0x11],
        'X' => [0x11, 0x11, 0x0A, 0x04, 0x0A, 0x11, 0x11],
        'Y' => [0x11, 0x11, 0x0A, 0x04, 0x04, 0x04, 0x04],
        '0' => [0x0E, 0x11, 0x13, 0x15, 0x19, 0x11, 0x0E],
        '1' => [0x04, 0x0C, 0x04, 0x04, 0x04, 0x04, 0x0E],
        '2' => [0x0E, 0x11, 0x01, 0x02, 0x04, 0x08, 0x1F],
        '3' => [0x1F, 0x02, 0x04, 0x02, 0x01, 0x11, 0x0E],
        '4' => [0x02, 0x06, 0x0A, 0x12, 0x1F, 0x02, 0x02],
        '7' => [0x1F, 0x01, 0x02, 0x04, 0x08, 0x08, 0x08],
        '8' => [0x0E, 0x11, 0x11, 0x0E, 0x11, 0x11, 0x0E],
        '9' => [0x0E, 0x11, 0x11, 0x0F, 0x01, 0x02, 0x0C],
        '5' => [0x1F, 0x10, 0x1E, 0x01, 0x01, 0x11, 0x0E],
        '6' => [0x06, 0x08, 0x10, 0x1E, 0x11, 0x11, 0x0E],
        ':' => [0x00, 0x04, 0x00, 0x00, 0x00, 0x04, 0x00],
        '+' => [0x00, 0x04, 0x04, 0x1F, 0x04, 0x04, 0x00],
        '(' => [0x02, 0x04, 0x08, 0x08, 0x08, 0x04, 0x02],
        ')' => [0x08, 0x04, 0x02, 0x02, 0x02, 0x04, 0x08],
        '.' => [0x00, 0x00, 0x00, 0x00, 0x00, 0x04, 0x00],
        '-' => [0x00, 0x00, 0x00, 0x1F, 0x00, 0x00, 0x00],
        ' ' => [0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00],
        _ => [0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00],
    }
}

fn draw_text(img: &mut ImageBuffer<Rgb<u8>, Vec<u8>>, x0: u32, y0: u32, text: &str, color: Rgb<u8>) {
    for (i, c) in text.chars().enumerate() {
        let rows = glyph_rows(c.to_ascii_uppercase());
        for (ry, row) in rows.iter().enumerate() {
            for rx in 0..5 {
                if row & (0x10 >> rx) != 0 {
                    let (x, y) = (x0 + i as u32 * 6 + rx, y0 + ry as u32);
                    if x < img.width() && y < img.height() {
                        img.put_pixel(x, y, color);
                    }
                }
            }
        }
    }
}
