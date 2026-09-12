//! Headless mock screenshot of the Phase 2 editable message-box UI.
//!
//! egui can't render headless, so this composes an honest mock of the editor
//! window: the decoded/edited TEXT and the true-font RASTER are real —
//! produced from the ROM by the same code the UI uses
//! (`font_map::decode_editable_text`, `font_map::encode_message_checked`,
//! `message_raster::rasterize_message`) — only the window chrome (title bar,
//! text-field border) is drawn rather than real egui widgets.
//!
//! Shows the Intro message before and after a typed edit ("Dinosaur Land." →
//! "Dinosaur World"), with per-message byte budgets and a real rejection
//! message for over-long input.
//!
//! ```sh
//! cargo run --bin render_message_editor -- --out=docs/screenshots/message-edit.png --rom=smw.smc
//! ```

use ab_glyph::{Font, FontRef, Glyph, Point, PxScale, ScaleFont};
use image::{imageops, Rgb, RgbImage};
use smwe_rom::font_map::{decode_editable_text, encode_message_checked, FontMap};

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
    mono: FontRef<'static>,
    sans: FontRef<'static>,
    sans_bold: FontRef<'static>,
}

/// Draw one line of text; returns the advance width in px.
fn draw_text(
    img: &mut RgbImage,
    font: &FontRef,
    text: &str,
    x: i32,
    y: i32,
    px: f32,
    color: Rgb<u8>,
) -> i32 {
    let scaled = font.as_scaled(PxScale::from(px));
    let mut caret_x = x as f32;
    let baseline = y as f32 + scaled.ascent();
    let mut prev = None;
    for ch in text.chars() {
        let id = font.glyph_id(ch);
        if let Some(p) = prev {
            caret_x += scaled.kern(p, id);
        }
        let glyph = Glyph {
            id,
            scale: PxScale::from(px),
            position: Point { x: caret_x, y: baseline },
        };
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
    (caret_x - x as f32) as i32
}

use smw_editor::render_util::{fill_rect, rect_border};
fn main() -> anyhow::Result<()> {
    let args: Vec<String> = env_args();
    let output = args
        .iter()
        .find_map(|a| a.strip_prefix("--out="))
        .unwrap_or("docs/screenshots/message-edit.png");
    let rom_path = args
        .iter()
        .find_map(|a| a.strip_prefix("--rom="))
        .or_else(|| args.iter().skip(1).find(|a| !a.starts_with("--")).map(|a| a.as_str()))
        .unwrap_or("smw.smc");

    let fonts = Fonts {
        mono: load_font(MONO_CANDIDATES)?,
        sans: load_font(SANS_CANDIDATES)?,
        sans_bold: load_font(SANS_BOLD_CANDIDATES)?,
    };

    // Real data path, identical to the UI: parse ROM, decode, edit, re-encode.
    let rom = smwe_rom::SmwRom::from_file(rom_path)?;
    let map = FontMap::real();
    let msg_idx = 0; // Intro
    let bytes = &rom.message_boxes.messages[msg_idx];
    let budget = bytes.len();
    let text = decode_editable_text(&map, bytes);

    let edited_text = text.replacen("Dinosaur Land.", "Dinosaur World", 1);
    assert!(edited_text != text, "demo edit did not apply");
    let edited_bytes = encode_message_checked(&map, bytes, budget, &edited_text)?;
    assert_eq!(edited_bytes.len(), budget, "demo edit should stay in budget");

    // A real rejection message for the caption (19-char line).
    let bad_text = format!("{}\n{}", "A".repeat(19), text.lines().skip(1).collect::<Vec<_>>().join("\n"));
    let bad_err = encode_message_checked(&map, bytes, budget, &bad_text).unwrap_err().to_string();

    // True-font rasters, exactly as the UI builds them.
    let gfx = smwe_rom::message_raster::decompress_message_font(&rom.rom)?;
    let raster_before = smwe_rom::message_raster::rasterize_message(
        smwe_rom::font_map::message_cells(bytes),
        &gfx,
    );
    let raster_after = smwe_rom::message_raster::rasterize_message(
        smwe_rom::font_map::message_cells(&edited_bytes),
        &gfx,
    );

    // ---- Compose the mock window ----
    let (w, h) = (1300u32, 780u32);
    let mut img = RgbImage::new(w, h);
    let bg = Rgb([0xF2, 0xF2, 0xF2]);
    let ink = Rgb([0x1A, 0x1A, 0x1A]);
    let gray = Rgb([0x66, 0x66, 0x66]);
    let green = Rgb([0x1A, 0x7A, 0x1A]);
    let red = Rgb([0xC0, 0x30, 0x30]);
    for p in img.pixels_mut() {
        *p = bg;
    }

    // Title bar.
    fill_rect(&mut img, 0, 0, w, 52, Rgb([0x2B, 0x2B, 0x2B]));
    draw_text(
        &mut img,
        &fonts.sans_bold,
        "Message Box Editor \u{2014} Phase 2: editable text (headless mock; text + raster are real ROM output)",
        24,
        15,
        19.0,
        Rgb([0xFF, 0xFF, 0xFF]),
    );

    let panel_w = 612u32;
    let panel_x = [24u32, 664u32];
    let panels = [
        ("BEFORE \u{2014} vanilla ROM bytes", &text, None, &raster_before),
        ("AFTER \u{2014} typed \u{201C}Dinosaur World\u{201D}", &edited_text, Some(1usize), &raster_after),
    ];
    for (pi, (title, panel_text, hl_line, raster)) in panels.iter().enumerate() {
        let x = panel_x[pi];
        let mut y = 76u32;
        draw_text(&mut img, &fonts.sans_bold, title, x as i32, y as i32, 17.0, ink);
        y += 34;
        // Text-field mock.
        let field_h = 8 * 21 + 16;
        fill_rect(&mut img, x, y, panel_w, field_h, Rgb([0xFF, 0xFF, 0xFF]));
        rect_border(&mut img, x, y, panel_w, field_h, Rgb([0x99, 0x99, 0x99]));
        for (li, line) in panel_text.lines().enumerate() {
            let ly = y + 8 + li as u32 * 21;
            if Some(li) == *hl_line {
                fill_rect(&mut img, x + 1, ly - 2, panel_w - 2, 21, Rgb([0xFF, 0xF2, 0xCC]));
            }
            draw_text(&mut img, &fonts.mono, line, (x + 10) as i32, ly as i32, 15.0, ink);
        }
        y += field_h + 10;
        draw_text(
            &mut img,
            &fonts.sans,
            &format!("Text encodes to {} / {} bytes", edited_bytes.len(), budget),
            x as i32,
            y as i32,
            14.0,
            green,
        );
        y += 26;
        draw_text(&mut img, &fonts.sans, "Raster (true SMW font), live:", x as i32, y as i32, 14.0, gray);
        y += 24;
        let big = imageops::resize(*raster, raster.width() * 3, raster.height() * 3, imageops::FilterType::Nearest);
        imageops::replace(&mut img, &big, x as i64, y as i64);
        rect_border(&mut img, x, y, big.width(), big.height(), Rgb([0x99, 0x99, 0x99]));
    }

    // Caption: real rejection message + honesty note.
    let cy = 700u32;
    draw_text(
        &mut img,
        &fonts.sans,
        &format!("Over-long input is refused, not truncated: \u{201C}{bad_err}\u{201D}"),
        24,
        cy as i32,
        14.0,
        red,
    );
    draw_text(
        &mut img,
        &fonts.sans,
        "Mock window chrome \u{2014} the text field content, byte counts, and rasters are produced by the real editor code paths from the ROM.",
        24,
        (cy + 26) as i32,
        13.0,
        gray,
    );

    img.save(output)?;
    println!("wrote {output} ({w}x{h})");
    Ok(())
}

fn env_args() -> Vec<String> {
    std::env::args().collect()
}
