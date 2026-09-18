//! Headless mock screenshot of the Lunar Magic "Edit Manual" dialog (v1.91).
//!
//! egui can't render headless, so this composes an honest mock of the
//! dialog: every string on screen is real — the byte values come from the
//! real ROM (level 0x105's first standard object and first sprite, encoded
//! with the same `smw_editor::edit_manual` codec the UI uses), and the
//! "Decodes as:" lines are the exact `object_decoded_summary` /
//! `sprite_decoded_summary` strings the dialog shows. Only the window chrome
//! (title bar, text fields, buttons) is drawn rather than real egui widgets.
//! The dimmed background is a real `level_png_bytes` render of level 0x105.
//!
//! ```sh
//! cargo run --bin render_edit_manual -- --rom=smw.smc --out=docs/screenshots/edit-manual.png
//! ```

use ab_glyph::{Font, FontRef, Glyph, Point, PxScale, ScaleFont};
use image::{imageops::FilterType, Rgb, RgbImage};
use smw_editor::{
    edit_manual::{
        decode_object_bytes,
        encode_object_bytes,
        encode_sprite_bytes,
        object_decoded_summary,
        sprite_decoded_summary,
    },
    level_png_export::{level_png_bytes, LevelPngOptions},
    render_util::{fill_rect, rect_border},
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
                if px_x >= 0 && px_y >= 0 && (px_x as u32) < img.width() && (px_y as u32) < img.height() {
                    let d = img.get_pixel(px_x as u32, px_y as u32).0;
                    let s = color.0;
                    let a = (v * 255.0) as u16;
                    let inv = 255 - a;
                    img.put_pixel(
                        px_x as u32,
                        px_y as u32,
                        Rgb([
                            ((s[0] as u16 * a + d[0] as u16 * inv) / 255) as u8,
                            ((s[1] as u16 * a + d[1] as u16 * inv) / 255) as u8,
                            ((s[2] as u16 * a + d[2] as u16 * inv) / 255) as u8,
                        ]),
                    );
                }
            });
        }
        caret_x += scaled.h_advance(id);
        prev = Some(id);
    }
}

/// Word-wrap `text` into lines that fit `max_width_px` at `px` size.
fn wrap<'a>(font: &FontRef, text: &'a str, px: f32, max_width_px: f32) -> Vec<String> {
    let scaled = font.as_scaled(PxScale::from(px));
    let width_of = |s: &str| -> f32 {
        let mut w = 0.0;
        let mut prev = None;
        for ch in s.chars() {
            let id = font.glyph_id(ch);
            if let Some(p) = prev {
                w += scaled.kern(p, id);
            }
            w += scaled.h_advance(id);
            prev = Some(id);
        }
        w
    };
    let mut lines = Vec::new();
    let mut cur = String::new();
    for word in text.split_whitespace() {
        let trial = if cur.is_empty() { word.to_string() } else { format!("{cur} {word}") };
        if width_of(&trial) > max_width_px && !cur.is_empty() {
            lines.push(std::mem::take(&mut cur));
        }
        if !cur.is_empty() {
            cur.push(' ');
        }
        cur.push_str(word);
    }
    if !cur.is_empty() {
        lines.push(cur);
    }
    lines
}

fn draw_hex_field(img: &mut RgbImage, font: &FontRef, x: u32, y: u32, label: &str, value: &str) {
    let ink = Rgb([0xE8, 0xE8, 0xE8]);
    let dim = Rgb([0xA8, 0xA8, 0xA8]);
    draw_text(img, font, label, x as i32, y as i32, 12.0, dim);
    fill_rect(img, x, y + 20, 84, 32, Rgb([0x12, 0x14, 0x16]));
    rect_border(img, x, y + 20, 84, 32, Rgb([0x6A, 0x6E, 0x74]));
    draw_text(img, font, value, (x + 10) as i32, (y + 27) as i32, 16.0, ink);
}

fn draw_button(img: &mut RgbImage, font: &FontRef, x: u32, y: u32, label: &str, enabled: bool) {
    let (bg, ink) = if enabled {
        (Rgb([0x2F, 0x6F, 0xBD]), Rgb([0xFF, 0xFF, 0xFF]))
    } else {
        (Rgb([0x3A, 0x3D, 0x42]), Rgb([0xA8, 0xA8, 0xA8]))
    };
    let w = 96u32;
    fill_rect(img, x, y, w, 34, bg);
    rect_border(img, x, y, w, 34, Rgb([0x6A, 0x6E, 0x74]));
    draw_text(img, font, label, (x + 14) as i32, (y + 8) as i32, 14.0, ink);
}

