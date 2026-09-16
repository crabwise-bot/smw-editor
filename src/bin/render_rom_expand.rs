//! Headless mock screenshot of the Expand ROM dialog (File > Expand ROM...).
//!
//! egui can't render headless, so this composes an honest mock of the dialog:
//! every number on screen is real — produced from the ROM by the same code
//! the UI uses (`rom_expansion::expand_rom`, `expansion_targets`, the free-
//! space byte scan). Only the window chrome (title bar, radio circles,
//! buttons) is drawn rather than real egui widgets.
//!
//! ```sh
//! cargo run --bin render_rom_expand -- --out=docs/screenshots/rom-expand.png --rom=smw.smc
//! ```
//!
//! `--expand-copy <path>` instead expands that file in place (with the same
//! `.bak` backup the UI makes) and re-opens it through `SmwRom::from_file`,
//! proving an expanded image loads cleanly.

use ab_glyph::{Font, FontRef, Glyph, Point, PxScale, ScaleFont};
use image::{Rgb, RgbImage};
use smw_editor::render_util::{fill_rect, rect_border};
use smwe_rom::rom_expansion::{compute_checksum, expand_rom, expansion_targets, format_size};

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
        caret_x += scaled.h_advance(id);
        prev = Some(id);
    }
}

fn free_kb(bytes: &[u8]) -> usize {
    bytes.iter().filter(|&&b| b == 0xFF).count() / 0x400
}

fn env_args() -> Vec<String> {
    std::env::args().skip(1).collect()
}

