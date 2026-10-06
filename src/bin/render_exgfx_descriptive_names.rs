//! Headless screenshot of the "Allow Descriptive GFX File Names" feature
//! (Lunar Magic v3.70 parity).
//!
//! egui can't render headless, so this composes an honest mock: a table of
//! REAL `smwe_rom::gfx_filename::parse_gfx_filename` results over a mix of
//! strict, descriptive, and rejected names; a real ExGFX file inserted into
//! an in-memory expanded copy of the real ROM via the real
//! `smwe_rom::exgfx::ExGfxData` path (the real ROM is never modified), with
//! the descriptive suffix kept via the real `ExGfxFileNames` store — the
//! mock ExGFX Manager file-list row shows the real index, the real tile
//! count, and the real remembered name. The insert status line is quoted
//! verbatim from the `UiLevelEditor` code path.
//!
//! ```sh
//! cargo run --bin render_exgfx_descriptive_names -- --out=docs/screenshots/exgfx-descriptive-names.png --rom=smw.smc
//! ```

use ab_glyph::{Font, FontRef, Point, PxScale, ScaleFont};
use image::{Rgb, RgbImage};
use smwe_rom::{
    exgfx::{ExGfxData, EXGFX_FILE_BYTES},
    gfx_filename::{parse_gfx_filename, GfxFileKind, ParsedGfxFileName},
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
        prev = Some(id);
        let glyph = id.with_scale_and_position(px, Point { x: caret_x, y: baseline });
        if let Some(out) = scaled.outline_glyph(glyph) {
            let bb = out.px_bounds();
            out.draw(|gx, gy, v| {
                let px_x = bb.min.x as i32 + gx as i32;
                let px_y = bb.min.y as i32 + gy as i32;
                if px_x >= 0 && px_y >= 0 && (px_x as u32) < img.width() && (px_y as u32) < img.height() {
                    let dst = img.get_pixel(px_x as u32, px_y as u32);
                    let a = v;
                    let r = (color[0] as f32 * a + dst[0] as f32 * (1.0 - a)) as u8;
                    let g = (color[1] as f32 * a + dst[1] as f32 * (1.0 - a)) as u8;
                    let b = (color[2] as f32 * a + dst[2] as f32 * (1.0 - a)) as u8;
                    img.put_pixel(px_x as u32, px_y as u32, Rgb([r, g, b]));
                }
            });
        }
        caret_x += scaled.h_advance(id);
    }
}

fn describe_parse(p: &Option<ParsedGfxFileName>) -> String {
    match p {
        None => "rejected".to_string(),
        Some(parsed) => {
            let kind = match parsed.kind {
                GfxFileKind::Vanilla => "GFX",
                GfxFileKind::ExGfx => "ExGFX",
            };
            match &parsed.descriptive_text {
                None => format!("{kind} {:#04X}, strict", parsed.index),
                Some(t) => format!("{kind} {:#04X}, “{t}”", parsed.index),
            }
        }
    }
}

fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let output =
        args.iter().find_map(|a| a.strip_prefix("--out=")).unwrap_or("docs/screenshots/exgfx-descriptive-names.png");
    let rom_path = args
        .iter()
        .find_map(|a| a.strip_prefix("--rom="))
        .or_else(|| args.iter().skip(1).find(|a| !a.starts_with("--")).map(|a| a.as_str()))
        .unwrap_or("smw.smc");

    let sans = load_font(SANS_CANDIDATES)?;
    let sans_bold = load_font(SANS_BOLD_CANDIDATES)?;

    // Real ROM, expanded in memory (the real ROM is never modified).
    let rom_bytes = std::fs::read(rom_path)?;
    let header_offset = if rom_bytes.len() % 0x400 == 0x200 { 0x200 } else { 0 };
    let (smc_header, body) = rom_bytes.split_at(header_offset);
    let expanded = smwe_rom::rom_expansion::expand_rom(
        &smwe_rom::snes_utils::rom::Rom::new(body.to_vec()).map_err(|e| anyhow::anyhow!("{e:?}"))?,
        0x40_0000,
    )
    .map_err(|e| anyhow::anyhow!("{e:?}"))?;
    let mut scratch = smc_header.to_vec();
    scratch.extend_from_slice(expanded.bytes());

    // Real insert path, then re-parse like the editor does on project load.
    let file_name = "ExGFX80Mario tiles.bin";
    let parsed = parse_gfx_filename(file_name).expect("descriptive name must parse");
    assert_eq!(parsed.index, 0x80);
    assert_eq!(parsed.descriptive_text.as_deref(), Some("Mario tiles"));
    let raw: Vec<u8> = (0..EXGFX_FILE_BYTES).map(|i| (i & 0xFF) as u8).collect();
    let mut data = ExGfxData::parse(&scratch);
    data.insert_raw(parsed.index, raw).map_err(|e| anyhow::anyhow!("{e:?}"))?;
    data.write_to_rom(&mut scratch, header_offset).map_err(|e| anyhow::anyhow!("{e:?}"))?;
    let data = ExGfxData::parse(&scratch);
    let file = data.files.get(&0x80).expect("ExGFX80 must parse back");
    let tile_count = file.tiles.len();
    assert_eq!(tile_count, 0x400, "a 32 KiB 4bpp file holds 1024 tiles");
    // The descriptive suffix is editor metadata — remember it the same way
    // the manager does (per-user store, in-memory here).
    let mut names = smw_editor::exgfx_file_names::ExGfxFileNames::default();
    names.set(0x80, parsed.descriptive_text.as_deref());
    let remembered = names.get(0x80).unwrap_or("").to_string();

    // The status line, quoted verbatim from `pick_exgfx_insert`.
    let status_line = format!(
        "Read {file_name} ({EXGFX_FILE_BYTES} bytes, {tile_count} tiles) → file index ExGFX{:03X}. \
         Pick the file index, then Insert.",
        parsed.index
    );

    // ── Compose the mock ───────────────────────────────────────────────
    const W: u32 = 760;
    const H: u32 = 640;
    let mut img = RgbImage::from_pixel(W, H, Rgb([30, 30, 38]));
    for y in 0..44u32 {
        for x in 0..W {
            img.put_pixel(x, y, Rgb([42, 42, 54]));
        }
    }
    draw_text(&mut img, &sans_bold, "Allow Descriptive GFX File Names (LM v3.70)", 16, 8, 21.0, Rgb([235, 235, 245]));
    draw_text(
        &mut img,
        &sans,
        "Options > “Allow Descriptive GFX File Names”: ON (LM’s default).",
        16,
        52,
        13.0,
        Rgb([170, 200, 170]),
    );

    let mut y = 86u32;
    draw_text(
        &mut img,
        &sans_bold,
        "Real parser output (smwe_rom::gfx_filename::parse_gfx_filename):",
        16,
        y as i32,
        14.0,
        Rgb([235, 235, 245]),
    );
    y += 30;
    let names_table = [
        "ExGFX80.bin",
        "ExGFX80Mario tiles.bin",
        "ExGFXFFF.bin",
        "GFX0C.bin",
        "GFX12forest.bin",
        "ExGFX8.bin",
        "GFX80T.bin",
        "ExGFXZZ.bin",
    ];
    for n in names_table {
        let result = describe_parse(&parse_gfx_filename(n));
        let color = if result == "rejected" { Rgb([220, 130, 130]) } else { Rgb([170, 210, 170]) };
        draw_text(&mut img, &sans, n, 24, y as i32, 13.0, Rgb([235, 220, 160]));
        draw_text(&mut img, &sans, &result, 300, y as i32, 13.0, color);
        y += 24;
    }
    y += 14;

    draw_text(
        &mut img,
        &sans_bold,
        "ExGFX Manager — file list row (real insert into an in-memory expanded ROM):",
        16,
        y as i32,
        14.0,
        Rgb([235, 235, 245]),
    );
    y += 30;
    draw_text(&mut img, &sans_bold, "File", 24, y as i32, 13.0, Rgb([160, 160, 175]));
    draw_text(&mut img, &sans_bold, "Tiles", 300, y as i32, 13.0, Rgb([160, 160, 175]));
    y += 26;
    draw_text(&mut img, &sans, "ExGFX080", 24, y as i32, 14.0, Rgb([235, 220, 160]));
    draw_text(&mut img, &sans, &format!("“{remembered}”"), 130, y as i32, 13.0, Rgb([150, 150, 165]));
    draw_text(&mut img, &sans, &tile_count.to_string(), 300, y as i32, 14.0, Rgb([200, 200, 210]));
    y += 40;

    draw_text(
        &mut img,
        &sans_bold,
        "Insert status line (verbatim from the insert flow):",
        16,
        y as i32,
        14.0,
        Rgb([235, 235, 245]),
    );
    y += 28;
    // Wrap the status line over two drawn lines (fixed width for the mock).
    let first = &status_line[..status_line.find("Pick the file index").unwrap_or(status_line.len())];
    let rest = &status_line[first.len()..];
    draw_text(&mut img, &sans, first.trim_end(), 24, y as i32, 12.5, Rgb([200, 200, 210]));
    y += 24;
    draw_text(&mut img, &sans, rest.trim(), 24, y as i32, 12.5, Rgb([200, 200, 210]));
    y += 40;

    draw_text(
        &mut img,
        &sans,
        "With the option OFF, descriptively named files are refused — the manager tells the",
        16,
        y as i32,
        12.5,
        Rgb([150, 150, 165]),
    );
    y += 22;
    draw_text(
        &mut img,
        &sans,
        "user to rename to ExGFX###.bin or re-enable the option. Extract still defaults to",
        16,
        y as i32,
        12.5,
        Rgb([150, 150, 165]),
    );
    y += 22;
    draw_text(&mut img, &sans, "ExGFX###.bin (LM-compatible).", 16, y as i32, 12.5, Rgb([150, 150, 165]));

    img.save(output)?;
    println!("wrote {output} (parsed {file_name} -> 0x80 “{remembered}”, {tile_count} tiles)");
    Ok(())
}
