//! Headless mock screenshot of the "Share Data Between Levels to Save Space"
//! dialog (File > Levels > Share Data Between Levels to Save Space...).
//!
//! egui can't render headless, so this composes an honest mock of the dialog:
//! every string on screen is real — the dialog description, the duplicate
//! groups from `find_duplicate_groups` on the real ROM, and the status line
//! from a real `share_data_between_levels` run on a scratch copy. Only the
//! window chrome (title bar, buttons) is drawn rather than real egui widgets.
//! The binary bails out if the post-share checksum is invalid.
//!
//! ```sh
//! cargo run --bin render_share_data -- --rom=smw.smc --out=docs/screenshots/share-data.png
//! ```

use ab_glyph::{Font, FontRef, Glyph, Point, PxScale, ScaleFont};
use image::{Rgb, RgbImage};
use smw_editor::render_util::{fill_rect, rect_border};
use smwe_rom::{
    level_sharing::{find_duplicate_groups, share_data_between_levels, BlockKind, DuplicateGroup},
    rom_expansion::compute_checksum,
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

fn kind_label(kind: BlockKind) -> &'static str {
    match kind {
        BlockKind::L1 => "Layer 1",
        BlockKind::Sprites => "Sprites",
        BlockKind::L2 => "Layer 2",
    }
}

fn levels_label(g: &DuplicateGroup) -> String {
    let shown: Vec<String> = g.holders.iter().take(6).map(|(l, _)| format!("{l:03X}")).collect();
    if g.holders.len() > 6 {
        format!("{} (+{})", shown.join(", "), g.holders.len() - 6)
    } else {
        shown.join(", ")
    }
}

fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let rom_path = args.iter().find_map(|a| a.strip_prefix("--rom=")).unwrap_or("smw.smc");
    let output = args.iter().find_map(|a| a.strip_prefix("--out=")).unwrap_or("docs/screenshots/share-data.png");

    let rom_bytes = std::fs::read(rom_path)?;
    let header_offset = 0x200 * usize::from(rom_bytes.len() % 0x400 == 0x200);

    // ---- Real analysis + real share run on a scratch copy ----
    let groups = find_duplicate_groups(&rom_bytes, header_offset)?;
    let mut scratch = rom_bytes.clone();
    let report = share_data_between_levels(&mut scratch, header_offset)?;

    // The binary refuses to render a screenshot of a corrupt result.
    let body = &scratch[header_offset..];
    let checksum = compute_checksum(body);
    let stored = u16::from_le_bytes([body[0x7FDE], body[0x7FDF]]);
    anyhow::ensure!(checksum == stored, "post-share checksum invalid");

    let status = if report.groups_merged == 0 {
        "No duplicate level data found — nothing changed.".to_string()
    } else {
        format!(
            "Shared data across {} level(s): {} duplicate group(s) merged, {} block(s) erased, {} bytes reclaimed \
             as free space.",
            report.levels_shared, report.groups_merged, report.blocks_erased, report.bytes_reclaimed
        )
    };
    eprintln!(
        "groups={} levels_shared={} blocks_erased={} bytes_reclaimed={} checksum=OK",
        report.groups_merged, report.levels_shared, report.blocks_erased, report.bytes_reclaimed
    );

    let sans = load_font(SANS_CANDIDATES)?;
    let sans_bold = load_font(SANS_BOLD_CANDIDATES)?;
    let mono = load_font(MONO_CANDIDATES)?;

    // ---- Compose the mock dialog ----
    let (w, h) = (980u32, 860u32);
    let mut img = RgbImage::new(w, h);
    let bg = Rgb([0x1B, 0x1D, 0x20]);
    let panel = Rgb([0x25, 0x28, 0x2C]);
    let titlebar = Rgb([0x12, 0x14, 0x16]);
    let ink = Rgb([0xE8, 0xE8, 0xE8]);
    let dim = Rgb([0xA8, 0xA8, 0xA8]);
    let accent = Rgb([0x4D, 0x9F, 0xFF]);
    for p in img.pixels_mut() {
        *p = bg;
    }

    let (dx, dy, dw) = (30u32, 24u32, 920u32);
    let dh = 812u32;
    fill_rect(&mut img, dx, dy, dw, dh, panel);
    rect_border(&mut img, dx, dy, dw, dh, Rgb([0x4A, 0x4E, 0x54]));
    fill_rect(&mut img, dx, dy, dw, 40, titlebar);
    draw_text(
        &mut img,
        &sans_bold,
        "Share Data Between Levels to Save Space",
        (dx + 16) as i32,
        (dy + 11) as i32,
        17.0,
        ink,
    );
    draw_text(
        &mut img,
        &sans,
        "headless mock \u{2014} description, duplicate groups and report are real",
        (dx + 470) as i32,
        (dy + 14) as i32,
        12.0,
        dim,
    );

    // Description (identical strings to the real dialog).
    let mut y = dy + 62u32;
    for line in [
        "Scans all 512 levels for byte-identical Layer 1, sprite,",
        "and Layer 2 data blocks. Levels holding identical blocks",
        "are repointed at a single shared copy, and the freed",
        "blocks are erased \u{2014} reclaiming them as free space.",
    ] {
        draw_text(&mut img, &sans, line, (dx + 20) as i32, y as i32, 14.0, ink);
        y += 22;
    }
    y += 6;
    for line in [
        "Sharing is invisible to the game: every level loads",
        "exactly the same data afterwards. A restore point is",
        "created first, so you can undo from the Restore menu.",
    ] {
        draw_text(&mut img, &sans, line, (dx + 20) as i32, y as i32, 14.0, dim);
        y += 22;
    }
    y += 10;

    // Buttons.
    fill_rect(&mut img, dx + 20, y, 130, 34, Rgb([0x2F, 0x6F, 0xBD]));
    draw_text(&mut img, &sans_bold, "Share Data", (dx + 38) as i32, (y + 8) as i32, 14.0, Rgb([0xFF, 0xFF, 0xFF]));
    fill_rect(&mut img, dx + 162, y, 110, 34, Rgb([0x3A, 0x3D, 0x42]));
    rect_border(&mut img, dx + 162, y, 110, 34, Rgb([0x6A, 0x6E, 0x74]));
    draw_text(&mut img, &sans, "Close", (dx + 196) as i32, (y + 8) as i32, 14.0, ink);
    y += 56;

    // Duplicate-group preview table (real data).
    draw_text(&mut img, &sans_bold, "Duplicate groups found (first 10):", (dx + 20) as i32, y as i32, 14.0, ink);
    y += 26;
    draw_text(&mut img, &sans, "Kind", (dx + 20) as i32, y as i32, 12.0, dim);
    draw_text(&mut img, &sans, "Levels sharing", (dx + 120) as i32, y as i32, 12.0, dim);
    draw_text(&mut img, &sans, "Block", (dx + 560) as i32, y as i32, 12.0, dim);
    draw_text(&mut img, &sans, "Duplicates", (dx + 660) as i32, y as i32, 12.0, dim);
    y += 20;
    for g in groups.iter().take(10) {
        let dupes = g.holders.len() - 1;
        draw_text(&mut img, &sans, kind_label(g.kind), (dx + 20) as i32, y as i32, 12.0, accent);
        draw_text(&mut img, &mono, &levels_label(g), (dx + 120) as i32, y as i32, 12.0, ink);
        draw_text(&mut img, &mono, &format!("{} B", g.block_len), (dx + 560) as i32, y as i32, 12.0, ink);
        draw_text(&mut img, &mono, &format!("{dupes} × {} B", g.block_len), (dx + 660) as i32, y as i32, 12.0, ink);
        y += 20;
    }
    if groups.len() > 10 {
        draw_text(
            &mut img,
            &sans,
            &format!("\u{2026} and {} more group(s)", groups.len() - 10),
            (dx + 20) as i32,
            y as i32,
            12.0,
            dim,
        );
        y += 24;
    }
    y += 8;

    // Status line (real report string).
    draw_text(&mut img, &sans_bold, "Result:", (dx + 20) as i32, y as i32, 14.0, ink);
    y += 24;
    // Wrap the status text manually at ~110 chars.
    let mut line = String::new();
    for word in status.split(' ') {
        if line.len() + word.len() + 1 > 100 {
            draw_text(&mut img, &sans, &line, (dx + 20) as i32, y as i32, 13.0, Rgb([0x9F, 0xD6, 0x8A]));
            y += 20;
            line.clear();
        }
        if !line.is_empty() {
            line.push(' ');
        }
        line.push_str(word);
    }
    if !line.is_empty() {
        draw_text(&mut img, &sans, &line, (dx + 20) as i32, y as i32, 13.0, Rgb([0x9F, 0xD6, 0x8A]));
    }

    img.save(output)?;
    println!("wrote {output}");
    Ok(())
}
