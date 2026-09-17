// Headless proof render for the Direct Map16 access PR
// (docs/screenshots/direct-map16.png).
//
// egui cannot render headless, so this follows the established
// render_drag_handles.rs precedent: the real ROM is decompressed through the
// real emulator path, two DM16 objects are authored with the real
// `smwe_rom::direct_map16` types and stamped through the real `tile_at()`
// repetition path into the WRAM block map (using the exact addressing of
// `UiLevelEditor::set_block_id_at`), the level is drawn with the shared
// `render_layer` path, and the DM16 overlay geometry is drawn with the exact
// fill/stroke colors from `dm16_editor.rs` (purple = DM16 object,
// cyan = selected DM16 object). A corner swatch shows the real 2x2 pattern
// tiles (VRAM/CGRAM/Map16) that object A repeats.
//
// Usage:
//   --rom=PATH --level=0x105 --out=docs/screenshots/direct-map16.png
use std::sync::Arc;

use anyhow::Context;
use smwe_emu::{emu::CheckedMem, rom::Rom as EmuRom, Cpu};
use smwe_rom::direct_map16::{DirectMap16Condition, DirectMap16Object};

fn arg(name: &str, default: &str) -> String {
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        if a == name {
            return args.next().unwrap_or_else(|| default.to_string());
        }
    }
    default.to_string()
}

use smw_editor::render_util::{fill_rect_raw, render_layer, render_map16_sub_tile, stroke_rect};

/// Same addressing as `UiLevelEditor::set_block_id_at` (horizontal, layer 1).
fn set_block_id_at(cpu: &mut Cpu, block_x: u32, block_y: u32, block_id: u16) {
    let idx = (block_x / 16) * (16 * 27) + block_y * 16 + (block_x % 16);
    cpu.mem.store_u8(0x7EC800 + idx, (block_id & 0xFF) as u8);
    cpu.mem.store_u8(0x7FC800 + idx, ((block_id >> 8) & 0x01) as u8);
}

fn block_id_at(cpu: &mut Cpu, block_x: u32, block_y: u32) -> u16 {
    let idx = (block_x / 16) * (16 * 27) + block_y * 16 + (block_x % 16);
    cpu.mem.load_u8(0x7EC800 + idx) as u16 | (((cpu.mem.load_u8(0x7FC800 + idx) as u16) & 0x3F) << 8)
}

fn stamp_dm16(cpu: &mut Cpu, obj: &DirectMap16Object) {
    for ly in 0..obj.h {
        for lx in 0..obj.w {
            set_block_id_at(cpu, obj.x + lx, obj.y + ly, obj.tile_at(lx, ly));
        }
    }
}

/// Most common block ID in a tile rectangle of screen 0.
fn mode_tile(cpu: &mut Cpu, x0: u32, y0: u32, w: u32, h: u32, exclude: u16) -> u16 {
    let mut counts = std::collections::HashMap::new();
    for y in y0..y0 + h {
        for x in x0..x0 + w {
            let id = block_id_at(cpu, x, y);
            if id != exclude {
                *counts.entry(id).or_insert(0u32) += 1;
            }
        }
    }
    counts.into_iter().max_by_key(|&(_, c)| c).map(|(id, _)| id).unwrap_or(exclude)
}

/// Top-`n` most common non-blank tiles in a rectangle, for the demo pattern.
fn top_tiles(cpu: &mut Cpu, x0: u32, y0: u32, w: u32, h: u32, blank: u16, n: usize) -> Vec<u16> {
    let mut counts = std::collections::HashMap::new();
    for y in y0..y0 + h {
        for x in x0..x0 + w {
            let id = block_id_at(cpu, x, y);
            if id != blank {
                *counts.entry(id).or_insert(0u32) += 1;
            }
        }
    }
    let mut v: Vec<(u16, u32)> = counts.into_iter().collect();
    v.sort_by_key(|&(_, c)| std::cmp::Reverse(c));
    let mut out: Vec<u16> = v.into_iter().take(n).map(|(id, _)| id).collect();
    while out.len() < n {
        out.push(*out.first().unwrap_or(&blank));
    }
    out
}

