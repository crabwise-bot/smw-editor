//! Headless screenshot of the palette editor's per-level custom palette
//! (Lunar Magic v3.30 parity: "Auto-Enable custom palette on edit").
//!
//! egui can't render headless, so this composes an honest mock of the
//! custom-palette rows the UI adds at the top of the palette editor window.
//! Everything stateful is real program output:
//! - The palette state goes through the real `smw_editor::undo::UndoableData`
//!   engine (same one-undo-step semantics the UI calls). `MirrorPalettes`
//!   mirrors the editor's private `EditablePalettes` byte-for-byte.
//! - The custom-palette entry goes through the real
//!   `smwe_rom::level::custom_palette::CustomPaletteData` RATS codec: the
//!   scratch ROM copy is written with the same merge-on-save logic as
//!   `UiLevelEditor::save_to_rom`, then re-parsed. The script asserts the
//!   shared palette tables are byte-identical before/after and the block
//!   round-trips with exactly one entry for level 0x105.
//! - Palette colors are read from the real ROM for level 0x105 through the
//!   same `0x00B0B0 + index*0x18` address math the editor's load path uses.
//!
//! The three panels show: shared-table mode before the edit → the first
//! edit auto-enabling the custom palette → the saved RATS block with the
//! shared tables untouched.
//!
//! ```sh
//! cargo run --bin render_palette_custom -- --out=docs/screenshots/palette-custom.png --rom=smw.smc
//! ```

