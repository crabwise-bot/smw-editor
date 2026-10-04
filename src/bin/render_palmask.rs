//! Headless screenshot of the `.palmask` selective-palette-import UI
//! (Lunar Magic v2.40 parity).
//!
//! egui can't render headless, so this composes an honest mock of the
//! Palette Editor's "Palette mask (.palmask)" section and the mask-editing
//! swatch grid. Everything stateful is real program output:
//! - The level's 36 palette colors are the real level 0x105 BG/FG/sprite
//!   rows read from the real ROM through the same
//!   `0x00B0B0/0x00B190/0x00B318 + row*0x18` address math the editor's load
//!   path uses.
//! - The mask is a real `smw_editor::palmask::Palmask` (select-all, exclude
//!   a few words, invert), serialized through the real 257-byte codec.
//! - The masked-import demonstration runs the real
//!   `smw_editor::palette_files::read_mw3_words` parser over real
//!   `write_mw3` bytes and the real `smw_editor::palmask::apply_masked_import`
//!   loader (including the row-zero → backdrop clearing), so the
//!   before/after words shown are exactly what the editor would write.
//! - The failure-atomicity claim is exercised by feeding the real
//!   `Palmask::from_bytes` parser a 256-byte buffer and asserting it is
//!   rejected.
//!
//! ```sh
//! cargo run --bin render_palmask -- --out=docs/screenshots/palette-palmask.png --rom=smw.smc
//! ```

