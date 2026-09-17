//! Headless screenshot of the Background Tile Map Editor.
//!
//! egui can't render headless, so this composes an honest mock of the editor
//! window: both canvases are rendered by the real editor code path
//! (`render_util::render_bg_tilemap`) from the real ROM — parsed level
//! background, BG Map16 block words, and VRAM/CGRAM after
//! `decompress_sublevel` + `fetch_anim_frame`. Only the window chrome
//! (title bar, labels) is drawn rather than real egui widgets.
//!
//! The AFTER canvas shows the same demo edit the real-ROM round-trip test
//! applies: a 6x4 rectangle of tile $7B at cols 8..13, rows 10..13.
//! The caption shows the real result of Lunar Magic's "Add Offset to
//! Background Tiles" (+16) over that rectangle, computed by the same
//! `bg_tile_offset` helper the UI uses.
//!
//! ```sh
//! cargo run --bin render_bg_tilemap -- --out=docs/screenshots/bg-tilemap.png --rom=smw.smc
//! ```

use std::{path::Path, sync::Arc};

use ab_glyph::{Font, FontRef, Glyph, Point, PxScale, ScaleFont};
use image::{imageops, Rgb, RgbImage};
use smw_editor::render_util::{
    bg_map16_block_words,
    fill_rect,
    rect_border,
    render_bg_tilemap,
    BG_TILEMAP_CANVAS_H,
    BG_TILEMAP_CANVAS_W,
};
use smwe_emu::{emu::CheckedMem, rom::Rom as EmuRom, Cpu};
use smwe_rom::{
    level::{
        background::{bg_cell_index, bg_tile_offset, BG_TILEMAP_LEN},
        Layer2Data,
    },
    SmwRom,
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

/// Draw one line of text.
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
                    let (px_x, px_y) = (px_x as u32, px_y as u32);
                    if px_x < img.width() && px_y < img.height() {
                        let d = img.get_pixel(px_x, px_y).0;
                        let s = color.0;
                        let a = (v * 255.0) as u16;
                        let inv = 255 - a;
                        img.put_pixel(
                            px_x,
                            px_y,
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

fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let output = args.iter().find_map(|a| a.strip_prefix("--out=")).unwrap_or("docs/screenshots/bg-tilemap.png");
    let rom_path = args
        .iter()
        .find_map(|a| a.strip_prefix("--rom="))
        .or_else(|| args.iter().skip(1).find(|a| !a.starts_with("--")).map(|a| a.as_str()))
        .unwrap_or("smw.smc");
    let level_arg = args.iter().find_map(|a| a.strip_prefix("--level="));

    let raw = std::fs::read(Path::new(rom_path)).expect("cannot read ROM");
    let rom_bytes = if raw.len() % 0x400 == 0x200 { raw[0x200..].to_vec() } else { raw };

    // Real data path, identical to the UI: parse the ROM's level list, take a
    // background level's tilemap and Map16 bank.
    let smw_rom = SmwRom::from_file(rom_path)?;
    let level_num: u32 = match level_arg {
        Some(s) => u32::from_str_radix(s.trim_start_matches("0x"), 16)?,
        None => smw_rom
            .levels
            .iter()
            .position(|l| matches!(l.layer2, Layer2Data::Background(_)))
            .map(|i| i as u32)
            .ok_or_else(|| anyhow::anyhow!("no background level found"))?,
    };
    let (tiles, page) = match &smw_rom.levels[level_num as usize].layer2 {
        Layer2Data::Background(bg) => (bg.tile_ids().to_vec(), bg.high_byte()),
        _ => anyhow::bail!("level {level_num:03X} has no background layer"),
    };
    assert_eq!(tiles.len(), BG_TILEMAP_LEN);

    // Emulator path, identical to the UI's level load: decompress the level so
    // VRAM holds the real tile graphics and CGRAM the real palette.
    let mut emu_rom = EmuRom::new(rom_bytes);
    emu_rom.load_symbols(include_str!("../../symbols/SMW_U.sym"));
    let mut cpu = Cpu::new(CheckedMem::new(Arc::new(emu_rom)));
    smwe_emu::emu::decompress_sublevel(&mut cpu, level_num as u16);
    smwe_emu::emu::fetch_anim_frame(&mut cpu);
    let block_words = bg_map16_block_words(&mut cpu);

    let fonts = (load_font(SANS_CANDIDATES)?, load_font(SANS_BOLD_CANDIDATES)?);
    let (sans, sans_bold) = (&fonts.0, &fonts.1);
    let before = render_bg_tilemap(&tiles, page, &block_words, &cpu.mem.vram, &cpu.mem.cgram);

    // Demo edit: paint a 6x4 rectangle of tile $7B (same as the round-trip test).
    let mut after_tiles = tiles.clone();
    for row in 10..14u32 {
        for col in 8..14u32 {
            after_tiles[bg_cell_index(col, row).unwrap()] = 0x7B;
        }
    }
    let after = render_bg_tilemap(&after_tiles, page, &block_words, &cpu.mem.vram, &cpu.mem.cgram);

    // Real "Add Offset to Background Tiles" result over the painted rectangle.
    let rect_cells: Vec<(u32, u32)> = (10..14u32).flat_map(|r| (8..14u32).map(move |c| (c, r))).collect();
    let offset_result = bg_tile_offset(&after_tiles, &rect_cells, page, 16)
        .map(|r| {
            format!(
                "Offset +16 over the rectangle: moved {} tiles, bank {} -> {}{}",
                r.edits.len(),
                page,
                r.new_page,
                if r.clamped > 0 { format!(", {} clamped to the bank edge", r.clamped) } else { String::new() }
            )
        })
        .unwrap_or_else(|| "Offset +16: nothing to do.".to_string());

    let nonempty_before: u32 = tiles.iter().filter(|&&t| t != 0).count() as u32;

    // ---- Compose the mock window ----
    let cw = BG_TILEMAP_CANVAS_W;
    let ch = BG_TILEMAP_CANVAS_H;
    let (w, h) = (2 * cw + 72, ch + 210);
    let mut img = RgbImage::new(w, h);
    let ink = Rgb([0x1A, 0x1A, 0x1A]);
    let gray = Rgb([0x66, 0x66, 0x66]);
    for p in img.pixels_mut() {
        *p = Rgb([0xF2, 0xF2, 0xF2]);
    }
    fill_rect(&mut img, 0, 0, w, 52, Rgb([0x2B, 0x2B, 0x2B]));
    draw_text(
        &mut img,
        sans_bold,
        &format!(
            "Background Tile Map Editor \u{2014} level {level_num:03X}, Map16 bank {page} (blocks ${:03X}-${:03X})",
            page as u16 * 0x100,
            page as u16 * 0x100 + 0xFF
        ),
        24,
        15,
        19.0,
        Rgb([0xFF, 0xFF, 0xFF]),
    );

    let titles =
        ["BEFORE \u{2014} vanilla ROM background", "AFTER \u{2014} painted a 6\u{00D7}4 rectangle of tile $7B"];
    let canvases = [&before, &after];
    let counts = [nonempty_before, after_tiles.iter().filter(|&&t| t != 0).count() as u32];
    for (i, ((title, canvas), count)) in titles.iter().zip(canvases.iter()).zip(counts.iter()).enumerate() {
        let x = 24 + i as u32 * (cw + 24);
        let y = 76u32;
        draw_text(&mut img, sans_bold, title, x as i32, y as i32, 16.0, ink);
        let cy = y + 30;
        let rgb: RgbImage = image::ImageBuffer::from_raw(cw, ch, rgba_to_rgb(canvas)).expect("canvas buffer");
        imageops::replace(&mut img, &rgb, x as i64, cy as i64);
        rect_border(&mut img, x, cy, cw, ch, Rgb([0x99, 0x99, 0x99]));
        draw_text(
            &mut img,
            sans,
            &format!("{count} non-empty tiles \u{00B7} 32\u{00D7}27 Map16 cells \u{00B7} 1x zoom"),
            x as i32,
            (cy + ch + 10) as i32,
            13.0,
            gray,
        );
    }

    let cap_y = ch + 150;
    draw_text(&mut img, sans, &offset_result, 24, cap_y as i32, 14.0, ink);
    draw_text(
        &mut img,
        sans,
        "Mock window chrome \u{2014} both canvases are rendered by the real editor code path from the ROM.",
        24,
        (cap_y + 26) as i32,
        13.0,
        gray,
    );
    draw_text(
        &mut img,
        sans,
        "Shift+Right-click pattern fill, selection resize handles, undo/redo, bank switch, remap and \
         clipboard copy are live in the editor window.",
        24,
        (cap_y + 48) as i32,
        13.0,
        gray,
    );

    img.save(output)?;
    println!("wrote {output} ({w}x{h}) level={level_num:03X} page={page}");
    Ok(())
}

fn rgba_to_rgb(rgba: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(rgba.len() / 4 * 3);
    for px in rgba.chunks_exact(4) {
        out.extend_from_slice(&px[..3]);
    }
    out
}
