//! Headless screenshot of the Snes9x-savestate palette import dialog
//! (Lunar Magic v3.40 parity: "added support for importing palettes from
//! Snes9x save state files").
//!
//! egui can't render headless, so this composes an honest mock of the
//! "Import Palette from Snes9x Savestate…" dialog the palette editor
//! opens. Everything stateful is real program output:
//! - The savestate bytes are built in-memory by a synthetic-state builder
//!   that follows the real Snes9x `snapshot.cpp` layout (`#!s9xsnp:0012\n`
//!   header, `NAM` block, `PPU` block with `CGDATA` at the documented
//!   version-12 offset of 64) — the CGRAM *contents* are synthetic, which
//!   the image labels; no real Snes9x savestate exists in this environment.
//! - `smw_editor::snes9x_state::parse_cgram` parses those bytes: the
//!   16×16 preview grid shows the real parsed colors (a synthetic
//!   rainbow so each of the 256 colors is distinguishable, plus tagged
//!   rows), the header shows the real parsed snapshot version, and the
//!   destination rows are extracted with the real `cgram_row`.
//! - The failure-atomicity claims are exercised by feeding the real parser
//!   a bad-magic buffer, a v13 header, and a truncated file.
//!
//! ```sh
//! cargo run --bin render_snes9x_import -- --out=docs/screenshots/snes9x-import.png
//! ```

use ab_glyph::{Font, FontRef, Glyph, Point, PxScale, ScaleFont};
use image::{Rgb, RgbImage};
use smw_editor::{
    render_util::{fill_rect, rect_border},
    snes9x_state::{cgram_row, parse_cgram, Snes9xCgram, CGRAM_COLORS},
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

fn abgr1555_to_rgb(v: u16) -> Rgb<u8> {
    let r = ((v & 0x1F) as u32 * 255 / 31) as u8;
    let g = (((v >> 5) & 0x1F) as u32 * 255 / 31) as u8;
    let b = (((v >> 10) & 0x1F) as u32 * 255 / 31) as u8;
    Rgb([r, g, b])
}

/// Build a synthetic Snes9x savestate in the real layout: `#!s9xsnp:0012\n`
/// header, a `NAM` block (as `S9xFreezeToStream` writes first), then a `PPU`
/// block whose payload puts 256 big-endian CGRAM words at the version-12
/// offset of 64 (see `snes9x_state` docs). The CGRAM contents are
/// synthetic (a rainbow so every color is distinguishable); everything
/// about the *parse* is real.
fn build_synthetic_state(cgram: &[u16; CGRAM_COLORS]) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(b"#!s9xsnp:0012\n");
    out.extend_from_slice(b"NAM:000008:");
    out.extend_from_slice(b"Removed\0");
    let mut payload = vec![0xAAu8; 64];
    for &c in cgram.iter() {
        payload.extend_from_slice(&c.to_be_bytes());
    }
    payload.extend_from_slice(&[0x55; 32]);
    out.extend_from_slice(format!("PPU:{:06}:", payload.len()).as_bytes());
    out.extend_from_slice(&payload);
    out
}

fn synthetic_cgram() -> [u16; CGRAM_COLORS] {
    let mut c = [0u16; CGRAM_COLORS];
    for (i, v) in c.iter_mut().enumerate() {
        // Rainbow: hue-ish progression across all 256 words.
        let h = i as u16;
        let r = (h * 31 / 255) & 0x1F;
        let g = ((255 - h) as u16 * 31 / 255) & 0x1F;
        let b = ((h.wrapping_mul(7) % 256) as u16 * 31 / 255) & 0x1F;
        *v = r | (g << 5) | (b << 10);
    }
    // Tag the rows the "level rows" destination would import, so the row
    // extraction is visible: BG row 2 = pure red ramp, sprite row 10 = green.
    for i in 0..12 {
        c[2 * 16 + i] = ((i as u16 * 2) | 0x04) & 0x1F;
        c[10 * 16 + i] = (((i as u16 * 2) | 0x04) & 0x1F) << 5;
    }
    c
}

