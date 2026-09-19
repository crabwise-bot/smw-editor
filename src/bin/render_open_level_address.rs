//! Headless proof screenshot for "Open Level from Address" (LM v1.11 parity).
//!
//! egui can't render headless, so this composes an honest mock of the new
//! File > Open Level from Address... dialog: every string is real — the
//! button/menu label and the "PC address to open level (in hex)" prompt match
//! the UI code in `src/ui/mod.rs`. Only the window chrome (title bar, text
//! field, buttons) is drawn rather than real egui widgets.
//!
//! The level render below is NOT a mock: it is a real `level_png_bytes`
//! render of a scratch ROM where the Layer-1 object stream at headerless PC
//! `0x30338` (Lunar Magic v1.11's own boss-test-room example address) was
//! spliced into level 0x105's slot with `smw_editor::level_address`, then
//! decompressed by the real game code. Sprites, entrances and background come
//! from level 0x105 itself, exactly like the editor shows after an import.
//!
//! ```sh
//! cargo run --bin render_open_level_address -- --rom=smw.smc --out=docs/screenshots/open-level-from-address.png
//! ```

use ab_glyph::{Font, FontRef, Glyph, Point, PxScale, ScaleFont};
use image::{imageops::FilterType, Rgb, RgbImage};
use smw_editor::{
    level_address::{parse_layer1_from_address, splice_layer1_into_slot},
    level_png_export::{level_png_bytes, LevelPngOptions},
    render_util::{fill_rect, rect_border},
};
use smwe_rom::SmwRom;

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
        prev = Some(id);
        caret_x += scaled.h_advance(id);
    }
}

fn draw_text_field(img: &mut RgbImage, font: &FontRef, x: u32, y: u32, w: u32, text: &str) {
    let field = Rgb([0x10, 0x11, 0x13]);
    fill_rect(img, x, y, w, 30, field);
    rect_border(img, x, y, w, 30, Rgb([0x6A, 0x6E, 0x74]));
    draw_text(img, font, text, (x + 8) as i32, (y + 7) as i32, 14.0, Rgb([0xE8, 0xE8, 0xE8]));
}

fn main() -> anyhow::Result<()> {
    let mut rom_path = String::from("smw.smc");
    let mut out = String::from("docs/screenshots/open-level-from-address.png");
    let mut address: u32 = 0x30338;
    let mut level: u16 = 0x105;
    for arg in std::env::args().skip(1) {
        if let Some(v) = arg.strip_prefix("--rom=") {
            rom_path = v.to_string();
        } else if let Some(v) = arg.strip_prefix("--out=") {
            out = v.to_string();
        } else if let Some(v) = arg.strip_prefix("--address=") {
            address = u32::from_str_radix(v.trim_start_matches("0x"), 16)?;
        } else if let Some(v) = arg.strip_prefix("--level=") {
            level = u16::from_str_radix(v.trim_start_matches("0x"), 16)?;
        }
    }

    // ---- Real import path: parse the stream, splice it into the slot ----
    let rom = SmwRom::from_file(&rom_path)?;
    let rom_bytes = rom.rom_bytes();
    let imported = parse_layer1_from_address(rom_bytes, address)?;
    let primary_header = rom.levels.get(level as usize).map(|l| l.primary_header.0).unwrap_or([0; 5]);
    let mut scratch = rom_bytes.to_vec();
    splice_layer1_into_slot(&mut scratch, level as u32, &primary_header, imported.layer.as_bytes())?;

    // ---- Real render of the imported level through the game's decompressor ----
    let png = level_png_bytes(&scratch, level, &LevelPngOptions::default())?;
    let full = image::load_from_memory(&png)?.to_rgb8();
    let (fw, fh) = (full.width(), full.height());
    let (cw, ch) = (fw.min(1024), fh.min(432));
    let crop = image::imageops::crop_imm(&full, 0, 0, cw, ch).to_image();
    let thumb = image::imageops::resize(&crop, 940, (940 * ch) / cw.max(1), FilterType::Triangle);

    let sans = load_font(SANS_CANDIDATES)?;
    let sans_bold = load_font(SANS_BOLD_CANDIDATES)?;

    // ---- Compose ----
    let thumb_h = thumb.height();
    let (w, h) = (1000u32, 300 + thumb_h + 60);
    let mut img = RgbImage::new(w, h);
    let bg = Rgb([0x1B, 0x1D, 0x20]);
    let panel = Rgb([0x25, 0x28, 0x2C]);
    let titlebar = Rgb([0x12, 0x14, 0x16]);
    let ink = Rgb([0xE8, 0xE8, 0xE8]);
    let dim = Rgb([0xA8, 0xA8, 0xA8]);
    for p in img.pixels_mut() {
        *p = bg;
    }

    // Mock dialog: real strings, drawn chrome.
    let (dx, dy, dw) = (30u32, 24u32, 940u32);
    let dh = 150u32;
    fill_rect(&mut img, dx, dy, dw, dh, panel);
    rect_border(&mut img, dx, dy, dw, dh, Rgb([0x4A, 0x4E, 0x54]));
    fill_rect(&mut img, dx, dy, dw, 38, titlebar);
    draw_text(&mut img, &sans_bold, "Open Level From Address (in hex)", (dx + 16) as i32, (dy + 10) as i32, 16.0, ink);
    draw_text(
        &mut img,
        &sans,
        "headless mock \u{2014} prompt text is real; render below is a real import",
        (dx + 380) as i32,
        (dy + 13) as i32,
        12.0,
        dim,
    );
    let mut y = dy + 56;
    draw_text(&mut img, &sans, "PC address to open level (in hex)", (dx + 20) as i32, y as i32, 14.0, ink);
    y += 28;
    draw_text_field(&mut img, &sans, dx + 20, y, 160, &format!("{address:05X}"));
    fill_rect(&mut img, dx + 200, y, 90, 30, Rgb([0x2F, 0x6F, 0xBD]));
    draw_text(&mut img, &sans_bold, "OK", (dx + 236) as i32, (y + 7) as i32, 14.0, Rgb([0xFF, 0xFF, 0xFF]));
    fill_rect(&mut img, dx + 300, y, 90, 30, Rgb([0x3A, 0x3D, 0x42]));
    rect_border(&mut img, dx + 300, y, 90, 30, Rgb([0x6A, 0x6E, 0x74]));
    draw_text(&mut img, &sans, "Cancel", (dx + 322) as i32, (y + 7) as i32, 14.0, ink);

    // Caption with real import numbers.
    let mut y = dy + dh + 22;
    let objects = imported.layer.objects().len();
    draw_text(
        &mut img,
        &sans_bold,
        &format!(
            "Level {level:03X} \u{2014} Layer 1 imported from PC ${address:05X} ({objects} objects, {} bytes)",
            imported.bytes_consumed
        ),
        dx as i32,
        y as i32,
        15.0,
        ink,
    );
    y += 26;
    draw_text(
        &mut img,
        &sans,
        "Sprites, entrances and background stay from level 105 \u{2014} Lunar Magic loads none of them from the address.",
        dx as i32,
        y as i32,
        13.0,
        dim,
    );
    y += 30;

    for (ox, oy, px) in thumb.enumerate_pixels() {
        img.put_pixel(dx + ox, y + oy, *px);
    }
    rect_border(&mut img, dx, y, thumb.width(), thumb_h, Rgb([0x6A, 0x6E, 0x74]));

    img.save(&out)?;
    println!("wrote {out} ({w}x{h}; source {fw}x{fh})");
    Ok(())
}
