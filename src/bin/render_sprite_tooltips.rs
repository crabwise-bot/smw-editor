//! Headless screenshot of the sprite picker's extension-byte variant
//! tooltips (Lunar Magic v3.30 parity).
//!
//! egui can't render headless, so this composes an honest mock of hovering a
//! sprite picker entry. The tooltip strings are the exact output of the
//! unit-tested `sprite_tooltip` in `sprite_catalog.rs` (verified by
//! `cargo test sprite_tooltip_lists_extra_bit_variants`; probed via a
//! temporary test on 2026-09-19):
//!
//! ```text
//! 7B — Goal Point
//! Extra bits 2: Goal Point (Secret Exit 2)
//! Extra bits 3: Goal Point (Secret Exit 3)
//! ```
//!
//! ```sh
//! cargo run --bin render_sprite_tooltips -- --out=docs/screenshots/sprite-extbyte-tooltips.png
//! ```

use ab_glyph::{Font, FontRef, Glyph, Point, PxScale, ScaleFont};
use image::{Rgb, RgbImage};
use smw_editor::render_util::{fill_rect, rect_border};

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

/// Exact output of `sprite_tooltip` (see module docs).
const TOOLTIP_7B: &[&str] =
    &["7B — Goal Point", "Extra bits 2: Goal Point (Secret Exit 2)", "Extra bits 3: Goal Point (Secret Exit 3)"];
const TOOLTIP_0F: &[&str] = &["0F — Goomba"];

/// Mock a sprite picker row with the hover tooltip beside it.
fn draw_picker_with_tooltip(
    img: &mut RgbImage, sans: &FontRef, sans_bold: &FontRef, x: u32, y: u32, w: u32, hovered: &str, tooltip: &[&str],
) {
    let white = Rgb([235, 235, 240]);
    let dim = Rgb([120, 124, 132]);
    fill_rect(img, x, y, w, 30 * 3 + 44, Rgb([43, 46, 53]));
    rect_border(img, x, y, w, 30 * 3 + 44, Rgb([100, 104, 112]));
    draw_text(img, sans_bold, "Sprites", x as i32 + 10, y as i32 + 6, 14.0, white);
    let rows = [("0F", "Goomba"), ("7B", "Goal Point"), ("0D", "Buzzy Beetle")];
    for (i, (id, name)) in rows.iter().enumerate() {
        let ry = y + 34 + i as u32 * 30;
        let label = format!("{id}  {name}");
        if label.starts_with(hovered) {
            fill_rect(img, x + 6, ry, w - 12, 26, Rgb([58, 110, 165]));
        }
        draw_text(img, sans, &label, x as i32 + 12, ry as i32 + 4, 13.0, white);
    }
    // Tooltip popup.
    let tx = x + w + 12;
    let ty = y + 40;
    let tw = 380u32;
    let th = 20 + tooltip.len() as u32 * 20 + 10;
    fill_rect(img, tx, ty, tw, th, Rgb([24, 25, 29]));
    rect_border(img, tx, ty, tw, th, Rgb([150, 154, 162]));
    for (i, line) in tooltip.iter().enumerate() {
        draw_text(img, sans, line, tx as i32 + 10, (ty + 8 + i as u32 * 20) as i32, 13.0, white);
    }
    draw_text(img, sans, "(hover tooltip)", tx as i32, (ty + th + 6) as i32, 12.0, dim);
}

fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let output =
        args.iter().find_map(|a| a.strip_prefix("--out=")).unwrap_or("docs/screenshots/sprite-extbyte-tooltips.png");

    let sans = load_font(SANS_CANDIDATES)?;
    let sans_bold = load_font(SANS_BOLD_CANDIDATES)?;

    let w = 1500u32;
    let h = 360u32;
    let mut img = RgbImage::new(w, h);
    fill_rect(&mut img, 0, 0, w, h, Rgb([26, 28, 33]));
    let white = Rgb([235, 235, 240]);
    let dim = Rgb([120, 124, 132]);

    draw_text(
        &mut img,
        &sans_bold,
        "Sprite picker — extension-byte variant tooltips  (Lunar Magic v3.30 parity)",
        24,
        16,
        20.0,
        white,
    );

    draw_picker_with_tooltip(&mut img, &sans, &sans_bold, 24, 60, 260, "7B", TOOLTIP_7B);
    draw_picker_with_tooltip(&mut img, &sans, &sans_bold, 740, 60, 260, "0F", TOOLTIP_0F);

    draw_text(
        &mut img,
        &sans,
        "Headless mock — tooltip strings are the exact unit-tested sprite_tooltip output; sprites without variants show just the name.",
        24,
        (h - 28) as i32,
        13.0,
        dim,
    );

    img.save(output)?;
    println!("wrote {output}");
    Ok(())
}
