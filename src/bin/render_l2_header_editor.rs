//! Headless mock screenshot of the new "Layer 2 Header" section in the
//! level editor's Level Header window.
//!
//! egui can't render headless, so this composes an honest mock: the 5 header
//! bytes are real — parsed from the ROM by the same code the UI uses
//! (`smwe_rom::level::Level::parse` → `Layer2Data::Objects { header, .. }`) —
//! and the before/after edit is verified by writing the edited bytes into a
//! scratch ROM copy and re-parsing (the same write `save_to_rom` performs).
//! Only the window chrome (title bar, sliders) is drawn, not real egui
//! widgets.
//!
//! Shows level 0x009 (a vanilla level with Layer 2 objects) before and after
//! changing header byte 0 from its vanilla value.
//!
//! ```sh
//! cargo run --bin render_l2_header_editor -- --out=docs/screenshots/l2-header-editing.png --rom=/path/to/smw.smc
//! ```

use ab_glyph::{Font, FontRef, Glyph, Point, PxScale, ScaleFont};
use image::{Rgb, RgbImage};
use smwe_rom::level::{Layer2Data, Level, LAYER2_HEADER_SIZE};
use smwe_rom::snes_utils::addr::{AddrPc, AddrSnes};
use smwe_rom::snes_utils::rom::Rom;
use smwe_rom::SmwRom;

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
    mono: FontRef<'static>,
    sans: FontRef<'static>,
    sans_bold: FontRef<'static>,
}

