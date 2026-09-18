//! Headless mock screenshot of the "Delete Levels from ROM" dialog
//! (File > Levels > Delete Levels from ROM...).
//!
//! egui can't render headless, so this composes an honest mock of the dialog:
//! every string on screen is real — the exact description, button, legend,
//! and warning text the UI uses. The per-level coloring is real data computed
//! from the actual ROM with the same code the dialog uses
//! (`level_modified_vs`, `GAMEPLAY_CRITICAL_LEVELS`, and the overworld-placed
//! level scan via `level_number_for_index`). The status line is a real
//! `delete_levels` report from running the deletion on an in-memory copy of
//! the ROM. Only the window chrome and widget shapes are drawn rather than
//! real egui widgets.
//!
//! ```sh
//! cargo run --bin render_delete_levels -- --rom=smw.smc --out=docs/screenshots/delete-levels.png
//! ```

use ab_glyph::{Font, FontRef, Glyph, Point, PxScale, ScaleFont};
use image::{Rgb, RgbImage};
use smw_editor::render_util::{fill_rect, rect_border};
use smwe_rom::{
    level_deletion::{delete_levels, level_modified_vs, GAMEPLAY_CRITICAL_LEVELS},
    overworld::level_number_for_index,
    snes_utils::rom::Rom,
    SmwRom,
};

const SANS_CANDIDATES: &[&str] = &["/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf"];
const SANS_BOLD_CANDIDATES: &[&str] = &["/usr/share/fonts/truetype/dejavu/DejaVuSans-Bold.ttf"];
const LEVEL_COUNT: usize = 0x200;

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

fn draw_button(img: &mut RgbImage, font: &FontRef, x: u32, y: u32, w: u32, label: &str, enabled: bool) {
    let (bg, ink) = if enabled {
        (Rgb([0x2F, 0x6F, 0xBD]), Rgb([0xFF, 0xFF, 0xFF]))
    } else {
        (Rgb([0x3A, 0x3D, 0x42]), Rgb([0xA8, 0xA8, 0xA8]))
    };
    fill_rect(img, x, y, w, 34, bg);
    rect_border(img, x, y, w, 34, Rgb([0x6A, 0x6E, 0x74]));
    draw_text(img, font, label, (x + 12) as i32, (y + 8) as i32, 14.0, ink);
}

fn draw_checkbox(img: &mut RgbImage, font: &FontRef, x: u32, y: u32, label: &str, color: Rgb<u8>, checked: bool) {
    fill_rect(img, x, y, 16, 16, Rgb([0x3A, 0x3D, 0x42]));
    rect_border(img, x, y, 16, 16, Rgb([0x6A, 0x6E, 0x74]));
    if checked {
        draw_text(img, font, "✓", x as i32 + 1, y as i32 - 3, 15.0, Rgb([0x4D, 0x9F, 0xFF]));
    }
    draw_text(img, font, label, (x + 22) as i32, y as i32 - 2, 13.0, color);
}

fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let rom_path = args.iter().find_map(|a| a.strip_prefix("--rom=")).unwrap_or("smw.smc");
    let output = args.iter().find_map(|a| a.strip_prefix("--out=")).unwrap_or("docs/screenshots/delete-levels.png");

    let rom_bytes = std::fs::read(rom_path)?;
    let header_offset = usize::from(rom_bytes.len() % 0x400 == 0x200) * 0x200;

    // ---- Real classifications, same code the dialog uses ----
    let rom = SmwRom::from_rom(Rom::new(rom_bytes.clone())?)?;
    let mut modified = vec![false; LEVEL_COUNT];
    for level in 0..LEVEL_COUNT {
        // Vanilla ROM vs itself: nothing modified (honest demo baseline).
        modified[level] = level_modified_vs(&rom_bytes, &rom_bytes, level as u16, header_offset);
    }
    let mut critical = vec![false; LEVEL_COUNT];
    for &level in GAMEPLAY_CRITICAL_LEVELS {
        critical[level as usize] = true;
    }
    let tiles = &rom.overworld.layer1_tiles;
    let mut overworld_placed = 0;
    for idx in 0..tiles.len() {
        if let Some(level) = level_number_for_index(tiles, idx) {
            if !critical[level as usize] {
                overworld_placed += 1;
            }
            critical[level as usize] = true;
        }
    }
    let n_critical = critical.iter().filter(|&&c| c).count();
    eprintln!("critical levels: {n_critical} ({overworld_placed} overworld-placed beyond title/demo)");

    // ---- Real deletion on an in-memory copy: the status line ----
    // Demo selection: one ordinary level + one critical (overworld-placed)
    // level, both visible in the 000-07F mock window.
    let demo_selected = [0x012u16, 0x005];
    let mut scratch = rom_bytes.clone();
    let report = delete_levels(&mut scratch, &demo_selected, header_offset)?;
    let status = format!(
        "Deleted {} level(s); reclaimed {} bytes in {} erased block(s).",
        report.deleted.len(),
        report.bytes_reclaimed,
        report.blocks_erased
    );
    eprintln!("status: {status}");

    // Demo selection: one ordinary level + one critical level checked.
    let mut selected = vec![false; LEVEL_COUNT];
    for &l in &demo_selected {
        selected[l as usize] = true;
    }
    let critical_selected: Vec<u16> = demo_selected.iter().copied().filter(|&l| critical[l as usize]).collect();
    let warning = format!(
        "⚠ Deleting gameplay-critical level(s) {} can break the game",
        critical_selected.iter().map(|l| format!("{l:03X}")).collect::<Vec<_>>().join(", ")
    );

    let sans = load_font(SANS_CANDIDATES)?;
    let sans_bold = load_font(SANS_BOLD_CANDIDATES)?;

    // ---- Compose the mock dialog ----
    let (w, h) = (980u32, 860u32);
    let mut img = RgbImage::new(w, h);
    let bg = Rgb([0x1B, 0x1D, 0x20]);
    let panel = Rgb([0x25, 0x28, 0x2C]);
    let titlebar = Rgb([0x12, 0x14, 0x16]);
    let ink = Rgb([0xE8, 0xE8, 0xE8]);
    let dim = Rgb([0xA8, 0xA8, 0xA8]);
    let red = Rgb([0xFF, 0x78, 0x78]);
    let yellow = Rgb([0xFF, 0xCD, 0x5A]);
    for p in img.pixels_mut() {
        *p = bg;
    }

    let (dx, dy, dw) = (30u32, 24u32, 920u32);
    let dh = 812u32;
    fill_rect(&mut img, dx, dy, dw, dh, panel);
    rect_border(&mut img, dx, dy, dw, dh, Rgb([0x4A, 0x4E, 0x54]));
    fill_rect(&mut img, dx, dy, dw, 40, titlebar);
    draw_text(&mut img, &sans_bold, "Delete Levels from ROM", (dx + 16) as i32, (dy + 11) as i32, 17.0, ink);
    draw_text(
        &mut img,
        &sans,
        "headless mock — strings and level classifications are real",
        (dx + 340) as i32,
        (dy + 14) as i32,
        12.0,
        dim,
    );

    let mut y = dy + 62u32;
    for line in [
        "Replaces each selected level's data with the vanilla test level",
        "and erases the old data blocks, reclaiming them as free space.",
        "A restore point is created first, and the ROM checksum is repaired.",
    ] {
        draw_text(&mut img, &sans, line, (dx + 20) as i32, y as i32, 14.0, ink);
        y += 22;
    }
    y += 8;
    // Quick-select row (real labels from the UI).
    draw_text(&mut img, &sans, "Quick select:", (dx + 20) as i32, (y + 6) as i32, 14.0, ink);
    let mut bx = dx + 140;
    for label in ["All", "Modified", "Unmodified", "None"] {
        draw_button(&mut img, &sans, bx, y, 96, label, true);
        bx += 106;
    }
    y += 46;
    draw_text(&mut img, &sans, "2 level(s) selected", (dx + 20) as i32, y as i32, 14.0, ink);
    y += 22;
    draw_text(
        &mut img,
        &sans,
        "Yellow = modified since the ROM was opened · Red = gameplay-critical",
        (dx + 20) as i32,
        y as i32,
        12.0,
        dim,
    );
    y += 24;

    // Checkbox grid: levels 000-07F (8 x 16), real colors, demo checks.
    let grid_top = y;
    let (cols, rows) = (8u32, 16u32);
    let (cell_w, cell_h) = (104u32, 24u32);
    let grid_w = cols * cell_w;
    let grid_h = rows * cell_h;
    fill_rect(&mut img, dx + 20, grid_top, grid_w, grid_h, Rgb([0x1E, 0x21, 0x25]));
    rect_border(&mut img, dx + 20, grid_top, grid_w, grid_h, Rgb([0x4A, 0x4E, 0x54]));
    for level in 0..(cols * rows) as usize {
        let (col, row) = (level as u32 % cols, level as u32 / cols);
        let (cx, cy) = (dx + 24 + col * cell_w, grid_top + 4 + row * cell_h);
        let color = if critical[level] {
            red
        } else if modified[level] {
            yellow
        } else {
            ink
        };
        draw_checkbox(&mut img, &sans, cx, cy, &format!("{level:03X}"), color, selected[level]);
    }
    // Scrollbar hint (the real dialog scrolls through all 512).
    fill_rect(&mut img, dx + 20 + grid_w + 6, grid_top, 10, grid_h, Rgb([0x3A, 0x3D, 0x42]));
    fill_rect(&mut img, dx + 20 + grid_w + 6, grid_top, 10, grid_h / 4, Rgb([0x6A, 0x6E, 0x74]));
    y = grid_top + grid_h + 14;

    // Real warning text (exact format the UI uses), demo selection hits 005.
    draw_text(&mut img, &sans, &warning, (dx + 20) as i32, y as i32, 14.0, red);
    y += 22;
    draw_text(&mut img, &sans, "(title/demo or overworld-placed levels).", (dx + 20) as i32, y as i32, 14.0, red);
    y += 36;

    // Buttons: Delete enabled with the real count.
    draw_button(&mut img, &sans_bold, dx + 20, y, 190, "Delete 2 level(s)", true);
    draw_button(&mut img, &sans, dx + 222, y, 110, "Close", true);
    y += 48;

    // Real status line from the in-memory deletion.
    draw_text(&mut img, &sans, &status, (dx + 20) as i32, y as i32, 13.0, Rgb([0x8F, 0xD6, 0x8F]));
    y += 24;
    draw_text(
        &mut img,
        &sans,
        &format!("{n_critical} levels flagged gameplay-critical in this ROM (title/demo + overworld-placed)."),
        (dx + 20) as i32,
        y as i32,
        12.0,
        dim,
    );

    img.save(output)?;
    println!("wrote {output}");
    Ok(())
}
