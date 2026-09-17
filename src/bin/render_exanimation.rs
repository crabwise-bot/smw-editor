//! Headless ExAnimation preview renderer.
//!
//! Loads a level through the real emulator path, applies a demo (or
//! CLI-specified) [`smwe_rom::exanimation::ExAnimation`] one tick per output
//! frame, and writes PNGs. Assemble them into a GIF with PIL
//! (`duration=133, loop=0`) for PR screenshots.
//!
//! Usage:
//!   render_exanimation --level=0x105 --rom=smw.smc --out=/tmp/exanim --frames=8 \
//!     --line=0x1000:0x2000,0x2010,0x2020 --palrot=0x00:0x7C1F,0x03E0
//!   render_exanimation --dump-atlas=/tmp/vram_atlas.png --level=0x105 --rom=smw.smc
//!
//! `--line=DEST:S0,S1,...` adds a Line8x8 frame: each step copies one 8x8
//! tile's graphics from the listed VRAM word addresses to DEST.
//! `--palrot=DEST:C0,C1,...` adds a palette-rotate frame on CGRAM DEST.

use std::{env, path::Path, sync::Arc};

use image::{ImageBuffer, Rgb};
use smw_editor::render_util::{read_color, render_layer};
use smwe_emu::{emu::CheckedMem, rom::Rom as EmuRom, Cpu};
use smwe_rom::{
    exanimation::{apply_tick, ExAnimFrame, ExAnimFrameKind, ExAnimTrigger, ExAnimation},
    graphics::gfx_file::Tile,
};

fn hex(s: &str) -> u16 {
    u16::from_str_radix(s.trim().trim_start_matches("0x").trim_start_matches('$'), 16).unwrap_or(0)
}

fn main() {
    let args: Vec<String> = env::args().collect();
    let level = args.iter().find_map(|a| a.strip_prefix("--level=")).map(|s| hex(s)).unwrap_or(0x105);
    let rom_path =
        args.iter().find_map(|a| a.strip_prefix("--rom=")).map(Path::new).unwrap_or_else(|| Path::new("smw.smc"));
    let out_dir = args.iter().find_map(|a| a.strip_prefix("--out=")).unwrap_or("/tmp/exanim");
    let nframes: usize =
        args.iter().find_map(|a| a.strip_prefix("--frames=")).and_then(|s| s.parse().ok()).unwrap_or(8);

    let raw = std::fs::read(rom_path).expect("cannot read ROM");
    let rom_bytes = if raw.len() % 0x400 == 0x200 { raw[0x200..].to_vec() } else { raw };
    let mut emu_rom = EmuRom::new(rom_bytes);
    emu_rom.load_symbols(include_str!("../../symbols/SMW_U.sym"));
    let mut cpu = Cpu::new(CheckedMem::new(Arc::new(emu_rom)));

    smwe_emu::emu::decompress_sublevel(&mut cpu, level);
    smwe_emu::emu::fetch_anim_frame(&mut cpu);

    if let Some(atlas_path) = args.iter().find_map(|a| a.strip_prefix("--dump-atlas=")) {
        dump_vram_atlas(&cpu, atlas_path);
        return;
    }

    // Build the demo animation from CLI args (or a built-in default).
    let mut frames: Vec<ExAnimFrame> = Vec::new();
    for spec in args.iter().filter_map(|a| a.strip_prefix("--line=")) {
        let (dest, srcs) = spec.split_once(':').expect("--line=DEST:S0,S1,...");
        let srcs: Vec<u16> = srcs.split(',').map(hex).collect();
        let n = srcs.len().max(1) as u16;
        frames.push(ExAnimFrame {
            kind:            ExAnimFrameKind::Line8x8,
            dest:            hex(dest),
            speed:           0,
            trigger:         ExAnimTrigger::Always,
            frames:          n,
            units_per_frame: 1,
            payload:         srcs,
        });
    }
    for spec in args.iter().filter_map(|a| a.strip_prefix("--palrot=")) {
        let (dest, colors) = spec.split_once(':').expect("--palrot=DEST:C0,C1,...");
        let ring: Vec<u16> = colors.split(',').map(hex).collect();
        let n = ring.len().max(1);
        frames.push(ExAnimFrame {
            kind:            ExAnimFrameKind::PaletteRotate,
            dest:            hex(dest),
            speed:           0,
            trigger:         ExAnimTrigger::Always,
            frames:          n as u16,
            units_per_frame: n as u8,
            payload:         ring,
        });
    }
    if frames.is_empty() {
        eprintln!("no --line= or --palrot= given and no default; nothing to animate");
        std::process::exit(2);
    }
    let anim = ExAnimation { frames, disable_original: false };

    let vertical = cpu.mem.load_u8(0x5B) & 1 != 0;
    let (width, height) = if vertical { (32 * 16, 28 * 16 * 16) } else { (32 * 16 * 16, 27 * 16) };
    // Render at full width (render_layer maps tilemap pixels 1:1 into the
    // buffer); crop down to GIF size afterwards with PIL.

    std::fs::create_dir_all(out_dir).expect("create out dir");
    for i in 0..nframes {
        apply_tick(&anim, i as u64, &mut cpu.mem.vram, &mut cpu.mem.cgram);
        let mut pixels = vec![0u8; (width * height * 3) as usize];
        {
            let backdrop = read_color(&cpu.mem.cgram, 0);
            for px in pixels.chunks_exact_mut(3) {
                px.copy_from_slice(&backdrop);
            }
        }
        render_layer(&mut cpu, true, width, &mut pixels);
        render_layer(&mut cpu, false, width, &mut pixels);
        let path = format!("{out_dir}/frame_{i:02}.png");
        ImageBuffer::<Rgb<u8>, _>::from_raw(width, height, pixels).expect("image buffer").save(&path).unwrap();
        println!("wrote {path}");
    }
}

/// Dump the 2048 4bpp VRAM tiles as a labeled atlas (64 cols), colored with
/// CGRAM palette row 0, for picking demo source/dest tiles.
fn dump_vram_atlas(cpu: &Cpu, path: &str) {
    const COLS: usize = 64;
    let tile_count = cpu.mem.vram.len() / 32;
    let rows = tile_count.div_ceil(COLS);
    let (w, h) = (COLS * 8, rows * 8);
    let mut img: ImageBuffer<Rgb<u8>, Vec<u8>> = ImageBuffer::new(w as u32, h as u32);
    let mut pal = [[0u8; 3]; 16];
    for i in 0..16 {
        pal[i] = read_color(&cpu.mem.cgram, i);
    }
    for t in 0..tile_count {
        let bytes = &cpu.mem.vram[t * 32..(t + 1) * 32];
        let Ok((_, tile)) = Tile::from_4bpp(bytes) else { continue };
        let (tx, ty) = ((t % COLS) * 8, (t / COLS) * 8);
        for (pi, &ci) in tile.color_indices.iter().enumerate() {
            let c = pal[(ci & 0xF) as usize];
            img.put_pixel((tx + pi % 8) as u32, (ty + pi / 8) as u32, Rgb(c));
        }
    }
    // Red gridlines every 8 tiles so tile numbers are easy to read off.
    for gx in (0..COLS).step_by(8) {
        for y in 0..h {
            img.put_pixel((gx * 8) as u32, y as u32, Rgb([255, 0, 0]));
        }
    }
    img.save(path).expect("save atlas");
    println!("wrote {path} ({tile_count} tiles)");
}