/// Draw one line of text; returns the advance width in px.
fn draw_text(
    img: &mut RgbImage,
    font: &FontRef,
    text: &str,
    x: i32,
    y: i32,
    px: f32,
    color: Rgb<u8>,
) -> i32 {
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

/// Draw one hex byte slider mock; `highlight` marks the edited byte.
fn draw_byte_row(
    img: &mut RgbImage,
    fonts: &Fonts,
    x: u32,
    y: u32,
    index: usize,
    byte: u8,
    highlight: bool,
    ink: Rgb<u8>,
) {
    draw_text(img, &fonts.sans, &format!("Byte {index}:"), x as i32, (y + 7) as i32, 14.0, ink);
    let fx = x + 90;
    let (w, h) = (200u32, 32u32);
    let fill = if highlight { Rgb([0xFF, 0xF2, 0xC0]) } else { Rgb([0xFF, 0xFF, 0xFF]) };
    fill_rect(img, fx, y, w, h, fill);
    rect_border(img, fx, y, w, h, Rgb([0x99, 0x99, 0x99]));
    draw_text(img, &fonts.mono, &format!("{byte:02X}"), (fx + 12) as i32, (y + 6) as i32, 15.0, ink);
    // Fake slider thumb at the right edge.
    fill_rect(img, fx + w - 26, y + 4, 22, h - 8, Rgb([0xCC, 0xCC, 0xCC]));
    rect_border(img, fx + w - 26, y + 4, 22, h - 8, Rgb([0x88, 0x88, 0x88]));
}

use smw_editor::render_util::{fill_rect, rect_border};
fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let output = args
        .iter()
        .find_map(|a| a.strip_prefix("--out="))
        .unwrap_or("docs/screenshots/l2-header-editing.png");
    let rom_path = args
        .iter()
        .find_map(|a| a.strip_prefix("--rom="))
        .or_else(|| args.iter().skip(1).find(|a| !a.starts_with("--")).map(|a| a.as_str()))
        .unwrap_or("smw.smc");

    let fonts = Fonts {
        mono: load_font(MONO_CANDIDATES)?,
        sans: load_font(SANS_CANDIDATES)?,
        sans_bold: load_font(SANS_BOLD_CANDIDATES)?,
    };

    // Real data path, identical to the UI: parse the ROM, take level 0x009's
    // Layer 2 object header.
    let rom = SmwRom::from_file(rom_path)?;
    let Layer2Data::Objects { header, objects } = &rom.levels[0x9].layer2 else {
        anyhow::bail!("level 009 has no Layer 2 objects in this ROM");
    };
    let vanilla = *header;
    anyhow::ensure!(vanilla.len() == LAYER2_HEADER_SIZE);

    // AFTER: the edit the screenshot shows — byte 0 bumped by one.
    let mut edited = vanilla;
    edited[0] = edited[0].wrapping_add(1);
    anyhow::ensure!(edited != vanilla);

    // Prove the write path: patch the edited header into a scratch ROM copy
    // at the Layer 2 pointer (exactly what save_to_rom writes), re-parse,
    // and confirm the new header comes back with the object stream intact.
    let mut bytes = rom.rom.bytes().to_vec();
    let tbl_pc = AddrPc::try_from_lorom(AddrSnes(0x05E600 + 0x9 * 3))?.as_index() as u32;
    let s = &bytes[tbl_pc as usize..tbl_pc as usize + 3];
    let l2_ptr = u32::from_le_bytes([s[0], s[1], s[2], 0]);
    let data_pc = AddrPc::try_from_lorom(AddrSnes(l2_ptr))?.as_index() as usize;
    bytes[data_pc..data_pc + LAYER2_HEADER_SIZE].copy_from_slice(&edited);
    let reparsed = Level::parse(&Rom::new(bytes)?, 0x9)?;
    let Layer2Data::Objects { header: header2, objects: objects2 } = &reparsed.layer2 else {
        anyhow::bail!("level 009 lost its Layer 2 objects after the header edit");
    };
    anyhow::ensure!(*header2 == edited, "edited header did not round-trip");
    anyhow::ensure!(objects.as_bytes() == objects2.as_bytes(), "object stream changed");

    // ---- Compose the mock window ----
    let (w, h) = (1200u32, 700u32);
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
        "Level Editor \u{2014} Level Header window, Layer 2 Header section (headless mock; bytes are real ROM output)",
        24,
        15,
        19.0,
        Rgb([0xFF, 0xFF, 0xFF]),
    );

    let panels: [(&str, [u8; 5], Option<usize>); 2] = [
        ("BEFORE \u{2014} vanilla header (copied verbatim before this change)", vanilla, None),
        ("AFTER \u{2014} byte 0 edited, written back on save", edited, Some(0)),
    ];
    let panel_x = [24u32, 624u32];
    for (pi, (title, hdr, hl)) in panels.iter().enumerate() {
        let x = panel_x[pi];
        let mut y = 76u32;
        draw_text(&mut img, &fonts.sans_bold, title, x as i32, y as i32, 16.0, ink);
        y += 34;
        draw_text(&mut img, &fonts.sans, "Level 009 \u{2014} Layer 2 objects", x as i32, y as i32, 13.0, gray);
        y += 30;
        for (i, b) in hdr.iter().enumerate() {
            draw_byte_row(&mut img, &fonts, x, y, i, *b, hl == &Some(i), ink);
            y += 42;
        }
        y += 6;
        let joined = hdr.iter().map(|b| format!("{b:02X}")).collect::<Vec<_>>().join(" ");
        draw_text(&mut img, &fonts.mono, &format!("[ {joined} ]"), x as i32, y as i32, 14.0, gray);
    }

    // Caption: what the section means + honesty note.
    let cy = 560u32;
    let van = vanilla.iter().map(|b| format!("{b:02X}")).collect::<Vec<_>>().join(" ");
    let edt = edited.iter().map(|b| format!("{b:02X}")).collect::<Vec<_>>().join(" ");
    draw_text(
        &mut img,
        &fonts.sans,
        &format!("Edit round-trip verified on the real ROM: [ {van} ] \u{2192} [ {edt} ], object stream byte-identical."),
        24,
        cy as i32,
        14.0,
        green,
    );
    draw_text(
        &mut img,
        &fonts.sans,
        "The game skips these 5 bytes (SMWDisX bank_05.asm: +5 \u{201C}to ignore Layer 2's header\u{201D});",
        24,
        (cy + 28) as i32,
        13.0,
        gray,
    );
    draw_text(
        &mut img,
        &fonts.sans,
        "in the vanilla ROM they mirror the level's primary header. Lunar Magic copies them verbatim \u{2014} now they're editable.",
        24,
        (cy + 50) as i32,
        13.0,
        gray,
    );
    draw_text(
        &mut img,
        &fonts.sans,
        "Mock window chrome \u{2014} the bytes, the edit, and the re-parse check are produced by smwe_rom from the ROM.",
        24,
        (cy + 76) as i32,
        13.0,
        gray,
    );

    img.save(output)?;
    println!("wrote {output} ({w}x{h})");
    Ok(())
}
