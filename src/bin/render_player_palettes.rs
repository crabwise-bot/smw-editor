//! Headless mock screenshot of the Palette Editor's new "Player Colors" section.
//!
//! egui can't render headless, so this composes an honest mock of the section:
//! the four palette rows are the real `PlayerColors` bytes from the ROM,
//! read through the exact code path the editor uses
//! (`smwe_rom::player_palette::PlayerPalettes::parse`), and the selected
//! swatch/picker show a real color from that data. Only the window chrome,
//! tab row, and color-picker frame are drawn rather than real egui widgets.
//!
//! ```sh
//! cargo run --bin render_player_palettes -- --out=docs/screenshots/player-palettes.png --rom=smw.smc
//! ```

use ab_glyph::{Font, FontRef, Glyph, Point, PxScale, ScaleFont};
use image::{Rgb, RgbImage};
use smwe_rom::player_palette::{PlayerPalettes, PLAYER_PALETTE_COLORS, PLAYER_PALETTE_NAMES};

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

fn fill_rect(img: &mut RgbImage, x: u32, y: u32, w: u32, h: u32, c: Rgb<u8>) {
    for yy in y..(y + h).min(img.height()) {
        for xx in x..(x + w).min(img.width()) {
            img.put_pixel(xx, yy, c);
        }
    }
}

fn rect_border(img: &mut RgbImage, x: u32, y: u32, w: u32, h: u32, c: Rgb<u8>) {
    fill_rect(img, x, y, w, 1, c);
    fill_rect(img, x, y + h - 1, w, 1, c);
    fill_rect(img, x, y, 1, h, c);
    fill_rect(img, x + w - 1, y, 1, h, c);
}

