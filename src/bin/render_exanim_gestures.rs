//! Headless renderer for the LM v3.50 ExAnimation frame gestures (backlog #42).
//!
//! Loads a level through the real emulator path, then renders one demo
//! frame's per-step source tiles — before the gesture, and after each of the
//! four modifier gestures — as rows of real 4bpp VRAM tile graphics (CGRAM
//! palette row 0). The four "after" states are produced by the real model
//! methods (`insert_frame_at_start`, `delete_frame_at_start`,
//! `rotate_steps_right`, `rotate_steps_left`), so the pixels show exactly
//! what the gestures do to the frame values.
//!
//! Row labels are added afterwards with PIL (the PR body explains them); the
//! binary renders only the unlabeled tile grid and prints the per-row step
//! values to stdout for the labeling script.
//!
//! Usage:
//!   render_exanim_gestures --rom=smw.smc --level=0x105 --out=/tmp/exanim_gestures.png

use std::{env, path::Path, sync::Arc};

use image::{ImageBuffer, Rgb};
use smw_editor::render_util::read_color;
use smwe_emu::{emu::CheckedMem, rom::Rom as EmuRom, Cpu};
use smwe_rom::{
    exanimation::{ExAnimFrame, ExAnimFrameKind, ExAnimTrigger},
    graphics::gfx_file::Tile,
};

/// 2x zoom on 8x8 tiles.
const CELL: u32 = 16;
const UNITS_PER_STEP: usize = 2;
const ROW_GAP: u32 = 6;

fn hex(s: &str) -> u16 {
    u16::from_str_radix(s.trim().trim_start_matches("0x").trim_start_matches('$'), 16).unwrap_or(0)
}

fn main() {
    let args: Vec<String> = env::args().collect();
    let level = args.iter().find_map(|a| a.strip_prefix("--level=")).map(|s| hex(s)).unwrap_or(0x105);
    let rom_path =
        args.iter().find_map(|a| a.strip_prefix("--rom=")).map(Path::new).unwrap_or_else(|| Path::new("smw.smc"));
    let out = args.iter().find_map(|a| a.strip_prefix("--out=")).unwrap_or("/tmp/exanim_gestures.png");

    let raw = std::fs::read(rom_path).expect("cannot read ROM");
    let rom_bytes = if raw.len() % 0x400 == 0x200 { raw[0x200..].to_vec() } else { raw };
    let mut emu_rom = EmuRom::new(rom_bytes);
    emu_rom.load_symbols(include_str!("../../symbols/SMW_U.sym"));
    let mut cpu = Cpu::new(CheckedMem::new(Arc::new(emu_rom)));
    smwe_emu::emu::decompress_sublevel(&mut cpu, level);
    smwe_emu::emu::fetch_anim_frame(&mut cpu);

    // Demo frame: 4 steps × 2 units, eight visually distinct source tiles
    // (red/orange/green/blue/yellow tiles from the level's real VRAM).
    let demo = ExAnimFrame {
        kind:            ExAnimFrameKind::Line8x8,
        dest:            0x1000,
        speed:           1,
        trigger:         ExAnimTrigger::Always,
        frames:          4,
        units_per_frame: UNITS_PER_STEP as u8,
        payload:         vec![0x46B0, 0x4960, 0x0910, 0x4770, 0x1650, 0x1E90, 0x0900, 0x4F40],
    };

    let mut rows: Vec<(&str, ExAnimFrame)> = Vec::new();
    rows.push(("before", demo.clone()));
    let mut insert = demo.clone();
    insert.insert_frame_at_start();
    rows.push(("ctrl_down_insert", insert));
    let mut delete = demo.clone();
    delete.delete_frame_at_start();
    rows.push(("ctrl_up_delete", delete));
    let mut rot_r = demo.clone();
    rot_r.rotate_steps_right();
    rows.push(("ctrlshift_down_rotateright", rot_r));
    let mut rot_l = demo.clone();
    rot_l.rotate_steps_left();
    rows.push(("ctrlshift_up_rotateleft", rot_l));

    let max_steps = rows.iter().map(|(_, f)| f.frames as u32).max().unwrap_or(1);
    let w = max_steps * UNITS_PER_STEP as u32 * CELL;
    let h = rows.len() as u32 * CELL + (rows.len() as u32 - 1) * ROW_GAP;
    let mut img: ImageBuffer<Rgb<u8>, Vec<u8>> = ImageBuffer::new(w, h);

    let mut pal = [[0u8; 3]; 16];
    for i in 0..16 {
        pal[i] = read_color(&cpu.mem.cgram, i);
    }

    for (r, (name, frame)) in rows.iter().enumerate() {
        let y0 = r as u32 * (CELL + ROW_GAP);
        let steps: Vec<String> = (0..frame.frames as usize)
            .map(|s| {
                frame.payload[s * UNITS_PER_STEP..(s + 1) * UNITS_PER_STEP]
                    .iter()
                    .map(|v| format!("${v:04X}"))
                    .collect::<Vec<_>>()
                    .join(" ")
            })
            .collect();
        println!("{name}: steps={} [{}]", frame.frames, steps.join(" | "));
        for s in 0..frame.frames as usize {
            for u in 0..UNITS_PER_STEP {
                let word = frame.payload[s * UNITS_PER_STEP + u] as usize;
                let off = word * 2;
                let x0 = (s * UNITS_PER_STEP + u) as u32 * CELL;
                draw_tile(&cpu.mem.vram, off, &pal, &mut img, x0, y0);
            }
        }
    }
    img.save(out).expect("save grid");
    println!("wrote {out}");
}

fn draw_tile(
    vram: &[u8], word_off: usize, pal: &[[u8; 3]; 16], img: &mut ImageBuffer<Rgb<u8>, Vec<u8>>, x0: u32, y0: u32,
) {
    let blank = word_off + 32 > vram.len();
    let tile = if blank { None } else { Tile::from_4bpp(&vram[word_off..word_off + 32]).ok().map(|(_, t)| t) };
    for py in 0..8u32 {
        for px in 0..8u32 {
            let c = match &tile {
                Some(t) => pal[(t.color_indices[(py * 8 + px) as usize] & 0xF) as usize],
                None => [64, 0, 64], // magenta = out of range (shouldn't happen)
            };
            let rgb = Rgb(c);
            for dy in 0..2 {
                for dx in 0..2 {
                    img.put_pixel(x0 + px * 2 + dx, y0 + py * 2 + dy, rgb);
                }
            }
        }
    }
}
