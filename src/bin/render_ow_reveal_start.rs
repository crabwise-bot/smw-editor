//! Screenshot: Mario/Luigi overworld starting positions on the real main map.
//!
//! Renders submap 0 from the real ROM with all destruction events active
//! (the same emulated path the editor's preview uses), then draws the M/L
//! start markers at the real `$009EF0` starting positions parsed by
//! `smwe_rom::overworld::start_positions` — the same marker look as the
//! editor's canvas overlay (dark disc, colored ring, letter).
//!
//! Usage: `cargo run --bin render_ow_reveal_start -- --rom=smw.smc
//! --out=docs/screenshots/ow-start-positions.png`
//!
//! Never commits or copies the ROM; it is only read for rendering.

use std::{env, path::Path, sync::Arc};

use ab_glyph::{Font, FontRef, Glyph, Point, PxScale, ScaleFont};
use image::{ImageBuffer, Rgb};
use smwe_emu::{emu::CheckedMem, rom::Rom as EmuRom, Cpu};
use smwe_rom::overworld::start_positions::OverworldStartPositions;

const VRAM_L1_TILEMAP_BASE: usize = 0x2000 * 2;
const VRAM_L2_TILEMAP_BASE: usize = 0x3000 * 2;
const OW_COLS: u32 = 64;
const OW_ROWS: u32 = 64;

use smw_editor::render_util::render_tile;

const HEADER: u32 = 58;

fn main() {
    let args: Vec<String> = env::args().collect();
    let rom_path = args
        .iter()
        .find_map(|a| a.strip_prefix("--rom="))
        .map(Path::new)
        .or_else(|| args.iter().skip(1).find(|a| !a.starts_with("--")).map(|a| Path::new(a)))
        .unwrap_or_else(|| Path::new("smw.smc"));
    let output = args.iter().find_map(|a| a.strip_prefix("--out=")).unwrap_or("ow_start_positions.png");

    let raw = std::fs::read(rom_path).expect("cannot read smw.smc");
    let rom_bytes = if raw.len() % 0x400 == 0x200 { raw[0x200..].to_vec() } else { raw };

    // Real starting positions straight from the ROM table.
    let starts = OverworldStartPositions::parse(&rom_bytes, 0).expect("start positions parse");
    let (m, l) = (starts.mario, starts.luigi);

    let mut emu_rom = EmuRom::new(rom_bytes.clone());
    emu_rom.load_symbols(include_str!("../../symbols/SMW_U.sym"));
    let mut cpu = Cpu::new(CheckedMem::new(Arc::new(emu_rom)));
    for addr in 0x1F02u32..=0x1F60 {
        cpu.mem.store_u8(addr, 0xFF);
    }
    smwe_emu::emu::load_overworld(&mut cpu, 0);

    // 512x512: the main map is 32x32 Map16 blocks = 64x64 8x8 tiles.
    let (w, h) = (512u32, 512u32);
    let mut pixels = vec![0u8; (w * h * 3) as usize];
    render_bg_full(&cpu.mem.vram, VRAM_L2_TILEMAP_BASE, w, &cpu.mem.cgram, &mut pixels);
    render_bg_full(&cpu.mem.vram, VRAM_L1_TILEMAP_BASE, w, &cpu.mem.cgram, &mut pixels);
    let map = ImageBuffer::<Rgb<u8>, _>::from_raw(w, h, pixels).expect("image buffer");

    let mut out =
        ImageBuffer::from_fn(
            w,
            h + HEADER,
            |x, y| {
                if y < HEADER {
                    Rgb([18, 18, 24])
                } else {
                    *map.get_pixel(x, y - HEADER)
                }
            },
        );

    draw_text(
        &mut out,
        "Overworld starting positions (SNES $009EF0) — all events active",
        10,
        22,
        15.0,
        Rgb([235, 235, 240]),
    );
    let detail = format!(
        "M/L: submap {} tile ({},{}) px ({},{}) — vanilla: both markers overlap",
        m.submap, m.tile_x, m.tile_y, m.pixel_x, m.pixel_y
    );
    draw_text(&mut out, &detail, 10, 46, 13.0, Rgb([170, 170, 178]));

    // Markers use the main-map pixel coordinates, exactly like the editor's
    // canvas overlay.
    draw_start_marker(&mut out, m.pixel_x as u32, m.pixel_y as u32 + HEADER, "M", Rgb([255, 90, 90]));
    draw_start_marker(&mut out, l.pixel_x as u32, l.pixel_y as u32 + HEADER, "L", Rgb([90, 230, 120]));

    out.save(output).expect("save png");
    println!("wrote {output}");
}

