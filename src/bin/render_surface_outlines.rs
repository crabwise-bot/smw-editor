// Headless screenshot for the "Tile Surface Outlines" (Lunar Magic v3.00;
// water tiles v3.70) view option: renders a real-ROM level and paints the same
// outlines the editor overlay draws — white for solid tiles, the game's own
// SlopeHeights surface polyline for slopes, blue for water, red for hurt
// blocks — computed from the WRAM block maps plus the acts-like table through
// smwe_rom::tile_surface::surface_kind.
//   --rom=PATH --level=0x105 --out=docs/screenshots/surface-outlines.png
// If --level is omitted, the level (0x000..=0x1FF) with the most outlined
// slope/water/hurt tiles is picked automatically.
use std::{collections::HashMap, sync::Arc};

use anyhow::Context;
use smw_editor::{
    level_png_export::{block_at, level_geom_of, LevelGeom, BLOCK_MAP_BASE},
    render_util::{render_layer, stroke_rect},
};
use smwe_emu::{emu::CheckedMem, rom::Rom as EmuRom, Cpu};
use smwe_rom::{
    map16_expanded::act_as_of,
    tile_surface::{surface_kind, SurfaceKind},
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

fn has_arg(name: &str) -> bool {
    std::env::args().any(|a| a == name)
}

fn load_cpu(rom_bytes: &[u8], level: u16) -> Cpu {
    let mut emu_rom = EmuRom::new(rom_bytes.to_vec());
    emu_rom.load_symbols(include_str!("../../symbols/SMW_U.sym"));
    let mut cpu = Cpu::new(CheckedMem::new(Arc::new(emu_rom)));
    smwe_emu::emu::decompress_sublevel(&mut cpu, level);
    cpu
}

/// Bresenham line on the raw RGB buffer.
fn draw_line(pixels: &mut [u8], width: u32, x0: i32, y0: i32, x1: i32, y1: i32, rgb: [u8; 3]) {
    let (mut x, mut y) = (x0, y0);
    let (dx, dy) = ((x1 - x0).abs(), -(y1 - y0).abs());
    let (sx, sy) = ((x1 >= x0) as i32 * 2 - 1, (y1 >= y0) as i32 * 2 - 1);
    let mut err = dx + dy;
    let h = pixels.len() as i32 / (width as i32 * 3);
    loop {
        if x >= 0 && y >= 0 && (x as u32) < width && (y as u32) < h as u32 {
            let o = ((y as u32 * width + x as u32) * 3) as usize;
            pixels[o..o + 3].copy_from_slice(&rgb);
        }
        if x == x1 && y == y1 {
            break;
        }
        let e2 = 2 * err;
        if e2 >= dy {
            err += dy;
            x += sx;
        }
        if e2 <= dx {
            err += dx;
            y += sy;
        }
    }
}

/// Surface outline per tile: (tx, ty, kind).
fn outlined_tiles(
    cpu: &mut Cpu, acts: &HashMap<u16, u16>, g: &LevelGeom, object_tileset: u8,
) -> Vec<(u32, u32, SurfaceKind)> {
    let (tw, th) = (g.width / 16, g.height / 16);
    let mut out = Vec::new();
    for ty in 0..th {
        for tx in 0..tw {
            let id = block_at(cpu, g, tx, ty, BLOCK_MAP_BASE);
            if id == 0 {
                continue;
            }
            if let Some(kind) = surface_kind(act_as_of(acts, id), object_tileset) {
                out.push((tx, ty, kind));
            }
        }
    }
    out
}

fn main() -> anyhow::Result<()> {
    let rom_path = arg("--rom", "smw.smc");
    let out_path = arg("--out", "docs/screenshots/surface-outlines.png");

    let raw = std::fs::read(&rom_path).with_context(|| format!("read {rom_path}"))?;
    let rom_bytes: Vec<u8> = if raw.len() % 0x400 == 0x200 { raw[0x200..].to_vec() } else { raw };
    let acts = smwe_rom::map16_expanded::read_acts_table(&rom_bytes, 0).unwrap_or_default();
    eprintln!("acts table: {} non-identity entries", acts.len());

    // The object tileset comes from the level header; read per level when
    // auto-picking. Header byte 4 low nibble (see headers.rs fg_bg_gfx).
    let tileset_of = |level: u16| -> u8 {
        let addr = 0x05F600 + level as usize; // byte 4 of the primary header
        rom_bytes.get(addr).copied().unwrap_or(0) & 0x0F
    };

    let level: u16 = if has_arg("--level") {
        u16::from_str_radix(arg("--level", "0x105").trim_start_matches("0x"), 16).unwrap_or(0x105)
    } else {
        // Pick the level with the most slope/water/hurt outlines (the
        // interesting ones); solid-only levels are boring.
        let mut best: Option<(u16, usize)> = None;
        for lvl in 0x000..=0x1FFu16 {
            let mut cpu = load_cpu(&rom_bytes, lvl);
            let g = level_geom_of(&mut cpu);
            let marked = outlined_tiles(&mut cpu, &acts, &g, tileset_of(lvl));
            let interesting = marked.iter().filter(|(_, _, k)| !matches!(k, SurfaceKind::Solid)).count();
            if interesting >= 12 && best.is_none_or(|(_, n)| interesting > n) {
                eprintln!("candidate {lvl:#05X}: {interesting} slope/water/hurt tiles");
                best = Some((lvl, interesting));
            }
        }
        let (picked, n) = best.context("no level with >= 12 slope/water/hurt tiles found")?;
        eprintln!("auto-picked level {picked:#05X} ({n} slope/water/hurt tiles)");
        picked
    };

    // ── Render the level (same WRAM state the editor overlay reads) ──
    let mut cpu = load_cpu(&rom_bytes, level);
    let g = level_geom_of(&mut cpu);
    let tileset = tileset_of(level);
    let marked = outlined_tiles(&mut cpu, &acts, &g, tileset);
    let counts = marked.iter().fold([0; 4], |mut c, (_, _, k)| {
        c[match k {
            SurfaceKind::Solid => 0,
            SurfaceKind::Slope(_) => 1,
            SurfaceKind::Water => 2,
            SurfaceKind::Hurt => 3,
        }] += 1;
        c
    });
    eprintln!(
        "level {level:#05X} (tileset {tileset:#04X}): {} outlined (solid {}, slope {}, water {}, hurt {})",
        marked.len(),
        counts[0],
        counts[1],
        counts[2],
        counts[3]
    );
    if has_arg("--dump") {
        for &(tx, ty, kind) in &marked {
            let id = block_at(&mut cpu, &g, tx, ty, BLOCK_MAP_BASE);
            eprintln!("  tile ({tx},{ty}): block {id:#05X} act-as {:#05X} -> {kind:?}", act_as_of(&acts, id));
        }
    }
    if marked.is_empty() {
        anyhow::bail!("level {level:#05X} has no outlined tiles; pick another level");
    }

    let mut pixels = vec![0u8; (g.width * g.height * 3) as usize];
    render_layer(&mut cpu, false, g.width, &mut pixels);
    render_layer(&mut cpu, true, g.width, &mut pixels);

    const WHITE: [u8; 3] = [240, 240, 240];
    const BLUE: [u8; 3] = [90, 160, 255];
    const RED: [u8; 3] = [255, 70, 70];
    for &(tx, ty, kind) in &marked {
        let (px, py) = (tx * 16, ty * 16);
        match kind {
            SurfaceKind::Solid => stroke_rect(&mut pixels, g.width, px, py, 16, 16, WHITE, 2),
            SurfaceKind::Water => stroke_rect(&mut pixels, g.width, px, py, 16, 16, BLUE, 2),
            SurfaceKind::Hurt => stroke_rect(&mut pixels, g.width, px, py, 16, 16, RED, 2),
            SurfaceKind::Slope(heights) => {
                let mut prev: Option<(i32, i32)> = None;
                for (i, &h) in heights.iter().enumerate() {
                    let x = px as i32 + (i as i32 * 16 + 8) / 16;
                    let y = py as i32 + h.min(16) as i32;
                    if let Some((qx, qy)) = prev {
                        draw_line(&mut pixels, g.width, qx, qy, x, y, WHITE);
                    }
                    prev = Some((x, y));
                }
            }
        }
    }

    // ── Crop around the marked region ──
    let m = 64u32;
    let (bx0, by0, bx1, by1) =
        marked.iter().fold((u32::MAX, u32::MAX, 0u32, 0u32), |(x0, y0, x1, y1), &(tx, ty, _)| {
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
