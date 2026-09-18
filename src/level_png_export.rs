//! Level → PNG export (Lunar Magic v2.30 "Export level to PNG", v3.20
//! "Export Multiple Levels to Image Files").
//!
//! This is the shared render pipeline behind both the File-menu export
//! actions and the headless `render_level` binary. It drives the real
//! emulator (`decompress_sublevel` + the composed VRAM tilemap path from
//! `render_util`), so exports match what the editor shows — backdrop color,
//! Layer 2 then Layer 1, then sprites — and renders from ROM bytes with
//! unsaved edits already merged in (the caller merges open tabs first, like
//! the BPS/IPS export does).

use std::sync::Arc;

use anyhow::{Context, Result};
use image::{ImageBuffer, Rgb};
use smwe_emu::{emu::CheckedMem, rom::Rom as EmuRom, Cpu};

use crate::render_util::{read_color, render_layer, render_sp_tile};

/// Options for a level PNG export.
#[derive(Debug, Clone, Copy)]
pub struct LevelPngOptions {
    /// Render Layer 1 (foreground objects).
    pub include_layer1:  bool,
    /// Render Layer 2 / background tilemap (no-op cost on levels without one).
    pub include_layer2:  bool,
    /// Run the sprite engine and paint OAM sprites.
    pub include_sprites: bool,
}

impl Default for LevelPngOptions {
    fn default() -> Self {
        Self { include_layer1: true, include_layer2: true, include_sprites: true }
    }
}

/// A rendered level: 24-bit RGB pixels, `width * height` in size.
pub struct LevelPng {
    pub width:  u32,
    pub height: u32,
    pub pixels: Vec<u8>,
}

/// Number of SMW translevels LM's batch export addresses.
pub const LEVEL_COUNT: u16 = 0x200;

/// Per-level filename for batch exports, e.g. `level_105.png`.
///
/// Hex, zero-padded to 3 digits — matches how the editor names translevels
/// everywhere else.
pub fn level_export_filename(level: u16) -> String {
    format!("level_{level:03X}.png")
}

/// Build an emulator CPU with `level` decompressed, ready to render.
///
/// Shared by the export pipeline and the headless `render_level` binary
/// (which needs the CPU for its `--inspect=` debug mode).
pub fn load_level_cpu(rom_bytes: &[u8], level: u16) -> Result<Cpu> {
    let rom_bytes = if rom_bytes.len() % 0x400 == 0x200 { &rom_bytes[0x200..] } else { rom_bytes };
    let mut emu_rom = EmuRom::new(rom_bytes.to_vec());
    emu_rom.load_symbols(include_str!("../symbols/SMW_U.sym"));
    let mut cpu = Cpu::new(CheckedMem::new(Arc::new(emu_rom)));

    smwe_emu::emu::decompress_sublevel(&mut cpu, level);
    Ok(cpu)
}

/// Render `level` to raw RGB pixels.
///
/// Logic mirrors the `render_level` headless binary exactly (screen-length
/// table, vertical layout, backdrop fill, layer order, sprite pass) so the
/// two can never drift apart.
pub fn render_level_png(rom_bytes: &[u8], level: u16, opts: &LevelPngOptions) -> Result<LevelPng> {
    let mut cpu = load_level_cpu(rom_bytes, level)?;

    let level_mode = cpu.mem.load_u8(0x1925);
    let vertical = cpu.mem.load_u8(0x5B) & 1 != 0;
    let renderer_table = cpu.mem.cart.resolve("CODE_058955").context("symbol CODE_058955 not found")? + 9;
    let renderer = cpu.mem.load_u24(renderer_table + (level_mode as u32) * 3);
    let l2_renderers = [cpu.mem.cart.resolve("CODE_058B8D"), cpu.mem.cart.resolve("CODE_058C71")];
    let has_layer2 = l2_renderers.contains(&Some(renderer));

    let scr_len = match (vertical, has_layer2) {
        (false, false) => 0x20,
        (true, false) => 0x1C,
        (false, true) => 0x10,
        (true, true) => 0x0E,
    };
    let screens = scr_len as u32;
    let (width, height) = if vertical { (32 * 16, screens * 16 * 16) } else { (screens * 16 * 16, 27 * 16) };

    // Fill the canvas with the level's backdrop colour (CGRAM index 0) the way
    // the SNES shows it behind layers 1/2, instead of leaving it black. This
    // matches what the GUI editor paints under the GL tiles.
    let mut pixels = vec![0u8; (width * height * 3) as usize];
    {
        let backdrop = read_color(&cpu.mem.cgram, 0);
        for px in pixels.chunks_exact_mut(3) {
            px.copy_from_slice(&backdrop);
        }
    }
    if opts.include_layer2 {
        render_layer(&mut cpu, true, width, &mut pixels);
    }
    if opts.include_layer1 {
        render_layer(&mut cpu, false, width, &mut pixels);
    }
    if opts.include_sprites {
        render_sprites(&mut cpu, width, &mut pixels);
    }

    Ok(LevelPng { width, height, pixels })
}

