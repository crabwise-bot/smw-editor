//! Headless mock screenshot of the "Export Multiple Levels to Image Files"
//! dialog (File > Levels > Export Multiple Levels to Image Files...).
//!
//! egui can't render headless, so this composes an honest mock of the dialog:
//! every string on screen is real — produced from the same code the UI uses
//! (`LEVEL_COUNT`, `level_export_filename`, the dialog defaults). Only the
//! window chrome (title bar, text fields, checkboxes, buttons) is drawn rather
//! than real egui widgets. The thumbnail strip at the bottom is NOT a mock:
//! those are real `level_png_bytes` exports of the ROM's actual levels,
//! downscaled.
//!
//! ```sh
//! cargo run --bin render_export_levels -- --rom=smw.smc --out=docs/screenshots/export-levels-png.png
//! ```

use ab_glyph::{Font, FontRef, Glyph, Point, PxScale, ScaleFont};
use image::{imageops::FilterType, Rgb, RgbImage};
use smw_editor::{
    level_png_export::{level_export_filename, level_png_bytes, LevelPngOptions, LEVEL_COUNT},
    render_util::{fill_rect, rect_border},
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

fn draw_checkbox(img: &mut RgbImage, font: &FontRef, x: u32, y: u32, label: &str, checked: bool) {
    let box_c = Rgb([0x3A, 0x3D, 0x42]);
    let ink = Rgb([0xE8, 0xE8, 0xE8]);
    let accent = Rgb([0x4D, 0x9F, 0xFF]);
    fill_rect(img, x, y, 18, 18, box_c);
    rect_border(img, x, y, 18, 18, Rgb([0x6A, 0x6E, 0x74]));
    if checked {
        draw_text(img, font, "\u{2713}", x as i32 + 2, y as i32 - 2, 16.0, accent);
    }
    draw_text(img, font, label, (x + 26) as i32, y as i32 - 2, 14.0, ink);
}

fn draw_text_field(img: &mut RgbImage, font: &FontRef, x: u32, y: u32, w: u32, value: &str) {
    let field = Rgb([0x12, 0x14, 0x16]);
    fill_rect(img, x, y, w, 28, field);
    rect_border(img, x, y, w, 28, Rgb([0x6A, 0x6E, 0x74]));
    draw_text(img, font, value, (x + 8) as i32, (y + 5) as i32, 14.0, Rgb([0xE8, 0xE8, 0xE8]));
}

fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let rom_path = args.iter().find_map(|a| a.strip_prefix("--rom=")).unwrap_or("smw.smc");
    let output = args.iter().find_map(|a| a.strip_prefix("--out=")).unwrap_or("docs/screenshots/export-levels-png.png");

    let rom_bytes = std::fs::read(rom_path)?;
    let opts = LevelPngOptions::default();

    // ---- Real exports for the thumbnail strip (same path the UI calls) ----
    // Levels are whole-map renders (up to 8192 px wide); crop the top-left
    // 1024x432 viewport region so the thumbnail stays readable.
    let mut thumbs: Vec<(u16, u32, u32, RgbImage)> = Vec::new();
    for level in [0x000u16, 0x105, 0x1FF] {
        let png = level_png_bytes(&rom_bytes, level, &opts)?;
        let full = image::load_from_memory(&png)?.to_rgb8();
        let (fw, fh) = (full.width(), full.height());
        let (cw, ch) = (fw.min(1024), fh.min(432));
        let crop = image::imageops::crop_imm(&full, 0, 0, cw, ch).to_image();
        let th = image::imageops::resize(&crop, 300, (300 * ch) / cw.max(1), FilterType::Triangle);
        thumbs.push((level, fw, fh, th));
    }

    let sans = load_font(SANS_CANDIDATES)?;
    let sans_bold = load_font(SANS_BOLD_CANDIDATES)?;

    // ---- Compose the mock dialog ----
    let (w, h) = (980u32, 760u32);
    let mut img = RgbImage::new(w, h);
    let bg = Rgb([0x1B, 0x1D, 0x20]);
    let panel = Rgb([0x25, 0x28, 0x2C]);
    let titlebar = Rgb([0x12, 0x14, 0x16]);
    let ink = Rgb([0xE8, 0xE8, 0xE8]);
    let dim = Rgb([0xA8, 0xA8, 0xA8]);
    for p in img.pixels_mut() {
        *p = bg;
    }

    let (dx, dy, dw) = (30u32, 24u32, 920u32);
    let dh = 700u32;
    fill_rect(&mut img, dx, dy, dw, dh, panel);
    rect_border(&mut img, dx, dy, dw, dh, Rgb([0x4A, 0x4E, 0x54]));
    // Title bar.
    fill_rect(&mut img, dx, dy, dw, 40, titlebar);
    draw_text(
        &mut img,
        &sans_bold,
        "Export Multiple Levels to Image Files",
        (dx + 16) as i32,
        (dy + 11) as i32,
        17.0,
        ink,
    );
    draw_text(
        &mut img,
        &sans,
        "headless mock \u{2014} range, defaults and filenames are real; thumbnails are real exports",
        (dx + 430) as i32,
        (dy + 14) as i32,
        12.0,
        dim,
    );

    let mut y = dy + 62u32;
    draw_text(
        &mut img,
        &sans,
        &format!(
            "Renders each level in the range as a PNG image,\none file per level ({} \u{2026} {}).",
            level_export_filename(0),
            level_export_filename(LEVEL_COUNT - 1)
        ),
        (dx + 20) as i32,
        y as i32,
        14.0,
        ink,
    );
    y += 56;
    // Range row.
    draw_text(&mut img, &sans, "From level (hex):", (dx + 20) as i32, (y + 5) as i32, 14.0, ink);
    draw_text_field(&mut img, &sans, dx + 170, y, 90, "000");
    draw_text(&mut img, &sans, "To level (hex):", (dx + 280) as i32, (y + 5) as i32, 14.0, ink);
    draw_text_field(&mut img, &sans, dx + 420, y, 90, &format!("{:03X}", LEVEL_COUNT - 1));
    y += 44;
    // Layer toggles (dialog defaults: all on).
    draw_checkbox(&mut img, &sans, dx + 20, y, "Layer 1", true);
    draw_checkbox(&mut img, &sans, dx + 160, y, "Layer 2", true);
    draw_checkbox(&mut img, &sans, dx + 300, y, "Sprites", true);
    y += 40;
    // Output folder row.
    draw_text(&mut img, &sans, "Output folder:", (dx + 20) as i32, (y + 5) as i32, 14.0, ink);
    draw_text(&mut img, &sans, "(not chosen)", (dx + 150) as i32, (y + 5) as i32, 14.0, dim);
    fill_rect(&mut img, dx + 300, y, 110, 30, Rgb([0x3A, 0x3D, 0x42]));
    rect_border(&mut img, dx + 300, y, 110, 30, Rgb([0x6A, 0x6E, 0x74]));
    draw_text(&mut img, &sans, "Choose...", (dx + 322) as i32, (y + 6) as i32, 14.0, ink);
    y += 52;
    // Buttons: Export is disabled until a folder is chosen (real behavior).
    fill_rect(&mut img, dx + 20, y, 110, 34, Rgb([0x3A, 0x3D, 0x42]));
    draw_text(&mut img, &sans, "Export", (dx + 48) as i32, (y + 8) as i32, 14.0, dim);
    fill_rect(&mut img, dx + 142, y, 110, 34, Rgb([0x2F, 0x6F, 0xBD]));
    draw_text(&mut img, &sans_bold, "Close", (dx + 176) as i32, (y + 8) as i32, 14.0, Rgb([0xFF, 0xFF, 0xFF]));
    y += 52;

    // Thumbnail strip: real level exports, downscaled.
    draw_text(&mut img, &sans_bold, "Sample output (real exports):", (dx + 20) as i32, y as i32, 14.0, ink);
    y += 28;
    let mut tx = dx + 20;
    for (level, fw, fh, th) in &thumbs {
        let (tw, thh) = (th.width(), th.height());
        if tx + tw + 180 < dx + dw {
            for (ox, oy, px) in th.enumerate_pixels() {
                img.put_pixel(tx + ox, y + oy, *px);
            }
            rect_border(&mut img, tx, y, tw, thh, Rgb([0x6A, 0x6E, 0x74]));
            draw_text(
                &mut img,
                &sans,
                &format!("{} (full export {fw}\u{00D7}{fh})", level_export_filename(*level),),
                tx as i32,
                (y + thh + 6) as i32,
                12.0,
                dim,
            );
            tx += tw + 30;
        }
    }

    img.save(output)?;
    println!("wrote {output}");
    Ok(())
}
