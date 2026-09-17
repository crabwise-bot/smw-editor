//! Headless mock screenshot of the new "Change Properties in Sprite Header"
//! dialog (Lunar Magic parity, audit item 14).
//!
//! egui can't render headless, so this composes an honest mock: every value
//! shown is real — parsed from the ROM by the same code the UI uses
//! (`smwe_rom::level::headers::SpriteHeader` accessors for the byte fields,
//! `SpriteHeaderExt` for the LM 3.00 options, sprite count from the parsed
//! level). The before/after edit is verified by writing the edited byte into
//! a scratch ROM copy at the sprite pointer (exactly what `save_to_rom`
//! writes) and re-parsing, and by a `SpriteHeaderExtData` write/parse
//! round-trip on the scratch copy. Only the window chrome (title bar,
//! dropdowns, checkboxes) is drawn, not real egui widgets.
//!
//! Shows level 0x105 before and after toggling sprite buoyancy in the
//! dialog, plus the LM 3.00 spawn-range / smart-spawning options.
//!
//! ```sh
//! cargo run --bin render_sprite_header_editor -- --out=docs/screenshots/sprite-header-editor.png --rom=/path/to/smw.smc
//! ```

use ab_glyph::{Font, FontRef, Glyph, Point, PxScale, ScaleFont};
use image::{Rgb, RgbImage};
use smwe_rom::{
    level::{
        headers::SpriteHeader,
        sprite_header_ext::{SpawnRange, SpriteHeaderExt, SpriteHeaderExtData, MAX_SPRITES_LM300},
        Level,
    },
    snes_utils::{
        addr::{AddrPc, AddrSnes},
        rom::Rom,
    },
    SmwRom,
};

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
    mono:      FontRef<'static>,
    sans:      FontRef<'static>,
    sans_bold: FontRef<'static>,
}

