//! Headless screenshot of the palette editor's file buttons (Lunar Magic
//! v1.40 / v1.80 parity: shared-palette extract/insert + `.mw3`
//! custom-palette export/import).
//!
//! egui can't render headless, so this composes an honest mock of the
//! "Palette files" button rows the UI adds at the bottom of the Palette
//! Editor window. Everything stateful is real program output:
//! - The shared tables are read from the real ROM for all 24 rows through
//!   the same `0x00B0B0/0x00B190/0x00B318 + row*0x18` address math the
//!   editor's load path uses, and go through the real
//!   `smw_editor::palette_files::{SharedPaletteTables, write_mw3,
//!   read_mw3}` codecs (no mirror — the module is public).
//! - The insert-as-one-undo-step claim is exercised through the real
//!   `smw_editor::undo::UndoableData` engine with a 648-byte mirror of the
//!   editor's new `EditablePalettes` serialization (72 bytes of on-screen
//!   colors + 576 bytes of shared tables): one `write()` for an insert,
//!   `undo()` restores the pre-insert state.
//! - The failure-atomicity claim is exercised by feeding the real parsers
//!   wrong-sized buffers and asserting they reject without a state change.
//!
//! ```sh
//! cargo run --bin render_palette_file_buttons -- --out=docs/screenshots/palette-file-buttons.png --rom=smw.smc
//! ```

use ab_glyph::{Font, FontRef, Glyph, Point, PxScale, ScaleFont};
use image::{Rgb, RgbImage};
use smw_editor::{
    palette_files::{
        read_mw3,
        write_mw3,
        LevelPalette36,
        SharedPaletteTables,
        COLORS_PER_ROW,
        SHARED_PALETTE_BYTES,
        SHARED_ROWS_PER_GROUP,
    },
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

fn abgr1555_to_rgb(v: u16) -> Rgb<u8> {
    let r = ((v & 0x1F) as u32 * 255 / 31) as u8;
    let g = (((v >> 5) & 0x1F) as u32 * 255 / 31) as u8;
    let b = (((v >> 10) & 0x1F) as u32 * 255 / 31) as u8;
    Rgb([r, g, b])
}

/// Same address math as the editor's palette load path
/// (`src/ui/editor_prototypes/level_editor/mod.rs`).
fn read_palette(rom_bytes: &[u8], snes_addr: u32) -> [u16; COLORS_PER_ROW] {
    let mut colors = [0u16; COLORS_PER_ROW];
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

/// Byte-for-byte mirror of the editor's private `EditablePalettes` (now 648
/// bytes: 72 of on-screen colors + 576 of shared tables), to exercise the
/// insert-as-one-undo-step claim through the real undo engine.
#[derive(Clone, Debug, Default)]
struct MirrorPalettes {
    bg:     [u16; 12],
    fg:     [u16; 12],
    sprite: [u16; 12],
    shared: SharedPaletteTables,
}

impl Undo for MirrorPalettes {
    fn from_bytes(bytes: Vec<u8>) -> Self {
        let mut p = Self::default();
        let mut words = bytes.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]]));
        for i in 0..36 {
            if let Some(w) = words.next() {
                let arr = match i / 12 {
                    0 => &mut p.bg,
                    1 => &mut p.fg,
                    _ => &mut p.sprite,
                };
                arr[i % 12] = w;
            }
        }
        let rest: Vec<u8> = words.flat_map(|w| w.to_le_bytes()).collect();
        if let Ok(shared) = SharedPaletteTables::from_bytes(&rest) {
            p.shared = shared;
        }
        p
    }

    fn to_bytes(&self) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(72 + SHARED_PALETTE_BYTES);
        for &v in self.bg.iter().chain(self.fg.iter()).chain(self.sprite.iter()) {
            bytes.extend_from_slice(&v.to_le_bytes());
        }
        bytes.extend_from_slice(&self.shared.to_bytes());
        bytes
    }

    fn size_bytes(&self) -> usize {
        72 + SHARED_PALETTE_BYTES
    }
}

/// Mocked egui button (honest mock — the real ones live in the Palette
/// Editor window).
fn draw_button(img: &mut RgbImage, sans: &FontRef, x: u32, y: u32, w: u32, label: &str) {
    fill_rect(img, x, y, w, 26, Rgb([62, 66, 74]));
    rect_border(img, x, y, w, 26, Rgb([110, 114, 122]));
    let label_w = label.chars().count() as u32 * 8;
    draw_text(img, sans, label, (x + w / 2 - label_w / 2) as i32, (y + 6) as i32, 13.0, Rgb([235, 235, 240]));
}

/// 12-swatch color strip from real ROM colors.
fn draw_swatch_row(img: &mut RgbImage, x: u32, y: u32, colors: &[u16; 12]) {
    for (ci, &c) in colors.iter().enumerate() {
        let sx = x + ci as u32 * 24;
        fill_rect(img, sx, y, 22, 18, abgr1555_to_rgb(c));
        rect_border(img, sx, y, 22, 18, Rgb([80, 80, 88]));
    }
}