use ab_glyph::{Font, FontRef, Glyph, Point, PxScale, ScaleFont};
use image::{Rgb, RgbImage};
use smw_editor::{
    render_util::{fill_rect, rect_border},
    undo::{Undo, UndoableData},
};
use smwe_rom::{
    level::custom_palette::{CustomPalette, CustomPaletteData},
    snes_utils::addr::{AddrPc, AddrSnes},
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

fn shared_table_range(snes_addr: u32, header_offset: usize) -> Option<(usize, usize)> {
    let pc = AddrPc::try_from_lorom(AddrSnes(snes_addr)).ok()?.as_index() + header_offset;
    Some((pc, pc + 24))
}

/// Draw a mocked checkbox row.
fn draw_checkbox(img: &mut RgbImage, sans: &FontRef, x: u32, y: u32, checked: bool, label: &str) {
    let white = Rgb([235, 235, 240]);
    fill_rect(img, x, y, 16, 16, Rgb([30, 32, 38]));
    rect_border(img, x, y, 16, 16, Rgb([110, 114, 122]));
    if checked {
        draw_text(img, sans, "✓", (x + 2) as i32, (y - 2) as i32, 15.0, white);
    }
    draw_text(img, sans, label, (x + 24) as i32, y as i32, 13.0, white);
}

/// Draw the 12-swatch BG row with the selected cell ringed.
fn draw_bg_row(
    img: &mut RgbImage, sans: &FontRef, x: u32, y: u32, colors: &[u16; 12], selected: usize, index: u8, custom: bool,
) {
    let gray = Rgb([170, 174, 182]);
    let source = if custom { "custom".to_string() } else { format!("index {index:X}") };
    draw_text(img, sans, &format!("BG Palette ({source})"), x as i32, y as i32, 13.0, gray);
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
    let output = args.iter().find_map(|a| a.strip_prefix("--out=")).unwrap_or("docs/screenshots/palette-custom.png");
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
    let palette_fg = level.primary_header.palette_fg();
    let palette_sprite = level.primary_header.palette_sprite();
    let bg0 = read_palette(rom_bytes, 0x00B0B0 + palette_bg as u32 * 0x18);
    let fg0 = read_palette(rom_bytes, 0x00B190 + palette_fg as u32 * 0x18);
    let sp0 = read_palette(rom_bytes, 0x00B318 + palette_sprite as u32 * 0x18);

    // The pristine ROM has no custom-palette block.
    assert!(CustomPaletteData::parse(rom_bytes).is_err(), "pristine ROM must have no SMWECPLT block");

    // ── Simulated editor session ─────────────────────────────────────────
    // Panel 1: shared-table mode, nothing edited yet.
    let mut palettes = UndoableData::new(MirrorPalettes { bg: bg0, fg: fg0, sprite: sp0 });
    let mut custom = CustomPaletteData::default();
    let mut enabled = false;
    let auto_enable = true;

    // Panel 2: the first edit (hex FF0000 on BG color 3, one undo step)
    // auto-enables the custom palette, seeded from the live colors.
    let new_raw: u16 = 0x001F; // full red in ABGR1555
    palettes.write(|p| p.bg[3] = new_raw);
    if auto_enable && !enabled {
        enabled = true;
        let (bg, fg, sprite) = palettes.read(|p| (p.bg, p.fg, p.sprite));
        custom.set(0x105, CustomPalette { bg, fg, sprite });
    }
    assert!(enabled, "auto-enable must have fired on the first edit");
    assert_eq!(custom.get(0x105).unwrap().bg[3], 0x001F);
    assert_eq!(custom.get(0x105).unwrap().bg[0], bg0[0], "entry seeded from shared colors");

    // Panel 3: save to a scratch ROM copy with the same merge-on-save
    // logic as `UiLevelEditor::save_to_rom`; the shared tables must be
    // byte-identical afterwards.
    let raw = std::fs::read(rom_path)?;
    let header_offset = if raw.len() % 0x400 == 0x200 { 0x200 } else { 0 };
    let mut scratch = raw.clone();
    let shared_ranges = [
        shared_table_range(0x00B0B0 + palette_bg as u32 * 0x18, header_offset).unwrap(),
        shared_table_range(0x00B190 + palette_fg as u32 * 0x18, header_offset).unwrap(),
        shared_table_range(0x00B318 + palette_sprite as u32 * 0x18, header_offset).unwrap(),
    ];
    let mut merged = CustomPaletteData::parse(&scratch).unwrap_or_default();
    merged.set(0x105, custom.get(0x105).unwrap().clone());
    merged.write_to_rom(&mut scratch, header_offset).expect("write_to_rom must succeed");
    let back = CustomPaletteData::parse(&scratch).expect("block must parse after save");
    assert_eq!(back.len(), 1);
    assert_eq!(back.get(0x105).unwrap().bg[3], 0x001F, "saved entry keeps the edit");
    for (lo, hi) in &shared_ranges {
        assert_eq!(
            &scratch[*lo..*hi],
            &raw[*lo..*hi],
            "shared palette tables must be untouched by a custom-palette save"
        );
    }

    // ── Compose the image ────────────────────────────────────────────────
    let w = 1420u32;
    let h = 470u32;
    let mut img = RgbImage::new(w, h);
    fill_rect(&mut img, 0, 0, w, h, Rgb([26, 28, 33]));
    let white = Rgb([235, 235, 240]);
    let dim = Rgb([120, 124, 132]);
    let green = Rgb([120, 200, 130]);

    draw_text(
        &mut img,
        &sans_bold,
        "Palette Editor — per-level custom palette  (Lunar Magic v3.30 parity)",
        24,
        16,
        20.0,
        white,
    );

    // Panel 1: shared mode.
    {
        let (x0, y0, ww, wh) = (24u32, 56u32, 440u32, 340u32);
        fill_rect(&mut img, x0, y0, ww, wh, Rgb([43, 46, 53]));
        rect_border(&mut img, x0, y0, ww, wh, Rgb([100, 104, 112]));
        fill_rect(&mut img, x0, y0, ww, 26, Rgb([52, 56, 64]));
        draw_text(&mut img, &sans_bold, "Palette Editor", (x0 + 10) as i32, (y0 + 5) as i32, 14.0, white);
        draw_checkbox(&mut img, &sans, x0 + 10, y0 + 40, false, "Enable custom palette");
        draw_text(
            &mut img,
            &sans,
            &format!("editing shared tables (BG {palette_bg:X}, FG {palette_fg:X}, sprite {palette_sprite:X})"),
            (x0 + 10) as i32,
            (y0 + 64) as i32,
            13.0,
            dim,
        );
        draw_checkbox(&mut img, &sans, x0 + 10, y0 + 88, true, "Auto-enable custom palette on edit");
        draw_bg_row(&mut img, &sans, x0 + 10, y0 + 122, &bg0, 3, palette_bg, false);
        draw_text(
            &mut img,
            &sans,
            "1. Before the edit: the level shares the game's",
            (x0 + 4) as i32,
            (y0 + wh + 8) as i32,
            13.0,
            dim,
        );
        draw_text(
            &mut img,
            &sans,
            &format!("palette tables with every other level on index {palette_bg:X}."),
            (x0 + 4) as i32,
            (y0 + wh + 28) as i32,
            13.0,
            dim,
        );
    }

    // Panel 2: auto-enabled by the first edit.
    {
        let (x0, y0, ww, wh) = (488u32, 56u32, 440u32, 340u32);
        fill_rect(&mut img, x0, y0, ww, wh, Rgb([43, 46, 53]));
        rect_border(&mut img, x0, y0, ww, wh, Rgb([100, 104, 112]));
        fill_rect(&mut img, x0, y0, ww, 26, Rgb([52, 56, 64]));
        draw_text(&mut img, &sans_bold, "Palette Editor", (x0 + 10) as i32, (y0 + 5) as i32, 14.0, white);
        draw_checkbox(&mut img, &sans, x0 + 10, y0 + 40, true, "Enable custom palette");
        draw_text(&mut img, &sans, "edits stay private to this level", (x0 + 10) as i32, (y0 + 64) as i32, 13.0, green);
        draw_checkbox(&mut img, &sans, x0 + 10, y0 + 88, true, "Auto-enable custom palette on edit");
        let edited = palettes.read(|p| p.bg);
        draw_bg_row(&mut img, &sans, x0 + 10, y0 + 122, &edited, 3, palette_bg, true);
        draw_text(
            &mut img,
            &sans,
            "2. First edit auto-enables: BG color 3 → FF0000,",
            (x0 + 4) as i32,
            (y0 + wh + 8) as i32,
            13.0,
            dim,
        );
        draw_text(
            &mut img,
            &sans,
            "seeded from the shared colors, one undo step.",
            (x0 + 4) as i32,
            (y0 + wh + 28) as i32,
            13.0,
            dim,
        );
    }

    // Panel 3: the saved RATS block.
    {
        let (x0, y0, ww, wh) = (952u32, 56u32, 440u32, 340u32);
        fill_rect(&mut img, x0, y0, ww, wh, Rgb([43, 46, 53]));
        rect_border(&mut img, x0, y0, ww, wh, Rgb([100, 104, 112]));
        fill_rect(&mut img, x0, y0, ww, 26, Rgb([52, 56, 64]));
        draw_text(&mut img, &sans_bold, "Saved ROM", (x0 + 10) as i32, (y0 + 5) as i32, 14.0, white);
        draw_text(&mut img, &sans, "RATS block: STAR … \"SMWECPLT\"", (x0 + 10) as i32, (y0 + 44) as i32, 13.0, white);
        draw_text(&mut img, &sans, "entries: 1   (level 0x105)", (x0 + 10) as i32, (y0 + 68) as i32, 13.0, white);
        draw_text(
            &mut img,
            &sans,
            "entry BG[3] = 001F (the FF0000 edit)",
            (x0 + 10) as i32,
            (y0 + 92) as i32,
            13.0,
            white,
        );
        draw_text(&mut img, &sans, "shared tables: byte-identical ✓", (x0 + 10) as i32, (y0 + 116) as i32, 13.0, green);
        // Swatch strip of the saved custom entry's BG row.
        let saved_bg = back.get(0x105).unwrap().bg;
        for (ci, &c) in saved_bg.iter().enumerate() {
            let sx = x0 + 10 + ci as u32 * 30;
            let sy = y0 + 150;
            fill_rect(&mut img, sx, sy, 28, 22, abgr1555_to_rgb(c));
            rect_border(&mut img, sx, sy, 28, 22, Rgb([80, 80, 88]));
        }
        draw_text(
            &mut img,
            &sans,
            "3. On save the level's private entry lands in the",
            (x0 + 4) as i32,
            (y0 + wh + 8) as i32,
            13.0,
            dim,
        );
        draw_text(
            &mut img,
            &sans,
            "SMWECPLT block; shared tables stay untouched.",
            (x0 + 4) as i32,
            (y0 + wh + 28) as i32,
            13.0,
            dim,
        );
    }

    draw_text(
        &mut img,
        &sans,
        "Real CustomPaletteData RATS codec + UndoableData; BG colors and shared-table check from the real ROM (level 0x105). Row layout is a mock of the egui window.",
        24,
        (h - 28) as i32,
        13.0,
        dim,
    );

    img.save(output)?;
    println!("wrote {output}");
    Ok(())
}
