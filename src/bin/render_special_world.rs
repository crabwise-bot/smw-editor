// Headless screenshot for the "Special World Passed" (Lunar Magic v1.10)
// view option.
//
// The game records a beaten Special World as bit 7 of two bytes in the
// OWLevelTileSettings scratch area (SMWDisX rammap.asm, base 0x1EA2):
//   +$48 (0x1EEA): CODE_00AD25 loads OWSpecialColors (autumn overworld
//                  palettes) instead of OverworldColors.
//   +$49 (0x1EEB): UploadGFXFile uploads GFX $31 (post-special-world koopa
//                  graphics) in place of GFX $01, and sprite init/draw code
//                  applies the koopa color swap.
// Setting both bits before the emulator's level/overworld init reproduces
// exactly what the game shows after Special World is beaten (verified: the
// surgical toggle refresh produces byte-identical VRAM/CGRAM to a fresh
// load with the flag set).
//
// Top row: the changed sprite-GFX VRAM tiles (koopas), plain vs passed.
// Bottom row: overworld submap 0, plain vs passed (autumn palette swap).
//   --rom=PATH --out=docs/screenshots/special-world-passed.png
use std::sync::Arc;

use image::{ImageBuffer, Rgb};
use smw_editor::render_util::render_tile;
use smwe_emu::{emu::CheckedMem, rom::Rom as EmuRom, Cpu};

const VRAM_L2_TILEMAP_BASE: usize = 0x2000 * 2;
const VRAM_L1_TILEMAP_BASE: usize = 0x3000 * 2;
const OW_COLS: u32 = 64;
const OW_ROWS: u32 = 64;

fn arg(name: &str, default: &str) -> String {
    let prefix = format!("{name}=");
    let mut args = std::env::args().skip(1).peekable();
    while let Some(a) = args.next() {
        if let Some(v) = a.strip_prefix(&prefix) {
            return v.to_string();
        }
        if a == name {
            return args.next().unwrap_or_else(|| default.to_string());
        }
    }
    default.to_string()
}

fn new_cpu(rom_bytes: &[u8]) -> Cpu {
    let mut emu_rom = EmuRom::new(rom_bytes.to_vec());
    emu_rom.load_symbols(include_str!("../../symbols/SMW_U.sym"));
    Cpu::new(CheckedMem::new(Arc::new(emu_rom)))
}

fn load_level(rom_bytes: &[u8], level: u16, passed: bool) -> Cpu {
    let mut cpu = new_cpu(rom_bytes);
    smwe_emu::emu::special_world::set_special_world_passed(&mut cpu, passed);
    smwe_emu::emu::decompress_sublevel(&mut cpu, level);
    cpu
}

fn load_overworld(rom_bytes: &[u8], submap: u8, passed: bool) -> Cpu {
    let mut cpu = new_cpu(rom_bytes);
    smwe_emu::emu::special_world::set_special_world_passed(&mut cpu, passed);
    smwe_emu::emu::load_overworld(&mut cpu, submap);
    cpu
}

fn tilemap_vram_addr(base: usize, col: u32, row: u32) -> usize {
    let quadrant = ((row / 32) * 2) + (col / 32);
    let sub_row = row % 32;
    let sub_col = col % 32;
    let quadrant_offset = quadrant * 32 * 32 * 2;
    let idx = quadrant_offset + ((sub_row * 32 + sub_col) * 2);
    base + idx as usize
}

