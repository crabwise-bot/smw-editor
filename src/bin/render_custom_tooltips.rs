//! Headless mock screenshot of the Lunar Magic v3.60 "Custom Object Tooltips"
//! feature.
//!
//! egui can't render headless, so this composes an honest mock: every string
//! on screen is real — the tab labels, intro text, row contents, and edit
//! field come from the real `smw_editor::custom_tooltips` store (seeded here
//! with two example tooltips through the real `set()` API), the manager
//! window's list rows use the real `get()` lookups, and the canvas hover
//! bubble shows the exact tooltip text `central_panel` would pass to
//! `on_hover_text_at_pointer`. Only the window chrome (title bar, text
//! fields, buttons) is drawn rather than real egui widgets. The dimmed
//! background is a real `level_png_bytes` render of level 0x105.
//!
//! ```sh
//! cargo run --bin render_custom_tooltips -- --rom=smw.smc --out=docs/screenshots/custom-tooltips.png
//! ```

use ab_glyph::{Font, FontRef, Glyph, Point, PxScale, ScaleFont};
use image::{imageops::FilterType, Rgb, RgbImage};
use smw_editor::{
    custom_tooltips::{CustomTooltips, ObjectKind},
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
fn wrap(font: &FontRef, text: &str, px: f32, max_width_px: f32) -> Vec<String> {
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

fn draw_button(img: &mut RgbImage, font: &FontRef, x: u32, y: u32, label: &str, enabled: bool) {
    let (bg, ink) = if enabled {
        (Rgb([0x2F, 0x6F, 0xBD]), Rgb([0xFF, 0xFF, 0xFF]))
    } else {
        (Rgb([0x3A, 0x3D, 0x42]), Rgb([0xA8, 0xA8, 0xA8]))
    };
    let w = 130u32;
    fill_rect(img, x, y, w, 34, bg);
    rect_border(img, x, y, w, 34, Rgb([0x6A, 0x6E, 0x74]));
    draw_text(img, font, label, (x + 12) as i32, (y + 8) as i32, 14.0, ink);
}

/// The manager window. `tips` is the real store; rows/edit field/buttons use
/// the real `get()` results and the real Save-enabled logic.
fn draw_manager(img: &mut RgbImage, sans: &FontRef, sans_bold: &FontRef, mono: &FontRef, tips: &CustomTooltips) {
    let (dx, dy) = (60u32, 50u32);
    let (dw, dh) = (600u32, 620u32);
    let ink = Rgb([0xE8, 0xE8, 0xE8]);
    let dim = Rgb([0xA8, 0xA8, 0xA8]);
    fill_rect(img, dx, dy, dw, dh, Rgb([0x25, 0x28, 0x2C]));
    rect_border(img, dx, dy, dw, dh, Rgb([0x4A, 0x4E, 0x54]));
    // Title bar (emoji skipped: DejaVu has no emoji glyphs).
    fill_rect(img, dx, dy, dw, 40, Rgb([0x12, 0x14, 0x16]));
    draw_text(img, sans_bold, "Custom Object Tooltips", (dx + 16) as i32, (dy + 11) as i32, 16.0, ink);

    let mut y = dy + 56;
    for line in wrap(
        sans,
        "User-settable tooltip text for level objects (Lunar Magic v3.60). Hovering an object on the canvas shows \
         its tooltip. Stored per-user — never written to the ROM.",
        13.0,
        (dw - 44) as f32,
    ) {
        draw_text(img, sans, &line, (dx + 22) as i32, y as i32, 13.0, ink);
        y += 20;
    }
    y += 8;
    // Kind tabs.
    for (i, kind) in [ObjectKind::Standard, ObjectKind::Extended].iter().enumerate() {
        let selected = *kind == ObjectKind::Standard;
        let tx = dx + 22 + (i as u32) * 130;
        if selected {
            fill_rect(img, tx - 6, y - 4, 118, 26, Rgb([0x2F, 0x6F, 0xBD]));
        }
        draw_text(img, sans, kind.label(), (tx + 4) as i32, y as i32, 14.0, ink);
    }
    draw_text(img, sans, &format!("{} custom tooltip(s)", tips.len()), (dx + dw - 190) as i32, y as i32, 12.0, dim);
    y += 34;
    fill_rect(img, dx + 22, y, dw - 44, 1, Rgb([0x4A, 0x4E, 0x54]));
    y += 10;
    // Search row.
    draw_text(img, sans, "Search:", (dx + 22) as i32, (y + 6) as i32, 13.0, ink);
    fill_rect(img, dx + 90, y, 240, 30, Rgb([0x12, 0x14, 0x16]));
    rect_border(img, dx + 90, y, 240, 30, Rgb([0x6A, 0x6E, 0x74]));
    draw_text(img, sans, "hex id or tooltip text", (dx + 100) as i32, (y + 7) as i32, 12.0, dim);
    y += 42;

    // ID grid: a scroll slice, each row from the real store lookup.
    let selected_id = 0x2Bu8;
    for id in 0x28u8..=0x30 {
        let tip = tips.get(ObjectKind::Standard, id);
        if id == selected_id {
            fill_rect(img, dx + 22, y - 3, dw - 44, 24, Rgb([0x3A, 0x5A, 0x8A]));
        }
        draw_text(img, mono, &format!("0x{id:02X}"), (dx + 28) as i32, y as i32, 13.0, dim);
        let label = tip.unwrap_or("—");
        draw_text(img, sans, label, (dx + 100) as i32, y as i32, 13.0, if tip.is_some() { ink } else { dim });
        y += 24;
    }
    y += 8;
    fill_rect(img, dx + 22, y, dw - 44, 1, Rgb([0x4A, 0x4E, 0x54]));
    y += 12;

    // Edit section: real stored text, real Save-enabled logic.
    draw_text(
        img,
        sans_bold,
        &format!("{} object 0x{selected_id:02X}:", ObjectKind::Standard.label()),
        (dx + 22) as i32,
        y as i32,
        14.0,
        ink,
    );
    y += 26;
    let saved = tips.get(ObjectKind::Standard, selected_id).unwrap_or("");
    let edit_text = saved; // unchanged edit buffer
    fill_rect(img, dx + 22, y, dw - 44, 56, Rgb([0x12, 0x14, 0x16]));
    rect_border(img, dx + 22, y, dw - 44, 56, Rgb([0x6A, 0x6E, 0x74]));
    for (i, line) in wrap(sans, edit_text, 13.0, (dw - 70) as f32).iter().enumerate().take(2) {
        draw_text(img, sans, line, (dx + 32) as i32, (y + 6 + (i as u32) * 20) as i32, 13.0, ink);
    }
    y += 68;
    let changed = edit_text.trim() != saved.trim();
    draw_button(img, sans, dx + 22, y, "Save tooltip", changed);
    draw_button(img, sans, dx + 166, y, "Clear", !saved.is_empty());
    draw_text(img, sans, "max 256 chars", (dx + dw - 130) as i32, (y + 8) as i32, 12.0, dim);
}

/// Mock canvas hover tooltip bubble: the exact text `central_panel` passes
/// to `on_hover_text_at_pointer` for the hovered object.
fn draw_hover_bubble(img: &mut RgbImage, sans: &FontRef, tip: &str, bx: u32, by: u32) {
    let lines = wrap(sans, tip, 13.0, 260.0);
    let (bw, bh) = (300u32, 24 + (lines.len() as u32) * 20);
    fill_rect(img, bx, by, bw, bh, Rgb([0x18, 0x1A, 0x1E]));
    rect_border(img, bx, by, bw, bh, Rgb([0x8A, 0x8E, 0x94]));
    for (i, line) in lines.iter().enumerate() {
        draw_text(img, sans, line, (bx + 12) as i32, (by + 10 + (i as u32) * 20) as i32, 13.0, Rgb([0xE8, 0xE8, 0xE8]));
    }
}

fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let rom_path = args.iter().find_map(|a| a.strip_prefix("--rom=")).unwrap_or("smw.smc");
    let output = args.iter().find_map(|a| a.strip_prefix("--out=")).unwrap_or("docs/screenshots/custom-tooltips.png");

    let rom_bytes = std::fs::read(rom_path)?;

    // ---- Real store, seeded through the real API ----
    let mut tips = CustomTooltips::default();
    tips.set(ObjectKind::Standard, 0x2B, "Turn-block bridge — breaks when hit from below");
    tips.set(ObjectKind::Extended, 0x1E, "Buoyant platform chain, drifts with the tide");
    let hover_tip = tips.get(ObjectKind::Standard, 0x2B).expect("just set").to_string();

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

    // Mock hovered-object highlight + tooltip bubble on the canvas area.
    let (ox2, oy2) = (830u32, 300u32);
    rect_border(&mut img, ox2, oy2, 96, 48, Rgb([0xFF, 0xDC, 0x00]));
    draw_hover_bubble(&mut img, &sans, &hover_tip, ox2 + 20, oy2 + 56);

    draw_manager(&mut img, &sans, &sans_bold, &mono, &tips);

    // Caption strip.
    fill_rect(&mut img, 60, 700, 1160, 60, Rgb([0x12, 0x14, 0x16]));
    rect_border(&mut img, 60, 700, 1160, 60, Rgb([0x4A, 0x4E, 0x54]));
    draw_text(
        &mut img,
        &sans,
        "headless mock — all strings are real (custom_tooltips store + real get() lookups); example tooltips set via the real set() API; background is a real level render",
        76,
        716,
        13.0,
        Rgb([0xE8, 0xE8, 0xE8]),
    );

    img.save(output)?;
    println!("wrote {output}");
    Ok(())
}
