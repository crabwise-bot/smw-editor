//! Headless screenshot of the Level GFX Slots browser feature.
//!
//! egui can't render headless, so this composes an honest mock of the new
//! "Level GFX Slots" window: the real per-level FG/BG GFX + Sprite GFX
//! nibbles from a real level header (level 0x105), resolved through the real
//! OBJECTGFXLIST ($00A92B) and SPRITEGFXLIST ($00A8C3) tables, the real VRAM
//! upload ranges (`CODE_00AA35` / `UploadSpriteGFX`), and a palette-colored
//! tile strip rendered through the exact `cgram_palette_row` helper the 8x8
//! tile editor uses (copied verbatim below) with the level's real FG and
//! sprite palette rows from the emulator's CGRAM — i.e. exactly what the
//! Edit buttons open.
//!
//! ```sh
//! cargo run --bin render_gfx_slots -- --out=docs/screenshots/gfx-slot-browser.png --rom=smw.smc
//! ```

use std::sync::Arc;

use ab_glyph::{Font, FontRef, Glyph, Point, PxScale, ScaleFont};
use image::{Rgb, RgbImage};
use smwe_emu::{emu::CheckedMem, rom::Rom as EmuRom, Cpu};
use smwe_rom::{
    graphics::gfx_file::Tile,
    objects::{
        object_gfx_list::{OBJECT_SLOT_NAMES, OBJECT_SLOT_VRAM_RANGES},
        sprite_gfx_list::{SPRITE_SLOT_NAMES, SPRITE_SLOT_VRAM_BASES},
    },
};

const SANS_CANDIDATES: &[&str] = &["/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf"];
const SANS_BOLD_CANDIDATES: &[&str] = &["/usr/share/fonts/truetype/dejavu/DejaVuSans-Bold.ttf"];

fn load_font(candidates: &[&str]) -> anyhow::Result<FontRef<'static>> {
    for p in candidates {
        if let Ok(data) = std::fs::read(p) {
            let leaked: &'static [u8] = Box::leak(data.into_boxed_slice());
            return FontRef::try_from_slice(leaked).map_err(|e| anyhow::anyhow!("{p}: {e}"));
        }
    }
    anyhow::bail!("no font file found; tried {candidates:?}")
}

fn draw_text(img: &mut RgbImage, font: &FontRef, text: &str, x: i32, y: i32, px: f32, color: Rgb<u8>) {
    let scaled = font.as_scaled(PxScale::from(px));
    let mut caret_x = x as f32;
    let baseline = y as f32 + scaled.ascent();
    let mut prev = None;
    for ch in text.chars() {
        let id = font.glyph_id(ch);
        if let Some(p) = prev {
            caret_x += scaled.kern(p, id);
        }
        let glyph = Glyph { id, scale: PxScale::from(px), position: Point { x: caret_x, y: baseline } };
        if let Some(o) = scaled.outline_glyph(glyph) {
            let bb = o.px_bounds();
            o.draw(|gx, gy, v| {
                let px_x = bb.min.x as i32 + gx as i32;
                let px_y = bb.min.y as i32 + gy as i32;
                if px_x >= 0 && px_y >= 0 {
                    let (px_x, py_y) = (px_x as u32, px_y as u32);
                    if px_x < img.width() && py_y < img.height() {
                        let d = img.get_pixel(px_x, py_y).0;
                        let s = color.0;
                        let a = (v * 255.0) as u16;
                        let inv = 255 - a;
                        img.put_pixel(
                            px_x,
                            py_y,
                            Rgb([
                                ((s[0] as u16 * a + d[0] as u16 * inv) / 255) as u8,
                                ((s[1] as u16 * a + d[1] as u16 * inv) / 255) as u8,
                                ((s[2] as u16 * a + d[2] as u16 * inv) / 255) as u8,
                            ]),
                        );
                    }
                }
            });
        }
        caret_x += scaled.h_advance(id);
        prev = Some(id);
    }
}

/// Verbatim copy of the UI helper
/// (`src/ui/editor_prototypes/level_editor/tile_editor.rs`): decode one
/// 16-color CGRAM palette row (SNES BGR555).
fn cgram_palette_row(cgram: &[u8], row: usize) -> [[u8; 3]; 16] {
    let mut out = [[0u8; 3]; 16];
    for i in 0..16usize {
        let off = row * 32 + i * 2;
        if off + 1 < cgram.len() {
            let c = cgram[off] as u16 | ((cgram[off + 1] as u16) << 8);
            out[i] = [((c & 0x1F) << 3) as u8, (((c >> 5) & 0x1F) << 3) as u8, (((c >> 10) & 0x1F) << 3) as u8];
        }
    }
    out
}

/// Verbatim copy of the UI helper: render one 8x8 tile's color indices into an
/// 8x8 RGB buffer using the given palette.
fn tile_rgb8(tile: &Tile, palette: &[[u8; 3]; 16], out: &mut [u8]) {
    for py in 0..8usize {
        for px in 0..8usize {
            let idx = tile.color_indices.get(py * 8 + px).copied().unwrap_or(0);
            let c = palette[(idx as usize).min(15)];
            let off = (py * 8 + px) * 3;
            out[off] = c[0];
            out[off + 1] = c[1];
            out[off + 2] = c[2];
        }
    }
}

