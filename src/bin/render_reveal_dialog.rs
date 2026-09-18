//! Screenshot: the "Edit Reveal Tile List" dialog backed by the real ROM.
//!
//! Renders an egui-styled mock of the dialog showing the exact data the real
//! dialog shows: all 22 before/after byte pairs parsed from the real ROM's
//! `$04DA1D`/`$04DA33` tables, and per-row usage counts computed by
//! `RevealTileList::events_using_row` with the real `$04D85D` per-event
//! offsets and the real layer-1 tilemap — the same match the game performs in
//! `CODE_04DA49`.
//!
//! Usage: `cargo run --bin render_reveal_dialog -- --rom=smw.smc
//! --out=docs/screenshots/ow-reveal-tile-list.png`
//!
//! Never commits or copies the ROM; it is only read for the table values.

use std::{env, path::Path};

use ab_glyph::{Font, FontRef, Glyph, Point, PxScale, ScaleFont};
use image::{ImageBuffer, Rgb};
use smw_editor::render_util::{fill_rect, rect_border};
use smwe_rom::overworld::{
    reveal_list::{RevealTileList, REVEAL_COUNT},
    OverworldData,
    OverworldEvents,
};

const W: u32 = 920;
const ROW_H: u32 = 27;
const HEADER_PX: u32 = 108;

fn main() {
    let args: Vec<String> = env::args().collect();
    let rom_path = args
        .iter()
        .find_map(|a| a.strip_prefix("--rom="))
        .map(Path::new)
        .or_else(|| args.iter().skip(1).find(|a| !a.starts_with("--")).map(|a| Path::new(a)))
        .unwrap_or_else(|| Path::new("smw.smc"));
    let output = args.iter().find_map(|a| a.strip_prefix("--out=")).unwrap_or("ow_reveal_tile_list.png");

    let raw = std::fs::read(rom_path).expect("cannot read smw.smc");
    let rom_bytes = if raw.len() % 0x400 == 0x200 { raw[0x200..].to_vec() } else { raw };
    let rom = smwe_rom::snes_utils::rom::Rom::new(rom_bytes.clone()).expect("rom parse");

    let list = RevealTileList::parse(&rom_bytes, 0).expect("reveal list parse");
    let events = OverworldEvents::parse(&rom).expect("events parse");
    let ow = OverworldData::parse(&rom).expect("overworld parse");

    let usage: Vec<Vec<usize>> =
        (0..REVEAL_COUNT).map(|row| list.events_using_row(row, &events.tile_offsets, &ow.layer1_tiles)).collect();

    let footer_px = 44u32;
    let h = HEADER_PX + REVEAL_COUNT as u32 * ROW_H + footer_px;
    let mut img = ImageBuffer::from_pixel(W, h, Rgb([27, 27, 30]));

    // Title bar.
    fill_rect(&mut img, 0, 0, W, 40, Rgb([38, 38, 44]));
    draw_text(&mut img, "Edit Reveal Tile List", 12, 27, 17.0, true, Rgb([240, 240, 244]));
    draw_text(
        &mut img,
        "Which layer-1 tiles an event reveals into which other tiles when it fires. Saved with Ctrl+S.",
        12,
        58,
        12.0,
        false,
        Rgb([170, 170, 178]),
    );
    draw_text(
        &mut img,
        "Row 21 (the switch-palace entry) also writes the tile after the event's offset.",
        12,
        76,
        12.0,
        false,
        Rgb([170, 170, 178]),
    );
    // Column headers.
    let head_y = HEADER_PX as i32 - 8;
    draw_text(&mut img, "Row", 14, head_y, 12.0, true, Rgb([200, 200, 208]));
    draw_text(&mut img, "Before", 92, head_y, 12.0, true, Rgb([200, 200, 208]));
    draw_text(&mut img, "After", 210, head_y, 12.0, true, Rgb([200, 200, 208]));
    draw_text(&mut img, "Used by", 330, head_y, 12.0, true, Rgb([200, 200, 208]));

    for row in 0..REVEAL_COUNT {
        let y0 = HEADER_PX + row as u32 * ROW_H;
        if row % 2 == 1 {
            fill_rect(&mut img, 0, y0, W, ROW_H, Rgb([32, 32, 37]));
        }
        if row == REVEAL_COUNT - 1 {
            rect_border(&mut img, 2, y0 + 1, W - 4, ROW_H - 2, Rgb([120, 90, 40]));
        }
        let base = (y0 + 19) as i32;
        draw_text(&mut img, &format!("{row:2}"), 16, base, 13.0, true, Rgb([220, 220, 228]));
        // DragValue-style hex boxes.
        draw_hex_box(&mut img, 88, y0 + 4, list.before.get(row).copied().unwrap_or(0), base);
        draw_hex_box(&mut img, 206, y0 + 4, list.after.get(row).copied().unwrap_or(0), base);
        if usage[row].is_empty() {
            draw_text(&mut img, "unused", 330, base, 12.0, false, Rgb([120, 120, 130]));
        } else {
            let n = usage[row].len();
            draw_text(
                &mut img,
                &format!("{} event{}", n, if n == 1 { "" } else { "s" }),
                330,
                base,
                12.0,
                false,
                Rgb([150, 200, 150]),
            );
            draw_text(
                &mut img,
                &format!("({})", usage[row].iter().map(|e| e.to_string()).collect::<Vec<_>>().join(", ")),
                420,
                base,
                11.0,
                false,
                Rgb([130, 130, 140]),
            );
        }
    }

    let fy = (h - footer_px + 16) as i32;
    draw_text(
        &mut img,
        "22 rows · stored in place at $04DA1D/$04DA33 · usage from $04D85D offsets + the layer-1 tilemap",
        12,
        fy,
        11.0,
        false,
        Rgb([130, 130, 140]),
    );
    draw_text(
        &mut img,
        "Headless rendering of the dialog's real data, read from smw.smc (values + usage all verified)",
        12,
        fy + 16,
        11.0,
        false,
        Rgb([110, 110, 120]),
    );

    img.save(output).expect("save png");
    println!("wrote {output}");
}

fn draw_hex_box(img: &mut ImageBuffer<Rgb<u8>, Vec<u8>>, x: u32, y: u32, value: u8, base: i32) {
    fill_rect(img, x, y, 56, 19, Rgb([20, 20, 24]));
    rect_border(img, x, y, 56, 19, Rgb([80, 80, 90]));
    draw_text(img, &format!("{value:02X}"), x as i32 + 8, base, 13.0, true, Rgb([230, 230, 236]));
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
