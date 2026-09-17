//! Headless mock screenshot of the Layer 2 scroll mode pickers.
//!
//! egui can't render headless, so this composes an honest mock of the level
//! editor's secondary-header scroll UI: every label is real — produced by the
//! exact functions the UI calls (`smwe_rom::level::scroll::paired_scroll_label`,
//! `VSCROLL_NAMES`, `HSCROLL_ENTRIES`) — and the selected values come from a
//! real vanilla level parsed from the ROM. Only the window chrome and
//! combo-box frames are drawn rather than real egui widgets.
//!
//! ```sh
//! cargo run --bin render_scroll_modes -- --out=docs/screenshots/layer2-scroll-modes.png --rom=smw.smc
//! ```

use ab_glyph::{Font, FontRef, Glyph, Point, PxScale, ScaleFont};
use image::{Rgb, RgbImage};
use smwe_rom::level::scroll::{paired_scroll_label, HSCROLL_ENTRIES, VSCROLL_NAMES};

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

use smw_editor::render_util::{fill_rect, rect_border};

fn combo(img: &mut RgbImage, font: &FontRef, x: u32, y: u32, w: u32, label: &str, ink: Rgb<u8>, gray: Rgb<u8>) {
    let h = 32u32;
    fill_rect(img, x, y, w, h, Rgb([0xFF, 0xFF, 0xFF]));
    rect_border(img, x, y, w, h, Rgb([0x99, 0x99, 0x99]));
    draw_text(img, font, label, (x + 10) as i32, (y + 7) as i32, 14.0, ink);
    draw_text(img, font, "\u{25BE}", (x + w - 26) as i32, (y + 7) as i32, 14.0, gray);
}

fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let output =
        args.iter().find_map(|a| a.strip_prefix("--out=")).unwrap_or("docs/screenshots/layer2-scroll-modes.png");
    let rom_path = args
        .iter()
        .find_map(|a| a.strip_prefix("--rom="))
        .or_else(|| args.iter().skip(1).find(|a| !a.starts_with("--")).map(|a| a.as_str()))
        .unwrap_or("smw.smc");

    let sans = load_font(SANS_CANDIDATES)?;
    let sans_bold = load_font(SANS_BOLD_CANDIDATES)?;

    // Real data path: parse the ROM, read a vanilla level's scroll nibble.
    let rom = smwe_rom::SmwRom::from_file(rom_path)?;
    let level_idx = 0x105usize;
    let scroll_nibble = rom.levels[level_idx].secondary_header.layer2_scroll();
    let selected_paired = paired_scroll_label(scroll_nibble);

    let (w, h) = (1180u32, 860u32);
    let mut img = RgbImage::new(w, h);
    let bg = Rgb([0xF2, 0xF2, 0xF2]);
    let ink = Rgb([0x1A, 0x1A, 0x1A]);
    let gray = Rgb([0x66, 0x66, 0x66]);
    for p in img.pixels_mut() {
        *p = bg;
    }

    // Title bar.
    fill_rect(&mut img, 0, 0, w, 52, Rgb([0x2B, 0x2B, 0x2B]));
    draw_text(
        &mut img,
        &sans_bold,
        "Level Editor \u{2014} Layer 2 scroll modes (headless mock; every label is the real editor string)",
        24,
        15,
        18.0,
        Rgb([0xFF, 0xFF, 0xFF]),
    );

    // ---- Left: paired preset dropdown (open) ----
    let lx = 24u32;
    let mut y = 84u32;
    draw_text(&mut img, &sans_bold, "Paired preset", lx as i32, y as i32, 16.0, ink);
    y += 30;
    draw_text(
        &mut img,
        &sans,
        &format!("Level 0x{level_idx:03X} \u{2014} scroll nibble: {scroll_nibble}"),
        lx as i32,
        y as i32,
        14.0,
        gray,
    );
    y += 30;
    draw_text(&mut img, &sans, "Layer 2 Scroll:", lx as i32, (y + 6) as i32, 14.0, ink);
    let combo_x = lx + 150;
    let combo_w = 380u32;
    combo(&mut img, &sans, combo_x, y, combo_w, &selected_paired, ink, gray);
    y += 44;
    // Open dropdown: all 16 real labels, selected row highlighted.
    let row_h = 28u32;
    let drop_y = y;
    for preset in 0..16u8 {
        let ry = drop_y + preset as u32 * row_h;
        let label = paired_scroll_label(preset);
        if preset == scroll_nibble {
            fill_rect(&mut img, combo_x, ry, combo_w, row_h, Rgb([0xD6, 0xE8, 0xFA]));
        } else {
            fill_rect(&mut img, combo_x, ry, combo_w, row_h, Rgb([0xFF, 0xFF, 0xFF]));
        }
        draw_text(&mut img, &sans, &label, (combo_x + 10) as i32, (ry + 6) as i32, 13.0, ink);
    }
    rect_border(&mut img, combo_x, drop_y, combo_w, row_h * 16, Rgb([0x66, 0x66, 0x66]));
    let mut note_y = drop_y + row_h * 16 + 16;
    draw_text(
        &mut img,
        &sans,
        "Presets 0\u{2013}7: vanilla game. 8\u{2013}11: LM 3.00",
        lx as i32,
        note_y as i32,
        13.0,
        gray,
    );
    note_y += 22;
    draw_text(
        &mut img,
        &sans,
        "(Medium 2/3/4, Slow 2). Replaces the old 0\u{2013}15 numeric slider.",
        lx as i32,
        note_y as i32,
        13.0,
        gray,
    );

    // ---- Right: separate H/V dropdowns (LM 3.40) ----
    let rx = 640u32;
    let mut ry = 84u32;
    draw_text(&mut img, &sans_bold, "Separate H/V (LM 3.40)", rx as i32, ry as i32, 16.0, ink);
    ry += 30;
    draw_text(&mut img, &sans, "Stored in $06FA00 (SHCvvvvv); $05F000 nibble", rx as i32, ry as i32, 14.0, gray);
    ry += 24;
    draw_text(&mut img, &sans, "becomes the horizontal setting.", rx as i32, ry as i32, 14.0, gray);
    ry += 34;
    // Checkbox row (mock).
    fill_rect(&mut img, rx, ry + 2, 16, 16, Rgb([0xFF, 0xFF, 0xFF]));
    rect_border(&mut img, rx, ry + 2, 16, 16, Rgb([0x99, 0x99, 0x99]));
    draw_text(&mut img, &sans, "\u{2713}", (rx + 2) as i32, ry as i32, 14.0, ink);
    draw_text(&mut img, &sans, "Separate H/V Scroll:", (rx + 26) as i32, (ry + 2) as i32, 14.0, ink);
    ry += 34;
    draw_text(&mut img, &sans, "H Scroll:", rx as i32, (ry + 6) as i32, 14.0, ink);
    combo(&mut img, &sans, rx + 110, ry, 380, "Auto-Scroll Right Fast 2", ink, gray);
    ry += 44;
    draw_text(&mut img, &sans, "V Scroll:", rx as i32, (ry + 6) as i32, 14.0, ink);
    combo(&mut img, &sans, rx + 110, ry, 380, "Auto-Scroll Down Fast", ink, gray);
    ry += 52;
    draw_text(&mut img, &sans_bold, "H dropdown entries (28):", rx as i32, ry as i32, 14.0, ink);
    ry += 26;
    // Show a sample of the H entries: first 3 speeds, last auto-scroll, etc.
    let sample_idx = [0usize, 8, 16, 21, 22, 27];
    for &i in &sample_idx {
        let e = &HSCROLL_ENTRIES[i];
        draw_text(
            &mut img,
            &sans,
            &format!("{} {}", if e.h_bit { "[auto]" } else { "[speed]" }, e.name),
            rx as i32,
            ry as i32,
            13.0,
            gray,
        );
        ry += 22;
    }
    draw_text(&mut img, &sans, "\u{2026} 9 speeds + 7 Not Used + 12 auto-scrolls", rx as i32, ry as i32, 13.0, gray);
    ry += 30;
    draw_text(&mut img, &sans_bold, "V dropdown entries (32):", rx as i32, ry as i32, 14.0, ink);
    ry += 26;
    for i in [8usize, 16, 21, 22, 27] {
        draw_text(&mut img, &sans, VSCROLL_NAMES[i], rx as i32, ry as i32, 13.0, gray);
        ry += 22;
    }
    draw_text(&mut img, &sans, "\u{2026} incl. Fast + 12 auto-scrolls (Up/Down)", rx as i32, ry as i32, 13.0, gray);

    // Caption.
    let cy = 800u32;
    draw_text(
        &mut img,
        &sans,
        "Mock window chrome \u{2014} all labels from smwe_rom::level::scroll; the selected preset is level 0x105's real nibble.",
        24,
        cy as i32,
        13.0,
        gray,
    );
    draw_text(
        &mut img,
        &sans,
        "Vanilla ROMs keep $06FA00 = $FF (uninstalled) until separate mode is enabled; MWL byte 17 round-trips the extension.",
        24,
        (cy + 22) as i32,
        13.0,
        gray,
    );

    img.save(output)?;
    println!("wrote {output} ({w}x{h}), selected={selected_paired}");
    Ok(())
}