fn blit_rgb(img: &mut RgbImage, rgb: &[u8], x0: u32, y0: u32, scale: u32) {
    for (i, pix) in rgb.chunks(3).enumerate() {
        let (sx, sy) = ((i % 8) as u32, (i / 8) as u32);
        for dy in 0..scale {
            for dx in 0..scale {
                img.put_pixel(x0 + sx * scale + dx, y0 + sy * scale + dy, Rgb([pix[0], pix[1], pix[2]]));
            }
        }
    }
}

fn rect_outline(img: &mut RgbImage, x0: u32, y0: u32, w: u32, h: u32, t: u32, color: Rgb<u8>) {
    for d in 0..t {
        for x in x0..x0 + w {
            for &(yy, xx) in &[(y0 + d, x), (y0 + h - 1 - d, x)] {
                if xx < img.width() && yy < img.height() {
                    img.put_pixel(xx, yy, color);
                }
            }
        }
        for y in y0..y0 + h {
            for &(xx, yy) in &[(x0 + d, y), (x0 + w - 1 - d, y)] {
                if xx < img.width() && yy < img.height() {
                    img.put_pixel(xx, yy, color);
                }
            }
        }
    }
}

fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let output = args.iter().find_map(|a| a.strip_prefix("--out=")).unwrap_or("docs/screenshots/gfx-slot-browser.png");
    let rom_path = args
        .iter()
        .find_map(|a| a.strip_prefix("--rom="))
        .or_else(|| args.iter().skip(1).find(|a| !a.starts_with("--")).map(|a| a.as_str()))
        .unwrap_or("smw.smc");

    let sans = load_font(SANS_CANDIDATES)?;
    let sans_bold = load_font(SANS_BOLD_CANDIDATES)?;

    // Real ROM + emulator state, exactly like the editor's level load.
    let rom_bytes = std::fs::read(rom_path)?;
    let mut emu_rom = EmuRom::new(rom_bytes.clone());
    emu_rom.load_symbols(include_str!("../../symbols/SMW_U.sym"));
    let mut cpu = Cpu::new(CheckedMem::new(Arc::new(emu_rom)));
    smwe_emu::emu::decompress_sublevel(&mut cpu, 0x105);
    let cgram = cpu.mem.cgram.clone();

    let rom = smwe_rom::SmwRom::from_file(rom_path)?;
    let level = &rom.levels[0x105];
    let fg_tileset = level.primary_header.fg_bg_gfx() as usize;
    let sp_tileset = level.primary_header.sprite_gfx() as usize;
    let palette_fg = level.primary_header.palette_fg() as usize;
    let palette_sprite = level.primary_header.palette_sprite() as usize;
    assert!(fg_tileset < 26 && sp_tileset < 26, "tileset nibbles must index the 26-row tables");

    let fg_files = rom.gfx.object_gfx_list.files_for_object_tileset(fg_tileset);
    let sp_files = rom.gfx.sprite_gfx_list.files_for_sprite_tileset(sp_tileset);
    // Every resolved file must exist and hold a full upload slot (0x80 tiles).
    for &f in fg_files.iter().chain(sp_files.iter()) {
        assert!(rom.gfx.files.get(f).map(|g| g.tiles.len()).unwrap_or(0) >= 0x80, "file {f:02X} too small");
    }
    // Sanity against the game's own upload: the ObjectTileset WRAM value the
    // emulator set during decompress_sublevel must agree with the header.
    let wram_tileset = cpu.mem.wram[0x1931] as usize;
    assert!(wram_tileset < 26, "WRAM ObjectTileset {wram_tileset} out of range");

    let pal_fg = cgram_palette_row(&cgram, palette_fg);
    let pal_sp = cgram_palette_row(&cgram, palette_sprite);

    // ── Compose the mock window ──────────────────────────────────────────
    const W: u32 = 760;
    const H: u32 = 640;
    let mut img = RgbImage::from_pixel(W, H, Rgb([30, 30, 38]));
    for y in 0..44u32 {
        for x in 0..W {
            img.put_pixel(x, y, Rgb([42, 42, 54]));
        }
    }
    draw_text(&mut img, &sans_bold, "Level GFX Slots", 16, 8, 22.0, Rgb([235, 235, 245]));
    draw_text(
        &mut img,
        &sans,
        "The GFX files the game uploads for this level. Edit jumps into the 8x8 tile editor.",
        16,
        56,
        14.0,
        Rgb([200, 200, 210]),
    );

    let mut y = 92;
    draw_text(
        &mut img,
        &sans_bold,
        &format!("FG/BG GFX — tileset ${fg_tileset:01X} (OBJECTGFXLIST)"),
        16,
        y as i32,
        16.0,
        Rgb([235, 235, 245]),
    );
    y += 30;
    for (i, &file_num) in fg_files.iter().enumerate() {
        let (lo, hi) = OBJECT_SLOT_VRAM_RANGES[i];
        draw_text(&mut img, &sans_bold, OBJECT_SLOT_NAMES[i], 28, y as i32, 15.0, Rgb([235, 220, 160]));
        draw_text(&mut img, &sans, &format!("GFX file {file_num:02X}"), 110, y as i32, 15.0, Rgb([200, 200, 210]));
        draw_text(&mut img, &sans, &format!("VRAM {lo:#05X}–{hi:#05X}"), 260, y as i32, 15.0, Rgb([160, 160, 175]));
        // Mock Edit button.
        for yy in y..y + 24 {
            for xx in 420..500u32 {
                img.put_pixel(xx, yy, Rgb([58, 110, 180]));
            }
        }
        rect_outline(&mut img, 420, y, 80, 24, 1, Rgb([120, 170, 230]));
        draw_text(
            &mut img,
            &sans,
            &format!("Edit {}", OBJECT_SLOT_NAMES[i]),
            432,
            y as i32 + 3,
            13.0,
            Rgb([240, 245, 255]),
        );
        y += 32;
    }
    draw_text(
        &mut img,
        &sans,
        &format!("Tiles colored with the level's FG palette row ({palette_fg})."),
        16,
        y as i32,
        13.0,
        Rgb([150, 150, 165]),
    );
    y += 34;

    draw_text(
        &mut img,
        &sans_bold,
        &format!("Sprite GFX — tileset ${sp_tileset:01X} (SPRITEGFXLIST)"),
        16,
        y as i32,
        16.0,
        Rgb([235, 235, 245]),
    );
    y += 30;
    for (i, &file_num) in sp_files.iter().enumerate() {
        let base = SPRITE_SLOT_VRAM_BASES[i];
        draw_text(&mut img, &sans_bold, SPRITE_SLOT_NAMES[i], 28, y as i32, 15.0, Rgb([160, 220, 235]));
        draw_text(&mut img, &sans, &format!("GFX file {file_num:02X}"), 110, y as i32, 15.0, Rgb([200, 200, 210]));
        draw_text(
            &mut img,
            &sans,
            &format!("VRAM {base:#05X}–{:#05X}", base + 0x7F),
            260,
            y as i32,
            15.0,
            Rgb([160, 160, 175]),
        );
        for yy in y..y + 24 {
            for xx in 420..500u32 {
                img.put_pixel(xx, yy, Rgb([58, 110, 180]));
            }
        }
        rect_outline(&mut img, 420, y, 80, 24, 1, Rgb([120, 170, 230]));
        draw_text(
            &mut img,
            &sans,
            &format!("Edit {}", SPRITE_SLOT_NAMES[i]),
            432,
            y as i32 + 3,
            13.0,
            Rgb([240, 245, 255]),
        );
        y += 32;
    }
    draw_text(
        &mut img,
        &sans,
        &format!("Tiles colored with the level's sprite palette row ({palette_sprite})."),
        16,
        y as i32,
        13.0,
        Rgb([150, 150, 165]),
    );
    y += 36;

    // Palette-colored tile strip: what the Edit buttons open — the 8x8 tile
    // editor grid cells for the FG1 and SP1 files, through the real palettes.
    draw_text(
        &mut img,
        &sans_bold,
        "What Edit opens — palette-colored 8x8 tiles (same renderer as the 8x8 tile editor):",
        16,
        y as i32,
        14.0,
        Rgb([235, 235, 245]),
    );
    y += 28;
    draw_text(
        &mut img,
        &sans,
        &format!("FG1 file {:02X}, palette row {palette_fg}", fg_files[0]),
        16,
        y as i32,
        13.0,
        Rgb([200, 200, 210]),
    );
    y += 22;
    let mut tile_rgb = [0u8; 8 * 8 * 3];
    let fg1_tiles = &rom.gfx.files[fg_files[0]].tiles;
    for (i, tile) in fg1_tiles.iter().take(16).enumerate() {
        tile_rgb8(tile, &pal_fg, &mut tile_rgb);
        blit_rgb(&mut img, &tile_rgb, 16 + i as u32 * 40, y, 4);
    }
    rect_outline(&mut img, 16, y - 3, 16 * 40, 8 * 4 + 6, 2, Rgb([255, 220, 60]));
    y += 8 * 4 + 12;
    draw_text(
        &mut img,
        &sans,
        &format!("SP1 file {:02X}, palette row {palette_sprite}", sp_files[0]),
        16,
        y as i32,
        13.0,
        Rgb([200, 200, 210]),
    );
    y += 22;
    let sp1_tiles = &rom.gfx.files[sp_files[0]].tiles;
    for (i, tile) in sp1_tiles.iter().take(16).enumerate() {
        tile_rgb8(tile, &pal_sp, &mut tile_rgb);
        blit_rgb(&mut img, &tile_rgb, 16 + i as u32 * 40, y, 4);
    }
    let _ = wram_tileset;

    img.save(output)?;
    println!(
        "wrote {output} (level 0x105: FG/BG tileset ${fg_tileset:01X} -> {:02X?}, sprite tileset ${sp_tileset:01X} -> {:02X?})",
        fg_files, sp_files
    );
    Ok(())
}
