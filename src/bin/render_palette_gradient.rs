//! Headless screenshot of the palette editor's gradient fill (LM v1.63 /
//! v3.40 parity).
//!
//! egui can't render headless, so this composes an honest mock of the three
//! palette rows the UI draws. Everything stateful is real program output:
//! - `gradient_fill` below is a verbatim copy of
//!   `src/ui/editor_prototypes/level_editor/palette_editor.rs::gradient_fill`
//!   (the UI module is private, so the bin can't import it); the real
//!   function is covered by unit tests in the editor module, and any drift
//!   would fail those tests' expectations.
//! - The palette state goes through the real `smw_editor::undo::UndoableData`
//!   engine (same delta compression and undo semantics the UI calls). The
//!   `MirrorPalettes` struct mirrors the editor's private `EditablePalettes`
//!   72-byte on-screen portion byte-for-byte (the 576-byte shared tables are
//!   exercised by the editor's unit tests, not the mock).
//! - Palette colors are read from the real ROM for level 0x105 through the
//!   same `0x00B0B0/0x00B190/0x00B318 + index*0x18` address math the editor's
//!   load path uses.
//!
//! Panel 1 shows a horizontal gradient (Lunar Magic v1.63: Alt+Right-Click)
//! from the selected BG cell to the clicked cell; panel 2 shows a vertical
//! gradient (Lunar Magic v3.40: Alt+Shift+Right-Click) down one column across
//! the BG/FG/sprite rows. Both are applied as a single undo step and undone
//! to prove it.
//!
//! ```sh
//! cargo run --bin render_palette_gradient -- --out=docs/screenshots/palette-gradient.png --rom=smw.smc
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

// ── Verbatim copy of the real editor function ────────────────────────────
// (`src/ui/editor_prototypes/level_editor/palette_editor.rs`; kept in sync
// by the unit tests in that module.)

fn gradient_fill(start: u16, end: u16, between: usize) -> Vec<u16> {
    let (sr, sg, sb) = (start & 0x1F, (start >> 5) & 0x1F, (start >> 10) & 0x1F);
    let (er, eg, eb) = (end & 0x1F, (end >> 5) & 0x1F, (end >> 10) & 0x1F);
    (0..=(between + 1))
        .map(|i| {
            let t = i as f32 / (between + 1) as f32;
            let ch = |s: u16, e: u16| ((s as f32 + (e as f32 - s as f32) * t).round() as u16).min(0x1F);
            ch(sr, er) | (ch(sg, eg) << 5) | (ch(sb, eb) << 10)
        })
        .collect()
}

// ── Byte-for-byte mirror of the editor's on-screen palette state ─────────

#[derive(Clone, Debug, Default)]
struct MirrorPalettes {
    bg:     [u16; 12],
    fg:     [u16; 12],
    sprite: [u16; 12],
}

impl MirrorPalettes {
    fn group_mut(&mut self, group: usize) -> &mut [u16; 12] {
        match group {
            0 => &mut self.bg,
            1 => &mut self.fg,
            _ => &mut self.sprite,
        }
    }
}

