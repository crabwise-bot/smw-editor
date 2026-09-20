//! Headless proof screenshot for title-moves export/import (LM v1.91 parity).
//!
//! egui can't render headless, so this composes an honest mock of the new
//! Demo-input row in the Title Screen / Credits window: the row label, the
//! "+ Step" / "- Step" buttons, and the new "Export title moves…" /
//! "Import title moves…" buttons use the exact strings from
//! `src/ui/editor_prototypes/level_editor/title_credits_editor.rs`. Only the
//! window chrome is drawn rather than real egui widgets.
//!
//! The demo-step table below is NOT a mock: it is the real ROM's title demo
//! input sequence, parsed by `TitleCreditsData::parse` from the real ROM,
//! then round-tripped through the real `.smwtm` file format
//! (`encode_title_moves_file` → `decode_title_moves_file`). The header byte
//! dump shows the real exported file bytes (magic "SMWTMV1", region code,
//! length prefix, first payload bytes).
//!
//! ```sh
//! cargo run --bin render_title_moves -- --rom=~/workspace/smw-editor/smw.smc --out=docs/screenshots/title-moves-export.png
//! ```

use ab_glyph::{Font, FontRef, Glyph, Point, PxScale, ScaleFont};
use image::{Rgb, RgbImage};
use smw_editor::render_util::{fill_rect, rect_border};
use smwe_rom::{
    title_credits::{decode_title_moves_file, encode_title_moves_file, TitleCreditsData},
    SmwRom,
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
        prev = Some(id);
        caret_x += scaled.h_advance(id);
    }
}

fn button_summary(buttons: u8) -> String {
    let mut names = Vec::new();
    for (mask, name) in [
        (0x80, "B"),
        (0x40, "Y"),
        (0x20, "Select"),
        (0x10, "Start"),
        (0x08, "Up"),
        (0x04, "Down"),
        (0x02, "Left"),
        (0x01, "Right"),
    ] {
        if buttons & mask != 0 {
            names.push(name);
        }
    }
    if names.is_empty() {
        "-".to_string()
    } else {
        names.join("+")
    }
}

fn draw_button(img: &mut RgbImage, font: &FontRef, label: &str, x: u32, y: u32) -> u32 {
    let pad_x = 14u32;
    let w = (label.len() as u32) * 8 + pad_x * 2;
    let h = 28u32;
    fill_rect(img, x, y, w, h, Rgb([0x3A, 0x3D, 0x42]));
    rect_border(img, x, y, w, h, Rgb([0x6A, 0x6E, 0x74]));
    draw_text(img, font, label, (x + pad_x) as i32, (y + 6) as i32, 14.0, Rgb([0xE8, 0xE8, 0xE8]));
    w
}