fn draw_text(
    img: &mut RgbImage,
    font: &FontRef,
    text: &str,
    x: i32,
    y: i32,
    px: f32,
    color: Rgb<u8>,
) {
    let scaled = font.as_scaled(PxScale::from(px));
    let mut caret_x = x as f32;
    let baseline = y as f32 + scaled.ascent();
    let mut prev = None;
    for ch in text.chars() {
        let id = font.glyph_id(ch);
        if let Some(p) = prev {
            caret_x += scaled.kern(p, id);
        }
        let glyph = Glyph {
            id,
            scale: PxScale::from(px),
            position: Point { x: caret_x, y: baseline },
        };
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

/// ABGR1555 word -> 8-bit RGB.
fn snesto_rgb(w: u16) -> Rgb<u8> {
    let r = ((w & 0x1F) as u32 * 255 / 31) as u8;
    let g = (((w >> 5) & 0x1F) as u32 * 255 / 31) as u8;
    let b = (((w >> 10) & 0x1F) as u32 * 255 / 31) as u8;
    Rgb([r, g, b])
}

fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let output = args
        .iter()
        .find_map(|a| a.strip_prefix("--out="))
        .unwrap_or("docs/screenshots/player-palettes.png");
    let rom_path = args
        .iter()
        .find_map(|a| a.strip_prefix("--rom="))
        .or_else(|| args.iter().skip(1).find(|a| !a.starts_with("--")).map(|a| a.as_str()))
        .unwrap_or("smw.smc");

    let sans = load_font(SANS_CANDIDATES)?;
    let sans_bold = load_font(SANS_BOLD_CANDIDATES)?;

    // Real data path: parse the ROM exactly like the editor's level-load does.
    let rom = smwe_rom::SmwRom::from_file(rom_path)?;
    let pp = PlayerPalettes::parse(&rom.rom)?;

    let (w, h) = (1180u32, 640u32);
    let mut img = RgbImage::new(w, h);
    let bg = Rgb([0xF2, 0xF2, 0xF2]);
    let ink = Rgb([0x1A, 0x1A, 0x1A]);
    let gray = Rgb([0x66, 0x66, 0x66]);
    for p in img.pixels_mut() {
        *p = bg;
    }

    // Title bar.
    fill_rect(&mut img, 0, 0, w, 52, Rgb([0x2B, 0x2B, 0x2B]));
    draw_text(
        &mut img,
        &sans_bold,
        "Palette Editor \u{2014} Player Colors (headless mock; swatches are the real ROM bytes)",
        24,
        15,
        19.0,
        Rgb([0xFF, 0xFF, 0xFF]),
    );

    let lx = 24u32;
    let mut y = 84u32;
    draw_text(&mut img, &sans_bold, "Player Colors (global \u{2014} SNES $00B2C8)", lx as i32, y as i32, 17.0, ink);
    y += 36;

    // Tab row (mock): four selectable labels, "Mario" selected.
    let mut tab_x = lx;
    for (i, name) in PLAYER_PALETTE_NAMES.iter().enumerate() {
        let tab_w = 130u32;
        if i == 0 {
            fill_rect(&mut img, tab_x, y, tab_w, 30, Rgb([0xD6, 0xE8, 0xFA]));
        }
        rect_border(&mut img, tab_x, y, tab_w, 30, Rgb([0x99, 0x99, 0x99]));
        draw_text(&mut img, &sans, name, (tab_x + 12) as i32, (y + 6) as i32, 14.0, ink);
        tab_x += tab_w + 8;
    }
    y += 48;

    // The four palette rows, real colors from the ROM.
    let cell = 34u32;
    for (p, name) in PLAYER_PALETTE_NAMES.iter().enumerate() {
        draw_text(&mut img, &sans, &format!("{name}:"), lx as i32, (y + 8) as i32, 14.0, gray);
        let sx = lx + 150;
        for c in 0..PLAYER_PALETTE_COLORS {
            let col = pp.palettes[p][c];
            let x = sx + c as u32 * cell;
            fill_rect(&mut img, x, y, cell, cell, snesto_rgb(col));
            rect_border(&mut img, x, y, cell, cell, Rgb([0x50, 0x50, 0x50]));
            // Selected swatch: Mario color 1 (the red shirt), white outline.
            if p == 0 && c == 1 {
                rect_border(&mut img, x - 2, y - 2, cell + 4, cell + 4, Rgb([0xFF, 0xFF, 0xFF]));
                rect_border(&mut img, x - 3, y - 3, cell + 6, cell + 6, Rgb([0x20, 0x20, 0x20]));
            }
        }
        y += cell + 16;
    }

    // Mock color picker for the selected swatch.
    y += 8;
    let sel = pp.palettes[0][1];
    draw_text(&mut img, &sans, "Mario color 1:", lx as i32, (y + 6) as i32, 15.0, ink);
    let pbx = lx + 140;
    fill_rect(&mut img, pbx, y, 120, 34, snesto_rgb(sel));
    rect_border(&mut img, pbx, y, 120, 34, Rgb([0x66, 0x66, 0x66]));
    draw_text(
        &mut img,
        &sans,
        &format!("ABGR1555 ${sel:04X} \u{2014} click the picker to edit, saves with Ctrl+S"),
        (pbx + 132) as i32,
        (y + 8) as i32,
        14.0,
        gray,
    );

    // Caption.
    let cy = 566u32;
    draw_text(
        &mut img,
        &sans,
        "Swatches read via smwe_rom::player_palette::PlayerPalettes::parse \u{2014} the exact",
        24,
        cy as i32,
        13.0,
        gray,
    );
    draw_text(
        &mut img,
        &sans,
        "call the editor makes on level load. Lunar Magic has no editor for these four",
        24,
        (cy + 22) as i32,
        13.0,
        gray,
    );
    draw_text(
        &mut img,
        &sans,
        "palettes (MarioGFXDMA copies the chosen one to CGRAM sprite row 8 at runtime).",
        24,
        (cy + 44) as i32,
        13.0,
        gray,
    );

    img.save(output)?;
    println!("wrote {output} ({w}x{h})");
    Ok(())
}
