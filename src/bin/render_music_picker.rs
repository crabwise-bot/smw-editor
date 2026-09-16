//! Headless mock screenshot of the named music-track picker.
//!
//! egui can't render headless, so this composes an honest mock of the level
//! editor's primary-header "Music:" row: the combo-box labels are real —
//! produced by the exact function the UI calls
//! (`smwe_rom::music::format_music_track`) — and the selected track is the
//! actual header music byte of a real vanilla level parsed from the ROM.
//! Only the window chrome and combo-box frame are drawn rather than real egui
//! widgets.
//!
//! ```sh
//! cargo run --bin render_music_picker -- --out=docs/screenshots/music-picker.png --rom=smw.smc
//! ```

use ab_glyph::{Font, FontRef, Glyph, Point, PxScale, ScaleFont};
use image::{Rgb, RgbImage};
use smwe_rom::music::{format_music_track, MUSIC_TRACK_COUNT};

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

use smw_editor::render_util::{fill_rect, rect_border};
fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let output = args.iter().find_map(|a| a.strip_prefix("--out=")).unwrap_or("docs/screenshots/music-picker.png");
    let rom_path = args
        .iter()
        .find_map(|a| a.strip_prefix("--rom="))
        .or_else(|| args.iter().skip(1).find(|a| !a.starts_with("--")).map(|a| a.as_str()))
        .unwrap_or("smw.smc");

    let sans = load_font(SANS_CANDIDATES)?;
    let sans_bold = load_font(SANS_BOLD_CANDIDATES)?;

    // Real data path: parse the ROM, read a vanilla level's header music byte.
    let rom = smwe_rom::SmwRom::from_file(rom_path)?;
    let level_idx = 0x05usize;
    let music_value = rom.levels[level_idx].primary_header.music();
    let selected_label = format_music_track(music_value);

    let (w, h) = (1180u32, 720u32);
    let mut img = RgbImage::new(w, h);
    let bg = Rgb([0xF2, 0xF2, 0xF2]);
    let ink = Rgb([0x1A, 0x1A, 0x1A]);
    let gray = Rgb([0x66, 0x66, 0x66]);
    for p in img.pixels_mut() {
        *p = bg;
    }

    // Title bar.
    fill_rect(&mut img, 0, 0, w, 52, Rgb([0x2B, 0x2B, 0x2B]));
    draw_text(
        &mut img,
        &sans_bold,
        "Level Editor \u{2014} Music track picker (headless mock; combo labels are the real editor strings)",
        24,
        15,
        19.0,
        Rgb([0xFF, 0xFF, 0xFF]),
    );

    // ---- Left: the picker mock ----
    let lx = 24u32;
    let mut y = 84u32;
    draw_text(&mut img, &sans_bold, "Primary header", lx as i32, y as i32, 17.0, ink);
    y += 34;
    draw_text(
        &mut img,
        &sans,
        &format!("Level 0x{level_idx:03X} \u{2014} header music byte: {music_value}"),
        lx as i32,
        y as i32,
        15.0,
        gray,
    );
    y += 34;

    // "Music:" label + combo box frame.
    draw_text(&mut img, &sans, "Music:", lx as i32, (y + 6) as i32, 15.0, ink);
    let combo_x = lx + 90;
    let combo_w = 300u32;
    let combo_h = 34u32;
    fill_rect(&mut img, combo_x, y, combo_w, combo_h, Rgb([0xFF, 0xFF, 0xFF]));
    rect_border(&mut img, combo_x, y, combo_w, combo_h, Rgb([0x99, 0x99, 0x99]));
    draw_text(&mut img, &sans, &selected_label, (combo_x + 10) as i32, (y + 8) as i32, 15.0, ink);
    draw_text(&mut img, &sans, "\u{25BE}", (combo_x + combo_w - 28) as i32, (y + 8) as i32, 15.0, gray);
    y += combo_h + 6;

    // Open dropdown: all 8 real labels, selected row highlighted.
    let row_h = 32u32;
    let drop_w = combo_w;
    draw_text(&mut img, &sans, "Dropdown (open):", lx as i32, (y + 4) as i32, 14.0, gray);
    y += 30;
    let drop_y = y;
    for t in 0..MUSIC_TRACK_COUNT {
        let ry = drop_y + t as u32 * row_h;
        let label = format_music_track(t);
        if t == music_value {
            fill_rect(&mut img, combo_x, ry, drop_w, row_h, Rgb([0xD6, 0xE8, 0xFA]));
        } else {
            fill_rect(&mut img, combo_x, ry, drop_w, row_h, Rgb([0xFF, 0xFF, 0xFF]));
        }
        draw_text(&mut img, &sans, &label, (combo_x + 10) as i32, (ry + 7) as i32, 15.0, ink);
    }
    rect_border(&mut img, combo_x, drop_y, drop_w, row_h * MUSIC_TRACK_COUNT as u32, Rgb([0x66, 0x66, 0x66]));

    // ---- Right: raw-byte fallback ----
    let rx = 560u32;
    let mut ry2 = 84u32;
    draw_text(&mut img, &sans_bold, "Raw-byte fallback", rx as i32, ry2 as i32, 17.0, ink);
    ry2 += 34;
    let wrap = |img: &mut RgbImage, text: &str, x: u32, y: &mut u32| {
        for line in text.split('\n') {
            draw_text(img, &sans, line, x as i32, *y as i32, 14.0, gray);
            *y += 22;
        }
    };
    wrap(
        &mut img,
        "Values outside 0\u{2013}7 (ROM hacks that\nremap the music table) never hide\nthe byte:",
        rx,
        &mut ry2,
    );
    ry2 += 8;
    let fallback_label = format_music_track(42);
    fill_rect(&mut img, rx, ry2, 300, 34, Rgb([0xFF, 0xFF, 0xFF]));
    rect_border(&mut img, rx, ry2, 300, 34, Rgb([0x99, 0x99, 0x99]));
    draw_text(&mut img, &sans, &fallback_label, (rx + 10) as i32, (ry2 + 8) as i32, 15.0, ink);
    ry2 += 52;
    wrap(
        &mut img,
        "Track names come from SMWDisX\nbank_05.asm `LevelMusicTable`\n(!BGM_OVERWORLD \u{2026} !BGM_BONUSGAME).",
        rx,
        &mut ry2,
    );

    // Caption.
    let cy = 660u32;
    draw_text(
        &mut img,
        &sans,
        "Mock window chrome \u{2014} every track label is produced by smwe_rom::music::format_music_track,",
        24,
        cy as i32,
        13.0,
        gray,
    );
    draw_text(
        &mut img,
        &sans,
        "the exact function the egui ComboBox calls; the selected value is level 0x005's real header byte.",
        24,
        (cy + 22) as i32,
        13.0,
        gray,
    );

    img.save(output)?;
    println!("wrote {output} ({w}x{h}), selected={selected_label}");
    Ok(())
}