fn tilemap_vram_addr(base: usize, col: u32, row: u32) -> usize {
    let quadrant = ((row / 32) * 2) + (col / 32);
    let sub_row = row % 32;
    let sub_col = col % 32;
    let quadrant_offset = quadrant * 32 * 32 * 2;
    let idx = quadrant_offset + ((sub_row * 32 + sub_col) * 2);
    base + idx as usize
}

fn render_bg_full(vram: &[u8], tilemap_base: usize, width: u32, cgram: &[u8], pixels: &mut [u8]) {
    for row in 0..OW_ROWS {
        for col in 0..OW_COLS {
            let addr = tilemap_vram_addr(tilemap_base, col, row);
            let t0 = vram[addr] as u16;
            let t1 = vram[addr + 1] as u16;
            render_tile(
                vram,
                cgram,
                (t0 | ((t1 & 3) << 8)) as usize,
                ((t1 >> 2) & 7) as usize,
                (t1 & 0x40) != 0,
                (t1 & 0x80) != 0,
                col * 8,
                row * 8,
                width,
                pixels,
            );
        }
    }
}

fn draw_disc(img: &mut ImageBuffer<Rgb<u8>, Vec<u8>>, cx: u32, cy: u32, r: u32, color: Rgb<u8>) {
    for dy in -(r as i32)..=r as i32 {
        for dx in -(r as i32)..=r as i32 {
            if dx * dx + dy * dy <= (r * r) as i32 {
                let (x, y) = (cx as i32 + dx, cy as i32 + dy);
                if x >= 0 && y >= 0 && (x as u32) < img.width() && (y as u32) < img.height() {
                    img.put_pixel(x as u32, y as u32, color);
                }
            }
        }
    }
}

fn draw_ring(img: &mut ImageBuffer<Rgb<u8>, Vec<u8>>, cx: u32, cy: u32, r: u32, color: Rgb<u8>) {
    let r2 = (r * r) as i32;
    let inner = ((r.saturating_sub(2)) * (r.saturating_sub(2))) as i32;
    for dy in -(r as i32)..=r as i32 {
        for dx in -(r as i32)..=r as i32 {
            let d2 = dx * dx + dy * dy;
            if d2 <= r2 && d2 >= inner {
                let (x, y) = (cx as i32 + dx, cy as i32 + dy);
                if x >= 0 && y >= 0 && (x as u32) < img.width() && (y as u32) < img.height() {
                    img.put_pixel(x as u32, y as u32, color);
                }
            }
        }
    }
}

fn draw_start_marker(img: &mut ImageBuffer<Rgb<u8>, Vec<u8>>, cx: u32, cy: u32, letter: &str, color: Rgb<u8>) {
    draw_disc(img, cx, cy, 10, Rgb([10, 10, 12]));
    draw_ring(img, cx, cy, 10, color);
    draw_text(img, letter, cx as i32 - 5, cy as i32 + 6, 15.0, color);
}

fn draw_text(img: &mut ImageBuffer<Rgb<u8>, Vec<u8>>, text: &str, x: i32, baseline_y: i32, px: f32, color: Rgb<u8>) {
    let data = std::fs::read("/usr/share/fonts/truetype/dejavu/DejaVuSans-Bold.ttf").expect("font");
    let leaked: &'static [u8] = Box::leak(data.into_boxed_slice());
    let font = FontRef::try_from_slice(leaked).expect("font parse");
    let scaled = font.as_scaled(PxScale::from(px));
    let mut caret_x = x as f32;
    let mut prev = None;
    for ch in text.chars() {
        let id = font.glyph_id(ch);
        if let Some(p) = prev {
            caret_x += scaled.kern(p, id);
        }
        let glyph = Glyph { id, scale: PxScale::from(px), position: Point { x: caret_x, y: baseline_y as f32 } };
        if let Some(o) = scaled.outline_glyph(glyph) {
            let bb = o.px_bounds();
            o.draw(|gx, gy, v| {
                let px_x = bb.min.x as i32 + gx as i32;
                let px_y = bb.min.y as i32 + gy as i32;
                if px_x >= 0 && px_y >= 0 {
                    let (px_x, px_y) = (px_x as u32, px_y as u32);
                    if px_x < img.width() && px_y < img.height() && v > 0.4 {
                        img.put_pixel(px_x, px_y, color);
                    }
                }
            });
        }
        prev = Some(id);
        caret_x += scaled.h_advance(id);
    }
}
