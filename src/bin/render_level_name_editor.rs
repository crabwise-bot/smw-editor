//! Headless mock screenshot of the world-editor level-name text field.
//!
//! egui can't render headless, so this composes an honest mock of the
//! tile-inspect panel: the level NAMES, the per-name tile BUDGET, and the
//! rejection message are real — produced from the ROM by the same code the UI
//! uses (`smwe_rom::overworld::level_names::decode_all`,
//! `check_name`, `encode_names`) — only the window chrome (title bar,
//! text-field border) is drawn rather than real egui widgets.
//!
//! Shows translevel 0x29 before/after typing a custom name
//! ("YOSHI'S ISLAND 1" -> "YOSHI'S HIDEOUT"), with the 19-tile budget
//! feedback and a real rejection message for over-long input.
//!
//! ```sh
//! cargo run --bin render_level_name_editor -- --out=docs/screenshots/custom-level-names.png --rom=/path/to/smw.smc
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
    mono: FontRef<'static>,
    sans: FontRef<'static>,
    sans_bold: FontRef<'static>,
}

fn fill_rect(img: &mut RgbImage, x: u32, y: u32, w: u32, h: u32, c: Rgb<u8>) {
    for yy in y..(y + h).min(img.height()) {
        for xx in x..(x + w).min(img.width()) {
            img.put_pixel(xx, yy, c);
        }
    }
}

fn rect_border(img: &mut RgbImage, x: u32, y: u32, w: u32, h: u32, c: Rgb<u8>) {
    fill_rect(img, x, y, w, 1, c);
    fill_rect(img, x, y + h - 1, w, 1, c);
    fill_rect(img, x, y, 1, h, c);
    fill_rect(img, x + w - 1, y, 1, h, c);
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
    (caret_x - x as f32) as i32
}

/// Draw a single-line text field mock with `text` inside.
fn draw_field(img: &mut RgbImage, fonts: &Fonts, x: u32, y: u32, w: u32, text: &str, ink: Rgb<u8>) {
    let h = 34u32;
    fill_rect(img, x, y, w, h, Rgb([0xFF, 0xFF, 0xFF]));
    rect_border(img, x, y, w, h, Rgb([0x99, 0x99, 0x99]));
    draw_text(img, &fonts.mono, text, (x + 10) as i32, (y + 8) as i32, 15.0, ink);
}

fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let output = args
        .iter()
        .find_map(|a| a.strip_prefix("--out="))
        .unwrap_or("docs/screenshots/custom-level-names.png");
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

    // Real data path, identical to the UI: strip header, decode all 93 names.
    let raw = std::fs::read(rom_path)?;
    let rom_bytes: &[u8] = if raw.len() % 0x400 == 0x200 { &raw[0x200..] } else { &raw[..] };
    let names = level_names::decode_all(rom_bytes, 0, false).ok_or_else(|| anyhow::anyhow!("decode failed"))?;
    anyhow::ensure!(names.len() == level_names::LEVEL_NAMES_COUNT);

    // Demo translevel: 0x29 == "YOSHI'S ISLAND 1" in the vanilla ROM.
    let tl = 0x29usize;
    let vanilla: String = names[tl].trim().to_string();
    anyhow::ensure!(vanilla == "YOSHI'S ISLAND 1", "unexpected vanilla name for TL 0x29: {vanilla:?}");

    // AFTER: the typed custom name, validated by the real check_name.
    let custom = level_names::check_name("yoshi's hideout")?;
    anyhow::ensure!(custom == "YOSHI'S HIDEOUT");

    // A real rejection message for the caption (24-char name).
    let bad_err = level_names::check_name(&"A".repeat(24)).unwrap_err().to_string();

    // Prove the save path fits: re-encode all 93 names with the override.
    let mut all = names.clone();
    all[tl] = custom.clone();
    let enc = level_names::encode_names(&all)?;
    anyhow::ensure!(enc.pool.len() <= level_names::STRINGS_PATCHED_LEN);

    // ---- Compose the mock window ----
    let (w, h) = (1200u32, 640u32);
    let mut img = RgbImage::new(w, h);
    let bg = Rgb([0xF2, 0xF2, 0xF2]);
    let ink = Rgb([0x1A, 0x1A, 0x1A]);
    let gray = Rgb([0x66, 0x66, 0x66]);
    let orange = Rgb([0xB0, 0x6A, 0x00]);
    let red = Rgb([0xC0, 0x30, 0x30]);
    for p in img.pixels_mut() {
        *p = bg;
    }

    // Title bar.
    fill_rect(&mut img, 0, 0, w, 52, Rgb([0x2B, 0x2B, 0x2B]));
    draw_text(
        &mut img,
        &fonts.sans_bold,
        "World Editor \u{2014} Level name editor (headless mock; names + budgets are real ROM output)",
        24,
        15,
        19.0,
        Rgb([0xFF, 0xFF, 0xFF]),
    );

    let panel_w = 552u32;
    let panel_x = [24u32, 624u32];
    let panels = [
        ("BEFORE \u{2014} vanilla name", vanilla.as_str(), None),
        ("AFTER \u{2014} typed \u{201C}YOSHI'S HIDEOUT\u{201D}", custom.as_str(), Some(())),
    ];
    for (pi, (title, field_text, is_custom)) in panels.iter().enumerate() {
        let x = panel_x[pi];
        let mut y = 76u32;
        draw_text(&mut img, &fonts.sans_bold, title, x as i32, y as i32, 17.0, ink);
        y += 34;
        draw_text(
            &mut img,
            &fonts.sans,
            &format!("Level tile (translevel 0x{tl:02X})"),
            x as i32,
            y as i32,
            14.0,
            gray,
        );
        y += 30;
        draw_text(&mut img, &fonts.sans, "Level name:", x as i32, (y + 6) as i32, 14.0, ink);
        draw_field(&mut img, &fonts, x + 110, y, panel_w - 110, field_text, ink);
        y += 48;
        let used = field_text.chars().count();
        draw_text(
            &mut img,
            &fonts.sans,
            &format!("Name encodes to {used} / {} tiles", level_names::MAX_NAME_CHARS),
            x as i32,
            y as i32,
            14.0,
            ink,
        );
        y += 28;
        if is_custom.is_some() {
            draw_text(
                &mut img,
                &fonts.sans,
                "Custom name \u{2014} needs the name-table relocation patch on save",
                x as i32,
                y as i32,
                14.0,
                orange,
            );
            y += 28;
            draw_text(&mut img, &fonts.sans, "A\u{2013}Z 0\u{2013}9 space # ' supported", x as i32, y as i32, 13.0, gray);
        } else {
            draw_text(
                &mut img,
                &fonts.sans,
                &format!("Vanilla: {vanilla}"),
                x as i32,
                y as i32,
                13.0,
                gray,
            );
        }
    }

    // Caption: real rejection message + encode stats + honesty note.
    let cy = 480u32;
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
        &format!(
            "Save path: encode_names with this override \u{2192} {} pool bytes of {} (relocated tables at $04A1B6)",
            enc.pool.len(),
            level_names::STRINGS_PATCHED_LEN,
        ),
        24,
        (cy + 28) as i32,
        13.0,
        gray,
    );
    draw_text(
        &mut img,
        &fonts.sans,
        "Mock window chrome \u{2014} the names, tile budgets, and error are produced by smwe_rom::overworld::level_names from the ROM.",
        24,
        (cy + 54) as i32,
        13.0,
        gray,
    );

    img.save(output)?;
    println!("wrote {output} ({w}x{h})");
    Ok(())
}
