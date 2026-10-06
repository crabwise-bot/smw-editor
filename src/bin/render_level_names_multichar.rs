//! Headless mock screenshot of the MultiChar tiles feature (LM v3.40 parity).
//!
//! egui can't render headless, so this composes an honest mock of the
//! world-editor level-name section: the vanilla "YELLOW SWITCH PALACE" and
//! "FOREST OF ILLUSION 1" names decoded from the ROM with the "Use MultiChar
//! Tiles" option ON (squished tiles as characters) and OFF (squished tiles as
//! `\XX` hex escapes), the tile-budget feedback, and the auto-encoding of
//! "LL" to the squished `$3A` tile — all produced by the real
//! `smwe_rom::overworld::level_names` code. Only the window chrome (title
//! bar, checkbox, text-field border) is drawn rather than real egui widgets.
//!
//! ```sh
//! cargo run --bin render_level_names_multichar -- --out=docs/screenshots/level-names-multichar.png --rom=/path/to/smw.smc
//! ```

use ab_glyph::{Font, FontRef, Glyph, Point, PxScale, ScaleFont};
use image::{Rgb, RgbImage};
use smwe_rom::overworld::level_names;

const MONO_CANDIDATES: &[&str] = &["/usr/share/fonts/truetype/dejavu/DejaVuSansMono.ttf"];
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

struct Fonts {
    mono:      FontRef<'static>,
    sans:      FontRef<'static>,
    sans_bold: FontRef<'static>,
}

fn draw_text(img: &mut RgbImage, font: &FontRef, text: &str, x: i32, y: i32, px: f32, color: Rgb<u8>) -> i32 {
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
                                ((s[1] as u16 * a + d[0] as u16 * inv) / 255) as u8,
                                ((s[2] as u16 * a + d[0] as u16 * inv) / 255) as u8,
                            ]),
                        );
                    }
                }
            });
        }
        caret_x += scaled.h_advance(id);
        prev = Some(id);
    }
    (caret_x - x as f32) as i32
}

use smw_editor::render_util::{fill_rect, rect_border};

fn draw_checkbox(img: &mut RgbImage, fonts: &Fonts, x: u32, y: u32, checked: bool, label: &str) {
    let ink = Rgb([0x1A, 0x1A, 0x1A]);
    let box_s = 18u32;
    fill_rect(img, x, y, box_s, box_s, Rgb([0xFF, 0xFF, 0xFF]));
    rect_border(img, x, y, box_s, box_s, Rgb([0x66, 0x66, 0x66]));
    if checked {
        draw_text(img, &fonts.sans_bold, "✓", x as i32 + 2, y as i32 - 2, 18.0, ink);
    }
    draw_text(img, &fonts.sans, label, (x + box_s + 8) as i32, y as i32 + 1, 14.0, ink);
}

fn draw_field(img: &mut RgbImage, fonts: &Fonts, x: u32, y: u32, w: u32, text: &str) {
    let h = 32u32;
    let ink = Rgb([0x1A, 0x1A, 0x1A]);
    fill_rect(img, x, y, w, h, Rgb([0xFF, 0xFF, 0xFF]));
    rect_border(img, x, y, w, h, Rgb([0x99, 0x99, 0x99]));
    draw_text(img, &fonts.mono, text, (x + 8) as i32, (y + 7) as i32, 14.0, ink);
}

fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let output =
        args.iter().find_map(|a| a.strip_prefix("--out=")).unwrap_or("docs/screenshots/level-names-multichar.png");
    let rom_path = args
        .iter()
        .find_map(|a| a.strip_prefix("--rom="))
        .or_else(|| args.iter().skip(1).find(|a| !a.starts_with("--")).map(|a| a.as_str()))
        .unwrap_or("smw.smc");

    let fonts = Fonts {
        mono:      load_font(MONO_CANDIDATES)?,
        sans:      load_font(SANS_CANDIDATES)?,
        sans_bold: load_font(SANS_BOLD_CANDIDATES)?,
    };

    // Real data path, identical to the UI.
    let raw = std::fs::read(rom_path)?;
    let rom_bytes: &[u8] = if raw.len() % 0x400 == 0x200 { &raw[0x200..] } else { &raw[..] };
    let names_on =
        level_names::decode_all(rom_bytes, 0, false, true).ok_or_else(|| anyhow::anyhow!("decode failed"))?;
    let names_off =
        level_names::decode_all(rom_bytes, 0, false, false).ok_or_else(|| anyhow::anyhow!("decode failed"))?;
    anyhow::ensure!(names_on.len() == level_names::LEVEL_NAMES_COUNT);

    // Find the squished-tile names.
    let yellow_on = names_on.iter().find(|n| n.starts_with("YELLOW SWITCH PALACE")).expect("yellow").trim().to_string();
    let yellow_off = names_off.iter().find(|n| n.contains("\\38")).expect("yellow escapes").trim().to_string();
    let forest_on = names_on.iter().find(|n| n.trim() == "FOREST OF ILLUSION 1").expect("forest").trim().to_string();

    // Real encode: "LL" -> $3A with multichar on.
    let ll_tiles = level_names::encode_name_to_tiles("LL", true)?;
    anyhow::ensure!(ll_tiles == vec![0x3A]);
    let ll_tiles_off = level_names::encode_name_to_tiles("LL", false)?;
    anyhow::ensure!(ll_tiles_off == vec![0x0B, 0x0B]);

    // Real budget: "YELLOW SWITCH PALACE" is 20 chars but 19 tiles.
    let tiles_on = level_names::count_name_tiles("YELLOW SWITCH PALACE", true);
    let tiles_off = level_names::count_name_tiles("YELLOW SWITCH PALACE", false);
    anyhow::ensure!(tiles_on == 19 && tiles_off == 20);

    // ---- Compose the mock ----
    let (w, h) = (1200u32, 720u32);
    let mut img = RgbImage::new(w, h);
    let bg = Rgb([0xF2, 0xF2, 0xF2]);
    let ink = Rgb([0x1A, 0x1A, 0x1A]);
    let gray = Rgb([0x66, 0x66, 0x66]);
    for p in img.pixels_mut() {
        *p = bg;
    }

    fill_rect(&mut img, 0, 0, w, 52, Rgb([0x2B, 0x2B, 0x2B]));
    draw_text(
        &mut img,
        &fonts.sans_bold,
        "World Editor \u{2014} Level names: MultiChar tiles (headless mock; names + budgets are real ROM output)",
        24,
        15,
        18.0,
        Rgb([0xFF, 0xFF, 0xFF]),
    );

    let mut y = 80u32;
    draw_text(
        &mut img,
        &fonts.sans_bold,
        "Use MultiChar Tiles: ON (Lunar Magic v3.40 default)",
        24,
        y as i32,
        16.0,
        ink,
    );
    y += 36;
    draw_checkbox(&mut img, &fonts, 24, y, true, "Use MultiChar Tiles");
    y += 36;
    draw_text(&mut img, &fonts.sans, "Squished tiles decode to their characters:", 24, y as i32, 14.0, gray);
    y += 28;
    draw_text(&mut img, &fonts.sans, "Level name:", 24, (y + 6) as i32, 14.0, ink);
    draw_field(&mut img, &fonts, 130, y, 500, &yellow_on);
    y += 44;
    draw_text(
        &mut img,
        &fonts.sans,
        &format!(
            "Name encodes to {tiles_on} / {} tiles (20 characters in 19 tiles via $3A=\"LL\")",
            level_names::MAX_NAME_TILES
        ),
        24,
        y as i32,
        13.0,
        gray,
    );
    y += 30;
    draw_text(&mut img, &fonts.sans, "Level name:", 24, (y + 6) as i32, 14.0, ink);
    draw_field(&mut img, &fonts, 130, y, 500, &forest_on);
    y += 52;

    draw_text(&mut img, &fonts.sans_bold, "Use MultiChar Tiles: OFF", 24, y as i32, 16.0, ink);
    y += 36;
    draw_checkbox(&mut img, &fonts, 24, y, false, "Use MultiChar Tiles");
    y += 36;
    draw_text(
        &mut img,
        &fonts.sans,
        "Squished tiles display as \\XX hex escapes (Lunar Magic v3.40 behavior):",
        24,
        y as i32,
        14.0,
        gray,
    );
    y += 28;
    draw_text(&mut img, &fonts.sans, "Level name:", 24, (y + 6) as i32, 14.0, ink);
    draw_field(&mut img, &fonts, 130, y, 640, &yellow_off);
    y += 44;
    draw_text(
        &mut img,
        &fonts.sans,
        &format!(
            "Typing \"LL\" encodes to tile $3A (one tile) when ON, two $0B tiles when OFF. Type \\XX to insert a specific tile."
        ),
        24,
        y as i32,
        13.0,
        gray,
    );
    y += 30;
    draw_text(
        &mut img,
        &fonts.sans,
        "Mock window chrome \u{2014} the names, escapes, tile budgets, and $3A encoding are produced by smwe_rom::overworld::level_names from the ROM.",
        24,
        y as i32,
        13.0,
        gray,
    );

    img.save(output)?;
    println!("wrote {output} ({w}x{h})");
    Ok(())
}
