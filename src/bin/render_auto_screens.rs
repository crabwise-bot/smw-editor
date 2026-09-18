//! Headless mock screenshot of the LM 3.40 "Auto-Set Number of Screens" behavior.
//!
//! egui can't render headless, so this composes an honest mock: the numbers
//! are real — a vanilla level is parsed from the ROM, its objects, exits and
//! sprites are scanned with the same anchor-tile algorithm the editor's
//! `auto_screens::screens_used` uses, and the before/after screen counts are
//! what `save_to_rom` would write. Only the window chrome and checkbox are
//! drawn rather than real egui widgets.
//!
//! ```sh
//! cargo run --bin render_auto_screens -- --out=docs/screenshots/auto-set-screens.png --rom=smw.smc
//! ```

use ab_glyph::{Font, FontRef, Glyph, Point, PxScale, ScaleFont};
use image::{Rgb, RgbImage};
use smw_editor::render_util::{fill_rect, rect_border};
use smwe_rom::{
    level::{Layer2Data, Level},
    objects::Object,
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

/// Mirror of `auto_screens::screens_used`, over the raw ROM parse: returns
/// (used screens, per-screen occupancy) for the level's long axis.
fn scan_level(level: &Level) -> (u32, Vec<bool>) {
    let vertical = level.secondary_header.vertical_level();
    let mut occupied = vec![false; 32];
    let mut mark = |screen: u32| {
        if screen < 32 {
            occupied[screen as usize] = true;
        }
    };
    let scan_objects = |bytes: &[u8], mark: &mut dyn FnMut(u32)| {
        let Some(raw) = Object::parse_from_layer(bytes) else { return };
        let mut screen: u8 = 0;
        for o in raw {
            if o.is_exit() {
                mark(o.screen_number() as u32);
            } else if o.is_screen_jump() {
                screen = o.screen_number();
            } else {
                if o.is_new_screen() {
                    screen = screen.saturating_add(1);
                }
                // Vertical raw format swaps axes (raw x holds the vertical
                // position); mirror EditableObject::from_raw.
                let (ax, ay) = if vertical {
                    (o.y() as u32, (screen as u32) * 16 + o.x() as u32)
                } else {
                    ((screen as u32) * 16 + o.x() as u32, o.y() as u32)
                };
                mark(if vertical { ay / 16 } else { ax / 16 });
            }
        }
    };
    scan_objects(level.layer1.as_bytes(), &mut mark);
    if let Layer2Data::Objects { objects, .. } = &level.layer2 {
        scan_objects(objects.as_bytes(), &mut mark);
    }
    for spr in &level.sprite_layer.sprites {
        let (xt, yt) = spr.xy_pos();
        let s = spr.screen_number() as u32;
        let (ax, ay) = if vertical {
            ((s % 2) * 16 + xt as u32, (s / 2) * 32 + yt as u32)
        } else {
            (s * 16 + xt as u32, yt as u32)
        };
        mark(if vertical { ay / 16 } else { ax / 16 });
    }
    let max = occupied.iter().rposition(|&b| b).map(|i| i as u32 + 1).unwrap_or(1);
    (max.clamp(1, 32), occupied)
}

fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let output = args.iter().find_map(|a| a.strip_prefix("--out=")).unwrap_or("docs/screenshots/auto-set-screens.png");
    let rom_path = args
        .iter()
        .find_map(|a| a.strip_prefix("--rom="))
        .or_else(|| args.iter().skip(1).find(|a| !a.starts_with("--")).map(|a| a.as_str()))
        .unwrap_or("smw.smc");

    let sans = load_font(SANS_CANDIDATES)?;
    let sans_bold = load_font(SANS_BOLD_CANDIDATES)?;

    // Real data path: parse the ROM, find a horizontal level whose occupied
    // screen count differs from its declared header length.
    let rom = smwe_rom::SmwRom::from_file(rom_path)?;
    let mut pick: Option<(usize, u32, u32, Vec<bool>)> = None;
    for (idx, level) in rom.levels.iter().enumerate() {
        if level.secondary_header.vertical_level() {
            continue;
        }
        let declared = level.primary_header.level_length() as u32 + 1;
        let (used, occupied) = scan_level(level);
        if used != declared && declared <= 20 {
            pick = Some((idx, declared, used, occupied));
            if used < declared {
                break; // prefer a visible shrink
            }
        }
    }
    let (level_idx, declared, used, occupied) = pick.expect("no horizontal level with used != declared found");
    println!("level {level_idx:03X}: declared {declared} screens, occupies {used}");

    let (w, h) = (1180u32, 560u32);
    let mut img = RgbImage::new(w, h);
    let bg = Rgb([0xF2, 0xF2, 0xF2]);
    let ink = Rgb([0x1A, 0x1A, 0x1A]);
    let gray = Rgb([0x66, 0x66, 0x66]);
    let green = Rgb([0x2E, 0x8B, 0x57]);
    let red = Rgb([0xB0, 0x30, 0x30]);
    for p in img.pixels_mut() {
        *p = bg;
    }

    // Title bar.
    fill_rect(&mut img, 0, 0, w, 52, Rgb([0x2B, 0x2B, 0x2B]));
    draw_text(
        &mut img,
        &sans_bold,
        "Level Header \u{2014} Auto-Set Number of Screens, LM 3.40 (headless mock; counts are real ROM data)",
        24,
        15,
        17.0,
        Rgb([0xFF, 0xFF, 0xFF]),
    );

    // ---- Left: the per-level control ----
    let lx = 24u32;
    let mut y = 84u32;
    draw_text(&mut img, &sans_bold, "Per-level setting", lx as i32, y as i32, 16.0, ink);
    y += 34;
    // Checked checkbox mock.
    fill_rect(&mut img, lx, y + 2, 16, 16, Rgb([0xFF, 0xFF, 0xFF]));
    rect_border(&mut img, lx, y + 2, 16, 16, Rgb([0x99, 0x99, 0x99]));
    draw_text(&mut img, &sans, "\u{2713}", (lx + 2) as i32, y as i32, 14.0, ink);
    draw_text(&mut img, &sans, "Auto-Set Screens:", (lx + 26) as i32, (y + 2) as i32, 14.0, ink);
    y += 36;
    for line in [
        "On save, the header's Number of Screens is set",
        "to the screens actually used by objects and",
        "sprites. Stored per-level in $06FA00 bit 5 (C",
        "of SHCvvvvv); touching it installs the table",
        "on ROMs that lack it ($FF \u{2192} encoded byte).",
    ] {
        draw_text(&mut img, &sans, line, lx as i32, y as i32, 13.0, gray);
        y += 22;
    }
    y += 12;
    draw_text(&mut img, &sans_bold, "Scan rule", lx as i32, y as i32, 14.0, ink);
    y += 26;
    for line in [
        "Anchor tile of each object/exit/sprite along",
        "the long axis (X here); min 1, max 32 screens.",
        "Wide objects count their anchor screen only.",
    ] {
        draw_text(&mut img, &sans, line, lx as i32, y as i32, 13.0, gray);
        y += 22;
    }

    // ---- Right: before/after for the real level ----
    let rx = 400u32;
    let mut ry = 84u32;
    draw_text(
        &mut img,
        &sans_bold,
        &format!("Level 0x{level_idx:03X} \u{2014} what save writes"),
        rx as i32,
        ry as i32,
        16.0,
        ink,
    );
    ry += 34;
    draw_text(
        &mut img,
        &sans,
        &format!("Declared in header: {declared} screens   \u{2192}   Auto-set on save: {used} screens"),
        rx as i32,
        ry as i32,
        14.0,
        ink,
    );
    ry += 34;
    draw_text(
        &mut img,
        &sans,
        "Screen strip: green = occupied, red outline = declared but empty",
        rx as i32,
        ry as i32,
        13.0,
        gray,
    );
    ry += 30;

    // Screen strip: one cell per screen, 32 max.
    let cell = 21u32;
    for i in 0..32u32 {
        let cx = rx + i * cell;
        if occupied[i as usize] {
            fill_rect(&mut img, cx + 1, ry + 1, cell - 2, 34, green);
        } else {
            fill_rect(&mut img, cx + 1, ry + 1, cell - 2, 34, Rgb([0xFF, 0xFF, 0xFF]));
        }
        let border = if (i as u32) < declared {
            if occupied[i as usize] {
                ink
            } else {
                red
            }
        } else {
            Rgb([0xCC, 0xCC, 0xCC])
        };
        rect_border(&mut img, cx, ry, cell, 36, border);
        if i % 4 == 0 {
            draw_text(&mut img, &sans, &format!("{i}"), (cx + 3) as i32, (ry + 42) as i32, 10.0, gray);
        }
    }
    ry += 90;
    draw_text(
        &mut img,
        &sans,
        &format!("Header byte 0 after save: {} (screens \u{2212} 1), $06FA00 gains bit 5 ($20).", used - 1),
        rx as i32,
        ry as i32,
        13.0,
        gray,
    );
    ry += 26;
    draw_text(
        &mut img,
        &sans,
        "Vanilla ROMs ($06FA00 = $FF) are never resized unless one of the",
        rx as i32,
        ry as i32,
        13.0,
        gray,
    );
    ry += 22;
    draw_text(&mut img, &sans, "scroll-extension controls is touched first.", rx as i32, ry as i32, 13.0, gray);

    img.save(output)?;
    println!("wrote {output}");
    Ok(())
}