/// Draw one line of text; returns the advance width in px.
fn draw_text(img: &mut RgbImage, font: &FontRef, text: &str, x: i32, y: i32, px: f32, color: Rgb<u8>) -> i32 {
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

/// Mock dropdown: label + a box showing the selected text.
fn draw_combo(img: &mut RgbImage, fonts: &Fonts, x: u32, y: u32, label: &str, selected: &str, ink: Rgb<u8>) -> u32 {
    draw_text(img, &fonts.sans, label, x as i32, (y + 6) as i32, 14.0, ink);
    let (bx, bw, bh) = (x + 250, 300u32, 32u32);
    fill_rect(img, bx, y, bw, bh, Rgb([0xFF, 0xFF, 0xFF]));
    rect_border(img, bx, y, bw, bh, Rgb([0x99, 0x99, 0x99]));
    draw_text(img, &fonts.sans, selected, (bx + 10) as i32, (y + 6) as i32, 14.0, ink);
    // Dropdown arrow.
    draw_text(img, &fonts.sans, "\u{25BE}", (bx + bw - 28) as i32, (y + 6) as i32, 14.0, Rgb([0x66, 0x66, 0x66]));
    y + bh + 12
}

/// Mock checkbox; `changed` draws it highlighted like a fresh edit.
fn draw_check(
    img: &mut RgbImage, fonts: &Fonts, x: u32, y: u32, label: &str, on: bool, changed: bool, ink: Rgb<u8>,
) -> u32 {
    let fill = if changed { Rgb([0xFF, 0xF2, 0xC0]) } else { Rgb([0xFF, 0xFF, 0xFF]) };
    fill_rect(img, x, y, 20, 20, fill);
    rect_border(img, x, y, 20, 20, Rgb([0x66, 0x66, 0x66]));
    if on {
        draw_text(img, &fonts.sans_bold, "\u{2713}", (x + 3) as i32, (y - 2) as i32, 18.0, ink);
    }
    draw_text(img, &fonts.sans, label, (x + 30) as i32, (y - 1) as i32, 14.0, ink);
    y + 32
}

use smw_editor::render_util::{fill_rect, rect_border};

/// One-line decode of the header byte, from the real `SpriteHeader` accessors.
fn decode_line(h: &SpriteHeader) -> String {
    format!(
        "memory 0x{:02X}  \u{00B7}  buoyancy {}  \u{00B7}  Layer 2 interaction {}",
        h.sprite_memory(),
        if h.sprite_buoyancy() { "ON" } else { "off" },
        if h.disable_layer2_interaction() { "DISABLED" } else { "enabled" },
    )
}

fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let output =
        args.iter().find_map(|a| a.strip_prefix("--out=")).unwrap_or("docs/screenshots/sprite-header-editor.png");
    let rom_path = args
        .iter()
        .find_map(|a| a.strip_prefix("--rom="))
        .or_else(|| args.iter().skip(1).find(|a| !a.starts_with("--")).map(|a| a.as_str()))
        .unwrap_or("smw.smc");

    let fonts = Fonts {
        mono:      load_font(MONO_CANDIDATES)?,
        sans:      load_font(SANS_CANDIDATES)?,
        sans_bold: load_font(SANS_BOLD_CANDIDATES)?,
    };

    // Real data path, identical to the UI: parse the ROM, take level 0x105's
    // sprite header byte + sprite count.
    let rom = SmwRom::from_file(rom_path)?;
    let vanilla = rom.levels[0x105].sprite_header.clone();
    let sprite_count = rom.levels[0x105].sprite_layer.sprites.len();

    // AFTER: the edit the screenshot shows — toggle sprite buoyancy on.
    let mut edited = vanilla.clone();
    edited.set_sprite_buoyancy(true);
    anyhow::ensure!(edited.as_byte() != vanilla.as_byte());

    // Prove the byte write path: patch the edited byte into a scratch ROM
    // copy at the sprite pointer (exactly what `save_to_rom` writes when the
    // dialog is dirty), re-parse, and confirm the byte round-trips with the
    // sprite stream byte-identical.
    let mut bytes = rom.rom.bytes().to_vec();
    let tbl_pc = AddrPc::try_from_lorom(AddrSnes(0x05EC00 + 0x105 * 2))?.as_index() as usize;
    let ptr = u16::from_le_bytes([bytes[tbl_pc], bytes[tbl_pc + 1]]);
    let data_pc = AddrPc::try_from_lorom(AddrSnes(ptr as u32 | 0x070000))?.as_index() as usize;
    bytes[data_pc] = edited.as_byte();
    let reparsed = Level::parse(&Rom::new(bytes.clone())?, 0x105)?;
    anyhow::ensure!(
        reparsed.sprite_header.as_byte() == edited.as_byte(),
        "edited sprite header byte did not round-trip"
    );
    anyhow::ensure!(
        reparsed.sprite_layer.as_bytes() == rom.levels[0x105].sprite_layer.as_bytes(),
        "sprite stream changed"
    );

    // Prove the LM 3.00 options path: write a non-default entry into a
    // scratch copy and confirm it parses back (the RATS-block write
    // `save_to_rom` performs when the dialog is dirty).
    let demo_ext = SpriteHeaderExt { spawn_range: SpawnRange::Wider, smart_spawning: true };
    let mut ext_data = SpriteHeaderExtData::default();
    ext_data.set(0x105, demo_ext)?;
    ext_data.write_to_rom(&mut bytes, 0)?;
    let ext_back = SpriteHeaderExtData::parse(&bytes)?;
    anyhow::ensure!(ext_back.get(0x105) == demo_ext, "sprite-header-ext did not round-trip");

    // ---- Compose the mock window ----
    let (w, h) = (1200u32, 780u32);
    let mut img = RgbImage::new(w, h);
    let bg = Rgb([0xF2, 0xF2, 0xF2]);
    let ink = Rgb([0x1A, 0x1A, 0x1A]);
    let gray = Rgb([0x66, 0x66, 0x66]);
    let green = Rgb([0x1E, 0x7A, 0x1E]);
    for p in img.pixels_mut() {
        *p = bg;
    }

    // Title bar.
    fill_rect(&mut img, 0, 0, w, 52, Rgb([0x2B, 0x2B, 0x2B]));
    draw_text(
        &mut img,
        &fonts.sans_bold,
        "Change Properties in Sprite Header \u{2014} level 105 (headless mock; bytes are real ROM output)",
        24,
        15,
        19.0,
        Rgb([0xFF, 0xFF, 0xFF]),
    );

    // ---- Left panel: BEFORE (vanilla byte) ----
    let x = 24u32;
    let mut y = 76u32;
    draw_text(&mut img, &fonts.sans_bold, "BEFORE \u{2014} vanilla header byte", x as i32, y as i32, 16.0, ink);
    y += 32;
    draw_text(
        &mut img,
        &fonts.sans,
        &format!("Level 105 \u{2014} {sprite_count} sprites (LM 3.00 limit: {MAX_SPRITES_LM300} for non-SA1 ROMs)"),
        x as i32,
        y as i32,
        13.0,
        gray,
    );
    y += 30;
    y = draw_combo(&mut img, &fonts, x, y, "Sprite memory setting", &format!("0x{:02X}", vanilla.sprite_memory()), ink);
    y = draw_check(
        &mut img,
        &fonts,
        x,
        y,
        "Disable Layer 2 interaction",
        vanilla.disable_layer2_interaction(),
        false,
        ink,
    );
    y = draw_check(&mut img, &fonts, x, y, "Sprite buoyancy", vanilla.sprite_buoyancy(), false, ink);
    y += 8;
    draw_text(&mut img, &fonts.sans_bold, "Lunar Magic 3.00 options", x as i32, y as i32, 14.0, ink);
    y += 28;
    y = draw_combo(&mut img, &fonts, x, y, "Sprite vertical spawning range", SpawnRange::Normal.label(), ink);
    y = draw_check(&mut img, &fonts, x, y, "Smart spawning", false, false, ink);
    y += 10;
    draw_text(
        &mut img,
        &fonts.mono,
        &format!("Raw header byte: 0x{:02X}", vanilla.as_byte()),
        x as i32,
        y as i32,
        14.0,
        gray,
    );
    y += 28;
    draw_text(&mut img, &fonts.sans, &decode_line(&vanilla), x as i32, y as i32, 13.0, gray);

    // ---- Right panel: AFTER (buoyancy toggled + LM 3.00 options) ----
    let x = 624u32;
    let mut y = 76u32;
    draw_text(&mut img, &fonts.sans_bold, "AFTER \u{2014} dialog edits, saved to ROM", x as i32, y as i32, 16.0, ink);
    y += 32;
    draw_text(
        &mut img,
        &fonts.sans,
        &format!("Level 105 \u{2014} {sprite_count} sprites (LM 3.00 limit: {MAX_SPRITES_LM300} for non-SA1 ROMs)"),
        x as i32,
        y as i32,
        13.0,
        gray,
    );
    y += 30;
    y = draw_combo(&mut img, &fonts, x, y, "Sprite memory setting", &format!("0x{:02X}", edited.sprite_memory()), ink);
    y = draw_check(
        &mut img,
        &fonts,
        x,
        y,
        "Disable Layer 2 interaction",
        edited.disable_layer2_interaction(),
        false,
        ink,
    );
    y = draw_check(&mut img, &fonts, x, y, "Sprite buoyancy", edited.sprite_buoyancy(), true, ink);
    y += 8;
    draw_text(&mut img, &fonts.sans_bold, "Lunar Magic 3.00 options", x as i32, y as i32, 14.0, ink);
    y += 28;
    y = draw_combo(&mut img, &fonts, x, y, "Sprite vertical spawning range", demo_ext.spawn_range.label(), ink);
    y = draw_check(&mut img, &fonts, x, y, "Smart spawning", demo_ext.smart_spawning, true, ink);
    y += 10;
    draw_text(
        &mut img,
        &fonts.mono,
        &format!("Raw header byte: 0x{:02X} (unsaved)", edited.as_byte()),
        x as i32,
        y as i32,
        14.0,
        gray,
    );
    y += 28;
    draw_text(&mut img, &fonts.sans, &decode_line(&edited), x as i32, y as i32, 13.0, gray);

    // ---- Captions ----
    let cy = 640u32;
    draw_text(
        &mut img,
        &fonts.sans,
        &format!(
            "Edit round-trip verified on the real ROM: 0x{:02X} \u{2192} 0x{:02X}, sprite stream byte-identical;",
            vanilla.as_byte(),
            edited.as_byte()
        ),
        24,
        cy as i32,
        14.0,
        green,
    );
    draw_text(
        &mut img,
        &fonts.sans,
        "LM 3.00 options round-trip through the editor-native SMWSPRH1 RATS block (Normal+off = default, stored nowhere).",
        24,
        (cy + 26) as i32,
        13.0,
        gray,
    );
    draw_text(
        &mut img,
        &fonts.sans,
        "Bit layout verified in SMWDisX bank_05.asm (AND #$3F \u{2192} $1692 sprite memory; AND #$C0 \u{2192} $190E buoyancy/L2).",
        24,
        (cy + 50) as i32,
        13.0,
        gray,
    );
    draw_text(
        &mut img,
        &fonts.sans,
        "Honest limit: the two LM 3.00 options need LM 3.00+'s sprite engine to affect gameplay; on a stock ROM they are stored intent.",
        24,
        (cy + 74) as i32,
        13.0,
        gray,
    );
    draw_text(
        &mut img,
        &fonts.sans,
        "Mock window chrome \u{2014} the bytes, the edit, and both re-parse checks are produced by smwe_rom from the ROM.",
        24,
        (cy + 98) as i32,
        13.0,
        gray,
    );

    img.save(output)?;
    println!("wrote {output}");
    Ok(())
}