fn main() -> anyhow::Result<()> {
    let args = env_args();
    let rom_path = args.iter().find_map(|a| a.strip_prefix("--rom=")).unwrap_or("smw.smc");

    // Verification mode: expand a scratch copy in place and re-open it.
    if let Some(copy) = args.iter().find_map(|a| a.strip_prefix("--expand-copy=")) {
        let target: usize = args
            .iter()
            .find_map(|a| a.strip_prefix("--target="))
            .map(|s| usize::from_str_radix(s.trim_start_matches("0x"), 16))
            .transpose()?
            .unwrap_or(0x40_0000);
        let file_bytes = std::fs::read(copy)?;
        let (smc, body) = smwe_rom::rom_expansion::split_smc_header(&file_bytes);
        let rom = smwe_rom::snes_utils::rom::Rom::new(body.to_vec())?;
        let expanded = expand_rom(&rom, target)?;
        let mut out = Vec::with_capacity(target + smc.map(|h| h.len()).unwrap_or(0));
        if let Some(h) = smc {
            out.extend_from_slice(h);
        }
        out.extend_from_slice(expanded.bytes());
        // Same .bak discipline as the UI.
        let bak = format!("{copy}.bak");
        std::fs::copy(copy, &bak)?;
        std::fs::write(copy, &out)?;
        let reopened = smwe_rom::SmwRom::from_file(copy)?;
        assert_eq!(reopened.rom.bytes().len(), target);
        assert_eq!(reopened.internal_header.rom_size_in_kb(), (target / 0x400) as u32);
        println!(
            "expanded {} -> {} and re-opened OK ({} levels parsed)",
            copy,
            format_size(target),
            reopened.levels.len()
        );
        return Ok(());
    }

    let output = args.iter().find_map(|a| a.strip_prefix("--out=")).unwrap_or("docs/screenshots/rom-expand.png");

    // ---- Real data path, identical to the UI ----
    let rom = smwe_rom::SmwRom::from_file(rom_path)?;
    let current = rom.rom.bytes().len();
    let current_size_byte = rom.rom.bytes()[0x7FD7];
    let map_mode = rom.internal_header.map_mode.to_string();
    let targets = expansion_targets(current);
    assert!(!targets.is_empty(), "ROM already at max size; nothing to show");
    let selected = *targets.last().unwrap(); // dialog defaults to the largest target
    let expanded = expand_rom(&rom.rom, selected)?;
    let new_size_byte = expanded.bytes()[0x7FD7];
    // Self-consistency check on the real ROM: recompute == stored.
    assert_eq!(compute_checksum(expanded.bytes()), {
        let b = expanded.bytes();
        u16::from_le_bytes([b[0x7FDE], b[0x7FDF]])
    });

    let before_kb = current / 0x400;
    let before_free = free_kb(rom.rom.bytes());
    let after_kb = selected / 0x400;
    let after_free = free_kb(expanded.bytes());

    let sans = load_font(SANS_CANDIDATES)?;
    let sans_bold = load_font(SANS_BOLD_CANDIDATES)?;
    let mono = load_font(MONO_CANDIDATES)?;

    // ---- Compose the mock dialog ----
    let (w, h) = (720u32, 600u32);
    let mut img = RgbImage::new(w, h);
    let bg = Rgb([0x1B, 0x1D, 0x20]);
    let panel = Rgb([0x25, 0x28, 0x2C]);
    let titlebar = Rgb([0x12, 0x14, 0x16]);
    let ink = Rgb([0xE8, 0xE8, 0xE8]);
    let dim = Rgb([0xA8, 0xA8, 0xA8]);
    let accent = Rgb([0x4D, 0x9F, 0xFF]);
    let green = Rgb([0x3D, 0xB0, 0x4E]);
    let track = Rgb([0x3A, 0x3D, 0x42]);
    for p in img.pixels_mut() {
        *p = bg;
    }

    let (dx, dy, dw) = (40u32, 30u32, 640u32);
    let dh = 540u32;
    fill_rect(&mut img, dx, dy, dw, dh, panel);
    rect_border(&mut img, dx, dy, dw, dh, Rgb([0x4A, 0x4E, 0x54]));
    // Title bar.
    fill_rect(&mut img, dx, dy, dw, 40, titlebar);
    draw_text(&mut img, &sans_bold, "Expand ROM", (dx + 16) as i32, (dy + 11) as i32, 17.0, ink);
    draw_text(
        &mut img,
        &sans,
        "headless mock \u{2014} sizes, header bytes and free space are real ROM output",
        (dx + 150) as i32,
        (dy + 14) as i32,
        12.0,
        dim,
    );

    let mut y = dy + 62u32;
    draw_text(
        &mut img,
        &sans,
        &format!("Current size: {} ({})", format_size(current), map_mode),
        (dx + 20) as i32,
        y as i32,
        15.0,
        ink,
    );
    y += 36;
    draw_text(&mut img, &sans_bold, "Expand to:", (dx + 20) as i32, y as i32, 15.0, ink);
    y += 30;
    for t in &targets {
        let sel = *t == selected;
        let cx = dx + 34;
        let cy = y + 8;
        // Radio circle.
        imageproc_draw_circle(&mut img, cx, cy, 8, if sel { accent } else { dim });
        if sel {
            fill_circle(&mut img, cx, cy, 4, accent);
        }
        draw_text(
            &mut img,
            &sans,
            &format!("{} ({} Mbit)", format_size(*t), t / 0x2_0000),
            (dx + 52) as i32,
            y as i32,
            15.0,
            ink,
        );
        y += 30;
    }
    y += 8;

    // Free-space bars (real byte scans).
    for (label, total_kb, free) in [("Before", before_kb, before_free), ("After", after_kb, after_free)] {
        draw_text(
            &mut img,
            &sans,
            &format!("{label}: {free} KB free of {total_kb} KB"),
            (dx + 20) as i32,
            y as i32,
            13.0,
            dim,
        );
        y += 24;
        let bw = 600u32;
        fill_rect(&mut img, dx + 20, y, bw, 16, track);
        let fw = (bw as usize * free / total_kb.max(1)) as u32;
        fill_rect(&mut img, dx + 20, y, fw, 16, green);
        rect_border(&mut img, dx + 20, y, bw, 16, Rgb([0x55, 0x59, 0x60]));
        y += 30;
    }

    draw_text(
        &mut img,
        &mono,
        &format!("header: ROM size byte 0x{current_size_byte:02X} \u{2192} 0x{new_size_byte:02X} \u{00B7} checksum recomputed \u{00B7} map mode kept"),
        (dx + 20) as i32,
        y as i32,
        12.5,
        dim,
    );
    y += 30;
    draw_text(
        &mut img,
        &sans,
        ".bak backup kept next to the ROM \u{00B7} unsaved edits are saved first",
        (dx + 20) as i32,
        y as i32,
        12.5,
        dim,
    );
    y += 34;

    // Buttons.
    fill_rect(&mut img, dx + 20, y, 110, 34, Rgb([0x2F, 0x6F, 0xBD]));
    draw_text(&mut img, &sans_bold, "Expand", (dx + 48) as i32, (y + 8) as i32, 14.0, Rgb([0xFF, 0xFF, 0xFF]));
    rect_border(&mut img, dx + 142, y, 110, 34, Rgb([0x6A, 0x6E, 0x74]));
    draw_text(&mut img, &sans, "Cancel", (dx + 172) as i32, (y + 8) as i32, 14.0, ink);

    img.save(output)?;
    println!("wrote {output} ({before_free} KB free -> {after_free} KB free)");
    Ok(())
}

// Small circle helpers (imageproc isn't a dependency; keep it local).
fn imageproc_draw_circle(img: &mut RgbImage, cx: u32, cy: u32, r: u32, color: Rgb<u8>) {
    let r2 = (r * r) as i32;
    for oy in -(r as i32)..=r as i32 {
        for ox in -(r as i32)..=r as i32 {
            let d2 = ox * ox + oy * oy;
            if (d2 - r2).abs() <= r as i32 {
                let (x, y) = (cx as i32 + ox, cy as i32 + oy);
                if x >= 0 && y >= 0 && (x as u32) < img.width() && (y as u32) < img.height() {
                    img.put_pixel(x as u32, y as u32, color);
                }
            }
        }
    }
}

fn fill_circle(img: &mut RgbImage, cx: u32, cy: u32, r: u32, color: Rgb<u8>) {
    let r2 = (r * r) as i32;
    for oy in -(r as i32)..=r as i32 {
        for ox in -(r as i32)..=r as i32 {
            if ox * ox + oy * oy <= r2 {
                let (x, y) = (cx as i32 + ox, cy as i32 + oy);
                if x >= 0 && y >= 0 && (x as u32) < img.width() && (y as u32) < img.height() {
                    img.put_pixel(x as u32, y as u32, color);
                }
            }
        }
    }
}
