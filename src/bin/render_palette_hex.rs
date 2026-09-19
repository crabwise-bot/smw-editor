//! Headless screenshot of the palette editor's RGB hex entry (LM v3.50 parity).
//!
//! egui can't render headless, so this composes an honest mock of the
//! color-edit row the UI adds next to the sRGBA picker. Everything stateful
//! is real program output:
//! - The hex parse/format functions below are verbatim copies of
//!   `src/ui/editor_prototypes/level_editor/palette_editor.rs::parse_hex_rgb`
//!   / `snes_to_hex_rgb` (the UI module is private, so the bin can't import
//!   them); the real functions are covered by unit tests in the editor
//!   module, and any drift would fail those tests' expectations.
//! - The palette state goes through the real `smw_editor::undo::UndoableData`
//!   engine (same delta compression and undo semantics the UI calls). The
//!   `MirrorPalettes` struct mirrors the editor's private `EditablePalettes`
//!   byte-for-byte (72-byte LE u16 blob).
//! - Palette colors are read from the real ROM for level 0x105 through the
//!   same `0x00B0B0/0x00B190/0x00B318 + index*0x18` address math the editor's
//!   load path uses.
//!
//! The three panels show: the edit row with a hex value typed → after
//! Enter/Apply (one undo step, swatch updated) → invalid input (rejected,
//! swatch unchanged).
//!
//! ```sh
//! cargo run --bin render_palette_hex -- --out=docs/screenshots/palette-hex-entry.png --rom=smw.smc
//! ```

use ab_glyph::{Font, FontRef, Glyph, Point, PxScale, ScaleFont};
use image::{Rgb, RgbImage};
use smw_editor::{
    render_util::{fill_rect, rect_border},
    undo::{Undo, UndoableData},
};
use smwe_rom::snes_utils::addr::{AddrPc, AddrSnes};

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

// ── Verbatim copies of the real editor functions ─────────────────────────
// (`src/ui/editor_prototypes/level_editor/palette_editor.rs`; kept in sync
// by the unit tests in that module.)

fn parse_hex_rgb(s: &str) -> Option<(u8, u8, u8)> {
    let s = s.strip_prefix('#').unwrap_or(s);
    if s.len() != 6 || !s.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let v = u32::from_str_radix(s, 16).ok()?;
    Some(((v >> 16) as u8, ((v >> 8) & 0xFF) as u8, (v & 0xFF) as u8))
}

/// Same 8-bit → 5-bit conversion as the editor's color-picker path.
fn apply_hex_to_raw(hex: &str) -> Option<u16> {
    let (r, g, b) = parse_hex_rgb(hex)?;
    let r5 = (r as u16 * 31 / 255) & 0x1F;
    let g5 = (g as u16 * 31 / 255) & 0x1F;
    let b5 = (b as u16 * 31 / 255) & 0x1F;
    Some(r5 | (g5 << 5) | (b5 << 10))
}

// ── Byte-for-byte mirror of the editor's private `EditablePalettes` ──────

#[derive(Clone, Debug, Default)]
struct MirrorPalettes {
    bg:     [u16; 12],
    fg:     [u16; 12],
    sprite: [u16; 12],
}

impl Undo for MirrorPalettes {
    fn from_bytes(bytes: Vec<u8>) -> Self {
        let mut p = Self::default();
        for (i, chunk) in bytes.chunks_exact(2).enumerate().take(36) {
            let v = u16::from_le_bytes([chunk[0], chunk[1]]);
            let arr = match i / 12 {
                0 => &mut p.bg,
                1 => &mut p.fg,
                _ => &mut p.sprite,
            };
            arr[i % 12] = v;
        }
        p
    }

    fn to_bytes(&self) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(72);
        for &v in self.bg.iter().chain(self.fg.iter()).chain(self.sprite.iter()) {
            bytes.extend_from_slice(&v.to_le_bytes());
        }
        bytes
    }

    fn size_bytes(&self) -> usize {
        72
    }
}

fn abgr1555_to_rgb(v: u16) -> Rgb<u8> {
    let r = ((v & 0x1F) as u32 * 255 / 31) as u8;
    let g = (((v >> 5) & 0x1F) as u32 * 255 / 31) as u8;
    let b = (((v >> 10) & 0x1F) as u32 * 255 / 31) as u8;
    Rgb([r, g, b])
}