/// Render `level` and PNG-encode it, ready to write to disk.
pub fn level_png_bytes(rom_bytes: &[u8], level: u16, opts: &LevelPngOptions) -> Result<Vec<u8>> {
    let img = render_level_png(rom_bytes, level, opts)?;
    let buf = ImageBuffer::<Rgb<u8>, _>::from_raw(img.width, img.height, img.pixels)
        .context("rendered pixel buffer has wrong length for its dimensions")?;
    let mut out = Vec::new();
    image::DynamicImage::ImageRgb8(buf)
        .write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png)
        .context("PNG encoding failed")?;
    Ok(out)
}

fn render_sprites(cpu: &mut Cpu, width: u32, pixels: &mut [u8]) {
    smwe_emu::emu::exec_sprites(cpu);
    for spr in (0..64).rev() {
        let x = cpu.mem.load_u8(0x300 + spr * 4) as u32;
        let y = cpu.mem.load_u8(0x301 + spr * 4) as u32;
        if y >= 0xE0 {
            continue;
        }
        let tile = cpu.mem.load_u16(0x302 + spr * 4);
        let size = cpu.mem.load_u8(0x460 + spr);
        if size & 0x02 != 0 {
            let (xn, xf) = if tile & 0x4000 == 0 { (0, 8) } else { (8, 0) };
            let (yn, yf) = if tile & 0x8000 == 0 { (0, 8) } else { (8, 0) };
            render_sp_tile(&cpu.mem.vram, &cpu.mem.cgram, x + xn, y + yn, tile, width, pixels);
            render_sp_tile(&cpu.mem.vram, &cpu.mem.cgram, x + xf, y + yn, tile + 1, width, pixels);
            render_sp_tile(&cpu.mem.vram, &cpu.mem.cgram, x + xn, y + yf, tile + 16, width, pixels);
            render_sp_tile(&cpu.mem.vram, &cpu.mem.cgram, x + xf, y + yf, tile + 17, width, pixels);
        } else {
            render_sp_tile(&cpu.mem.vram, &cpu.mem.cgram, x, y, tile, width, pixels);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn export_filename_is_hex_padded() {
        assert_eq!(level_export_filename(0x000), "level_000.png");
        assert_eq!(level_export_filename(0x105), "level_105.png");
        assert_eq!(level_export_filename(0x1FF), "level_1FF.png");
    }

    #[test]
    fn level_count_matches_smw_translevel_range() {
        assert_eq!(LEVEL_COUNT, 0x200);
    }

    /// Real-ROM render test (ignored by default; run with
    /// `ROM_PATH=~/workspace/smw-editor/smw.smc cargo test -p smw-editor --lib -- --ignored`).
    ///
    /// Level 0x105 (Yoshi's Island 1) is horizontal with no Layer 2, so the
    /// export must be exactly 0x20 screens wide and 27 blocks tall.
    #[test]
    #[ignore]
    fn real_rom_level_105_export_dimensions() {
        let rom_path = std::env::var("ROM_PATH").expect("ROM_PATH must point at a real SMW ROM");
        let rom_bytes = std::fs::read(&rom_path).expect("cannot read ROM");
        let img = render_level_png(&rom_bytes, 0x105, &LevelPngOptions::default()).expect("render level 0x105");
        assert_eq!(img.width, 0x20 * 16 * 16, "horizontal no-L2 level is 0x20 screens wide");
        assert_eq!(img.height, 27 * 16, "horizontal level is 27 blocks tall");
        assert_eq!(img.pixels.len(), (img.width * img.height * 3) as usize);

        // The canvas must actually be painted (not all backdrop): check that
        // at least some pixel differs from the CGRAM-0 backdrop fill.
        let backdrop = &img.pixels[..3];
        let any_different = img.pixels.chunks_exact(3).any(|px| px != backdrop);
        assert!(any_different, "rendered level must contain non-backdrop pixels");

        // PNG round-trip: encoded bytes decode back to the same dimensions.
        let png = level_png_bytes(&rom_bytes, 0x105, &LevelPngOptions::default()).expect("PNG encode level 0x105");
        assert!(png.starts_with(&[0x89, b'P', b'N', b'G']), "must be a real PNG stream");
        let decoded = image::load_from_memory(&png).expect("PNG must decode");
        assert_eq!(decoded.width(), img.width);
        assert_eq!(decoded.height(), img.height);
    }

    /// Smoke test: a few levels across the translevel range render without
    /// error (some test/debug levels are sparse — only dimensions matter).
    #[test]
    #[ignore]
    fn real_rom_several_levels_render() {
        let rom_path = std::env::var("ROM_PATH").expect("ROM_PATH must point at a real SMW ROM");
        let rom_bytes = std::fs::read(&rom_path).expect("cannot read ROM");
        for level in [0x000u16, 0x100, 0x1FF] {
            let img = render_level_png(&rom_bytes, level, &LevelPngOptions::default())
                .unwrap_or_else(|e| panic!("render level {level:03X}: {e}"));
            assert!(img.width > 0 && img.height > 0, "level {level:03X} has zero dimensions");
            assert_eq!(img.pixels.len(), (img.width * img.height * 3) as usize);
        }
    }
}
