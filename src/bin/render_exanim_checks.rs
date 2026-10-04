//! Headless screenshot for the LM v3.40 ExAnimation validation (backlog #44).
//!
//! Honest mock of the two new UI surfaces: the Options-menu "More
//! ExAnimation Checks" checkbox and the "ExAnimated Frames" dialog's warning
//! banner. Every warning line is the real [`smwe_rom::exanimation::ExAnimWarning::describe`]
//! output over a real [`smwe_rom::exanimation::ExAnimation`] built from real
//! [`smwe_rom::exanimation::ExAnimFrame`] values (disabled line/palette
//! destinations + a duplicated one-shot trigger number), run through the real
//! [`smwe_rom::exanimation::validate_animation`]. The one-shot trigger number
//! spinner shows the real model field; the dialog chrome around it is a mock.
//!
//! Usage:
//!   render_exanim_checks --out=docs/screenshots/exanim-checks.png

use ab_glyph::{Font, FontRef, Glyph, Point, PxScale, ScaleFont};
use image::{Rgb, RgbImage};
use smwe_rom::exanimation::{validate_animation, ExAnimFrame, ExAnimFrameKind, ExAnimTrigger, ExAnimation};

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

fn text_width(font: &FontRef, text: &str, px: f32) -> f32 {
    let scaled = font.as_scaled(PxScale::from(px));
    let mut w = 0.0f32;
    let mut prev = None;
    for ch in text.chars() {
        let id = font.glyph_id(ch);
        if let Some(p) = prev {
            w += scaled.kern(p, id);
        }
        w += scaled.h_advance(id);
        prev = Some(id);
    }
    w
}

/// Greedy word-wrap; returns the lines.
fn wrap(font: &FontRef, text: &str, px: f32, max_w: f32) -> Vec<String> {
    let mut lines = Vec::new();
    let mut cur = String::new();
    for word in text.split(' ') {
        let trial = if cur.is_empty() { word.to_owned() } else { format!("{cur} {word}") };
        if text_width(font, &trial, px) <= max_w {
            cur = trial;
        } else {
            if !cur.is_empty() {
                lines.push(cur);
            }
            cur = word.to_owned();
        }
    }
    if !cur.is_empty() {
        lines.push(cur);
    }
    lines
}

fn fill(img: &mut RgbImage, x0: u32, y0: u32, x1: u32, y1: u32, c: Rgb<u8>) {
    for y in y0..y1.min(img.height()) {
        for x in x0..x1.min(img.width()) {
            img.put_pixel(x, y, c);
        }
    }
}

fn stroke_rect(img: &mut RgbImage, x0: u32, y0: u32, x1: u32, y1: u32, c: Rgb<u8>) {
    for x in x0..x1 {
        img.put_pixel(x, y0, c);
        img.put_pixel(x, y1 - 1, c);
    }
    for y in y0..y1 {
        img.put_pixel(x0, y, c);
        img.put_pixel(x1 - 1, y, c);
    }
}

