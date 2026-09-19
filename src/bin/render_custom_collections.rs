//! Headless screenshot of the Custom Collections of Objects UI (LM v3.60 parity).
//!
//! egui can't render headless, so this composes an honest mock of the two
//! new UI surfaces. Everything stateful is real program output:
//! - The collections store is the real `smw_editor::custom_collections`
//!   model: collections/entries are added through the real API, saved to a
//!   temp JSON file, reloaded, and asserted identical before drawing.
//! - The ID-parsing and reserved-ID refusal come from the real functions
//!   (`parse_extended_id`, `add_entry` refusing 0x00/0x01) — the yellow
//!   status line in the mock is the real `Err` message.
//! - Entry/collection names are illustrative example data, as a user would
//!   type them; IDs are parsed from hex strings through the real parser.
//!
//! The two panels show: the manager window (toolbar toggle) and the
//! draw-mode left-panel picker with one entry armed for placement.
//!
//! ```sh
//! cargo run --bin render_custom_collections -- --out=docs/screenshots/custom-collections.png
//! ```

use ab_glyph::{Font, FontRef, Glyph, Point, PxScale, ScaleFont};
use image::{Rgb, RgbImage};
use smw_editor::{
    custom_collections::{format_extended_id, parse_extended_id, CustomCollections},
    render_util::{fill_rect, rect_border},
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

/// Small bordered button mock with a label; returns its right edge.
fn draw_button(img: &mut RgbImage, font: &FontRef, x: u32, y: u32, label: &str) -> u32 {
    let w = 14 + label.chars().count() as u32 * 7 + 8;
    let h = 22u32;
    fill_rect(img, x, y, w, h, Rgb([62, 66, 74]));
    rect_border(img, x, y, w, h, Rgb([110, 114, 122]));
    draw_text(img, font, label, (x + 7) as i32, (y + 4) as i32, 12.0, Rgb([235, 235, 240]));
    x + w
}

/// Window chrome mock: dark body, title bar, thin border.
fn draw_window(img: &mut RgbImage, font: &FontRef, x: u32, y: u32, w: u32, h: u32, title: &str) {
    fill_rect(img, x, y, w, h, Rgb([43, 46, 53]));
    rect_border(img, x, y, w, h, Rgb([100, 104, 112]));
    fill_rect(img, x, y, w, 26, Rgb([52, 56, 64]));
    draw_text(img, font, title, (x + 10) as i32, (y + 5) as i32, 14.0, Rgb([235, 235, 240]));
}

fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let output =
        args.iter().find_map(|a| a.strip_prefix("--out=")).unwrap_or("docs/screenshots/custom-collections.png");

    let sans = load_font(SANS_CANDIDATES)?;
    let sans_bold = load_font(SANS_BOLD_CANDIDATES)?;

    // ── Real store, driven through the real API ──────────────────────────
    let mut store = CustomCollections::default();
    store.add_collection("Level settings").map_err(|e| anyhow::anyhow!("{e}"))?;
    // IDs parsed from hex strings through the real parser, like the UI does.
    store
        .add_entry(0, "Scroll command", parse_extended_id("E0").map_err(|e| anyhow::anyhow!("{e}"))?)
        .map_err(|e| anyhow::anyhow!("{e}"))?;
    store
        .add_entry(0, "Palette command", parse_extended_id("$E5").map_err(|e| anyhow::anyhow!("{e}"))?)
        .map_err(|e| anyhow::anyhow!("{e}"))?;
    store.add_collection("Boss arena").map_err(|e| anyhow::anyhow!("{e}"))?;
    store
        .add_entry(1, "Boss HP tweak", parse_extended_id("0xF2").map_err(|e| anyhow::anyhow!("{e}"))?)
        .map_err(|e| anyhow::anyhow!("{e}"))?;

    // Save → reload → identical: the persistence the editor relies on.
    let tmp = std::env::temp_dir().join(format!("smwe-cc-shot-{}", std::process::id()));
    let path = tmp.join("custom_collections.json");
    store.save_to(&path)?;
    let reloaded = CustomCollections::load_from(&path);
    assert_eq!(reloaded, store, "JSON round trip must be lossless");
    std::fs::remove_dir_all(&tmp).ok();

    // The real refusal message for the reserved IDs (exit / screen jump).
    let mut probe = store.clone();
    let refusal = probe.add_entry(0, "Reserved test", 0x00).unwrap_err();
    assert!(probe.add_entry(0, "Reserved test", 0x01).is_err());

    // ── Compose the image ────────────────────────────────────────────────
    let w = 1240u32;
    let h = 640u32;
    let mut img = RgbImage::new(w, h);
    fill_rect(&mut img, 0, 0, w, h, Rgb([26, 28, 33]));
    let white = Rgb([235, 235, 240]);
    let gray = Rgb([170, 174, 182]);
    let dim = Rgb([120, 124, 132]);
    let yellow = Rgb([255, 210, 120]);

    draw_text(&mut img, &sans_bold, "Custom Collections of Objects  (Lunar Magic v3.60 parity)", 24, 16, 20.0, white);

    // ── Manager window mock ──────────────────────────────────────────────
    let (mx, my, mw, mh) = (24u32, 52u32, 700u32, 470u32);
    draw_window(&mut img, &sans_bold, mx, my, mw, mh, "Custom Collections of Objects");
    let mut ty = my + 34;
    draw_text(
        &mut img,
        &sans,
        "Lunar Magic v3.60: named groups of custom extended objects.",
        (mx + 12) as i32,
        ty as i32,
        13.0,
        gray,
    );
    ty += 20;
    let store_line = format!("Stored per-user at {} (not in the ROM).", CustomCollections::default_path().display());
    // Truncate to fit the window.
    let store_line = if store_line.len() > 96 { format!("{}…", &store_line[..95]) } else { store_line };
    draw_text(&mut img, &sans, &store_line, (mx + 12) as i32, ty as i32, 11.0, dim);
    ty += 30;

    // Collections column (left, 240 wide).
    let col_x = mx + 12;
    draw_text(&mut img, &sans_bold, "Collections", col_x as i32, ty as i32, 14.0, white);
    ty += 24;
    for (i, c) in store.collections.iter().enumerate() {
        if i == 0 {
            // Selected collection highlight.
            fill_rect(&mut img, col_x, ty - 3, 228, 22, Rgb([70, 130, 200]));
        }
        draw_text(
            &mut img,
            &sans,
            &format!("{} ({})", c.name, c.entries.len()),
            (col_x + 6) as i32,
            ty as i32,
            13.0,
            white,
        );
        ty += 26;
    }
    ty += 6;
    draw_text(&mut img, &sans, "New:", col_x as i32, (ty + 3) as i32, 12.0, gray);
    fill_rect(&mut img, col_x + 42, ty, 120, 22, Rgb([30, 32, 37]));
    rect_border(&mut img, col_x + 42, ty, 120, 22, Rgb([110, 114, 122]));
    draw_button(&mut img, &sans, col_x + 170, ty, "Add");
    ty += 30;
    let bx = draw_button(&mut img, &sans, col_x, ty, "Rename");
    draw_button(&mut img, &sans, bx + 6, ty, "Delete");

    // Entries column (right), starting at the same top as the collections.
    let ex = mx + 280;
    let mut ey = my + 108;
    let sel = &store.collections[0];
    draw_text(&mut img, &sans_bold, &format!("{} — entries", sel.name), ex as i32, ey as i32, 14.0, white);
    ey += 26;
    for e in &sel.entries {
        draw_text(
            &mut img,
            &sans,
            &format!("{}  {}", e.name, format_extended_id(e.extended_id)),
            ex as i32,
            ey as i32,
            13.0,
            white,
        );
        let mut bx = ex + 225;
        bx = draw_button(&mut img, &sans, bx, ey - 4, "Place") + 6;
        bx = draw_button(&mut img, &sans, bx, ey - 4, "Edit") + 6;
        draw_button(&mut img, &sans, bx, ey - 4, "Delete");
        ey += 30;
    }
    ey += 8;
    draw_text(&mut img, &sans, "Name:", ex as i32, (ey + 3) as i32, 12.0, gray);
    fill_rect(&mut img, ex + 52, ey, 150, 22, Rgb([30, 32, 37]));
    rect_border(&mut img, ex + 52, ey, 150, 22, Rgb([110, 114, 122]));
    draw_text(&mut img, &sans, "ID:", (ex + 212) as i32, (ey + 3) as i32, 12.0, gray);
    fill_rect(&mut img, ex + 238, ey, 52, 22, Rgb([30, 32, 37]));
    rect_border(&mut img, ex + 238, ey, 52, 22, Rgb([110, 114, 122]));
    draw_text(&mut img, &sans, "E0", (ex + 244) as i32, (ey + 3) as i32, 12.0, dim);
    draw_button(&mut img, &sans, ex + 300, ey, "Add entry");
    ey += 40;
    // The real refusal message from add_entry(0x00).
    draw_text(&mut img, &sans, &refusal, ex as i32, ey as i32, 11.0, yellow);

    // Status line at the window bottom.
    draw_text(
        &mut img,
        &sans,
        "Entry names are illustrative example data; the store above was built through the real API",
        (mx + 12) as i32,
        (my + mh - 26) as i32,
        11.0,
        dim,
    );

    // ── Draw-mode picker mock ────────────────────────────────────────────
    let (px, py, pw, ph) = (748u32, 52u32, 468u32, 470u32);
    draw_window(&mut img, &sans_bold, px, py, pw, ph, "Level Editor — left panel (Draw mode)");
    let mut qy = py + 38;
    // Armed banner (entry 0 of collection 0 is "armed").
    let armed = &store.collections[0].entries[0];
    draw_text(
        &mut img,
        &sans_bold,
        &format!("Placing custom object: {}/{}", store.collections[0].name, armed.name),
        (px + 12) as i32,
        qy as i32,
        13.0,
        white,
    );
    qy += 22;
    draw_button(&mut img, &sans, px + 12, qy, "✕ Cancel");
    draw_text(
        &mut img,
        &sans,
        "Click the canvas in Draw mode to place it.",
        (px + 100) as i32,
        (qy + 4) as i32,
        12.0,
        gray,
    );
    qy += 40;
    draw_text(
        &mut img,
        &sans_bold,
        "▾ Custom Collections of Objects (LM 3.60)",
        (px + 12) as i32,
        qy as i32,
        13.0,
        white,
    );
    qy += 26;
    for (ci, c) in store.collections.iter().enumerate() {
        draw_text(
            &mut img,
            &sans,
            &format!("  ▾ {} ({})", c.name, c.entries.len()),
            (px + 12) as i32,
            qy as i32,
            13.0,
            gray,
        );
        qy += 24;
        for (ei, e) in c.entries.iter().enumerate() {
            let armed_here = ci == 0 && ei == 0;
            if armed_here {
                fill_rect(&mut img, px + 24, qy - 3, pw - 48, 24, Rgb([58, 64, 74]));
            }
            draw_text(
                &mut img,
                &sans,
                &format!("{}  {}", e.name, format_extended_id(e.extended_id)),
                (px + 30) as i32,
                qy as i32,
                12.0,
                white,
            );
            let bx = draw_button(&mut img, &sans, px + pw - 110, qy - 4, if armed_here { "Armed ✓" } else { "Place" });
            let _ = bx;
            qy += 28;
        }
        qy += 4;
    }
    draw_button(&mut img, &sans, px + 12, qy, "Manage collections…");
    qy += 44;
    draw_text(&mut img, &sans, "Placing paints no tiles: custom extended", (px + 12) as i32, qy as i32, 12.0, dim);
    qy += 18;
    draw_text(&mut img, &sans, "objects are level-setting commands, and", (px + 12) as i32, qy as i32, 12.0, dim);
    qy += 18;
    draw_text(&mut img, &sans, "show up in the object overlay.", (px + 12) as i32, qy as i32, 12.0, dim);

    // ── Footer ───────────────────────────────────────────────────────────
    draw_text(
        &mut img,
        &sans,
        "Real CustomCollections model: add → save to JSON → reload → identical. Reserved IDs 00/01 (exit / screen jump) refused by the real API.",
        24,
        (h - 26) as i32,
        13.0,
        dim,
    );

    img.save(output)?;
    println!("wrote {output}");
    Ok(())
}
