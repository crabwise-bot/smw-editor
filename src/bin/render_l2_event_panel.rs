//! Headless mock screenshot of the new "Layer 2 events" panel section.
//!
//! egui can't render headless, so this composes an honest mock of the
//! `world_editor` events panel: every label, count, and entry description is
//! real — produced from the actual parsed ROM tables
//! (`smwe_rom::overworld::OverworldL2Events`) with the same formatting the UI
//! uses. Only the window chrome and widget frames are drawn rather than real
//! egui widgets.
//!
//! ```sh
//! cargo run --bin render_l2_event_panel -- --out=docs/screenshots/l2-events-panel.png --rom=smw.smc
//! ```

use ab_glyph::{Font, FontRef, Glyph, Point, PxScale, ScaleFont};
use image::{Rgb, RgbImage};
use smwe_rom::overworld::{L2EventEntry, L2EventKind, OverworldL2Events, OW_EVENT_COUNT};

const SANS: &str = "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf";
const MONO: &str = "/usr/share/fonts/truetype/dejavu/DejaVuSansMono.ttf";

fn load_font(path: &str) -> FontRef<'static> {
    let data = std::fs::read(path).unwrap_or_else(|_| panic!("font not found: {path}"));
    let leaked: &'static [u8] = Box::leak(data.into_boxed_slice());
    FontRef::try_from_slice(leaked).expect("font parse")
}

fn draw_text(img: &mut RgbImage, font: &FontRef, text: &str, x: i32, baseline_y: i32, px: f32, color: Rgb<u8>) {
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
                    if px_x < img.width() && px_y < img.height() {
                        let a = (v * 255.0) as u16;
                        let inv = 255 - a;
                        let d = img.get_pixel(px_x, px_y).0;
                        let s = color.0;
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
        prev = Some(id);
        caret_x += scaled.h_advance(id);
    }
}

fn describe(entry: &L2EventEntry) -> String {
    let (col, row) = entry.target_tile();
    match entry.kind() {
        L2EventKind::TileStream(n) => format!("stream {n} tiles -> ({col},{row})"),
        L2EventKind::TilemapCopy(off) => format!("tilemap copy WRAM+{off:#06X} -> ({col},{row})"),
    }
}

use smw_editor::render_util::fill_rect;
fn main() {
    let args: Vec<String> = std::env::args().collect();
    let rom_path = args.iter().find_map(|a| a.strip_prefix("--rom=")).unwrap_or("smw.smc");
    let out = args.iter().find_map(|a| a.strip_prefix("--out=")).unwrap_or("l2-events-panel.png");

    let raw = std::fs::read(rom_path).expect("cannot read ROM");
    let rom_bytes = if raw.len() % 0x400 == 0x200 { raw[0x200..].to_vec() } else { raw };
    let rom = smwe_rom::snes_utils::rom::Rom::new(rom_bytes).expect("rom parse");
    let l2 = OverworldL2Events::parse(&rom).expect("L2 events parse");

    let sans = load_font(SANS);
    let mono = load_font(MONO);
    let text_c = Rgb([230, 230, 235]);
    let dim_c = Rgb([150, 150, 165]);
    let accent_c = Rgb([120, 180, 250]);

    // Panel lines: (indent_px, text, font, size, color).
    let mut lines: Vec<(u32, String, bool, f32, Rgb<u8>)> = Vec::new();
    lines.push((8, "▾ Layer 2 events".to_string(), false, 16.0, text_c));
    lines.push((28, "☑ Show target markers on map".to_string(), false, 14.0, text_c));
    let events_with_l2 = (0..OW_EVENT_COUNT)
        .filter(|&e| {
            !l2.entries_for_event(e).unwrap_or(0..0).is_empty() || !l2.silent_l2_events_for(e as u8).is_empty()
        })
        .count();
    lines.push((
        28,
        format!(
            "{} table entries · {} events with L2 data · {} silent L2 rows",
            l2.entry_count(),
            events_with_l2,
            l2.silent_events.iter().filter(|s| s.is_l2).count()
        ),
        false,
        14.0,
        dim_c,
    ));
    lines.push((28, "Markers follow the event checkboxes; the animated".to_string(), false, 13.0, dim_c));
    lines.push((28, "L2 sequence itself runs in-game.".to_string(), false, 13.0, dim_c));

    // Expand event 1 (the first non-empty one); show one collapsed row after.
    for &event in &[1usize, 3] {
        let range = l2.entries_for_event(event).unwrap_or(0..0);
        let header =
            format!("{} Event {event}: entries {}..{}", if event == 1 { "▾" } else { "▸" }, range.start, range.end);
        lines.push((28, header, false, 14.0, accent_c));
        if event == 1 {
            for idx in range {
                if let Some(entry) = l2.entries.get(idx) {
                    lines.push((52, format!("[{idx:3}] {}", describe(entry)), true, 13.0, text_c));
                }
            }
            for s in l2.silent_l2_events_for(event as u8) {
                lines.push((52, format!("[silent] {}", describe(&s.as_entry())), true, 13.0, text_c));
            }
        }
    }
    // One silent-only event for flavor: find the first event with silent rows but no table entries.
    if let Some(ev) = (0..OW_EVENT_COUNT)
        .find(|&e| l2.entries_for_event(e).unwrap_or(0..0).is_empty() && !l2.silent_l2_events_for(e as u8).is_empty())
    {
        lines.push((28, format!("▾ Event {ev}: silent row only"), false, 14.0, accent_c));
        for s in l2.silent_l2_events_for(ev as u8) {
            lines.push((52, format!("[silent] {}", describe(&s.as_entry())), true, 13.0, text_c));
        }
    }

    let line_h = 24u32;
    let (w, h) = (620u32, lines.len() as u32 * line_h + 24);
    let mut img = RgbImage::new(w, h);
    fill_rect(&mut img, 0, 0, w, h, Rgb([22, 22, 28]));
    // panel border
    for x in 0..w {
        img.put_pixel(x, 0, Rgb([60, 60, 70]));
        img.put_pixel(x, h - 1, Rgb([60, 60, 70]));
    }
    for y in 0..h {
        img.put_pixel(0, y, Rgb([60, 60, 70]));
        img.put_pixel(w - 1, y, Rgb([60, 60, 70]));
    }
    for (i, (indent, text, is_mono, size, color)) in lines.iter().enumerate() {
        let font = if *is_mono { &mono } else { &sans };
        draw_text(&mut img, font, text, *indent as i32, (i as u32 * line_h + 20) as i32, *size, *color);
    }
    img.save(out).expect("save png");
    println!("wrote {out}");
}