/// Render VRAM tiles `first..first+count` as a `cols`-wide atlas at `scale`,
/// with the given sprite palette.
fn tile_atlas(cpu: &Cpu, first: usize, count: usize, cols: u32, palette: usize, scale: u32) -> Vec<u8> {
    let rows = count.div_ceil(cols as usize) as u32;
    let (w, h) = (cols * 8 * scale, rows * 8 * scale);
    let mut small = vec![0u8; (cols * 8 * rows * 8 * 3) as usize];
    for t in 0..count {
        render_tile(
            &cpu.mem.vram,
            &cpu.mem.cgram,
            first + t,
            palette,
            false,
            false,
            (t as u32 % cols) * 8,
            (t as u32 / cols) * 8,
            cols * 8,
            &mut small,
        );
    }
    // Nearest-neighbor upscale.
    let mut pixels = vec![0u8; (w * h * 3) as usize];
    for y in 0..h {
        for x in 0..w {
            let s = (((y / scale) * cols * 8 + x / scale) * 3) as usize;
            let d = ((y * w + x) * 3) as usize;
            pixels[d..d + 3].copy_from_slice(&small[s..s + 3]);
        }
    }
    pixels
}

fn render_ow(cpu: &Cpu, w: u32, h: u32) -> Vec<u8> {
    let mut pixels = vec![0u8; (w * h * 3) as usize];
    for &base in [VRAM_L2_TILEMAP_BASE, VRAM_L1_TILEMAP_BASE].iter() {
        for row in 0..OW_ROWS {
            for col in 0..OW_COLS {
                let addr = tilemap_vram_addr(base, col, row);
                let t0 = cpu.mem.vram[addr] as u16;
                let t1 = cpu.mem.vram[addr + 1] as u16;
                render_tile(
                    &cpu.mem.vram,
                    &cpu.mem.cgram,
                    (t0 | ((t1 & 3) << 8)) as usize,
                    ((t1 >> 2) & 7) as usize,
                    (t1 & 0x40) != 0,
                    (t1 & 0x80) != 0,
                    col * 8,
                    row * 8,
                    w,
                    &mut pixels,
                );
            }
        }
    }
    let _ = h;
    pixels
}

fn blit(dst: &mut [u8], dst_w: u32, dx: u32, dy: u32, src: &[u8], sw: u32, sh: u32) {
    for y in 0..sh {
        for x in 0..sw {
            let s = ((y * sw + x) * 3) as usize;
            let d = (((dy + y) * dst_w + dx + x) * 3) as usize;
            dst[d..d + 3].copy_from_slice(&src[s..s + 3]);
        }
    }
}

fn main() {
    let rom_path = arg("--rom", "smw.smc");
    let out_path = arg("--out", "docs/screenshots/special-world-passed.png");
    let raw = std::fs::read(&rom_path).expect("read ROM");
    let rom_bytes: Vec<u8> = if raw.len() % 0x400 == 0x200 { raw[0x200..].to_vec() } else { raw };

    // Koopa sprite-GFX tiles changed by the GFX $31 swap (verified range).
    let cpu_plain = load_level(&rom_bytes, 0x105, false);
    let cpu_passed = load_level(&rom_bytes, 0x105, true);
    let atlas_plain = tile_atlas(&cpu_plain, 0x680, 128, 16, 8, 4);
    let atlas_passed = tile_atlas(&cpu_passed, 0x680, 128, 16, 8, 4);

    // Overworld submap 0: the autumn palette swap.
    let ow_plain = render_ow(&load_overworld(&rom_bytes, 0, false), 512, 512);
    let ow_passed = render_ow(&load_overworld(&rom_bytes, 0, true), 512, 512);

    let (w, atlas_h, ow_h) = (1024u32, 256u32, 512u32);
    let mut full = vec![0u8; (w * (atlas_h + ow_h) * 3) as usize];
    blit(&mut full, w, 0, 0, &atlas_plain, 512, atlas_h);
    blit(&mut full, w, 512, 0, &atlas_passed, 512, atlas_h);
    blit(&mut full, w, 0, atlas_h, &ow_plain, 512, ow_h);
    blit(&mut full, w, 512, atlas_h, &ow_passed, 512, ow_h);

    let img = ImageBuffer::<Rgb<u8>, _>::from_raw(w, atlas_h + ow_h, full).unwrap();
    img.save(&out_path).expect("save png");
    println!("wrote {out_path}");
}