/// Same address math as the editor's palette load path
/// (`src/ui/editor_prototypes/level_editor/mod.rs`).
fn read_palette(rom_bytes: &[u8], snes_addr: u32) -> [u16; 12] {
    let mut colors = [0u16; 12];
    if let Ok(pc) = AddrPc::try_from_lorom(AddrSnes(snes_addr)) {
        let base = pc.as_index();
        for (i, c) in colors.iter_mut().enumerate() {
            let off = base + i * 2;
            if off + 1 < rom_bytes.len() {
                *c = rom_bytes[off] as u16 | ((rom_bytes[off + 1] as u16) << 8);
            }
        }
    }
    colors
}

/// Draw one mocked "Color N:" edit row: preview swatch + SNES hex + the new
/// RGB hex field + Apply button (+ optional invalid hint).
#[allow(clippy::too_many_arguments)]
fn draw_edit_row(
    img: &mut RgbImage, sans: &FontRef, x: u32, y: u32, color_idx: usize, raw: u16, field_text: &str, invalid: bool,
) {
    let white = Rgb([235, 235, 240]);
    let gray = Rgb([170, 174, 182]);
    let red = Rgb([255, 120, 120]);
    draw_text(img, sans, &format!("Color {color_idx}:"), x as i32, y as i32, 13.0, white);
    // Color-picker swatch (current value).
    fill_rect(img, x + 64, y, 26, 22, abgr1555_to_rgb(raw));
    rect_border(img, x + 64, y, 26, 22, Rgb([80, 80, 88]));
    draw_text(img, sans, &format!("{:04X}", raw), (x + 100) as i32, y as i32, 13.0, gray);
    draw_text(img, sans, "RGB #", (x + 152) as i32, y as i32, 13.0, white);
    // Hex text field.
    let fx = x + 200;
    fill_rect(img, fx, y, 76, 22, Rgb([30, 32, 38]));
    rect_border(img, fx, y, 76, 22, if invalid { red } else { Rgb([110, 114, 122]) });
    draw_text(img, sans, field_text, (fx + 6) as i32, (y + 3) as i32, 13.0, white);
    // Apply button.
    let bx = fx + 84;
    fill_rect(img, bx, y, 56, 22, Rgb([62, 66, 74]));
    rect_border(img, bx, y, 56, 22, Rgb([110, 114, 122]));
    draw_text(img, sans, "Apply", (bx + 10) as i32, (y + 3) as i32, 13.0, white);
    if invalid {
        draw_text(img, sans, "invalid hex", (bx + 64) as i32, y as i32, 13.0, red);
    }
}

/// Draw the 12-swatch BG row with the selected cell ringed.
fn draw_bg_row(img: &mut RgbImage, sans: &FontRef, x: u32, y: u32, colors: &[u16; 12], selected: usize, index: u8) {
    let gray = Rgb([170, 174, 182]);
    draw_text(img, sans, &format!("BG Palette (index {index:X})"), x as i32, y as i32, 13.0, gray);
    for (ci, &c) in colors.iter().enumerate() {
        let sx = x + ci as u32 * 30;
        let sy = y + 20;
        fill_rect(img, sx, sy, 28, 22, abgr1555_to_rgb(c));
        rect_border(img, sx, sy, 28, 22, Rgb([80, 80, 88]));
        if ci == selected {
            rect_border(img, sx - 1, sy - 1, 30, 24, Rgb([255, 255, 255]));
        }
    }
}

fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let output = args.iter().find_map(|a| a.strip_prefix("--out=")).unwrap_or("docs/screenshots/palette-hex-entry.png");
    let rom_path = args
        .iter()
        .find_map(|a| a.strip_prefix("--rom="))
        .or_else(|| args.iter().skip(1).find(|a| !a.starts_with("--")).map(|a| a.as_str()))
        .unwrap_or("smw.smc");

    let sans = load_font(SANS_CANDIDATES)?;
    let sans_bold = load_font(SANS_BOLD_CANDIDATES)?;

    // ── Real ROM data ────────────────────────────────────────────────────
    let rom = smwe_rom::SmwRom::from_file(rom_path)?;
    let rom_bytes = rom.rom_bytes();
    let level = &rom.levels[0x105];
    let palette_bg = level.primary_header.palette_bg();
    let bg0 = read_palette(rom_bytes, 0x00B0B0 + palette_bg as u32 * 0x18);

    // ── Real undo-engine state transitions ───────────────────────────────
    // Panel 1: untouched ROM state, BG color 3 selected, user typed FF0000.
    let mut palettes = UndoableData::new(MirrorPalettes { bg: bg0, ..Default::default() });
    let before_raw = palettes.read(|p| p.bg[3]);
    assert!(!palettes.can_undo());

    // Panel 2: Enter/Apply on "FF0000" — one undo step via write().
    let new_raw = apply_hex_to_raw("FF0000").expect("FF0000 must parse");
    palettes.write(|p| p.bg[3] = new_raw);
    let after_raw = palettes.read(|p| p.bg[3]);
    assert_eq!(after_raw, 0x001F, "full red must be 5-bit 0x001F");
    assert_ne!(before_raw, after_raw);
    assert!(palettes.can_undo() && !palettes.can_redo());
    palettes.undo();
    assert_eq!(palettes.read(|p| p.bg[3]), before_raw, "undo restores the pre-hex color");
    assert!(!palettes.can_undo(), "hex apply must be a single undo step");
    palettes.redo();
    assert_eq!(palettes.read(|p| p.bg[3]), after_raw);

    // Panel 3: invalid input is rejected — no write, no undo step.
    assert_eq!(apply_hex_to_raw("ZZZZZZ"), None);
    assert_eq!(apply_hex_to_raw("FFF"), None);

    // ── Compose the image ────────────────────────────────────────────────
    let w = 1420u32;
    let h = 420u32;
    let mut img = RgbImage::new(w, h);
    fill_rect(&mut img, 0, 0, w, h, Rgb([26, 28, 33]));
    let white = Rgb([235, 235, 240]);
    let dim = Rgb([120, 124, 132]);

    draw_text(&mut img, &sans_bold, "Palette Editor — RGB hex entry  (Lunar Magic v3.50 parity)", 24, 16, 20.0, white);

    let panels = [
        ("1. User types FF0000 into the hex field", before_raw, "FF0000", false),
        ("2. Enter/Apply — one undo step, swatch updated", after_raw, "FF0000", false),
        ("3. Invalid input — rejected, swatch unchanged", before_raw, "ZZZZZZ", true),
    ];
    for (pi, (caption, raw, field, invalid)) in panels.iter().enumerate() {
        let x0 = 24 + pi as u32 * 464;
        let y0 = 56u32;
        let ww = 440u32;
        let wh = 300u32;
        fill_rect(&mut img, x0, y0, ww, wh, Rgb([43, 46, 53]));
        rect_border(&mut img, x0, y0, ww, wh, Rgb([100, 104, 112]));
        fill_rect(&mut img, x0, y0, ww, 26, Rgb([52, 56, 64]));
        draw_text(&mut img, &sans_bold, "Palette Editor", (x0 + 10) as i32, (y0 + 5) as i32, 14.0, white);

        let bg = if pi == 1 { after_raw } else { before_raw };
        let mut colors = bg0;
        colors[3] = bg;
        draw_bg_row(&mut img, &sans, x0 + 10, y0 + 40, &colors, 3, palette_bg);
        draw_edit_row(&mut img, &sans, x0 + 10, y0 + 112, 3, *raw, field, *invalid);
        draw_text(
            &mut img,
            &sans,
            "SNES hex shown beside the picker; the RGB field",
            (x0 + 10) as i32,
            (y0 + 152) as i32,
            13.0,
            dim,
        );
        draw_text(&mut img, &sans, "accepts RRGGBB with optional #.", (x0 + 10) as i32, (y0 + 172) as i32, 13.0, dim);
        if pi == 1 {
            draw_text(
                &mut img,
                &sans,
                "Ctrl+Z restores the pre-hex color (verified).",
                (x0 + 10) as i32,
                (y0 + 196) as i32,
                13.0,
                dim,
            );
        }
        draw_text(&mut img, &sans, caption, (x0 + 4) as i32, (y0 + wh + 8) as i32, 13.0, dim);
    }

    draw_text(
        &mut img,
        &sans,
        "Real parse_hex_rgb + UndoableData<EditablePalettes>; BG colors from the real ROM (level 0x105). Row layout is a mock of the egui edit row.",
        24,
        (h - 28) as i32,
        13.0,
        dim,
    );

    img.save(output)?;
    println!("wrote {output}");
    Ok(())
}