fn main() -> anyhow::Result<()> {
    let mut rom_path = String::from("smw.smc");
    let mut out = String::from("docs/screenshots/title-moves-export.png");
    for arg in std::env::args().skip(1) {
        if let Some(v) = arg.strip_prefix("--rom=") {
            rom_path = v.to_string();
        } else if let Some(v) = arg.strip_prefix("--out=") {
            out = v.to_string();
        }
    }

    // ---- Real data path: parse the real ROM, export, re-import, verify ----
    let smw = SmwRom::from_file(&rom_path)?;
    let data: &TitleCreditsData = &smw.title_credits;
    let steps = &data.title_demo_inputs;
    let file_bytes = encode_title_moves_file(data.region, steps)?;
    let back = decode_title_moves_file(&file_bytes)?;
    assert_eq!(back.inputs.len(), steps.len());
    assert!(back.inputs.iter().zip(steps.iter()).all(|(a, b)| a.buttons == b.buttons && a.duration == b.duration));
    assert_eq!(back.region, data.region);
    let slot_max = data.layout().title_input_seq_max;
    let used = steps.len() * 2 + 1;

    let sans = load_font(SANS_CANDIDATES)?;
    let sans_bold = load_font(SANS_BOLD_CANDIDATES)?;
    let mono = load_font(MONO_CANDIDATES)?;

    let rows_shown = steps.len().min(14);
    let (w, h) = (940u32, 430u32 + (rows_shown as u32) * 24);
    let mut img = RgbImage::new(w, h);
    let bg = Rgb([0x1B, 0x1D, 0x20]);
    let panel = Rgb([0x25, 0x28, 0x2C]);
    let titlebar = Rgb([0x12, 0x14, 0x16]);
    let ink = Rgb([0xE8, 0xE8, 0xE8]);
    let dim = Rgb([0xA8, 0xA8, 0xA8]);
    let green = Rgb([0x96, 0xC8, 0x96]);
    for p in img.pixels_mut() {
        *p = bg;
    }

    // Mock window: real strings from title_credits_editor.rs, drawn chrome.
    let (dx, dy, dw) = (24u32, 20u32, 892u32);
    let mut y = dy + 52;
    fill_rect(&mut img, dx, dy, dw, h - 40, panel);
    rect_border(&mut img, dx, dy, dw, h - 40, Rgb([0x4A, 0x4E, 0x54]));
    fill_rect(&mut img, dx, dy, dw, 38, titlebar);
    let region_label = data.region.label();
    draw_text(
        &mut img,
        &sans_bold,
        &format!("Title Screen / Credits ({region_label})"),
        (dx + 16) as i32,
        (dy + 10) as i32,
        16.0,
        ink,
    );
    draw_text(
        &mut img,
        &sans,
        "headless mock — labels/buttons are real strings; step table and file dump are real data",
        (dx + 400) as i32,
        (dy + 13) as i32,
        12.0,
        dim,
    );

    draw_text(&mut img, &sans_bold, "Title screen", (dx + 16) as i32, y as i32, 15.0, ink);
    y += 30;
    draw_text(
        &mut img,
        &sans,
        &format!("Demo input: {used} / {slot_max} bytes"),
        (dx + 16) as i32,
        (y + 7) as i32,
        14.0,
        ink,
    );
    let mut bx = dx + 250;
    for label in ["+ Step", "- Step", "Export title moves…", "Import title moves…"] {
        bx += draw_button(&mut img, &sans, label, bx, y) + 10;
    }
    y += 44;
    draw_text(
        &mut img,
        &sans,
        &format!(
            "✓ Exported {} demo steps ({} bytes) to /tmp/title-moves.smwtm — real file bytes below",
            steps.len(),
            file_bytes.len()
        ),
        (dx + 16) as i32,
        y as i32,
        13.0,
        green,
    );
    y += 30;

    // Real step table (first rows_shown steps of the real ROM's demo).
    draw_text(
        &mut img,
        &sans_bold,
        "Real title demo steps from the ROM (verified round-trip):",
        (dx + 16) as i32,
        y as i32,
        13.0,
        ink,
    );
    y += 26;
    let cols = ["#", "Buttons", "Duration", "Held"];
    let cxs = [dx + 16, dx + 80, dx + 190, dx + 300];
    for (i, c) in cols.iter().enumerate() {
        draw_text(&mut img, &sans_bold, c, cxs[i] as i32, y as i32, 12.0, dim);
    }
    y += 22;
    for (i, s) in steps.iter().take(rows_shown).enumerate() {
        draw_text(&mut img, &mono, &format!("{i:02}"), cxs[0] as i32, y as i32, 12.0, dim);
        draw_text(&mut img, &mono, &format!("${:02X}", s.buttons), cxs[1] as i32, y as i32, 12.0, ink);
        draw_text(&mut img, &mono, &format!("${:02X}", s.duration), cxs[2] as i32, y as i32, 12.0, ink);
        draw_text(&mut img, &sans, &button_summary(s.buttons), cxs[3] as i32, y as i32, 12.0, ink);
        y += 24;
    }
    if steps.len() > rows_shown {
        draw_text(
            &mut img,
            &sans,
            &format!("… {} more steps", steps.len() - rows_shown),
            (dx + 16) as i32,
            y as i32,
            12.0,
            dim,
        );
        y += 24;
    }
    y += 10;

    // Real file-header dump.
    draw_text(&mut img, &sans_bold, "Real exported .smwtm file header:", (dx + 16) as i32, y as i32, 13.0, ink);
    y += 26;
    let magic: String = file_bytes[..7].iter().map(|&b| b as char).collect();
    let region_code = file_bytes[7];
    let payload_len = u32::from_le_bytes(file_bytes[8..12].try_into().unwrap());
    let payload_hex: Vec<String> =
        file_bytes[12..file_bytes.len().min(32)].iter().map(|b| format!("{b:02X}")).collect();
    draw_text(&mut img, &mono, &format!("magic   \"{magic}\""), (dx + 16) as i32, y as i32, 12.0, ink);
    y += 20;
    draw_text(
        &mut img,
        &mono,
        &format!("region  {region_code} ({region_label})"),
        (dx + 16) as i32,
        y as i32,
        12.0,
        ink,
    );
    y += 20;
    draw_text(
        &mut img,
        &mono,
        &format!("payload {payload_len} bytes (raw $FF-terminated input sequence)"),
        (dx + 16) as i32,
        y as i32,
        12.0,
        ink,
    );
    y += 20;
    draw_text(&mut img, &mono, &format!("bytes   {}", payload_hex.join(" ")), (dx + 16) as i32, y as i32, 12.0, dim);

    img.save(&out)?;
    println!("wrote {out} ({w}x{h}; {} demo steps, {} file bytes)", steps.len(), file_bytes.len());
    Ok(())
}