fn main() -> anyhow::Result<()> {
    let out = std::env::args()
        .find_map(|a| a.strip_prefix("--out=").map(str::to_owned))
        .unwrap_or_else(|| "/tmp/exanim_checks.png".to_owned());
    let sans = load_font(SANS_CANDIDATES)?;
    let sans_bold = load_font(SANS_BOLD_CANDIDATES)?;

    // ── Real model: four frames exercising both LM v3.40 warnings ──────────
    let line = |dest: u16, trigger: ExAnimTrigger, num: u8| ExAnimFrame {
        kind: ExAnimFrameKind::Line8x8,
        dest,
        speed: 1,
        trigger,
        trigger_num: num,
        frames: 2,
        units_per_frame: 1,
        payload: vec![0x1000, 0x1010],
    };
    let mut anim = ExAnimation::default();
    anim.frames.push(line(0x9000, ExAnimTrigger::Always, 0)); // disabled line slot
    anim.frames.push(ExAnimFrame {
        kind:            ExAnimFrameKind::Palette,
        dest:            0x0120, // disabled palette slot (past CGRAM)
        speed:           1,
        trigger:         ExAnimTrigger::Always,
        trigger_num:     0,
        frames:          2,
        units_per_frame: 2,
        payload:         vec![0x001F, 0x03E0, 0x7C00, 0x7FFF],
    });
    anim.frames.push(line(0x1000, ExAnimTrigger::OneShot, 5));
    anim.frames.push(line(0x1040, ExAnimTrigger::OneShot, 5)); // duplicate #5
    let warnings = validate_animation(&anim);
    let warning_lines: Vec<String> = warnings.iter().map(|w| w.describe(&anim)).collect();
    assert_eq!(warning_lines.len(), 3, "demo data must trip both checks");

    // ── Canvas ─────────────────────────────────────────────────────────────
    const W: u32 = 1020;
    const H: u32 = 860;
    let mut img = RgbImage::new(W, H);
    fill(&mut img, 0, 0, W, H, Rgb([22, 23, 28])); // app background

    let ink = Rgb([225, 228, 235]);
    let dim = Rgb([150, 156, 168]);
    let warn_bg = Rgb([72, 54, 10]);
    let warn_ink = Rgb([255, 200, 80]);
    let warn_text = Rgb([255, 226, 150]);

    // ── Options menu mock ──────────────────────────────────────────────────
    let mut y = 14;
    draw_text(&mut img, &sans_bold, "Options menu (Lunar Magic v3.40 \"General Options\" parity)", 16, y, 19.0, ink);
    y += 34;
    fill(&mut img, 16, y as u32, W - 16, (y + 148) as u32, Rgb([30, 32, 38]));
    stroke_rect(&mut img, 16, y as u32, W - 16, (y + 148) as u32, Rgb([60, 63, 72]));
    y += 14;
    draw_text(&mut img, &sans, "☑ Check Object Placement on Save", 30, y, 15.0, ink);
    y += 30;
    draw_text(&mut img, &sans, "☑ More ExAnimation Checks", 30, y, 15.0, ink);
    y += 26;
    for line in wrap(
        &sans,
        "When enabled, the ExAnimated Frames dialog warns about ExAnimation destinations set to \
         disabled slots and about the same one-shot trigger number assigned to more than one slot \
         (Lunar Magic v3.40). Uncheck to disable the warnings.",
        13.0,
        (W - 70) as f32,
    ) {
        draw_text(&mut img, &sans, &line, 44, y, 13.0, dim);
        y += 20;
    }

    // ── ExAnimated Frames dialog mock ──────────────────────────────────────
    y += 18;
    let dlg_y = y;
    fill(&mut img, 16, dlg_y as u32, W - 16, H - 14, Rgb([30, 32, 38]));
    stroke_rect(&mut img, 16, dlg_y as u32, W - 16, H - 14, Rgb([60, 63, 72]));
    // title bar
    fill(&mut img, 16, dlg_y as u32, W - 16, (dlg_y + 34) as u32, Rgb([44, 47, 55]));
    draw_text(&mut img, &sans_bold, "ExAnimated Frames", 30, dlg_y + 6, 17.0, ink);
    y = dlg_y + 46;

    // warning banner (real warning text)
    let banner_h =
        34 + warning_lines.iter().map(|l| wrap(&sans, l, 14.0, (W - 120) as f32).len() as i32 * 22).sum::<i32>();
    fill(&mut img, 28, y as u32, W - 28, (y + banner_h) as u32, warn_bg);
    stroke_rect(&mut img, 28, y as u32, W - 28, (y + banner_h) as u32, Rgb([140, 105, 25]));
    y += 10;
    draw_text(
        &mut img,
        &sans_bold,
        "⚠ ExAnimation checks (Lunar Magic v3.40) — click a warning to select its frame:",
        40,
        y,
        15.0,
        warn_ink,
    );
    y += 28;
    for wl in &warning_lines {
        for part in wrap(&sans, wl, 14.0, (W - 140) as f32) {
            draw_text(&mut img, &sans, &part, 56, y, 14.0, warn_text);
            y += 22;
        }
        y += 2;
    }
    y += 8;

    // frame list (left) + frame editor (right) for the selected frame #2
    let list_x0 = 28u32;
    let list_x1 = 400u32;
    let ed_x0 = 420u32;
    let rows = [
        ("#0 Line 8x8 @ $9000 · 2f", false),
        ("#1 Palette @ $0120 · 2f", false),
        ("#2 Line 8x8 @ $1000 · 2f", true),
        ("#3 Line 8x8 @ $1040 · 2f", false),
    ];
    let mut ry = y;
    for (label, sel) in rows {
        if sel {
            fill(&mut img, list_x0, ry as u32, list_x1, (ry + 28) as u32, Rgb([52, 78, 120]));
        }
        draw_text(&mut img, &sans, label, (list_x0 + 10) as i32, ry + 5, 14.0, ink);
        ry += 30;
    }
    draw_text(&mut img, &sans, "Type: Line 8x8", ed_x0 as i32, y, 15.0, ink);
    draw_text(&mut img, &sans, "Trigger: One-shot", ed_x0 as i32, y + 30, 15.0, ink);
    draw_text(
        &mut img,
        &sans,
        &format!("One-shot #: {} (Lunar Magic v3.40)", anim.frames[2].trigger_num),
        ed_x0 as i32,
        y + 60,
        15.0,
        ink,
    );
    draw_text(&mut img, &sans, "VRAM dest: $1000", ed_x0 as i32, y + 90, 15.0, ink);
    draw_text(
        &mut img,
        &sans,
        "Frame #2's trigger number 5 (real model field) is also used by frame #3 —",
        ed_x0 as i32,
        y + 124,
        13.0,
        dim,
    );
    draw_text(&mut img, &sans, "hence the duplicate-number warning above.", ed_x0 as i32, y + 144, 13.0, dim);

    // Crop the empty space below the content.
    let ch = ((y + 170).min(H as i32 - 14)) as u32;
    let img = image::imageops::crop(&mut img, 0, 0, W, ch).to_image();
    img.save(&out)?;
    println!("wrote {out} ({} warnings rendered)", warning_lines.len());
    Ok(())
}
