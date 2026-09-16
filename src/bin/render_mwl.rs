//! Headless mock screenshot of the `.mwl` import/export UI.
//!
//! egui can't render headless, so this composes an honest mock of the level
//! editor's toolbar row: the real Export/Import buttons the PR adds, plus a
//! status-bar line showing the real result string the export path produces.
//! The section table on the right is real — produced by exporting vanilla
//! level 0x105 from the ROM with the exact `smwe_rom::mwl::export_level`
//! function the UI calls. Only the window chrome and button frames are drawn
//! rather than real egui widgets.
//!
//! ```sh
//! cargo run --bin render_mwl -- --out=docs/screenshots/mwl-import-export.png --rom=smw.smc
//! ```

use ab_glyph::{Font, FontRef, Glyph, Point, PxScale, ScaleFont};
use image::{Rgb, RgbImage};
use smwe_rom::mwl;

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
    let output = args.iter().find_map(|a| a.strip_prefix("--out=")).unwrap_or("docs/screenshots/mwl-import-export.png");
    let rom_path = args
        .iter()
        .find_map(|a| a.strip_prefix("--rom="))
        .or_else(|| args.iter().skip(1).find(|a| !a.starts_with("--")).map(|a| a.as_str()))
        .unwrap_or("smw.smc");

    let sans = load_font(SANS_CANDIDATES)?;
    let sans_bold = load_font(SANS_BOLD_CANDIDATES)?;

    // Real data path: export vanilla level 0x105 with the exact codec the UI calls.
    let rom = smwe_rom::SmwRom::from_file(rom_path)?;
    let level_num = 0x105u32;
    let mwl = mwl::export_level(&rom, level_num)?;
    let encoded = mwl.encode()?;
    let status =
        format!("Exported level {:03X} \u{2192} level-{:03X}.mwl ({} bytes)", level_num, level_num, encoded.len());

    let section_names = [
        "Level info",
        "Layer 1",
        "Layer 2",
        "Sprites",
        "Palette",
        "Secondary entrances",
        "ExAnimation",
        "ExGFX/bypass",
    ];

    let (w, h) = (1180u32, 720u32);
    let mut img = RgbImage::new(w, h);
    let bg = Rgb([0xF2, 0xF2, 0xF2]);
    let ink = Rgb([0x1A, 0x1A, 0x1A]);
    let gray = Rgb([0x66, 0x66, 0x66]);
    let dark = Rgb([0x2B, 0x2B, 0x2B]);
    for p in img.pixels_mut() {
        *p = bg;
    }

    // Title bar.
    fill_rect(&mut img, 0, 0, w, 52, dark);
    draw_text(
        &mut img,
        &sans_bold,
        "Level Editor \u{2014} .mwl import/export (headless mock; section table is real ROM output)",
        24,
        15,
        19.0,
        Rgb([0xFF, 0xFF, 0xFF]),
    );

    // ---- Left: toolbar row mock ----
    let lx = 24u32;
    let mut y = 84u32;
    draw_text(&mut img, &sans_bold, "Toolbar (row 1)", lx as i32, y as i32, 17.0, ink);
    y += 36;
    // Mock icon buttons: save, reload, export, import.
    // (Glyphs stand in for the real phosphor icons the egui toolbar uses.)
    let labels = ["S", "\u{21BB}", "\u{2B07}", "\u{2B06}"];
    let tips = ["Save", "Reload", "Export", "Import"];
    let mut bx = lx;
    for (i, (glyph, tip)) in labels.iter().zip(tips.iter()).enumerate() {
        let highlight = i >= 2;
        fill_rect(&mut img, bx, y, 44, 34, if highlight { Rgb([0xD6, 0xE8, 0xFA]) } else { Rgb([0xFF, 0xFF, 0xFF]) });
        rect_border(&mut img, bx, y, 44, 34, Rgb([0x99, 0x99, 0x99]));
        draw_text(&mut img, &sans, glyph, (bx + 14) as i32, (y + 7) as i32, 17.0, ink);
        draw_text(&mut img, &sans, tip, (bx + 4) as i32, (y + 40) as i32, 11.0, gray);
        bx += 56;
    }
    y += 78;
    draw_text(
        &mut img,
        &sans,
        "Hover: \u{201C}Export level to Lunar Magic .mwl file\u{201D} /",
        lx as i32,
        y as i32,
        14.0,
        gray,
    );
    y += 22;
    draw_text(
        &mut img,
        &sans,
        "\u{201C}Import Lunar Magic .mwl file into this level\u{201D}",
        lx as i32,
        y as i32,
        14.0,
        gray,
    );
    y += 44;

    // Status bar mock with the real status string.
    draw_text(&mut img, &sans_bold, "Status bar", lx as i32, y as i32, 17.0, ink);
    y += 36;
    fill_rect(&mut img, lx, y, 520, 40, Rgb([0xFF, 0xFF, 0xFF]));
    rect_border(&mut img, lx, y, 520, 40, Rgb([0x99, 0x99, 0x99]));
    draw_text(&mut img, &sans, &status, (lx + 10) as i32, (y + 11) as i32, 13.0, Rgb([0x1E, 0x7A, 0x1E]));

    // ---- Right: real section table ----
    let rx = 600u32;
    let mut ry = 84u32;
    draw_text(
        &mut img,
        &sans_bold,
        &format!("level-{:03X}.mwl \u{2014} real export of vanilla level {:03X}", level_num, level_num),
        rx as i32,
        ry as i32,
        17.0,
        ink,
    );
    ry += 30;
    draw_text(
        &mut img,
        &sans,
        &format!("Header: \"LM\" version {:#06X} \u{00B7} {} sections", mwl.version, mwl::SECTION_COUNT),
        rx as i32,
        ry as i32,
        14.0,
        gray,
    );
    ry += 34;

    // Table header.
    draw_text(&mut img, &sans_bold, "Section", rx as i32, ry as i32, 14.0, ink);
    draw_text(&mut img, &sans_bold, "Bytes", (rx + 220) as i32, ry as i32, 14.0, ink);
    draw_text(&mut img, &sans_bold, "Descriptor / source", (rx + 300) as i32, ry as i32, 14.0, ink);
    ry += 24;
    fill_rect(&mut img, rx, ry, 556, 1, Rgb([0x99, 0x99, 0x99]));
    ry += 10;

    for (i, name) in section_names.iter().enumerate() {
        let section = &mwl.sections[i];
        draw_text(&mut img, &sans, &format!("{i}: {name}"), rx as i32, ry as i32, 14.0, ink);
        draw_text(
            &mut img,
            &sans,
            &format!("{}", section.len()),
            (rx + 220) as i32,
            ry as i32,
            14.0,
            if section.is_empty() { gray } else { ink },
        );
        let detail = match i {
            mwl::SECTION_LEVEL_INFO => {
                let info = mwl::decode_level_info(section).unwrap();
                format!("level {:03X}", info.level_num)
            }
            mwl::SECTION_LAYER1 | mwl::SECTION_LAYER2 | mwl::SECTION_SPRITES => {
                let (d, s, _) = mwl::decode_section(i, section).unwrap();
                format!("desc={d:#04X} src=${s:06X}")
            }
            _ => "(empty in v1)".to_string(),
        };
        draw_text(&mut img, &sans, &detail, (rx + 300) as i32, ry as i32, 14.0, gray);
        ry += 26;
    }

    ry += 12;
    let info = mwl::decode_level_info(&mwl.sections[mwl::SECTION_LEVEL_INFO]).unwrap();
    draw_text(
        &mut img,
        &sans,
        &format!(
            "Secondary header bytes: {:02X} {:02X} {:02X} {:02X}",
            info.secondary.0[0], info.secondary.0[1], info.secondary.0[2], info.secondary.0[3]
        ),
        rx as i32,
        ry as i32,
        14.0,
        gray,
    );

    img.save(output)?;
    println!("wrote {output}");
    Ok(())
}
