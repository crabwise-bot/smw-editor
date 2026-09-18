//! Headless mock screenshot of the new "Custom Overworld Sprite Record
//! Sizes" dialog (LM v3.51 parity — per-sprite record sizes, not per-submap
//! capacities).
//!
//! egui can't render headless, so this composes an honest mock: every size
//! and extra-byte count shown comes from the real ROM path on a scratch
//! in-memory copy — `create_size_table` (the ROM starts with no table) ->
//! `parse_size_table` -> place custom sprites with the derived counts ->
//! `write_custom_table` -> re-parse -> `write_size_table` in-place edit.
//! Only the dialog chrome (window borders, drag-value boxes, buttons) is
//! drawn rather than real egui widgets.
//!
//! ```sh
//! cargo run --bin render_ow_sprite_size_table -- --rom=/path/to/smw.smc --out=docs/screenshots/ow-sprite-size-table.png
//! ```

use ab_glyph::{Font, FontRef, Glyph, Point, PxScale, ScaleFont};
use image::{Rgb, RgbImage};
use smw_editor::render_util::{fill_rect, rect_border};
use smwe_rom::overworld::sprites::{self, CustomOwSprite, CustomSpriteTable, SpriteSizeTable, SIZE_TABLE_LEN};

const SANS_CANDIDATES: &[&str] = &["/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf"];
const SANS_BOLD_CANDIDATES: &[&str] = &["/usr/share/fonts/truetype/dejavu/DejaVuSans-Bold.ttf"];
const MONO_CANDIDATES: &[&str] = &["/usr/share/fonts/truetype/dejavu/DejaVuSansMono.ttf"];

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
                let (px_x, px_y) = (bb.min.x as i32 + gx as i32, bb.min.y as i32 + gy as i32);
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
    let output =
        args.iter().find_map(|a| a.strip_prefix("--out=")).unwrap_or("docs/screenshots/ow-sprite-size-table.png");
    let rom_path = args
        .iter()
        .find_map(|a| a.strip_prefix("--rom="))
        .or_else(|| args.iter().skip(1).find(|a| !a.starts_with("--")).map(|a| a.as_str()))
        .unwrap_or("smw.smc");

    let sans = load_font(SANS_CANDIDATES)?;
    let sans_bold = load_font(SANS_BOLD_CANDIDATES)?;
    let mono = load_font(MONO_CANDIDATES)?;

    // ── Real data: run the actual size-table write/read path on a scratch
    // in-memory ROM copy ────────────────────────────────────────────────
    let raw = std::fs::read(rom_path)?;
    let mut scratch: Vec<u8> = if raw.len() % 0x400 == 0x200 { raw[0x200..].to_vec() } else { raw };

    // The vanilla ROM has no size table.
    anyhow::ensure!(sprites::parse_size_table(&scratch, 0)?.is_none());

    // Create one with a few interesting sizes, exactly like the dialog's
    // Apply does on a ROM without a table.
    let mut sizes = SpriteSizeTable::default();
    sizes.set_size(0x01, 3)?; // sprite 01: fixed bytes only, 0 extra
    sizes.set_size(0x10, 7)?; // sprite 10: 7 total = 4 extra
    sizes.set_size(0x2A, 9)?; // sprite 2A: 9 total = 6 extra
    sizes.set_size(0x7F, 0xF)?; // sprite 7F: max, 12 extra
    sprites::create_size_table(&sizes, &mut scratch, 0)?;
    let parsed = sprites::parse_size_table(&scratch, 0)?.expect("size table was just created");
    anyhow::ensure!(parsed == sizes);

    // The derived extra-byte counts drive custom sprite encoding.
    let counts = sprites::extra_byte_counts(&scratch, 0);
    anyhow::ensure!(counts[0x01] == 0);
    anyhow::ensure!(counts[0x10] == 4);
    anyhow::ensure!(counts[0x2A] == 6);
    anyhow::ensure!(counts[0x7F] == 12);
    anyhow::ensure!(counts[0x11] == 1, "untouched sprites keep the default");

    // Place custom sprites with the derived extra-byte counts and
    // round-trip them through the real custom-table write/parse path.
    let mut custom = CustomSpriteTable::default();
    for &number in &[0x01u8, 0x10, 0x2A, 0x7F] {
        custom.submaps[0].push(CustomOwSprite {
            number,
            x: number,
            y: 30,
            height: 0,
            extra: vec![0xA5; counts[number as usize] as usize],
        });
    }
    sprites::write_custom_table(&custom, &mut scratch, 0)?;
    let custom_back = sprites::parse_custom_table(&scratch, 0)?.expect("custom table round trip");
    anyhow::ensure!(custom_back.submaps[0].len() == 4);
    anyhow::ensure!(custom_back.submaps[0][1].extra.len() == 4, "sprite 0x10 keeps its 4 extra bytes");

    // In-place edit, like the dialog's Apply on a ROM that already has a
    // table.
    let mut edited = parsed;
    edited.set_size(0x10, 5)?;
    sprites::write_size_table(&edited, &mut scratch, 0)?;
    let reparsed = sprites::parse_size_table(&scratch, 0)?.expect("table still there");
    anyhow::ensure!(reparsed.size_for(0x10) == 5);

    // ── Compose the mock dialog ───────────────────────────────────────────
    let (w, h) = (920u32, 760u32);
    let mut img = RgbImage::new(w, h);
    let bg = Rgb([0xF2, 0xF2, 0xF2]);
    let ink = Rgb([0x1A, 0x1A, 0x1A]);
    let gray = Rgb([0x66, 0x66, 0x66]);
    let amber = Rgb([0x9A, 0x6A, 0x10]);
    for p in img.pixels_mut() {
        *p = bg;
    }
    fill_rect(&mut img, 0, 0, w, 52, Rgb([0x2B, 0x2B, 0x2B]));
    draw_text(
        &mut img,
        &sans_bold,
        "World Editor \u{2014} Custom Overworld Sprite Record Sizes (headless mock; sizes + counts are real ROM output)",
        24,
        15,
        17.0,
        Rgb([0xFF, 0xFF, 0xFF]),
    );

    // Dialog window.
    let (dx, dy, dw) = (60u32, 76u32, 800u32);
    let dh = 620u32;
    fill_rect(&mut img, dx, dy, dw, dh, Rgb([0xFF, 0xFF, 0xFF]));
    rect_border(&mut img, dx, dy, dw, dh, Rgb([0x99, 0x99, 0x99]));
    fill_rect(&mut img, dx, dy, dw, 34, Rgb([0xE4, 0xE4, 0xE4]));
    draw_text(
        &mut img,
        &sans_bold,
        "Custom Overworld Sprite Record Sizes",
        (dx + 14) as i32,
        (dy + 8) as i32,
        14.0,
        ink,
    );

    let mut y = dy + 52;
    draw_text(
        &mut img,
        &sans,
        "Total record size per custom sprite, like Lunar Magic v3.51. 3 = fixed bytes only (no extra",
        (dx + 14) as i32,
        y as i32,
        12.0,
        gray,
    );
    y += 18;
    draw_text(
        &mut img,
        &sans,
        "bytes), 15 = max, default 4. Entry N is sprite N (01\u{2013}7F).",
        (dx + 14) as i32,
        y as i32,
        12.0,
        gray,
    );
    y += 18;
    draw_text(
        &mut img,
        &sans,
        "LM only uses this table to parse the custom sprite list \u{2014} sizes do nothing in-game",
        (dx + 14) as i32,
        y as i32,
        12.0,
        gray,
    );
    y += 18;
    draw_text(&mut img, &sans, "without a runtime patch.", (dx + 14) as i32, y as i32, 12.0, gray);
    y += 24;
    draw_text(
        &mut img,
        &sans,
        "Table created on save (this ROM had none): RATS block + $0DE18C pointer + $42 marker.",
        (dx + 14) as i32,
        y as i32,
        12.0,
        amber,
    );
    y += 32;

    // Rows for the demo sprites, showing each total size and the derived
    // extra-byte count — straight from the re-parsed table.
    draw_text(
        &mut img,
        &mono,
        "showing 4 of 127 entries (the rest are the default 4):",
        (dx + 14) as i32,
        y as i32,
        12.0,
        gray,
    );
    y += 28;
    for &number in &[0x01u8, 0x10, 0x2A, 0x7F] {
        let size = reparsed.size_for(number);
        let extra = size - 3;
        draw_text(&mut img, &mono, &format!("Sprite {number:02X}"), (dx + 14) as i32, (y + 4) as i32, 13.0, ink);
        // DragValue box with the total size.
        fill_rect(&mut img, dx + 150, y, 150, 26, Rgb([0xFF, 0xFF, 0xFF]));
        rect_border(&mut img, dx + 150, y, 150, 26, Rgb([0x99, 0x99, 0x99]));
        draw_text(&mut img, &mono, &format!("total bytes: {size}"), (dx + 158) as i32, (y + 5) as i32, 13.0, ink);
        draw_text(
            &mut img,
            &sans,
            &format!("= {extra} extra {}", if extra == 1 { "byte" } else { "bytes" }),
            (dx + 320) as i32,
            (y + 5) as i32,
            13.0,
            ink,
        );
        // Mini bar: total size out of 15.
        let bar_x = dx + 480;
        let bar_w = 150u32;
        fill_rect(&mut img, bar_x, y + 4, bar_w, 16, Rgb([0xDD, 0xDD, 0xDD]));
        let fill = (bar_w as usize * size as usize / 15) as u32;
        fill_rect(&mut img, bar_x, y + 4, fill, 16, Rgb([0x4A, 0x90, 0xD9]));
        y += 36;
    }
    anyhow::ensure!(SIZE_TABLE_LEN == 0x7F);

    // Buttons.
    y += 8;
    let bx = dx + 14;
    fill_rect(&mut img, bx, y, 90, 30, Rgb([0xFF, 0xFF, 0xFF]));
    rect_border(&mut img, bx, y, 90, 30, Rgb([0x99, 0x99, 0x99]));
    draw_text(&mut img, &sans, "Apply", (bx + 24) as i32, (y + 7) as i32, 13.0, ink);
    fill_rect(&mut img, bx + 104, y, 150, 30, Rgb([0xFF, 0xFF, 0xFF]));
    rect_border(&mut img, bx + 104, y, 150, 30, Rgb([0x99, 0x99, 0x99]));
    draw_text(&mut img, &sans, "Reset all to 4", (bx + 116) as i32, (y + 7) as i32, 13.0, ink);
    fill_rect(&mut img, bx + 268, y, 90, 30, Rgb([0xFF, 0xFF, 0xFF]));
    rect_border(&mut img, bx + 268, y, 90, 30, Rgb([0x99, 0x99, 0x99]));
    draw_text(&mut img, &sans, "Close", (bx + 290) as i32, (y + 7) as i32, 13.0, ink);

    draw_text(
        &mut img,
        &sans,
        "Per-sprite custom overworld sprite record sizes \u{2014} LM v3.51 parity \u{00B7} 127 entries (sprites 01\u{2013}7F),",
        24,
        (h - 40) as i32,
        12.0,
        gray,
    );
    draw_text(
        &mut img,
        &sans,
        "3\u{2013}15 total bytes each, default 4 \u{00B7} applies as one undo step; existing sprites' extra bytes resize to match",
        24,
        (h - 20) as i32,
        12.0,
        gray,
    );

    img.save(output)?;
    println!("wrote {output}");
    Ok(())
}
