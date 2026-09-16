//! Headless mock screenshot of the new SA-1 / ExLoROM / ExHiROM mapper support.
//!
//! egui can't render headless, so this composes an honest mock panel: every
//! number on screen is real — produced from synthetic in-memory ROMs by the
//! same code the editor uses (`RomInternalHeader::parse`,
//! `smwe_emu::rom::detect_mapper`, and the new `try_from_exlorom` /
//! `try_from_exhirom` address converters). Only the window chrome is drawn
//! rather than real egui widgets.
//!
//! ```sh
//! cargo run --bin render_sa1_mapper -- --out=docs/screenshots/sa1-mapper.png
//! ```

use ab_glyph::{Font, FontRef, Glyph, Point, PxScale, ScaleFont};
use image::{Rgb, RgbImage};
use smw_editor::render_util::{fill_rect, rect_border};
use smwe_emu::rom::{detect_mapper, Mapper};
use smwe_rom::{
    internal_header::RomInternalHeader,
    snes_utils::{
        addr::{AddrPc, AddrSnes},
        rom::Rom,
    },
};

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

#[allow(clippy::too_many_arguments)]
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

/// Synthetic in-memory ROM with a valid internal header (consistent
/// checksum/complement pair). No real ROM data is involved.
fn synthetic_rom(header_base: usize, name: &str, map_mode: u8, rom_type: u8) -> Vec<u8> {
    let mut buf = vec![0u8; 0x10000];
    let mut name_field = [b' '; 21];
    let name_bytes = name.as_bytes();
    name_field[..name_bytes.len()].copy_from_slice(name_bytes);
    buf[header_base..header_base + 21].copy_from_slice(&name_field);
    buf[header_base + 0x15] = map_mode;
    buf[header_base + 0x16] = rom_type;
    let checksum: u16 = 0x1234;
    buf[header_base + 0x1C..header_base + 0x1E].copy_from_slice(&(!checksum).to_le_bytes());
    buf[header_base + 0x1E..header_base + 0x20].copy_from_slice(&checksum.to_le_bytes());
    buf
}

struct Card {
    title:       &'static str,
    header_base: usize,
    map_mode:    u8,
    rom_type:    u8,
    /// (SNES addr, expected PC) sample mappings, computed through the real converters.
    samples:     Vec<(u32, u32)>,
}

fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let output = args.iter().find_map(|a| a.strip_prefix("--out=")).unwrap_or("docs/screenshots/sa1-mapper.png");

    // ---- Real data path: synthetic ROMs through the real parser + mapper ----
    let cards = [
        Card {
            title:       "SA-1 LoROM (Mode $23)",
            header_base: 0x7FC0,
            map_mode:    0x23,
            rom_type:    0x34,
            samples:     vec![
                (0x008000, AddrPc::try_from_lorom(AddrSnes(0x008000)).unwrap().0),
                (0x7DFFFF, AddrPc::try_from_lorom(AddrSnes(0x7DFFFF)).unwrap().0),
            ],
        },
        Card {
            title:       "ExLoROM (Mode $22)",
            header_base: 0x7FC0,
            map_mode:    0x22,
            rom_type:    0x02,
            samples:     vec![
                (0x008000, AddrPc::try_from_exlorom(AddrSnes(0x008000)).unwrap().0),
                (0x808000, AddrPc::try_from_exlorom(AddrSnes(0x808000)).unwrap().0),
                (0xFFFFFF, AddrPc::try_from_exlorom(AddrSnes(0xFFFFFF)).unwrap().0),
            ],
        },
        Card {
            title:       "ExHiROM (Mode $24)",
            header_base: 0xFFC0,
            map_mode:    0x24,
            rom_type:    0x02,
            samples:     vec![
                (0xC00000, AddrPc::try_from_exhirom(AddrSnes(0xC00000)).unwrap().0),
                (0x800000, AddrPc::try_from_exhirom(AddrSnes(0x800000)).unwrap().0),
                (0xBFFFFF, AddrPc::try_from_exhirom(AddrSnes(0xBFFFFF)).unwrap().0),
            ],
        },
    ];

    let sans = load_font(SANS_CANDIDATES)?;
    let sans_bold = load_font(SANS_BOLD_CANDIDATES)?;
    let mono = load_font(MONO_CANDIDATES)?;

    // ---- Compose the mock panel ----
    let (w, h) = (760u32, 830u32);
    let mut img = RgbImage::new(w, h);
    let bg = Rgb([0x1B, 0x1D, 0x20]);
    let panel = Rgb([0x25, 0x28, 0x2C]);
    let titlebar = Rgb([0x12, 0x14, 0x16]);
    let ink = Rgb([0xE8, 0xE8, 0xE8]);
    let dim = Rgb([0xA8, 0xA8, 0xA8]);
    let accent = Rgb([0x4D, 0x9F, 0xFF]);
    let green = Rgb([0x3D, 0xB0, 0x4E]);
    for p in img.pixels_mut() {
        *p = bg;
    }

    let (dx, dy, dw) = (30u32, 24u32, 700u32);
    let dh = 782u32;
    fill_rect(&mut img, dx, dy, dw, dh, panel);
    rect_border(&mut img, dx, dy, dw, dh, Rgb([0x4A, 0x4E, 0x54]));
    fill_rect(&mut img, dx, dy, dw, 40, titlebar);
    draw_text(
        &mut img,
        &sans_bold,
        "Mapper support: SA-1 / ExLoROM / ExHiROM",
        (dx + 16) as i32,
        (dy + 11) as i32,
        17.0,
        ink,
    );
    draw_text(
        &mut img,
        &sans,
        "headless mock \u{2014} every value below comes from the real header parser + mapper",
        (dx + 16) as i32,
        (dy + 44) as i32,
        12.0,
        dim,
    );

    let mut y = dy + 76u32;
    for card in &cards {
        let buf = synthetic_rom(card.header_base, "MAPPER TEST", card.map_mode, card.rom_type);
        let rom = Rom::new(buf.clone()).unwrap();
        let header = RomInternalHeader::parse(&rom).unwrap();
        // Sanity: the fixture really is what the card claims.
        assert_eq!(header.map_mode.as_u8(), card.map_mode);
        let mapper: Mapper = detect_mapper(&buf);
        let chip: String = header.rom_type.to_string();

        let ch = 156 + 22 * (card.samples.len() as u32);
        fill_rect(&mut img, dx + 16, y, dw - 32, ch, Rgb([0x2B, 0x2F, 0x34]));
        rect_border(&mut img, dx + 16, y, dw - 32, ch, Rgb([0x4A, 0x4E, 0x54]));
        let mut cy = y + 16;
        draw_text(&mut img, &sans_bold, card.title, (dx + 32) as i32, cy as i32, 15.0, accent);
        cy += 30;
        draw_text(
            &mut img,
            &mono,
            &format!(
                "header @ PC {:#06X} \u{00B7} map mode ${:02X} \u{2192} {} \u{00B7} chip ${:02X} \u{2192} {}",
                card.header_base, card.map_mode, header.map_mode, card.rom_type, chip
            ),
            (dx + 32) as i32,
            cy as i32,
            12.5,
            ink,
        );
        cy += 26;
        draw_text(
            &mut img,
            &mono,
            &format!("emulator mapper \u{2192} {mapper:?}"),
            (dx + 32) as i32,
            cy as i32,
            12.5,
            green,
        );
        cy += 24;
        draw_text(&mut img, &sans, "address samples (SNES \u{2192} PC):", (dx + 32) as i32, cy as i32, 12.5, dim);
        cy += 24;
        for (snes, pc) in &card.samples {
            draw_text(
                &mut img,
                &mono,
                &format!("${snes:06X} \u{2192} PC {pc:#06X}"),
                (dx + 48) as i32,
                cy as i32,
                12.5,
                ink,
            );
            cy += 22;
        }
        y += ch + 14;
    }

    draw_text(
        &mut img,
        &sans,
        "SA-1 packs map with their base LoROM/HiROM bus layout (S-CPU view) \u{00B7} Ex windows replace the mirrors",
        (dx + 16) as i32,
        y as i32,
        12.0,
        dim,
    );

    img.save(output)?;
    println!("wrote {output}");
    Ok(())
}
