//! Headless mock screenshot of the title/credits editor's new region support.
//!
//! egui can't render headless, so this composes an honest mock: every number
//! is real — the U.S. budgets are parsed from the vanilla U ROM with the same
//! `TitleCreditsData` the editor uses (region detected from the internal
//! header), and the Japanese slot map is computed from the actual
//! `LAYOUT_JP` constants (LM v1.30 parity). Only the window chrome and
//! widgets are drawn rather than real egui widgets. No copied game bytes are
//! involved.
//!
//! ```sh
//! cargo run --bin render_title_region -- --out=docs/screenshots/title-region.png --rom=smw.smc
//! ```

use ab_glyph::{Font, FontRef, Glyph, Point, PxScale, ScaleFont};
use image::{Rgb, RgbImage};
use smw_editor::render_util::{fill_rect, rect_border};
use smwe_rom::{
    title_credits::{TitleCreditsRegion, ENEMY_NAME_COUNT, ENEMY_NAME_LABELS},
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

fn draw_text(
    img: &mut RgbImage, font: &FontRef, bold: &FontRef, text: &str, x: i32, y: i32, px: f32, color: Rgb<u8>,
    is_bold: bool,
) {
    let font = if is_bold { bold } else { font };
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

fn draw_bar(img: &mut RgbImage, x: u32, y: u32, w: u32, used: usize, max: usize) {
    fill_rect(img, x, y, w, 12, Rgb([60, 60, 60]));
    let fill = ((used.min(max) as f32 / max as f32) * w as f32) as u32;
    fill_rect(img, x, y, fill.max(2), 12, Rgb([90, 160, 220]));
    rect_border(img, x, y, w, 12, Rgb([120, 120, 120]));
}

fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let output = args.iter().find_map(|a| a.strip_prefix("--out=")).unwrap_or("docs/screenshots/title-region.png");
    let rom_path = args.iter().find_map(|a| a.strip_prefix("--rom=")).unwrap_or("smw.smc");

    let smw = SmwRom::from_file(rom_path)?;
    let data = &smw.title_credits;
    let region = data.region;
    assert_eq!(region, TitleCreditsRegion::Us, "expected the U.S. reference ROM");
    let layout = region.layout();
    let jp = TitleCreditsRegion::Japanese.layout();

    let font = load_font(SANS_CANDIDATES)?;
    let bold = load_font(SANS_BOLD_CANDIDATES)?;

    let (w, h) = (780u32, 660u32);
    let mut img = RgbImage::new(w, h);
    fill_rect(&mut img, 0, 0, w, h, Rgb([30, 30, 30]));

    // Title bar.
    fill_rect(&mut img, 0, 0, w, 36, Rgb([52, 52, 52]));
    draw_text(
        &mut img,
        &font,
        &bold,
        &format!("Title Screen / Credits ({})", region.label()),
        12,
        8,
        17.0,
        Rgb([235, 235, 235]),
        true,
    );
    let mut y: i32 = 48;
    let text =
        |img: &mut RgbImage, s: &str, yy: i32, c: Rgb<u8>, b: bool| draw_text(img, &font, &bold, s, 14, yy, 14.0, c, b);

    text(&mut img, "Edits here are global and use vanilla fixed-size data slots.", y, Rgb([200, 200, 200]), false);
    y += 22;
    // The new region indicator line — accent bar to draw the eye.
    fill_rect(&mut img, 8, y as u32, 4, 20, Rgb([110, 200, 120]));
    text(
        &mut img,
        &format!("ROM region: {} — all slots below use the {} layout.", region.label(), region.label()),
        y,
        Rgb([170, 225, 175]),
        false,
    );
    y += 34;
    text(&mut img, "Title screen", y, Rgb([235, 235, 235]), true);
    y += 26;

    let demo_used = data.title_demo_inputs.len() * 2 + 1;
    text(
        &mut img,
        &format!("Demo input: {demo_used} / {} bytes", layout.title_input_seq_max),
        y,
        Rgb([200, 200, 200]),
        false,
    );
    y += 24;
    let logo_used = data.title_screen_stripe.len();
    text(
        &mut img,
        &format!("Logo stripe: {logo_used} / {} bytes", layout.title_stripe_max),
        y,
        Rgb([200, 200, 200]),
        false,
    );
    draw_bar(&mut img, 330, y as u32 + 2, 220, logo_used, layout.title_stripe_max);
    y += 24;
    let menu_used = data.player_select_stripe.len();
    text(
        &mut img,
        &format!("Menu stripe: {menu_used} / {} bytes", layout.player_select_stripe_max),
        y,
        Rgb([200, 200, 200]),
        false,
    );
    draw_bar(&mut img, 330, y as u32 + 2, 220, menu_used, layout.player_select_stripe_max);
    y += 36;

    text(&mut img, &format!("Ending enemy-name stripes ({ENEMY_NAME_COUNT} scenes)"), y, Rgb([235, 235, 235]), true);
    y += 26;
    for i in 0..ENEMY_NAME_COUNT {
        let col = (i / 7) as i32;
        let row = (i % 7) as i32;
        let used = data.enemy_name_stripes[i].len();
        let max = data.enemy_name_slot_size(i);
        draw_text(
            &mut img,
            &font,
            &bold,
            &format!("{i:02X} {} ({used}/{max} B)", ENEMY_NAME_LABELS[i]),
            14 + col * 370,
            y + row * 21,
            12.0,
            Rgb([190, 190, 190]),
            false,
        );
    }
    y += 7 * 21 + 14;

    // Japanese layout panel — the other half of this PR's story.
    fill_rect(&mut img, 10, y as u32, w - 20, 148, Rgb([36, 40, 36]));
    rect_border(&mut img, 10, y as u32, w - 20, 148, Rgb([110, 200, 120]));
    let mut py = y + 10;
    text(
        &mut img,
        "Japanese ROM layout — selected automatically when a J ROM is opened (Lunar Magic v1.30 parity)",
        py,
        Rgb([170, 225, 175]),
        true,
    );
    py += 26;
    text(
        &mut img,
        &format!(
            "Title stripe ${:06X} — {} B slot  ·  Menu stripe ${:06X} — {} B slot",
            jp.title_stripe.0, jp.title_stripe_max, jp.player_select_stripe.0, jp.player_select_stripe_max
        ),
        py,
        Rgb([200, 200, 200]),
        false,
    );
    py += 22;
    text(
        &mut img,
        &format!(
            "Demo input ${:06X} — {} B slot  ·  Submap operand ${:06X}  ·  Enemy names ${:06X}–${:06X}",
            jp.title_input_seq.0,
            jp.title_input_seq_max,
            jp.title_submap_operand.0,
            jp.enemy_name_starts[0].0,
            jp.enemy_name_end.0
        ),
        py,
        Rgb([200, 200, 200]),
        false,
    );
    py += 22;
    let slots: Vec<String> = (0..ENEMY_NAME_COUNT).map(|i| jp.enemy_name_slot_size(i).to_string()).collect();
    text(&mut img, &format!("J enemy scene slots (B): {}", slots.join(" ")), py, Rgb([200, 200, 200]), false);
    py += 22;
    text(
        &mut img,
        "J scenes are listed by number — the credits text is katakana (no Latin decode).",
        py,
        Rgb([160, 160, 160]),
        false,
    );

    // Mock disclaimer.
    draw_text(
        &mut img,
        &font,
        &bold,
        "Mock screenshot (egui cannot render headless): window chrome drawn; all numbers real — U budgets parsed from the vanilla U ROM, J slots from LAYOUT_JP.",
        14,
        h as i32 - 24,
        11.0,
        Rgb([130, 130, 130]),
        false,
    );

    img.save(output)?;
    println!("wrote {output}");
    Ok(())
}