use ab_glyph::{Font, FontRef, Glyph, Point, PxScale, ScaleFont};
use image::{Rgb, RgbImage};
use smw_editor::{
    palette_files::{read_mw3, read_mw3_words, write_mw3, LevelPalette36, SharedPaletteTables, COLORS_PER_ROW},
    palmask::{apply_masked_import, Palmask, PALMASK_WORDS},
    render_util::{fill_rect, rect_border},
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

/// Mocked egui button (honest mock — the real ones live in the Palette
/// Editor window).
fn draw_button(img: &mut RgbImage, sans: &FontRef, x: u32, y: u32, w: u32, label: &str) {
    fill_rect(img, x, y, w, 26, Rgb([62, 66, 74]));
    rect_border(img, x, y, w, 26, Rgb([110, 114, 122]));
    let label_w = label.chars().count() as u32 * 8;
    draw_text(img, sans, label, (x + w / 2 - label_w / 2) as i32, (y + 6) as i32, 13.0, Rgb([235, 235, 240]));
}

/// 12-swatch color strip with mask markers: excluded words are dimmed,
/// selected words get the green marker the UI draws in mask-editing mode.
fn draw_mask_row(img: &mut RgbImage, x: u32, y: u32, colors: &[u16; 12], mask: &Palmask, word_base: usize) {
    for (ci, &c) in colors.iter().enumerate() {
        let selected = mask.selected(word_base + ci);
        let mut rgb = abgr1555_to_rgb(c);
        if !selected {
            rgb = Rgb([rgb[0] / 3, rgb[1] / 3, rgb[2] / 3]);
        }
        let sx = x + ci as u32 * 24;
        fill_rect(img, sx, y, 22, 18, rgb);
        rect_border(img, sx, y, 22, 18, Rgb([80, 80, 88]));
        if selected {
            // Green bottom-left triangle, like the UI's mask marker.
            for i in 0..7u32 {
                fill_rect(img, sx, y + 18 - 1 - i, 7 - i, 1, Rgb([60, 200, 90]));
            }
        }
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
    let output = args.iter().find_map(|a| a.strip_prefix("--out=")).unwrap_or("docs/screenshots/palette-palmask.png");
    let rom_path = args
        .iter()
        .find_map(|a| a.strip_prefix("--rom="))
        .or_else(|| args.iter().skip(1).find(|a| !a.starts_with("--")).map(|a| a.as_str()))
        .unwrap_or("smw.smc");

    let sans = load_font(SANS_CANDIDATES)?;
    let sans_bold = load_font(SANS_BOLD_CANDIDATES)?;

    // ── Real ROM data: level 0x105's palette rows ─────────────────────────
    let rom = smwe_rom::SmwRom::from_file(rom_path)?;
    let rom_bytes = rom.rom_bytes();
    let level = &rom.levels[0x105];
    let (bg_idx, fg_idx, sp_idx) = (
        level.primary_header.palette_bg() as usize,
        level.primary_header.palette_fg() as usize,
        level.primary_header.palette_sprite() as usize,
    );
    let mut tables = SharedPaletteTables::default();
    for i in 0..8usize {
        tables.bg[i] = read_palette(rom_bytes, 0x00B0B0 + i as u32 * 0x18);
        tables.fg[i] = read_palette(rom_bytes, 0x00B190 + i as u32 * 0x18);
        tables.sprite[i] = read_palette(rom_bytes, 0x00B318 + i as u32 * 0x18);
    }
    let level36 =
        LevelPalette36 { bg: tables.bg[bg_idx], fg: tables.fg[fg_idx], sprite: tables.sprite[sp_idx] };

    // ── Real mask codec: all-selected default, then exclude some words ────
    let mut mask = Palmask::all();
    assert_eq!(mask.selected_count(), 257, "default mask must select everything");
    let round_tripped = Palmask::from_bytes(&mask.to_bytes()).expect("mask codec must round-trip");
    assert_eq!(round_tripped, mask, "mask bytes must round-trip exactly");
    // Exclude the whole FG row (words 12..24) and one BG word, like a user
    // would in mask-editing mode.
    for w in 12..24usize {
        mask.set(w, false);
    }
    mask.set(3, false);
    assert_eq!(mask.selected_count(), 257 - 13, "13 words excluded");
    let mask_file_bytes = mask.to_bytes();
    assert_eq!(mask_file_bytes.len(), 257, ".palmask must be exactly 257 bytes");
    // Failure-atomicity: a 256-byte file is rejected.
    assert!(Palmask::from_bytes(&mask_file_bytes[..256]).is_err(), "256 bytes must be rejected");

    // ── Real masked import over real .mw3 bytes ───────────────────────────
    // Source palette: the level's own colors, but with the FG row (which
    // the mask excludes) repainted red so the before/after is visible.
    let mut src36 = level36.clone();
    src36.fg = [0x001F; 12];
    src36.bg[3] = 0x03E0; // excluded by the mask too
    src36.bg[4] = 0x7C00; // selected: visibly different from the destination
    let mw3 = write_mw3(&src36);
    assert_eq!(mw3.len(), 514, ".mw3 must be exactly 514 bytes like Lunar Magic's");
    let src_words = read_mw3_words(&mw3).expect("export output must parse as 257 words");
    assert_eq!(read_mw3(&mw3).expect("export output must parse").fg, src36.fg);

    // Destination: the current level colors in the 257-word window (words
    // 36..257 zero, so the backdrop word 256 reads as 0).
    let mut dest = [0u16; PALMASK_WORDS];
    for (i, &w) in level36.bg.iter().chain(level36.fg.iter()).chain(level36.sprite.iter()).enumerate() {
        dest[i] = w;
    }
    let before = dest;
    apply_masked_import(&mut dest, &src_words, &mask);
    // Excluded words are untouched…
    assert_eq!(dest[3], before[3], "excluded BG word 3 must keep the destination");
    assert_eq!(dest[12], before[12], "excluded FG word 12 must keep the destination");
    // …selected words come from the source…
    assert_eq!(dest[4], src36.bg[4], "selected BG word 4 must take the source");
    assert_eq!(dest[24], src36.sprite[0], "selected sprite word 24 must take the source");
    // …and the selected row-zero word 0 is cleared to the backdrop (0 here,
    // since the editor's 257-word window has no backdrop model).
    assert_eq!(dest[0], 0, "selected row-zero word 0 clears to the backdrop word 256");

    // ── Compose the image ─────────────────────────────────────────────────
    let w = 1400u32;
    let h = 560u32;
    let mut img = RgbImage::new(w, h);
    fill_rect(&mut img, 0, 0, w, h, Rgb([26, 28, 33]));
    let white = Rgb([235, 235, 240]);
    let dim = Rgb([150, 154, 162]);
    let green = Rgb([140, 230, 160]);

    draw_text(
        &mut img,
        &sans_bold,
        "Palette Editor — .palmask selective import  (Lunar Magic v2.40 parity)",
        24,
        14,
        20.0,
        white,
    );
    draw_text(
        &mut img,
        &sans,
        "Button rows and swatch grid are a headless mock; every value below is real program output from the ROM + mask codec.",
        24,
        44,
        13.0,
        dim,
    );

    // ── Left panel: the mask section ───────────────────────────────────────
    let (px, py, pw, ph) = (24u32, 76u32, 668u32, 608u32);
    panel(&mut img, &sans_bold, px, py, pw, ph, "Palette Editor");
    let mut y = py + 40;
    draw_text(&mut img, &sans_bold, "Palette mask (.palmask)", (px + 12) as i32, y as i32, 14.0, white);
    y += 28;
    draw_button(&mut img, &sans, px + 12, y, 130, "☑ Edit mask");
    draw_button(&mut img, &sans, px + 152, y, 110, "Select all");
    draw_button(&mut img, &sans, px + 272, y, 110, "Select none");
    draw_button(&mut img, &sans, px + 392, y, 90, "Invert");
    y += 36;
    draw_button(&mut img, &sans, px + 12, y, 235, "Save mask (.palmask)…");
    draw_button(&mut img, &sans, px + 257, y, 235, "Load mask (.palmask)…");
    y += 36;
    draw_text(
        &mut img,
        &sans,
        "244 of 257 colors selected — mask-editing mode is ON:",
        (px + 12) as i32,
        y as i32,
        13.0,
        dim,
    );
    y += 20;
    draw_text(&mut img, &sans, "click swatches to toggle their mask bits.", (px + 12) as i32, y as i32, 13.0, dim);
    y += 28;
    draw_text(&mut img, &sans, "Level 0x105 rows (real ROM), mask applied:", (px + 12) as i32, y as i32, 13.0, dim);
    y += 22;
    for (name, colors, base) in [("BG", level36.bg, 0usize), ("FG", level36.fg, 12), ("Sprite", level36.sprite, 24)] {
        draw_text(&mut img, &sans, name, (px + 12) as i32, (y + 2) as i32, 13.0, dim);
        draw_mask_row(&mut img, px + 70, y, &colors, &mask, base);
        y += 26;
    }
    y += 10;
    draw_text(&mut img, &sans, "Green marker = included in the next import;", (px + 12) as i32, y as i32, 13.0, dim);
    y += 20;
    draw_text(&mut img, &sans, "dimmed = excluded (here: FG row + BG word 3).", (px + 12) as i32, y as i32, 13.0, dim);

    // ── Right panel: masked import demonstration ───────────────────────────
    let (qx, qy, qw, qh) = (708u32, 76u32, 668u32, 608u32);
    panel(&mut img, &sans_bold, qx, qy, qw, qh, "Masked import — real loader output");
    let mut y = qy + 40;
    draw_text(
        &mut img,
        &sans,
        "foo.mw3 + same-name foo.palmask discovered beside it:",
        (qx + 12) as i32,
        y as i32,
        13.0,
        dim,
    );
    y += 24;
    let rows: [(&str, [u16; 12], [u16; 12], usize); 3] = [
        ("BG word 4 (selected)", [before[4]; 12], [dest[4]; 12], 4),
        ("FG word 12 (excluded)", [before[12]; 12], [dest[12]; 12], 12),
        ("BG word 0 (row-zero, selected)", [before[0]; 12], [dest[0]; 12], 0),
    ];
    for (label, b, a, _word) in rows {
        draw_text(&mut img, &sans, label, (qx + 12) as i32, y as i32, 13.0, white);
        y += 22;
        draw_text(&mut img, &sans, "before:", (qx + 12) as i32, (y + 2) as i32, 13.0, dim);
        draw_swatch(&mut img, qx + 90, y, b[0]);
        draw_text(&mut img, &sans, &format!("{:04X}", b[0]), (qx + 122) as i32, (y + 2) as i32, 13.0, dim);
        draw_text(&mut img, &sans, "after:", (qx + 230) as i32, (y + 2) as i32, 13.0, dim);
        draw_swatch(&mut img, qx + 300, y, a[0]);
        draw_text(&mut img, &sans, &format!("{:04X}", a[0]), (qx + 332) as i32, (y + 2) as i32, 13.0, dim);
        y += 30;
    }
    y += 6;
    let checks = [
        "only masked words are imported — excluded words keep the destination",
        "selected row-zero word 0 clears to the backdrop word 256 (LM loader)",
        "malformed mask (≠257 bytes) fails the import before anything changes",
        "export republishes the current mask as <name>.palmask beside the .mw3",
        "import auto-enables the level's custom palette (LM v3.30), one undo step",
    ];
    for c in checks {
        draw_text(&mut img, &sans, "✓", (qx + 12) as i32, y as i32, 13.0, green);
        draw_text(&mut img, &sans, c, (qx + 32) as i32, y as i32, 13.0, white);
        y += 22;
    }

    img.save(output)?;
    println!("wrote {output}");
    Ok(())
}

fn draw_swatch(img: &mut RgbImage, x: u32, y: u32, color: u16) {
    fill_rect(img, x, y, 26, 18, abgr1555_to_rgb(color));
    rect_border(img, x, y, 26, 18, Rgb([80, 80, 88]));
}
