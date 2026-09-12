//! Headless mock screenshot of the world-editor event-ownership panel.
//!
//! egui can't render headless, so this composes an honest mock of the new
//! "Event ownership (by level)" panel: the translevel → event assignments,
//! level names, and per-event reveal-tile annotations are real — produced
//! from the ROM by the same code the UI uses
//! (`smwe_rom::overworld::event_ownership::EventOwnership::parse`,
//! `smwe_rom::overworld::level_names::decode_all`,
//! `smwe_rom::overworld::OverworldEvents::parse`) — only the window chrome
//! (title bar, combo-box borders) is drawn rather than real egui widgets.
//!
//! Shows translevels 0x27–0x2D before/after reassigning 0x29
//! (YOSHI'S ISLAND 1) from its vanilla event to event 0x2A, plus a real
//! validation rejection for an out-of-range event.
//!
//! ```sh
//! cargo run --bin render_event_ownership -- --out=docs/screenshots/event-ownership.png --rom=/path/to/smw.smc
//! ```

use ab_glyph::{Font, FontRef, Glyph, Point, PxScale, ScaleFont};
use image::{Rgb, RgbImage};
use smwe_rom::overworld::{event_ownership as eo, level_names, OverworldEvents};
use smwe_rom::snes_utils::rom::Rom;

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

/// Cosmetic mock of a combo box; the label text is real ROM data.
fn draw_combo(img: &mut RgbImage, fonts: &Fonts, x: u32, y: u32, w: u32, text: &str, highlight: bool) {
    let h = 30u32;
    let bg = if highlight { Rgb([0xFF, 0xF3, 0xD6]) } else { Rgb([0xFF, 0xFF, 0xFF]) };
    fill_rect(img, x, y, w, h, bg);
    rect_border(img, x, y, w, h, Rgb([0x99, 0x99, 0x99]));
    draw_text(img, &fonts.mono, text, (x + 8) as i32, (y + 7) as i32, 13.0, Rgb([0x1A, 0x1A, 0x1A]));
    draw_text(img, &fonts.sans, "\u{25BE}", (x + w - 24) as i32, (y + 6) as i32, 14.0, Rgb([0x66, 0x66, 0x66]));
}

/// Same label formatting as the UI's `event_option_label`: event number plus
/// the tile it reveals (from the real reveal-tile table).
fn event_label(events: &OverworldEvents, event: Option<u8>) -> String {
    match event {
        None => "None (no event)".to_string(),
        Some(e) => {
            let off = events.tile_offsets.get(e as usize).copied().unwrap_or(0);
            if off == 0 {
                format!("Event {e}")
            } else {
                format!("Event {e} \u{2014} reveals tile {off:#06X}")
            }
        }
    }
}

fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let output = args
        .iter()
        .find_map(|a| a.strip_prefix("--out="))
        .unwrap_or("docs/screenshots/event-ownership.png");
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

    // Real data path, identical to the UI.
    let raw = std::fs::read(rom_path)?;
    let rom_bytes: &[u8] = if raw.len() % 0x400 == 0x200 { &raw[0x200..] } else { &raw[..] };
    let ownership = eo::EventOwnership::parse(rom_bytes, 0)?;
    anyhow::ensure!(ownership.table.len() == eo::EVENT_OWNERSHIP_COUNT);
    let names = level_names::decode_all(rom_bytes, 0, false).ok_or_else(|| anyhow::anyhow!("decode names failed"))?;
    let rom = Rom::new(rom_bytes.to_vec()).map_err(|e| anyhow::anyhow!("Rom::new: {e:?}"))?;
    let events = OverworldEvents::parse(&rom)?;

    // Demo translevels 0x27..=0x2D; 0x29 == "YOSHI'S ISLAND 1" in the vanilla ROM.
    let tls: Vec<usize> = (0x27..=0x2D).collect();
    anyhow::ensure!(names[0x29].trim() == "YOSHI'S ISLAND 1", "unexpected name: {:?}", names[0x29]);

    // AFTER: reassign 0x29 to event 0x2A via the real set_event.
    let mut after = ownership.clone();
    after.set_event(0x29, Some(0x2A))?;
    anyhow::ensure!(after.event_for(0x29)? == Some(0x2A));

    // A real rejection message for the caption.
    let bad_err = after.set_event(0x29, Some(0x6F)).unwrap_err().to_string();

    // Prove the save path: in-place write, then re-parse.
    let mut patched = rom_bytes.to_vec();
    after.apply_to_rom(&mut patched, 0)?;
    let back = eo::EventOwnership::parse(&patched, 0)?;
    anyhow::ensure!(back.event_for(0x29)? == Some(0x2A));
    anyhow::ensure!(back.event_for(0x2D)? == ownership.event_for(0x2D)?);

    // ---- Compose the mock window ----
    let (w, h) = (1240u32, 700u32);
    let mut img = RgbImage::new(w, h);
    let bg = Rgb([0xF2, 0xF2, 0xF2]);
    let ink = Rgb([0x1A, 0x1A, 0x1A]);
    let gray = Rgb([0x66, 0x66, 0x66]);
    let red = Rgb([0xC0, 0x30, 0x30]);
    let hi = Rgb([0xB0, 0x6A, 0x00]);
    for p in img.pixels_mut() {
        *p = bg;
    }

    // Title bar.
    fill_rect(&mut img, 0, 0, w, 52, Rgb([0x2B, 0x2B, 0x2B]));
    draw_text(
        &mut img,
        &fonts.sans_bold,
        "World Editor \u{2014} Event ownership (headless mock; assignments + names + reveal tiles are real ROM output)",
        24,
        15,
        18.0,
        Rgb([0xFF, 0xFF, 0xFF]),
    );

    let panel_w = 576u32;
    let panel_x = [24u32, 640u32];
    let panels = [
        ("BEFORE \u{2014} vanilla assignments", &ownership, false),
        ("AFTER \u{2014} 0x29 reassigned to Event 0x2A", &after, true),
    ];
    for (pi, (title, eo_tbl, is_after)) in panels.iter().enumerate() {
        let x = panel_x[pi];
        let mut y = 72u32;
        draw_text(&mut img, &fonts.sans_bold, title, x as i32, y as i32, 17.0, ink);
        y += 36;
        draw_text(
            &mut img,
            &fonts.sans,
            "Which event triggers when each level is beaten ($05D608)",
            x as i32,
            y as i32,
            13.0,
            gray,
        );
        y += 30;
        for &tl in &tls {
            let changed = *is_after && tl == 0x29;
            let name: String = names[tl].trim().to_string();
            draw_text(
                &mut img,
                &fonts.mono,
                &format!("0x{tl:02X} {name}"),
                x as i32,
                (y + 6) as i32,
                13.0,
                if changed { hi } else { ink },
            );
            let label = event_label(&events, eo_tbl.event_for(tl)?);
            draw_combo(&mut img, &fonts, x + 268, y, panel_w - 268, &label, changed);
            y += 40;
        }
    }

    // Caption: real rejection message + save-path proof + honesty note.
    let cy = 560u32;
    draw_text(
        &mut img,
        &fonts.sans,
        &format!("Out-of-range event refused, not clamped: \u{201C}{bad_err}\u{201D}"),
        24,
        cy as i32,
        14.0,
        red,
    );
    draw_text(
        &mut img,
        &fonts.sans,
        "Save path: 93-byte table written back in place at SNES $05D608 (PC 0x2D608) \u{2014} no relocation patch needed.",
        24,
        (cy + 28) as i32,
        13.0,
        gray,
    );
    draw_text(
        &mut img,
        &fonts.sans,
        "Mock window chrome \u{2014} the assignments, level names, and reveal-tile annotations are produced by smwe_rom from the ROM.",
        24,
        (cy + 54) as i32,
        13.0,
        gray,
    );

    img.save(output)?;
    println!("wrote {output} ({w}x{h})");
    Ok(())
}
