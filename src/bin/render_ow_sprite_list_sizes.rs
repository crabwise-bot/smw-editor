//! Headless mock screenshot of the new "Custom Sprite List Sizes" dialog
//! (LM v3.51 parity).
//!
//! egui can't render headless, so this composes an honest mock: the per-submap
//! list sizes, sprite counts, and the validation error all come from the real
//! `CustomSpriteTable` code (set sizes -> insert sprites ->
//! `write_custom_table` -> re-parse -> one `set_list_size` refusal to model
//! the dialog's "too small" error state). Only the dialog chrome (window
//! borders, drag-value boxes, buttons) is drawn rather than real egui
//! widgets.
//!
//! ```sh
//! cargo run --bin render_ow_sprite_list_sizes -- --rom=/path/to/smw.smc --out=docs/screenshots/ow-sprite-list-sizes.png
//! ```

use ab_glyph::{Font, FontRef, Glyph, Point, PxScale, ScaleFont};
use image::{Rgb, RgbImage};
use smw_editor::render_util::{fill_rect, rect_border};
use smwe_rom::overworld::{
    sprites::{self, CustomOwSprite, CustomSpriteTable},
    SUBMAP_NAMES,
};

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
        args.iter().find_map(|a| a.strip_prefix("--out=")).unwrap_or("docs/screenshots/ow-sprite-list-sizes.png");
    let rom_path = args
        .iter()
        .find_map(|a| a.strip_prefix("--rom="))
        .or_else(|| args.iter().skip(1).find(|a| !a.starts_with("--")).map(|a| a.as_str()))
        .unwrap_or("smw.smc");

    let sans = load_font(SANS_CANDIDATES)?;
    let sans_bold = load_font(SANS_BOLD_CANDIDATES)?;
    let mono = load_font(MONO_CANDIDATES)?;

    // ── Real data: configure list sizes, place sprites, round-trip through
    // the real ROM write/parse path on a scratch copy ─────────────────────
    let raw = std::fs::read(rom_path)?;
    let rom_bytes: Vec<u8> = if raw.len() % 0x400 == 0x200 { raw[0x200..].to_vec() } else { raw };

    let mut table = CustomSpriteTable::default();
    let sizes = [8u8, 24, 24, 24, 12, 24, 24];
    for (submap, &size) in sizes.iter().enumerate() {
        table.set_list_size(submap, size).unwrap();
    }
    for i in 0..3u8 {
        table.submaps[0].push(CustomOwSprite {
            number: 0x10 + i,
            x:      i * 4,
            y:      30,
            height: 0,
            extra:  vec![0],
        });
    }
    for i in 0..12u8 {
        table.submaps[4].push(CustomOwSprite {
            number: 0x20 + i,
            x:      i * 4,
            y:      20,
            height: 0,
            extra:  vec![0],
        });
    }
    for i in 0..2u8 {
        table.submaps[5].push(CustomOwSprite {
            number: 0x30 + i,
            x:      i * 4,
            y:      40,
            height: 0,
            extra:  vec![0],
        });
    }
    let mut scratch = rom_bytes.clone();
    sprites::write_custom_table(&table, &mut scratch, 0)?;
    let table = sprites::parse_custom_table(&scratch, 0)?.expect("custom table round trip");
    anyhow::ensure!(table.list_sizes == sizes);
    anyhow::ensure!(table.submaps[4].len() == 12);
    anyhow::ensure!(table.room_for(4) == 0, "Valley of Bowser should be full");

    // The dialog's "too small" error state, from the real refusal logic: the
    // user drags Valley of Bowser from 12 down to 5 while it holds 12.
    let refused = table.clone().set_list_size(4, 5);
    anyhow::ensure!(refused.is_err());
    let err_text = format!(
        "{} already holds {} sprites \u{2014} raise the size or delete sprites first",
        SUBMAP_NAMES[4],
        table.submaps[4].len()
    );

    // ── Compose the mock dialog ───────────────────────────────────────────
    let (w, h) = (920u32, 700u32);
    let mut img = RgbImage::new(w, h);
    let bg = Rgb([0xF2, 0xF2, 0xF2]);
    let ink = Rgb([0x1A, 0x1A, 0x1A]);
    let gray = Rgb([0x66, 0x66, 0x66]);
    let red = Rgb([0xC0, 0x30, 0x30]);
    for p in img.pixels_mut() {
        *p = bg;
    }
    fill_rect(&mut img, 0, 0, w, 52, Rgb([0x2B, 0x2B, 0x2B]));
    draw_text(
        &mut img,
        &sans_bold,
        "World Editor \u{2014} Custom Sprite List Sizes (headless mock; sizes + counts are real ROM output)",
        24,
        15,
        17.0,
        Rgb([0xFF, 0xFF, 0xFF]),
    );

    // Dialog window.
    let (dx, dy, dw) = (60u32, 76u32, 800u32);
    let dh = 560u32;
    fill_rect(&mut img, dx, dy, dw, dh, Rgb([0xFF, 0xFF, 0xFF]));
    rect_border(&mut img, dx, dy, dw, dh, Rgb([0x99, 0x99, 0x99]));
    fill_rect(&mut img, dx, dy, dw, 34, Rgb([0xE4, 0xE4, 0xE4]));
    draw_text(&mut img, &sans_bold, "Custom Sprite List Sizes", (dx + 14) as i32, (dy + 8) as i32, 14.0, ink);

    let mut y = dy + 52;
    draw_text(
        &mut img,
        &sans,
        "How many custom sprites each submap's list may hold, like Lunar Magic v3.51.",
        (dx + 14) as i32,
        y as i32,
        12.0,
        gray,
    );
    y += 18;
    draw_text(
        &mut img,
        &sans,
        "0\u{2013}24 per submap; cannot go below the sprites already placed.",
        (dx + 14) as i32,
        y as i32,
        12.0,
        gray,
    );
    y += 30;

    // One row per submap. Valley of Bowser shows an in-progress edit (12 ->
    // 5) with the real validation error, exactly as the dialog renders it.
    let draft = [8u8, 24, 24, 24, 5, 24, 24];
    for submap in 0..7usize {
        let size = draft[submap];
        let used = table.submaps[submap].len();
        draw_text(&mut img, &sans, SUBMAP_NAMES[submap], (dx + 14) as i32, (y + 4) as i32, 13.0, ink);
        // DragValue box.
        fill_rect(&mut img, dx + 200, y, 64, 26, Rgb([0xFF, 0xFF, 0xFF]));
        rect_border(&mut img, dx + 200, y, 64, 26, Rgb([0x99, 0x99, 0x99]));
        draw_text(&mut img, &mono, &format!("{size}"), (dx + 208) as i32, (y + 5) as i32, 13.0, ink);
        // Usage bar.
        let bar_x = dx + 290;
        let bar_w = 240u32;
        fill_rect(&mut img, bar_x, y + 4, bar_w, 16, Rgb([0xDD, 0xDD, 0xDD]));
        let fill = (bar_w as usize * used.min(size as usize) / size.max(1) as usize) as u32;
        let bar_color = if used >= size as usize { Rgb([0xD8, 0x60, 0x60]) } else { Rgb([0x4A, 0x90, 0xD9]) };
        fill_rect(&mut img, bar_x, y + 4, fill, 16, bar_color);
        draw_text(
            &mut img,
            &mono,
            &format!("{used}/{size} used"),
            (bar_x + bar_w + 12) as i32,
            (y + 4) as i32,
            12.0,
            ink,
        );
        y += 32;
        if submap == 4 {
            draw_text(&mut img, &sans, &err_text, (dx + 200) as i32, y as i32, 12.0, red);
            y += 24;
        }
    }

    // Buttons: Apply is disabled while a row is too small, like the dialog.
    y += 10;
    let bx = dx + 14;
    fill_rect(&mut img, bx, y, 90, 30, Rgb([0xE8, 0xE8, 0xE8]));
    rect_border(&mut img, bx, y, 90, 30, Rgb([0xBB, 0xBB, 0xBB]));
    draw_text(&mut img, &sans, "Apply", (bx + 24) as i32, (y + 7) as i32, 13.0, Rgb([0x99, 0x99, 0x99]));
    fill_rect(&mut img, bx + 104, y, 120, 30, Rgb([0xFF, 0xFF, 0xFF]));
    rect_border(&mut img, bx + 104, y, 120, 30, Rgb([0x99, 0x99, 0x99]));
    draw_text(&mut img, &sans, "Reset to 24", (bx + 116) as i32, (y + 7) as i32, 13.0, ink);
    fill_rect(&mut img, bx + 238, y, 90, 30, Rgb([0xFF, 0xFF, 0xFF]));
    rect_border(&mut img, bx + 238, y, 90, 30, Rgb([0x99, 0x99, 0x99]));
    draw_text(&mut img, &sans, "Close", (bx + 260) as i32, (y + 7) as i32, 13.0, ink);

    draw_text(
        &mut img,
        &sans,
        "Per-submap custom sprite list sizes \u{2014} LM v3.51 parity \u{00B7} default 24 (native max), configurable 0\u{2013}24",
        24,
        (h - 40) as i32,
        12.0,
        gray,
    );
    draw_text(
        &mut img,
        &sans,
        "stored in the OWSPRITE RATS payload (v2) behind the $0EF55D pointer \u{00B7} applies as one undo step",
        24,
        (h - 20) as i32,
        12.0,
        gray,
    );

    img.save(output)?;
    println!("wrote {output}");
    Ok(())
}
