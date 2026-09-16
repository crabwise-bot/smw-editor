//! Headless title-screen Layer 3 renderer.
//!
//! Reproduces the title screen's exact hardware state through the REAL game
//! paths and rasterizes the Layer 3 tilemap (the "SUPER MARIO WORLD" logo and
//! menu, drawn by the fixed-slot title stripe image):
//!
//! 1. `decompress_sublevel(0xEB)` — the title screen IS level 235; this runs
//!    the real level init (GFX uploads to VRAM, level palettes to CGRAM).
//! 2. `load_title_screen_palette()` — runs the real `CODE_00ADA6` (title
//!    colors over palettes 0/1) + `CODE_00922F` (MainPalette → CGRAM), exactly
//!    as `GM04PrepTitleScreen` does.
//! 3. The title stripe image (`TITLE_SCREEN_STRIPE_SNES`) is parsed and applied
//!    to a byte-level VRAM mirror with the exact `LoadStripeImage` DMA
//!    semantics (SMWDisX bank_00.asm): 3-byte header `[dest-hi][dest-lo]`
//!    (VRAM word address) + `[flags]` (bit 7 = vertical → +32-word stride,
//!    bit 6 = RLE) + `[len-lo]`, 14-bit payload byte count − 1 across the
//!    flags low 6 bits and len-lo; payload is little-endian tile words; a
//!    first byte with bit 7 set terminates the image.
//! 4. The 64×64 Layer 3 tilemap at VRAM $5000 (`VRam_L3Tilemap`, `BG3SC` =
//!    $50|Size_64x64) is rasterized with 4bpp tiles from VRAM $4000
//!    (`VRam_L3Tiles`, `BG34NBA`) and CGRAM.
//!
//! ```sh
//! cargo run --bin render_title -- --out=/tmp/title.png --rom=smw.smc
//! ```

use std::{env, path::Path, sync::Arc};

use image::{ImageBuffer, Rgb};
use smwe_emu::{emu::CheckedMem, rom::Rom as EmuRom, Cpu};

/// Layer 3 tilemap base (SMWDisX `VRam_L3Tilemap` = VRAM word $5000;
/// `cpu.mem.vram` is byte-addressed, so byte offset $A000) and character base
/// (`VRam_L3Tiles` = VRAM word $4000 → byte offset $8000, from `BG34NBA`).
/// The tilemap is 64×64 (`BG3SC` = $50 | Size_64x64 in `SetUpScreen`).
const L3_TILEMAP_BASE: usize = 0xA000;
const L3_TILES_BASE: usize = 0x8000;
const TILEMAP_W: usize = 64;
const TILEMAP_H: usize = 64;

/// Apply one stripe image to `vram` (byte-addressed; VRAM word $5000 is byte
/// $A000) with the exact `LoadStripeImage` DMA semantics. `stripe` is the raw
/// fixed-slot bytes (without the FF terminator, though a leading 0x80+ byte
/// also stops parsing).
fn apply_stripe_image(vram: &mut [u8], stripe: &[u8]) {
    let mut i = 0usize;
    while i < stripe.len() {
        let b0 = stripe[i];
        if b0 & 0x80 != 0 {
            break; // end of stripe image
        }
        assert!(i + 4 <= stripe.len(), "truncated stripe header at {i:#x}");
        // Stripe dest is a VRAM *word* address; vram is byte-addressed.
        let mut dest = (((stripe[i] as usize) << 8) | stripe[i + 1] as usize) * 2;
        let flags = stripe[i + 2] as usize;
        let vertical = flags & 0x80 != 0;
        assert!(flags & 0x40 == 0, "RLE stripe commands are not supported");
        let nbytes = (((flags & 0x3F) << 8) | stripe[i + 3] as usize) + 1;
        assert!(nbytes % 2 == 0, "odd stripe payload at {i:#x}");
        assert!(i + 4 + nbytes <= stripe.len(), "truncated stripe payload at {i:#x}");
        // 32-word stride for vertical (interleaves two columns 32 apart in
        // the 64-wide tilemap), 1 word otherwise; *2 for byte addressing.
        let stride = if vertical { 64 } else { 2 };
        let mut j = i + 4;
        let end = j + nbytes;
        while j < end {
            // One DMA word: low byte to $2118, high byte to $2119.
            assert!(dest + 1 < vram.len(), "stripe write out of VRAM at {dest:#x}");
            vram[dest] = stripe[j];
            vram[dest + 1] = stripe[j + 1];
            dest += stride;
            j += 2;
        }
        i = end;
    }
}