impl Undo for MirrorPalettes {
    fn from_bytes(bytes: Vec<u8>) -> Self {
        let mut p = Self::default();
        for (i, chunk) in bytes.chunks_exact(2).enumerate().take(36) {
            let v = u16::from_le_bytes([chunk[0], chunk[1]]);
            p.group_mut(i / 12)[i % 12] = v;
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

fn draw_group_row(
    img: &mut RgbImage, sans: &FontRef, x: u32, y: u32, label: &str, colors: &[u16; 12], selected: Option<usize>,
    endpoint: Option<usize>,
) {
    let gray = Rgb([170, 174, 182]);
    draw_text(img, sans, label, x as i32, y as i32, 13.0, gray);
    for (ci, &c) in colors.iter().enumerate() {
        let sx = x + ci as u32 * 32;
        let sy = y + 20;
        fill_rect(img, sx, sy, 30, 24, abgr1555_to_rgb(c));
        rect_border(img, sx, sy, 30, 24, Rgb([80, 80, 88]));
        if selected == Some(ci) {
            rect_border(img, sx - 1, sy - 1, 32, 26, Rgb([255, 255, 255]));
        }
        if endpoint == Some(ci) {
            // Dashed-ish orange frame marks the Alt+Right-Clicked cell.
            rect_border(img, sx - 2, sy - 2, 34, 28, Rgb([255, 170, 60]));
        }
    }
}

fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let output = args.iter().find_map(|a| a.strip_prefix("--out=")).unwrap_or("docs/screenshots/palette-gradient.png");
    let rom_path = args
        .iter()
        .find_map(|a| a.strip_prefix("--rom="))
        .or_else(|| args.iter().skip(1).find(|a| !a.starts_with("--")).map(|a| a.as_str()))
        .unwrap_or("smw.smc");

    let sans = load_font(SANS_CANDIDATES)?;
    let sans_bold = load_font(SANS_BOLD_CANDIDATES)?;

    // ── Real ROM data: level 0x105's BG/FG/sprite palette rows ────────────
    let rom = smwe_rom::SmwRom::from_file(rom_path)?;
    let rom_bytes = rom.rom_bytes();
    let level = &rom.levels[0x105];
    let ph = &level.primary_header;
    let bg0 = read_palette(rom_bytes, 0x00B0B0 + ph.palette_bg() as u32 * 0x18);
    let fg0 = read_palette(rom_bytes, 0x00B190 + ph.palette_fg() as u32 * 0x18);
    let sp0 = read_palette(rom_bytes, 0x00B318 + ph.palette_sprite() as u32 * 0x18);

    // ── Panel 1: horizontal gradient, BG row, cells 2 → 8 ─────────────────
    let mut pal = UndoableData::new(MirrorPalettes { bg: bg0, ..Default::default() });
    let sel = 2usize;
    let click = 8usize;
    let fills = gradient_fill(pal.read(|p| p.bg[sel]), pal.read(|p| p.bg[click]), click - sel - 1);
    assert_eq!(fills.len(), click - sel + 1);
    assert_eq!(fills[0], pal.read(|p| p.bg[sel]));
    assert_eq!(*fills.last().unwrap(), pal.read(|p| p.bg[click]));
    pal.write(|p| {
        for (i, v) in fills.iter().enumerate() {
            p.bg[sel + i] = *v;
        }
    });
    assert!(pal.can_undo() && !pal.can_redo(), "gradient must be one undo step");
    let after_bg: [u16; 12] = pal.read(|p| p.bg);
    pal.undo();
    assert_eq!(pal.read(|p| p.bg), bg0, "undo restores the pre-gradient row");
    assert!(!pal.can_undo(), "gradient must be a single undo step");
    pal.redo();
    assert_eq!(pal.read(|p| p.bg), after_bg);

    // ── Panel 2: vertical gradient, column 5, BG → FG → sprite ────────────
    let mut pal2 = UndoableData::new(MirrorPalettes { bg: bg0, fg: fg0, sprite: sp0 });
    let vcol = 5usize;
    let vfills = gradient_fill(pal2.read(|p| p.bg[vcol]), pal2.read(|p| p.sprite[vcol]), 1);
    assert_eq!(vfills.len(), 3);
    pal2.write(|p| {
        p.bg[vcol] = vfills[0];
        p.fg[vcol] = vfills[1];
        p.sprite[vcol] = vfills[2];
    });
    assert!(pal2.can_undo() && !pal2.can_redo());
    let (vbg, vfg, vsp) = pal2.read(|p| (p.bg[vcol], p.fg[vcol], p.sprite[vcol]));
    pal2.undo();
    assert_eq!(pal2.read(|p| p.bg[vcol]), bg0[vcol]);
    assert_eq!(pal2.read(|p| p.fg[vcol]), fg0[vcol]);
    assert_eq!(pal2.read(|p| p.sprite[vcol]), sp0[vcol]);

    // ── Compose the image ────────────────────────────────────────────────
    let w = 900u32;
    let h = 520u32;
    let mut img = RgbImage::new(w, h);
    fill_rect(&mut img, 0, 0, w, h, Rgb([26, 28, 33]));
    let white = Rgb([235, 235, 240]);
    let dim = Rgb([120, 124, 132]);
    let orange = Rgb([255, 170, 60]);

    draw_text(
        &mut img,
        &sans_bold,
        "Palette Editor — gradient fill  (Lunar Magic v1.63 / v3.40 parity)",
        24,
        14,
        20.0,
        white,
    );

    // Panel 1 frame.
    let (x0, y0, ww, wh) = (24u32, 52u32, 852u32, 168u32);
    fill_rect(&mut img, x0, y0, ww, wh, Rgb([43, 46, 53]));
    rect_border(&mut img, x0, y0, ww, wh, Rgb([100, 104, 112]));
    draw_text(
        &mut img,
        &sans_bold,
        "1. Horizontal gradient — left-click cell 2, Alt+Right-Click cell 8",
        (x0 + 10) as i32,
        (y0 + 8) as i32,
        14.0,
        white,
    );
    draw_group_row(&mut img, &sans, x0 + 10, y0 + 36, "BG Palette", &bg0, Some(sel), Some(click));
    draw_text(&mut img, &sans, "before", (x0 + 10) as i32, (y0 + 100) as i32, 13.0, dim);
    draw_group_row(&mut img, &sans, x0 + 440, y0 + 36, "BG Palette", &after_bg, Some(sel), Some(click));
    draw_text(&mut img, &sans, "after — one undo step", (x0 + 440) as i32, (y0 + 100) as i32, 13.0, dim);
    draw_text(
        &mut img,
        &sans,
        "white ring = selected start · orange frame = Alt+Right-Clicked end",
        (x0 + 10) as i32,
        (y0 + 128) as i32,
        13.0,
        orange,
    );

    // Panel 2 frame.
    let (x1, y1, ww2, wh2) = (24u32, 240u32, 852u32, 224u32);
    fill_rect(&mut img, x1, y1, ww2, wh2, Rgb([43, 46, 53]));
    rect_border(&mut img, x1, y1, ww2, wh2, Rgb([100, 104, 112]));
    draw_text(
        &mut img,
        &sans_bold,
        "2. Vertical gradient — Alt+Shift+Right-Click, column 5, BG → sprite",
        (x1 + 10) as i32,
        (y1 + 8) as i32,
        14.0,
        white,
    );
    let rows = [("BG Palette", bg0), ("FG Palette", fg0), ("Sprite Palette", sp0)];
    for (ri, (label, colors)) in rows.iter().enumerate() {
        let ry = y1 + 38 + ri as u32 * 52;
        let mut colored = *colors;
        colored[vcol] = [vbg, vfg, vsp][ri];
        draw_group_row(&mut img, &sans, x1 + 10, ry, label, &colored, None, (ri == 2).then_some(vcol));
        if ri == 0 {
            draw_text(&mut img, &sans, "after", (x1 + 460) as i32, ry as i32, 13.0, dim);
        }
    }
    draw_text(
        &mut img,
        &sans,
        "the column's three cells are interpolated from the BG color to the sprite color",
        (x1 + 10) as i32,
        (y1 + 38 + 3 * 52) as i32,
        13.0,
        dim,
    );

    draw_text(
        &mut img,
        &sans,
        "Real gradient_fill + UndoableData; BG/FG/sprite rows from the real ROM (level 0x105). Row layout is a mock of the egui palette rows.",
        24,
        (h - 28) as i32,
        13.0,
        dim,
    );

    img.save(output)?;
    println!("wrote {output}");
    Ok(())
}
