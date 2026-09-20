//! Headless screenshot for Lunar Magic v1.91 "Check Object Placement on
//! Save" (Options menu).
//!
//! egui can't render headless, so this composes an honest mock of the
//! warning dialog: every string on screen is real — the heading line comes
//! from `placement_check::warning_heading`, each issue row is the exact
//! output of `placement_check::format_issue` for issues found by the real
//! `placement_check::check_items` over a constructed misplaced-object
//! scenario, and the hint line is `placement_check::SAVE_KEEPS_HINT`. Only
//! the window chrome and widget shapes are drawn rather than real egui
//! widgets.
//!
//! ```sh
//! cargo run --bin render_check_placement -- --out=docs/screenshots/check-object-placement.png
//! ```

use ab_glyph::{Font, FontRef, Glyph, Point, PxScale, ScaleFont};
use image::{Rgb, RgbImage};
use smw_editor::{
    placement_check::{check_items, format_issue, warning_heading, PlacedItem, PlacementItemKind, SAVE_KEEPS_HINT},
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

fn draw_button(img: &mut RgbImage, font: &FontRef, x: u32, y: u32, w: u32, label: &str) {
    let bg = Rgb([0x2F, 0x6F, 0xBD]);
    let ink = Rgb([0xFF, 0xFF, 0xFF]);
    fill_rect(img, x, y, w, 34, bg);
    rect_border(img, x, y, w, 34, Rgb([0x6A, 0x6E, 0x74]));
    draw_text(img, font, label, (x + 12) as i32, (y + 8) as i32, 14.0, ink);
}

fn arg(name: &str, default: &str) -> String {
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        if a == name {
            return args.next().unwrap_or_else(|| default.to_string());
        }
    }
    default.to_string()
}

fn main() -> anyhow::Result<()> {
    let out_path = arg("--out", "docs/screenshots/check-object-placement.png");

    // A constructed misplacement scenario on level $105 (horizontal, 3
    // screens = 48×27 tiles): an object past the last screen, a sprite
    // below the level, and a direct Map16 rectangle spilling past the right
    // edge. All three rows below are real `check_items`/`format_issue`
    // output, not hand-written strings.
    let issues = check_items(0x105, false, 3, &[
        PlacedItem::at(49, 10, PlacementItemKind::Object, "object $2A"),
        PlacedItem::at(5, 27, PlacementItemKind::Sprite, "sprite $7B"),
        PlacedItem::rect(46, 20, 4, 2, PlacementItemKind::DirectMap16, "direct Map16 object".to_string()),
    ]);
    assert_eq!(issues.len(), 3, "scenario must produce exactly three issues");
    let rows: Vec<String> = issues.iter().map(format_issue).collect();
    eprintln!("check_items found {} issues:", issues.len());
    for r in &rows {
        eprintln!("  {r}");
    }

    // ── Honest mock of the warning dialog ──
    let font = load_font(SANS_CANDIDATES)?;
    let font_bold = load_font(SANS_BOLD_CANDIDATES)?;
    let (w, h) = (960u32, 400u32);
    let mut img = RgbImage::new(w, h);
    let bg = Rgb([0x1B, 0x1D, 0x20]);
    let panel = Rgb([0x25, 0x28, 0x2C]);
    let titlebar = Rgb([0x12, 0x14, 0x16]);
    let ink = Rgb([0xE8, 0xE8, 0xE8]);
    let dim = Rgb([0xA8, 0xA8, 0xA8]);
    for p in img.pixels_mut() {
        *p = bg;
    }
    let (dx, dy, dw, dh) = (20u32, 20u32, 920u32, 360u32);
    fill_rect(&mut img, dx, dy, dw, dh, panel);
    rect_border(&mut img, dx, dy, dw, dh, Rgb([0x4A, 0x4E, 0x54]));
    fill_rect(&mut img, dx, dy, dw, 40, titlebar);
    draw_text(&mut img, &font_bold, "Object Placement Warning", (dx + 16) as i32, (dy + 11) as i32, 17.0, ink);

    // Real dialog strings from the actual code.
    draw_text(&mut img, &font, &warning_heading(issues.len()), (dx + 16) as i32, (dy + 62) as i32, 14.0, ink);
    let mut y = (dy + 94) as i32;
    for row in &rows {
        draw_text(&mut img, &font, "⚠", (dx + 20) as i32, y, 13.0, Rgb([0xFF, 0xCD, 0x5A]));
        draw_text(&mut img, &font, row, (dx + 44) as i32, y, 13.0, ink);
        y += 26;
    }
    draw_text(&mut img, &font, SAVE_KEEPS_HINT, (dx + 16) as i32, y + 8, 12.0, dim);

    draw_button(&mut img, &font, dx + 16, dy + dh - 50, 130, "Save anyway");
    draw_button(&mut img, &font, dx + 162, dy + dh - 50, 110, "Cancel");

    img.save(&out_path)?;
    eprintln!("wrote {out_path}");
    Ok(())
}