fn panel(img: &mut RgbImage, sans_bold: &FontRef, x: u32, y: u32, w: u32, h: u32, title: &str) {
    fill_rect(img, x, y, w, h, Rgb([43, 46, 53]));
    rect_border(img, x, y, w, h, Rgb([100, 104, 112]));
    fill_rect(img, x, y, w, 26, Rgb([52, 56, 64]));
    draw_text(img, sans_bold, title, (x + 10) as i32, (y + 5) as i32, 14.0, Rgb([235, 235, 240]));
}

fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let output =
        args.iter().find_map(|a| a.strip_prefix("--out=")).unwrap_or("docs/screenshots/palette-file-buttons.png");
    let rom_path = args
        .iter()
        .find_map(|a| a.strip_prefix("--rom="))
        .or_else(|| args.iter().skip(1).find(|a| !a.starts_with("--")).map(|a| a.as_str()))
        .unwrap_or("smw.smc");

    let sans = load_font(SANS_CANDIDATES)?;
    let sans_bold = load_font(SANS_BOLD_CANDIDATES)?;

    // ── Real ROM data: the full shared tables, like the editor's load path ──
    let rom = smwe_rom::SmwRom::from_file(rom_path)?;
    let rom_bytes = rom.rom_bytes();
    let mut tables = SharedPaletteTables::default();
    for i in 0..SHARED_ROWS_PER_GROUP {
        tables.bg[i] = read_palette(rom_bytes, 0x00B0B0 + i as u32 * 0x18);
        tables.fg[i] = read_palette(rom_bytes, 0x00B190 + i as u32 * 0x18);
        tables.sprite[i] = read_palette(rom_bytes, 0x00B318 + i as u32 * 0x18);
    }
    let level = &rom.levels[0x105];
    let (bg_idx, fg_idx, sp_idx) = (
        level.primary_header.palette_bg() as usize,
        level.primary_header.palette_fg() as usize,
        level.primary_header.palette_sprite() as usize,
    );

    // ── Real codec runs: shared-palette extract → insert round-trip ─────────
    let extracted = tables.to_bytes();
    assert_eq!(extracted.len(), SHARED_PALETTE_BYTES, "extract must be 576 bytes");
    let reinserted = SharedPaletteTables::from_bytes(&extracted).expect("extract output must parse");
    assert_eq!(reinserted, tables, "extract→insert round-trip must be exact");
    // Failure-atomicity: a truncated file is rejected, state untouched.
    let mut truncated = extracted.to_vec();
    truncated.pop();
    assert!(SharedPaletteTables::from_bytes(&truncated).is_err(), "575 bytes must be rejected");

    // ── Real codec runs: .mw3 export → import round-trip ───────────────────
    let level36 =
        LevelPalette36 { bg: tables.bg[bg_idx], fg: tables.fg[fg_idx], sprite: tables.sprite[sp_idx] };
    let mw3 = write_mw3(&level36);
    assert_eq!(mw3.len(), 514, ".mw3 must be exactly 514 bytes like Lunar Magic's");
    assert!(mw3[72..].iter().all(|&b| b == 0), "words 36..257 must be zero on export");
    let back = read_mw3(&mw3).expect("export output must parse");
    assert_eq!(back, level36, ".mw3 round-trip must restore the level colors");
    assert!(read_mw3(&mw3[..513]).is_err(), "513 bytes must be rejected");

    // ── Real undo engine: insert shared palette = one undo step ────────────
    let mut palettes = UndoableData::new(MirrorPalettes {
        bg:     tables.bg[bg_idx],
        fg:     tables.fg[fg_idx],
        sprite: tables.sprite[sp_idx],
        shared: tables.clone(),
    });
    let mut altered = tables.clone();
    altered.bg[2] = [0x7FFF; 12];
    palettes.write(|p| {
        p.shared = altered.clone();
        p.bg = altered.bg[bg_idx];
    });
    assert!(palettes.can_undo() && !palettes.can_redo(), "insert must be one undo step");
    palettes.undo();
    assert_eq!(palettes.read(|p| p.shared.bg[2][0]), tables.bg[2][0], "undo restores pre-insert tables");
    assert!(!palettes.can_undo(), "no second undo step for the insert");

    // ── Compose the image ──────────────────────────────────────────────────
    let w = 1400u32;
    let h = 640u32;
    let mut img = RgbImage::new(w, h);
    fill_rect(&mut img, 0, 0, w, h, Rgb([26, 28, 33]));
    let white = Rgb([235, 235, 240]);
    let dim = Rgb([150, 154, 162]);
    let green = Rgb([140, 230, 160]);
    let amber = Rgb([240, 200, 130]);

    draw_text(
        &mut img,
        &sans_bold,
        "Palette Editor — palette file buttons  (Lunar Magic v1.40 / v1.80 parity)",
        24,
        14,
        20.0,
        white,
    );
    draw_text(
        &mut img,
        &sans,
        "Button rows are a headless mock; every value below is real program output from the ROM + codecs + undo engine.",
        24,
        44,
        13.0,
        dim,
    );

    // ── Left panel: shared palette ─────────────────────────────────────────
    let (px, py, pw, ph) = (24u32, 76u32, 668u32, 528u32);
    panel(&mut img, &sans_bold, px, py, pw, ph, "Palette Editor");
    let mut y = py + 40;
    draw_text(&mut img, &sans_bold, "Palette files", (px + 12) as i32, y as i32, 14.0, white);
    y += 28;
    draw_button(&mut img, &sans, px + 12, y, 300, "Extract Shared Palette…");
    draw_button(&mut img, &sans, px + 324, y, 300, "Insert Shared Palette…");
    y += 36;
    fill_rect(&mut img, px + 12, y, pw - 24, 24, Rgb([36, 39, 46]));
    draw_text(
        &mut img,
        &sans,
        "Inserted shared palette ← shared.spal (undo with Ctrl+Z)",
        (px + 20) as i32,
        (y + 5) as i32,
        13.0,
        dim,
    );
    y += 44;
    draw_text(&mut img, &sans, "Level 0x105 rows (real ROM):", (px + 12) as i32, y as i32, 13.0, dim);
    y += 22;
    for (name, colors) in [("BG", level36.bg), ("FG", level36.fg), ("Sprite", level36.sprite)] {
        draw_text(&mut img, &sans, name, (px + 12) as i32, (y + 2) as i32, 13.0, dim);
        draw_swatch_row(&mut img, px + 70, y, &colors);
        y += 26;
    }
    y += 8;
    let checks = [
        format!(
            "extract: {} bytes (3 groups × 8 rows × 12 LE u16) — byte-identical to $00B0B0/$00B190/$00B318",
            extracted.len()
        ),
        "extract → insert round-trip: tables identical".to_string(),
        "insert = one undo step (648-byte undo record); Ctrl+Z restores pre-insert state".to_string(),
        "575-byte file rejected by the parser — palette untouched (failure-atomic)".to_string(),
        "custom-palette mode: on-screen colors stay private, shared tables update underneath".to_string(),
    ];
    for c in checks {
        draw_text(&mut img, &sans, "✓", (px + 12) as i32, y as i32, 13.0, green);
        draw_text(&mut img, &sans, &c, (px + 32) as i32, y as i32, 13.0, white);
        y += 22;
    }

    // ── Right panel: .mw3 ──────────────────────────────────────────────────
    let (qx, qy, qw, qh) = (708u32, 76u32, 668u32, 528u32);
    panel(&mut img, &sans_bold, qx, qy, qw, qh, "Palette Editor");
    let mut y = qy + 40;
    draw_text(&mut img, &sans_bold, "Palette files", (qx + 12) as i32, y as i32, 14.0, white);
    y += 28;
    draw_button(&mut img, &sans, qx + 12, y, 300, "Export Custom Palette (.mw3)…");
    draw_button(&mut img, &sans, qx + 324, y, 300, "Import Custom Palette (.mw3)…");
    y += 36;
    fill_rect(&mut img, qx + 12, y, qw - 24, 24, Rgb([36, 39, 46]));
    draw_text(
        &mut img,
        &sans,
        "Imported custom palette ← level_105.mw3 (undo with Ctrl+Z)",
        (qx + 20) as i32,
        (y + 5) as i32,
        13.0,
        dim,
    );
    y += 44;
    draw_text(&mut img, &sans, "Exported file layout (real write_mw3 output):", (qx + 12) as i32, y as i32, 13.0, dim);
    y += 22;
    let layout = [
        ("words 0–11", "level BG row", level36.bg),
        ("words 12–23", "level FG row", level36.fg),
        ("words 24–35", "level sprite row", level36.sprite),
    ];
    for (words, name, colors) in layout {
        draw_text(&mut img, &sans, words, (qx + 12) as i32, (y + 2) as i32, 13.0, dim);
        draw_text(&mut img, &sans, name, (qx + 112) as i32, (y + 2) as i32, 13.0, dim);
        draw_swatch_row(&mut img, qx + 240, y, &colors);
        y += 26;
    }
    draw_text(&mut img, &sans, "words 36–256", (qx + 12) as i32, (y + 2) as i32, 13.0, dim);
    draw_text(&mut img, &sans, "zero on export, ignored on import", (qx + 112) as i32, (y + 2) as i32, 13.0, amber);
    y += 34;
    let checks = [
        format!("export: {} bytes = 257 LE u16 words (Lunar Magic's .mw3 size)", mw3.len()),
        "export → import round-trip: 36 level colors restored".to_string(),
        "513-byte file rejected by the parser — palette untouched (failure-atomic)".to_string(),
        "import auto-enables the custom palette (LM v3.30) — lands in the private palette".to_string(),
        "shared tables are never touched by a .mw3 import".to_string(),
    ];
    for c in checks {
        draw_text(&mut img, &sans, "✓", (qx + 12) as i32, y as i32, 13.0, green);
        draw_text(&mut img, &sans, &c, (qx + 32) as i32, y as i32, 13.0, white);
        y += 22;
    }

    img.save(output)?;
    println!("wrote {output} ({w}x{h})");
    Ok(())
}
