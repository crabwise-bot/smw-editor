// Headless screenshot for the "Mark exit-enabled tiles" (Lunar Magic
// v3.31) view option: renders a real-ROM level and paints the same green
// markers the editor overlay draws, computed from the WRAM block maps plus
// the acts-like table through smwe_rom::block_behavior::is_exit_enabled.
//   --rom=PATH --level=0x105 --out=docs/screenshots/exit-enabled-tiles.png
// If --level is omitted, the first level (0x000..=0x1FF) with at least 10
// exit-enabled tiles is picked automatically.
use std::{collections::HashMap, sync::Arc};

use anyhow::Context;
use smw_editor::render_util::{fill_rect_raw, render_layer, stroke_rect};
use smwe_emu::{emu::CheckedMem, rom::Rom as EmuRom, Cpu};
use smwe_rom::{block_behavior::is_exit_enabled, map16_expanded::act_as_of};

fn arg(name: &str, default: &str) -> String {
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        if a == name {
            return args.next().unwrap_or_else(|| default.to_string());
        }
    }
    default.to_string()
}

fn has_arg(name: &str) -> bool {
    std::env::args().any(|a| a == name)
}

struct LevelGeom {
    vertical:   bool,
    has_layer2: bool,
    scr_len:    u32,
    scr_size:   u32,
    width:      u32,
    height:     u32,
    level_mode: u8,
}

fn load_cpu(rom_bytes: &[u8], level: u16) -> Cpu {
    let mut emu_rom = EmuRom::new(rom_bytes.to_vec());
    emu_rom.load_symbols(include_str!("../../symbols/SMW_U.sym"));
    let mut cpu = Cpu::new(CheckedMem::new(Arc::new(emu_rom)));
    smwe_emu::emu::decompress_sublevel(&mut cpu, level);
    cpu
}

fn geom_of(cpu: &mut Cpu) -> LevelGeom {
    let vertical = cpu.mem.load_u8(0x5B) & 1 != 0;
    let level_mode = cpu.mem.load_u8(0x1925);
    let renderer_table = cpu.mem.cart.resolve("CODE_058955").unwrap() + 9;
    let renderer = cpu.mem.load_u24(renderer_table + (level_mode as u32) * 3);
    let l2_renderers = [cpu.mem.cart.resolve("CODE_058B8D"), cpu.mem.cart.resolve("CODE_058C71")];
    let has_layer2 = l2_renderers.contains(&Some(renderer));
    let scr_len = match (vertical, has_layer2) {
        (false, false) => 0x20,
        (true, false) => 0x1C,
        (false, true) => 0x10,
        (true, true) => 0x0E,
    } as u32;
    let scr_size = if vertical { 16 * 32 } else { 16 * 27 };
    let (width, height) = if vertical { (32 * 16, scr_len * 16 * 16) } else { (scr_len * 16 * 16, 27 * 16) };
    LevelGeom { vertical, has_layer2, scr_len, scr_size, width, height, level_mode }
}

/// Map16 block ID at tile (tx, ty) on the block map starting at `lo_base`
/// (hi plane is 0x10000 above). Same screen/tile math the editor uses.
fn block_at(cpu: &mut Cpu, g: &LevelGeom, tx: u32, ty: u32, lo_base: u32) -> u16 {
    let idx = if g.vertical {
        let sub_x = tx / 16;
        let sub_y = ty / 32;
        let screen = sub_y * 2 + sub_x;
        screen * g.scr_size + (ty % 32) * 16 + (tx % 16)
    } else {
        (tx / 16) * g.scr_size + ty * 16 + (tx % 16)
    };
    cpu.mem.load_u8(lo_base + idx) as u16 | (((cpu.mem.load_u8(lo_base + 0x10000 + idx) as u16) & 0x3F) << 8)
}

/// All exit-enabled tiles in the level, in tile coordinates.
fn exit_tiles(cpu: &mut Cpu, acts: &HashMap<u16, u16>, g: &LevelGeom) -> Vec<(u32, u32)> {
    let l2_active = g.level_mode == 0x01 && g.has_layer2;
    let l2_off = g.scr_len * g.scr_size;
    let (tw, th) = (g.width / 16, g.height / 16);
    let mut out = Vec::new();
    for ty in 0..th {
        for tx in 0..tw {
            let id = block_at(cpu, g, tx, ty, 0x7EC800);
            if id != 0 && is_exit_enabled(act_as_of(acts, id), g.level_mode) {
                out.push((tx, ty));
                continue;
            }
            if l2_active {
                let id2 = block_at(cpu, g, tx, ty, 0x7EC800 + l2_off);
                if id2 != 0 && is_exit_enabled(act_as_of(acts, id2), g.level_mode) {
                    out.push((tx, ty));
                }
            }
        }
    }
    out
}

