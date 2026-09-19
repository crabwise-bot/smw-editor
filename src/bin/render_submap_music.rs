//! Screenshot: the "Overworld Submap Music" dialog backed by the real ROM.
//!
//! Renders an egui-styled mock of the LM v1.30 "Change Overworld Music"
//! dialog showing the exact data the real dialog shows: the 7 per-submap
//! track IDs parsed from the real ROM's `$048D8A` (`OverworldMusic`) table,
//! labeled with the same track names the dialog's combo boxes use.
//!
//! Usage: `cargo run --bin render_submap_music -- --rom=smw.smc
//! --out=docs/screenshots/ow-submap-music.png`
//!
//! Never commits or copies the ROM; it is only read for the table values.

use std::{env, path::Path};

use ab_glyph::{Font, FontRef, Glyph, Point, PxScale, ScaleFont};
use image::{ImageBuffer, Rgb};
use smw_editor::render_util::{fill_rect, rect_border};
use smwe_rom::overworld::{
    submap_music::{format_submap_music_track, SubmapMusic, SUBMAP_MUSIC_LEN},
    SUBMAP_NAMES,
};

const W: u32 = 620;
const ROW_H: u32 = 34;
const HEADER_PX: u32 = 100;

fn main() {
    let args: Vec<String> = env::args().collect();
    let rom_path = args
        .iter()
        .find_map(|a| a.strip_prefix("--rom="))
        .map(Path::new)
        .or_else(|| args.iter().skip(1).find(|a| !a.starts_with("--")).map(|a| Path::new(a)))
        .unwrap_or_else(|| Path::new("smw.smc"));
    let output = args.iter().find_map(|a| a.strip_prefix("--out=")).unwrap_or("ow_submap_music.png");

    let raw = std::fs::read(rom_path).expect("cannot read smw.smc");
    let rom_bytes = if raw.len() % 0x400 == 0x200 { raw[0x200..].to_vec() } else { raw };

    let music = SubmapMusic::parse(&rom_bytes, 0).expect("submap music parse");
    let in_sync = SubmapMusic::tables_in_sync(&rom_bytes, 0).unwrap_or(false);

    let footer_px = 62u32;
    let h = HEADER_PX + SUBMAP_MUSIC_LEN as u32 * ROW_H + footer_px;
    let mut img = ImageBuffer::from_pixel(W, h, Rgb([27, 27, 30]));

    // Title bar.
    fill_rect(&mut img, 0, 0, W, 40, Rgb([38, 38, 44]));
    draw_text(&mut img, "Overworld Submap Music", 12, 27, 17.0, true, Rgb([240, 240, 244]));
    draw_text(
        &mut img,
        "The music the game plays on each overworld submap. Saved with Ctrl+S.",
        12,
        58,
        12.0,
        false,
        Rgb([170, 170, 178]),
    );
    draw_text(
        &mut img,
        "Lunar Magic v1.30 \"Change Overworld Music\" parity",
        12,
        76,
        12.0,
        false,
        Rgb([150, 180, 150]),
    );

    // Column headers.
    let head_y = HEADER_PX as i32 - 8;
    draw_text(&mut img, "Submap", 14, head_y, 12.0, true, Rgb([200, 200, 208]));
    draw_text(&mut img, "Music", 250, head_y, 12.0, true, Rgb([200, 200, 208]));

    for submap in 0..SUBMAP_MUSIC_LEN {
        let y0 = HEADER_PX + submap as u32 * ROW_H;
        if submap % 2 == 1 {
            fill_rect(&mut img, 0, y0, W, ROW_H, Rgb([32, 32, 37]));
        }
        let base = (y0 + 23) as i32;
        let name = SUBMAP_NAMES.get(submap).copied().unwrap_or("Submap");
        draw_text(&mut img, name, 14, base, 13.0, true, Rgb([220, 220, 228]));
        // Combo-box-style track picker showing the real ROM value.
        let label = format_submap_music_track(music.tracks[submap]);
        draw_combo(&mut img, 246, y0 + 5, 300, &label, base);
    }

    let fy = (h - footer_px + 20) as i32;
    draw_text(
        &mut img,
        "Stored in place at $048D8A (overworld init) and $04DBC8 (submap swap).",
        12,
        fy,
        11.0,
        false,
        Rgb([130, 130, 140]),
    );
    draw_text(
        &mut img,
        &format!(
            "Headless rendering of the dialog's real data, read from smw.smc (mirror tables {})",
            if in_sync { "in sync" } else { "OUT OF SYNC" }
        ),
        12,
        fy + 18,
        11.0,
        false,
        Rgb([110, 110, 120]),
    );

    img.save(output).expect("save png");
    println!("wrote {output}");
}

fn draw_combo(img: &mut ImageBuffer<Rgb<u8>, Vec<u8>>, x: u32, y: u32, w: u32, label: &str, base: i32) {
    fill_rect(img, x, y, w, 24, Rgb([20, 20, 24]));
    rect_border(img, x, y, w, 24, Rgb([80, 80, 90]));
    draw_text(img, label, x as i32 + 10, base, 13.0, false, Rgb([230, 230, 236]));
    // Dropdown chevron.
    draw_text(img, "v", (x + w - 22) as i32, base, 13.0, true, Rgb([140, 140, 150]));
}

fn font(bold: bool) -> FontRef<'static> {
    use std::sync::OnceLock;
    static REGULAR: OnceLock<FontRef<'static>> = OnceLock::new();
    static BOLD: OnceLock<FontRef<'static>> = OnceLock::new();
    let (slot, path) = if bold {
        (&BOLD, "/usr/share/fonts/truetype/dejavu/DejaVuSans-Bold.ttf")
    } else {
        (&REGULAR, "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf")
    };
    slot.get_or_init(|| {
        let data = std::fs::read(path).expect("font");
        let leaked: &'static [u8] = Box::leak(data.into_boxed_slice());
        FontRef::try_from_slice(leaked).expect("font parse")
    })
    .clone()
}

fn draw_text(
    img: &mut ImageBuffer<Rgb<u8>, Vec<u8>>, text: &str, x: i32, baseline_y: i32, px: f32, bold: bool, color: Rgb<u8>,
) {
    let font = font(bold);
    let scaled = font.as_scaled(PxScale::from(px));
    let mut caret_x = x as f32;
    let mut prev = None;
    for ch in text.chars() {
        let id = font.glyph_id(ch);
        if let Some(p) = prev {
            caret_x += scaled.kern(p, id);
        }
        let glyph = Glyph { id, scale: PxScale::from(px), position: Point { x: caret_x, y: baseline_y as f32 } };
        if let Some(o) = scaled.outline_glyph(glyph) {
            let bb = o.px_bounds();
            o.draw(|gx, gy, v| {
                let px_x = bb.min.x as i32 + gx as i32;
                let px_y = bb.min.y as i32 + gy as i32;
                if px_x >= 0 && px_y >= 0 {
                    let (px_x, px_y) = (px_x as u32, px_y as u32);
                    if px_x < img.width() && px_y < img.height() && v > 0.4 {
                        img.put_pixel(px_x, px_y, color);
                    }
                }
            });
        }
        prev = Some(id);
        caret_x += scaled.h_advance(id);
    }
}