/// Find a `w`x`h` all-blank rectangle on screen 0, scanning top-down.
fn find_blank_run(cpu: &mut Cpu, blank: u16, w: u32, h: u32, y_start: u32) -> Option<(u32, u32)> {
    for y in y_start..27u32.saturating_sub(h + 1) {
        'x: for x in 0..16u32.saturating_sub(w + 1) {
            for dy in 0..h {
                for dx in 0..w {
                    if block_id_at(cpu, x + dx, y + dy) != blank {
                        continue 'x;
                    }
                }
            }
            return Some((x, y));
        }
    }
    None
}

fn main() -> anyhow::Result<()> {
    let rom_path = arg("--rom", "smw.smc");
    let level_num: u32 = u32::from_str_radix(arg("--level", "0x105").trim_start_matches("0x"), 16).unwrap_or(0x105);
    let out_path = arg("--out", "docs/screenshots/direct-map16.png");

    let raw = std::fs::read(&rom_path).with_context(|| format!("read {rom_path}"))?;
    let rom_bytes: Vec<u8> = if raw.len() % 0x400 == 0x200 { raw[0x200..].to_vec() } else { raw };

    // ── Decompress the level through the real emulator path ──
    let mut emu_rom = EmuRom::new(rom_bytes.clone());
    emu_rom.load_symbols(include_str!("../../symbols/SMW_U.sym"));
    let mut cpu = Cpu::new(CheckedMem::new(Arc::new(emu_rom)));
    smwe_emu::emu::decompress_sublevel(&mut cpu, level_num as u16);

    let vertical = cpu.mem.load_u8(0x5B) & 1 != 0;
    anyhow::ensure!(!vertical, "demo level must be horizontal");
    let width: u32 = 0x20 * 16 * 16;
    let height: u32 = 27 * 16;

    // ── Derive demo tiles from the real level data ──
    let blank = mode_tile(&mut cpu, 0, 0, 16, 8, 0xFFFF);
    let pattern = top_tiles(&mut cpu, 0, 16, 16, 11, blank, 4);
    eprintln!("blank={blank:#X} pattern={pattern:X?}");

    // ── Author two DM16 objects with the real ROM types ──
    let (ax, ay) = find_blank_run(&mut cpu, blank, 4, 3, 2).expect("no room for object A");
    let obj_a = DirectMap16Object {
        x:         ax,
        y:         ay,
        w:         4,
        h:         3,
        pw:        2,
        ph:        2,
        tiles:     pattern.clone(),
        condition: None,
    };
    let (bx, by) = find_blank_run(&mut cpu, blank, 2, 2, ay + 4).expect("no room for object B");
    let obj_b = DirectMap16Object {
        x:         bx,
        y:         by,
        w:         2,
        h:         2,
        pw:        2,
        ph:        2,
        tiles:     pattern.clone(),
        condition: Some(DirectMap16Condition { ram_addr: 0x1DFC, bit: 8 }),
    };
    eprintln!("object A at ({ax},{ay}) 4x3 pattern 2x2; object B at ({bx},{by}) 2x2 conditional");

    // ── Stamp through the real tile_at() repetition path ──
    stamp_dm16(&mut cpu, &obj_a);
    stamp_dm16(&mut cpu, &obj_b);

    // ── Render the level ──
    let mut pixels = vec![0u8; (width * height * 3) as usize];
    render_layer(&mut cpu, false, width, &mut pixels);
    render_layer(&mut cpu, true, width, &mut pixels);

    // ── Crop to screen 0 ──
    let cw: u32 = 16 * 16;
    let ch: u32 = 27 * 16;
    let mut frame: Vec<u8> = (0..ch)
        .flat_map(|y| {
            let row = (y * width * 3) as usize;
            pixels[row..row + (cw * 3) as usize].to_vec()
        })
        .collect();

    // ── DM16 overlays with the exact dm16_editor.rs colors ──
    // Object A: purple (unselected). Object B: cyan (selected).
    let overlay = |buf: &mut [u8], o: &DirectMap16Object, fill: [u8; 4], stroke: [u8; 3]| {
        let rx = o.x * 16;
        let ry = o.y * 16;
        fill_rect_raw(buf, cw, rx, ry, o.w * 16, o.h * 16, fill);
        stroke_rect(buf, cw, rx, ry, o.w * 16, o.h * 16, stroke, 2);
    };
    overlay(&mut frame, &obj_a, [180, 80, 255, 30], [190, 120, 255]);
    overlay(&mut frame, &obj_b, [0, 200, 255, 45], [0, 210, 255]);

    // ── Pattern swatch: the real 2x2 pattern tiles (VRAM/CGRAM/Map16) ──
    // Same source the Map16 picker renders from: the level's Map16 tileset
    // (level 0x105 uses tileset 0, as in render_map16.rs).
    let rom = smwe_rom::SmwRom::from_file(&rom_path).with_context(|| format!("parse {rom_path}"))?;
    let swatch_px = 2 * 48;
    let mut swatch = vec![0u8; (swatch_px * swatch_px * 4) as usize];
    for (i, &tile_id) in pattern.iter().enumerate() {
        let block = rom.map16_tilesets.get_map16_tile(tile_id as usize, 0).unwrap_or_else(|| {
            use smwe_rom::objects::map16::{Block, Tile8x8};
            Block::from_tuple((Tile8x8(0), Tile8x8(0), Tile8x8(0), Tile8x8(0)))
        });
        let subs = [block.upper_left, block.upper_right, block.lower_left, block.lower_right];
        let bx0 = (i % 2) as u32 * 48;
        let by0 = (i / 2) as u32 * 48;
        for (sub, t8) in subs.iter().enumerate() {
            let sub = sub as u32;
            // render the 8x8 sub-tile 3x scaled into its 24px quadrant
            let qx = bx0 + (sub % 2) * 24;
            let qy = by0 + (sub / 2) * 24;
            let mut tiny = vec![0u8; 8 * 8 * 4];
            render_map16_sub_tile(&cpu.mem.vram, &cpu.mem.cgram, t8.0, 0, 0, &mut tiny, 8);
            for py in 0..8u32 {
                for px in 0..8u32 {
                    let s = ((py * 8 + px) * 4) as usize;
                    for sy in 0..3u32 {
                        for sx in 0..3u32 {
                            let dx = qx + px * 3 + sx;
                            let dy = qy + py * 3 + sy;
                            let d = ((dy * swatch_px + dx) * 4) as usize;
                            swatch[d..d + 4].copy_from_slice(&tiny[s..s + 4]);
                        }
                    }
                }
            }
        }
    }
    // Blit the swatch into the top-right corner with a white border
    // (kept clear of the demo objects on the left).
    let (sx0, sy0) = (cw - swatch_px - 12, 8u32);
    for y in 0..swatch_px + 4 {
        for x in 0..swatch_px + 4 {
            let dx = sx0 + x;
            let dy = sy0 + y;
            if dx >= cw || dy >= ch {
                continue;
            }
            let o = ((dy * cw + dx) * 3) as usize;
            if x < 2 || y < 2 || x >= swatch_px + 2 || y >= swatch_px + 2 {
                frame[o..o + 3].copy_from_slice(&[255, 255, 255]);
            } else {
                let s = ((y - 2) * swatch_px + (x - 2)) as usize * 4;
                if swatch[s + 3] > 0 {
                    frame[o..o + 3].copy_from_slice(&swatch[s..s + 3]);
                }
            }
        }
    }
    let img = image::RgbImage::from_raw(cw, ch, frame).expect("crop size");
    img.save(&out_path)?;
    eprintln!("wrote {out_path}");
    Ok(())
}