fn read_color(cgram: &[u8], idx: usize) -> [u8; 3] {
    let off = idx * 2;
    if off + 1 >= cgram.len() {
        return [0, 0, 0];
    }
    let rgb = cgram[off] as u16 | ((cgram[off + 1] as u16) << 8);
    [((rgb & 0x1F) << 3) as u8, (((rgb >> 5) & 0x1F) << 3) as u8, (((rgb >> 10) & 0x1F) << 3) as u8]
}

fn main() -> anyhow::Result<()> {
    let args: Vec<String> = env::args().collect();
    let output = args.iter().find_map(|a| a.strip_prefix("--out=")).unwrap_or("/tmp/title.png");
    let rom_path = args
        .iter()
        .find_map(|a| a.strip_prefix("--rom="))
        .map(Path::new)
        .or_else(|| args.iter().skip(1).find(|a| !a.starts_with("--")).map(|a| Path::new(a)))
        .unwrap_or_else(|| Path::new("smw.smc"));

    // Real game path: boot the title screen level, then apply the title
    // palette exactly like GM04PrepTitleScreen.
    let raw = std::fs::read(rom_path)?;
    let rom_bytes = if raw.len() % 0x400 == 0x200 { raw[0x200..].to_vec() } else { raw };
    let mut emu_rom = EmuRom::new(rom_bytes);
    emu_rom.load_symbols(include_str!("../../symbols/SMW_U.sym"));
    let mut cpu = Cpu::new(CheckedMem::new(Arc::new(emu_rom)));
    smwe_emu::emu::decompress_sublevel(&mut cpu, 0xEB);
    smwe_emu::emu::load_title_screen_palette(&mut cpu);
    println!("title level + palette loaded via real routines");

    // Parse the title stripe from the ROM and apply it to a VRAM mirror with
    // exact DMA semantics.
    let rom = smwe_rom::SmwRom::from_file(rom_path)?;
    let stripe = &rom.title_credits.title_screen_stripe;
    println!("title stripe: {} bytes", stripe.len());
    let mut vram = cpu.mem.vram.clone();
    apply_stripe_image(&mut vram, stripe);

    // Rasterize the 32x32 Layer 3 tilemap at $5000, 4bpp tiles at $4000.
    let (w, h) = (TILEMAP_W as u32 * 8, TILEMAP_H as u32 * 8);
    let mut pixels = vec![0u8; (w * h * 3) as usize];
    // Backdrop color behind transparent pixels.
    let backdrop = read_color(&cpu.mem.cgram, 0);
    for px in pixels.chunks_exact_mut(3) {
        px.copy_from_slice(&backdrop);
    }
    for ty in 0..TILEMAP_H {
        for tx in 0..TILEMAP_W {
            let off = L3_TILEMAP_BASE + (ty * TILEMAP_W + tx) * 2;
            let t = vram[off] as u16 | ((vram[off + 1] as u16) << 8);
            let tile = (t & 0x3FF) as usize;
            let pal = ((t >> 10) & 0x7) as usize;
            let flip_x = t & 0x4000 != 0;
            let flip_y = t & 0x8000 != 0;
            let tile_base = L3_TILES_BASE + tile * 32;
            for py in 0..8u32 {
                for px in 0..8u32 {
                    let sx = if flip_x { 7 - px } else { px } as usize;
                    let sy = if flip_y { 7 - py } else { py } as usize;
                    let row_off = tile_base + sy * 2;
                    if row_off + 17 >= vram.len() {
                        continue;
                    }
                    let b0 = vram[row_off];
                    let b1 = vram[row_off + 1];
                    let b2 = vram[row_off + 16];
                    let b3 = vram[row_off + 17];
                    let bit = 7 - sx;
                    let c0 = (b0 >> bit) & 1;
                    let c1 = (b1 >> bit) & 1;
                    let c2 = (b2 >> bit) & 1;
                    let c3 = (b3 >> bit) & 1;
                    let ci = (c0 | (c1 << 1) | (c2 << 2) | (c3 << 3)) as usize;
                    if ci == 0 {
                        continue;
                    }
                    let rgb = read_color(&cpu.mem.cgram, pal * 16 + ci);
                    let o = (((ty as u32 * 8 + py) * w + tx as u32 * 8 + px) * 3) as usize;
                    pixels[o..o + 3].copy_from_slice(&rgb);
                }
            }
        }
    }

    let img = ImageBuffer::<Rgb<u8>, _>::from_raw(w, h, pixels).expect("image buffer");
    img.save(output)?;
    println!("wrote {output} ({w}x{h})");
    Ok(())
}