fn main() -> anyhow::Result<()> {
    let rom_path = arg("--rom", "smw.smc");
    let out_path = arg("--out", "docs/screenshots/exit-enabled-tiles.png");

    let raw = std::fs::read(&rom_path).with_context(|| format!("read {rom_path}"))?;
    let rom_bytes: Vec<u8> = if raw.len() % 0x400 == 0x200 { raw[0x200..].to_vec() } else { raw };
    let acts = smwe_rom::map16_expanded::read_acts_table(&rom_bytes, 0).unwrap_or_default();
    eprintln!("acts table: {} non-identity entries", acts.len());

    let level: u16 = if has_arg("--level") {
        u16::from_str_radix(arg("--level", "0x105").trim_start_matches("0x"), 16).unwrap_or(0x105)
    } else {
        // Pick the densest cluster: most exit-enabled tiles per bounding-box
        // tile, among levels with at least --min of them (default 10).
        let min: usize = arg("--min", "10").parse().unwrap_or(10);
        let mut best: Option<(u16, f32)> = None;
        for lvl in 0x000..=0x1FFu16 {
            let mut cpu = load_cpu(&rom_bytes, lvl);
            let g = geom_of(&mut cpu);
            let marked = exit_tiles(&mut cpu, &acts, &g);
            if marked.len() < min {
                continue;
            }
            let (bx0, by0, bx1, by1) =
                marked.iter().fold((u32::MAX, u32::MAX, 0u32, 0u32), |(x0, y0, x1, y1), &(tx, ty)| {
                    (x0.min(tx), y0.min(ty), x1.max(tx + 1), y1.max(ty + 1))
                });
            let area = ((bx1 - bx0).max(1) * (by1 - by0).max(1)) as f32;
            let density = marked.len() as f32 / area;
            eprintln!(
                "candidate {lvl:#05X}: {} tiles, bbox {}x{}, density {density:.3}",
                marked.len(),
                bx1 - bx0,
                by1 - by0
            );
            if best.is_none_or(|(_, d)| density > d) {
                best = Some((lvl, density));
            }
        }
        let (picked, density) = best.context("no level with >= 10 exit-enabled tiles found")?;
        eprintln!("auto-picked level {picked:#05X} (density {density:.3})");
        picked
    };

    // ── Render the level (same WRAM state the editor overlay reads) ──
    let mut cpu = load_cpu(&rom_bytes, level);
    let g = geom_of(&mut cpu);
    let marked = exit_tiles(&mut cpu, &acts, &g);
    eprintln!("level {level:#05X}: {} exit-enabled tiles", marked.len());
    if std::env::args().any(|a| a == "--dump") {
        for &(tx, ty) in &marked {
            let id = block_at(&mut cpu, &g, tx, ty, 0x7EC800);
            eprintln!("  tile ({tx},{ty}): block {id:#05X} act-as {:#05X}", act_as_of(&acts, id));
        }
    }
    if marked.is_empty() {
        anyhow::bail!("level {level:#05X} has no exit-enabled tiles; pick another level");
    }

    let mut pixels = vec![0u8; (g.width * g.height * 3) as usize];
    render_layer(&mut cpu, false, g.width, &mut pixels);
    render_layer(&mut cpu, true, g.width, &mut pixels);

    for &(tx, ty) in &marked {
        fill_rect_raw(&mut pixels, g.width, tx * 16, ty * 16, 16, 16, [70, 220, 110, 70]);
        stroke_rect(&mut pixels, g.width, tx * 16, ty * 16, 16, 16, [70, 220, 110], 2);
    }

    // ── Crop around the marked region ──
    let m = 64u32;
    let (bx0, by0, bx1, by1) = marked.iter().fold((u32::MAX, u32::MAX, 0u32, 0u32), |(x0, y0, x1, y1), &(tx, ty)| {
        (x0.min(tx), y0.min(ty), x1.max(tx + 1), y1.max(ty + 1))
    });
    let mut cx0 = bx0.saturating_mul(16).saturating_sub(m);
    let mut cy0 = by0.saturating_mul(16).saturating_sub(m);
    let mut cx1 = (bx1 * 16 + m).min(g.width);
    let mut cy1 = (by1 * 16 + m).min(g.height);
    // Minimum 640x360 crop for readability, centered on the marked region.
    for (c0, c1, full, min) in [(&mut cx0, &mut cx1, g.width, 640u32), (&mut cy0, &mut cy1, g.height, 360u32)] {
        let w = *c1 - *c0;
        if w < min {
            let need = (min - w) / 2;
            *c0 = c0.saturating_sub(need);
            *c1 = (*c0 + min).min(full);
            *c0 = c1.saturating_sub(min);
        }
    }
    let (cw, ch) = (cx1 - cx0, cy1 - cy0);
    let crop: Vec<u8> = (cy0..cy1)
        .flat_map(|y| {
            let row = (y * g.width * 3) as usize;
            let col = (cx0 * 3) as usize;
            pixels[row + col..row + col + (cw * 3) as usize].to_vec()
        })
        .collect();
    let img = image::RgbImage::from_raw(cw, ch, crop).expect("crop size");
    img.save(&out_path)?;
    eprintln!("wrote {out_path} ({cw}x{ch})");
    Ok(())
}