/// One Edit Manual dialog window. `bytes` and `summary` are the real dialog
/// strings for the selected entry.
#[allow(clippy::too_many_arguments)]
fn draw_dialog(
    img: &mut RgbImage, sans: &FontRef, sans_bold: &FontRef, mono: &FontRef, dx: u32, dy: u32, title: &str,
    intro: &str, bytes: [u8; 3], summary: &str,
) {
    let (dw, dh) = (560u32, 330u32);
    let ink = Rgb([0xE8, 0xE8, 0xE8]);
    let dim = Rgb([0xA8, 0xA8, 0xA8]);
    fill_rect(img, dx, dy, dw, dh, Rgb([0x25, 0x28, 0x2C]));
    rect_border(img, dx, dy, dw, dh, Rgb([0x4A, 0x4E, 0x54]));
    // Title bar.
    fill_rect(img, dx, dy, dw, 40, Rgb([0x12, 0x14, 0x16]));
    draw_text(img, sans_bold, title, (dx + 16) as i32, (dy + 11) as i32, 16.0, ink);

    let mut y = dy + 56;
    for line in wrap(sans, intro, 13.0, (dw - 44) as f32) {
        draw_text(img, sans, &line, (dx + 22) as i32, y as i32, 13.0, ink);
        y += 20;
    }
    y += 6;
    // Byte fields.
    for (i, label) in ["Byte 0", "Byte 1", "Byte 2"].iter().enumerate() {
        draw_hex_field(img, mono, dx + 22 + (i as u32) * 120, y, label, &format!("{:02X}", bytes[i]));
    }
    y += 70;
    draw_text(img, sans, "Decodes as:", (dx + 22) as i32, y as i32, 13.0, dim);
    y += 22;
    for line in wrap(mono, summary, 13.0, (dw - 44) as f32) {
        draw_text(img, mono, &line, (dx + 22) as i32, y as i32, 13.0, ink);
        y += 20;
    }
    y += 10;
    draw_button(img, sans, dx + 22, y, "Apply", true);
    draw_button(img, sans, dx + 132, y, "Reset", true);
    draw_button(img, sans, dx + 242, y, "Close", true);
    draw_text(img, sans, "(LM v1.91)", (dx + dw - 110) as i32, (y + 8) as i32, 12.0, dim);
}

fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let rom_path = args.iter().find_map(|a| a.strip_prefix("--rom=")).unwrap_or("smw.smc");
    let output = args.iter().find_map(|a| a.strip_prefix("--out=")).unwrap_or("docs/screenshots/edit-manual.png");

    let rom_bytes = std::fs::read(rom_path)?;
    let smw_rom = smwe_rom::SmwRom::from_file(rom_path)?;
    let level = &smw_rom.levels[0x105];

    // ---- Real stream bytes from the ROM, via the shared codec ----
    // First standard (non-extended, non-exit) object in level 0x105's stream.
    let raw_objects = smwe_rom::objects::Object::parse_from_layer(level.layer1.as_bytes())
        .ok_or_else(|| anyhow::anyhow!("couldn't parse level 0x105 object layer"))?;
    let obj = raw_objects
        .iter()
        .find(|o| o.is_standard() && !o.is_new_screen())
        .ok_or_else(|| anyhow::anyhow!("no standard object in level 0x105"))?;
    let obj_bytes =
        encode_object_bytes(obj.standard_object_number(), obj.settings(), false, obj.x(), obj.y(), obj.is_new_screen());
    let obj_id = decode_object_bytes(obj_bytes).id;
    let obj_summary = object_decoded_summary(obj_bytes, false).expect("real object bytes must decode");

    // First sprite in level 0x105.
    let spr = level.sprite_layer.sprites.first().ok_or_else(|| anyhow::anyhow!("no sprites in level 0x105"))?;
    let (sx, sy) = spr.xy_pos();
    let spr_bytes = encode_sprite_bytes(sx, sy, spr.screen_number(), spr.extra_bits(), spr.sprite_id());
    let spr_summary = sprite_decoded_summary(spr_bytes);

    // ---- Background: real level 0x105 render, dimmed ----
    let png = level_png_bytes(&rom_bytes, 0x105, &LevelPngOptions::default())?;
    let full = image::load_from_memory(&png)?.to_rgb8();
    let (fw, fh) = (full.width(), full.height());
    let (w, h) = (1280u32, 800u32);
    let scale = (w as f32 / fw as f32).max(h as f32 / fh as f32);
    let scaled =
        image::imageops::resize(&full, (fw as f32 * scale) as u32, (fh as f32 * scale) as u32, FilterType::Triangle);
    let ox = (scaled.width().saturating_sub(w)) / 2;
    let oy = (scaled.height().saturating_sub(h)) / 2;
    let crop = image::imageops::crop_imm(&scaled, ox, oy, w, h).to_image();
    let mut img = RgbImage::new(w, h);
    for (x, y, px) in crop.enumerate_pixels() {
        let d = px.0;
        img.put_pixel(x, y, Rgb([d[0] / 3, d[1] / 3, d[2] / 3]));
    }

    let sans = load_font(SANS_CANDIDATES)?;
    let sans_bold = load_font(SANS_BOLD_CANDIDATES)?;
    let mono = load_font(MONO_CANDIDATES)?;

    draw_dialog(
        &mut img,
        &sans,
        &sans_bold,
        &mono,
        60,
        60,
        &format!("Edit Manual — Object 0x{obj_id:02X}"),
        "Edit the selected entry's raw level-data bytes, Lunar Magic style. \
         Type 1–2 hex digits per byte ($ or 0x prefix accepted).",
        obj_bytes,
        &obj_summary,
    );
    draw_dialog(
        &mut img,
        &sans,
        &sans_bold,
        &mono,
        660,
        60,
        &format!("Edit Manual — Sprite 0x{:02X}", spr.sprite_id()),
        "Edit the selected entry's raw level-data bytes, Lunar Magic style. \
         Type 1–2 hex digits per byte ($ or 0x prefix accepted).",
        spr_bytes,
        &spr_summary,
    );

    // Caption strip.
    let dim = Rgb([0xE8, 0xE8, 0xE8]);
    fill_rect(&mut img, 60, 700, 1160, 60, Rgb([0x12, 0x14, 0x16]));
    rect_border(&mut img, 60, 700, 1160, 60, Rgb([0x4A, 0x4E, 0x54]));
    draw_text(
        &mut img,
        &sans,
        "headless mock — byte values and decoded summaries are real (level 0x105 ROM data, shared edit_manual codec); \
         background is a real level render",
        80,
        722,
        13.0,
        dim,
    );

    img.save(output)?;
    println!("wrote {output}");
    Ok(())
}