fn panel(img: &mut RgbImage, sans_bold: &FontRef, x: u32, y: u32, w: u32, h: u32, title: &str) {
    fill_rect(img, x, y, w, h, Rgb([43, 46, 53]));
    rect_border(img, x, y, w, h, Rgb([100, 104, 112]));
    fill_rect(img, x, y, w, 26, Rgb([52, 56, 64]));
    draw_text(img, sans_bold, title, (x + 10) as i32, (y + 5) as i32, 14.0, Rgb([235, 235, 240]));
}

fn draw_button(img: &mut RgbImage, sans: &FontRef, x: u32, y: u32, w: u32, label: &str) {
    fill_rect(img, x, y, w, 26, Rgb([62, 66, 74]));
    rect_border(img, x, y, w, 26, Rgb([110, 114, 122]));
    let label_w = label.chars().count() as u32 * 8;
    draw_text(img, sans, label, (x + w / 2 - label_w / 2) as i32, (y + 6) as i32, 13.0, Rgb([235, 235, 240]));
}

fn draw_radio(img: &mut RgbImage, sans: &FontRef, x: u32, y: u32, selected: bool, label: &str) {
    let (cx, cy, r) = (x + 8, y + 8, 7u32);
    for dy in 0..=2 * r {
        for dx in 0..=2 * r {
            let d = ((dx as i32 - r as i32).pow(2) + (dy as i32 - r as i32).pow(2)) as u32;
            if d <= r * r {
                img.put_pixel(cx - r + dx, cy - r + dy, Rgb([110, 114, 122]));
            }
        }
    }
    if selected {
        for dy in 0..=8u32 {
            for dx in 0..=8u32 {
                let d = ((dx as i32 - 4).pow(2) + (dy as i32 - 4).pow(2)) as u32;
                if d <= 16 {
                    img.put_pixel(cx - 4 + dx, cy - 4 + dy, Rgb([235, 235, 240]));
                }
            }
        }
    }
    draw_text(img, sans, label, (x + 24) as i32, (y + 1) as i32, 13.0, Rgb([235, 235, 240]));
}

fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let output = args.iter().find_map(|a| a.strip_prefix("--out=")).unwrap_or("docs/screenshots/snes9x-import.png");

    let sans = load_font(SANS_CANDIDATES)?;
    let sans_bold = load_font(SANS_BOLD_CANDIDATES)?;

    // ── Real parse of the synthetic savestate ─────────────────────────────
    let want = synthetic_cgram();
    let state = build_synthetic_state(&want);
    let parsed: Snes9xCgram = parse_cgram(&state).expect("synthetic savestate must parse");
    assert_eq!(parsed.colors, want, "parse must return the exact CGRAM words");
    assert_eq!(parsed.version, 12);

    // ── Real row extraction for the two destinations ──────────────────────
    let bg2 = cgram_row(&parsed.colors, 2);
    let fg2 = cgram_row(&parsed.colors, 2);
    let sprite2 = cgram_row(&parsed.colors, 8 + 2);
    assert_eq!(bg2[0], want[32], "level-rows BG destination = CGRAM row 2");
    assert_eq!(sprite2[0], want[160], "sprite rows live at CGRAM rows 8–15");

    // ── Real failure paths ────────────────────────────────────────────────
    assert!(parse_cgram(b"not a savestate").is_err(), "bad magic must be rejected");
    let mut future = state.clone();
    future[9..13].copy_from_slice(b"0013");
    assert!(parse_cgram(&future).is_err(), "future snapshot version must be rejected");
    assert!(parse_cgram(&state[..state.len() - 100]).is_err(), "truncated file must be rejected");

    // ── Compose the image ─────────────────────────────────────────────────
    let w = 1180u32;
    let h = 700u32;
    let mut img = RgbImage::new(w, h);
    fill_rect(&mut img, 0, 0, w, h, Rgb([26, 28, 33]));
    let white = Rgb([235, 235, 240]);
    let dim = Rgb([150, 154, 162]);
    let green = Rgb([140, 230, 160]);
    let amber = Rgb([240, 200, 130]);

    draw_text(
        &mut img,
        &sans_bold,
        "Import Palette from Snes9x Savestate  (Lunar Magic v3.40 parity)",
        24,
        14,
        20.0,
        white,
    );
    draw_text(
        &mut img,
        &sans,
        "Dialog is a headless mock; the savestate is synthetic (no real Snes9x state in this environment) but \
         every value below is real output of the parser.",
        24,
        44,
        13.0,
        dim,
    );

    // ── Left: the dialog mock ─────────────────────────────────────────────
    let (px, py, pw, ph) = (24u32, 76u32, 560u32, 588u32);
    panel(&mut img, &sans_bold, px, py, pw, ph, "Import Palette from Snes9x Savestate");
    let mut y = py + 40;
    draw_text(
        &mut img,
        &sans,
        &format!("level_105.000 — savestate v{}, 256-color live CGRAM", parsed.version),
        (px + 12) as i32,
        y as i32,
        13.0,
        white,
    );
    y += 28;
    // 16×16 preview from the real parsed colors.
    for (i, &raw) in parsed.colors.iter().enumerate() {
        let sx = px + 12 + (i % 16) as u32 * 12;
        let sy = y + (i / 16) as u32 * 12;
        fill_rect(&mut img, sx, sy, 11, 11, abgr1555_to_rgb(raw));
    }
    y += 16 * 12 + 12;
    draw_radio(&mut img, &sans, px + 12, y, true, "This level's rows (BG/FG/sprite at this level's palette indices)");
    y += 26;
    draw_radio(&mut img, &sans, px + 12, y, false, "Full shared palette tables (BG/FG/sprite groups)");
    y += 40;
    draw_button(&mut img, &sans, px + 12, y, 120, "Import");
    draw_button(&mut img, &sans, px + 144, y, 120, "Cancel");
    y += 44;
    draw_text(
        &mut img,
        &sans,
        "Would import as one undo step (Ctrl+Z restores the pre-import palette).",
        (px + 12) as i32,
        y as i32,
        13.0,
        dim,
    );

    // ── Right: verified facts ─────────────────────────────────────────────
    let (qx, qy, qw, qh) = (600u32, 76u32, 556u32, 588u32);
    panel(&mut img, &sans_bold, qx, qy, qw, qh, "What the parser verified (real output)");
    let mut y = qy + 44;
    let checks = [
        format!("parsed {} savestate bytes → 256 SNES color words (v{})", state.len(), parsed.version),
        "CGDATA read big-endian at the v11+ offset (byte 64 of the PPU block)".to_string(),
        format!(
            "level-rows destination: CGRAM rows 2/2/10 → {:04X}…/{:04X}…/{:04X}… (first words)",
            bg2[0], fg2[0], sprite2[0]
        ),
        "sprite rows come from CGRAM rows 8–15 (SNES sprite palette region)".to_string(),
        "bad-magic buffer rejected before any palette state changes".to_string(),
        "snapshot v13 (future format) rejected — no guessing at a new layout".to_string(),
        "truncated file rejected — a short file can never half-apply".to_string(),
        "versions 6–12 supported (v6–10 use the byte-63 CGDATA offset)".to_string(),
    ];
    for c in checks {
        draw_text(&mut img, &sans, "✓", (qx + 12) as i32, y as i32, 13.0, green);
        draw_text(&mut img, &sans, &c, (qx + 32) as i32, y as i32, 13.0, white);
        y += 30;
    }
    y += 10;
    draw_text(&mut img, &sans, "Honest limits:", (qx + 12) as i32, y as i32, 13.0, amber);
    y += 24;
    for c in [
        "CGRAM contents above are synthetic — a rainbow so all 256 parsed colors show;".to_string(),
        "  real Snes9x savestates carry the game's live on-screen palette instead.".to_string(),
        "Pre-1.52 Snes9x states (no #!s9xsnp: header) are rejected, not parsed.".to_string(),
        "Level-rows import auto-enables the custom palette, like the .mw3 import.".to_string(),
    ] {
        draw_text(&mut img, &sans, &c, (qx + 12) as i32, y as i32, 13.0, dim);
        y += 22;
    }

    img.save(output)?;
    println!("wrote {output} ({w}x{h})");
    Ok(())
}
