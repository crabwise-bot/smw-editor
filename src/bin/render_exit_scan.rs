//! Headless verification + honest-mock screenshot for "Scan for Undefined
//! Exits" (Tools > Scan for Undefined Exits...).
//!
//! egui can't render headless, so this composes an honest mock of the
//! results window: every string on screen is real — the exact summary line,
//! finding rows, and button labels the UI uses — and the findings are real
//! data from running the actual scan over the ROM (`scan_undefined_exits`).
//! Only the window chrome and widget shapes are drawn rather than real egui
//! widgets.
//!
//! ```sh
//! cargo run --bin render_exit_scan -- --rom=smw.smc --out=docs/screenshots/exit-scan.png
//! ```
//!
//! With `--report`, prints the full text report instead of rendering.

use std::time::Instant;

use ab_glyph::{Font, FontRef, Glyph, Point, PxScale, ScaleFont};
use image::{Rgb, RgbImage};
use smw_editor::{
    exit_scan::{scan_undefined_exits, ExitScanReport, UndefinedExitKind},
    render_util::{fill_rect, rect_border},
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

fn arg(name: &str, default: &str) -> String {
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        if a == name {
            return args.next().unwrap_or_else(|| default.to_string());
        }
    }
    default.to_string()
}

fn summary_line(report: &ExitScanReport) -> String {
    let n = report.findings.len();
    let mut s = format!(
        "Scanned {} levels — {} undefined exit{} found.",
        report.levels_scanned,
        n,
        if n == 1 { "" } else { "s" }
    );
    if report.levels_skipped > 0 {
        s.push_str(&format!(
            " ({} level{} skipped: unparseable data)",
            report.levels_skipped,
            if report.levels_skipped == 1 { "" } else { "s" }
        ));
    }
    s
}

fn finding_row(f: &smw_editor::exit_scan::UndefinedExit) -> String {
    match &f.kind {
        UndefinedExitKind::NoExitRecord => {
            format!("Level ${:03X}, screen {} — exit-enabled tiles but no screen exit defined", f.level, f.screen)
        }
        UndefinedExitKind::UndefinedDestination { via_secondary, destination } => {
            let via = via_secondary.map(|i| format!(" via secondary exit ${i:03X}")).unwrap_or_default();
            format!("Level ${:03X}, screen {} — exit leads to level ${destination:03X} (TEST){via}", f.level, f.screen)
        }
    }
}

/// Debug helper: for one level, print the exit records and the exit-tile
/// screens so a human can verify a finding is genuine.
fn debug_one_level(raw: &[u8], level_num: u16) -> anyhow::Result<()> {
    use std::collections::HashMap;

    use smw_editor::level_png_export::{block_at, level_geom_of, load_level_cpu, screen_at, BLOCK_MAP_BASE};
    use smwe_rom::{
        block_behavior::is_exit_enabled,
        level::{
            object_layer::{ExtendedInstance, ObjectInstance},
            Level,
        },
        map16_expanded::{act_as_of, read_acts_table},
        snes_utils::rom::Rom,
    };

    let stripped: &[u8] = if raw.len() % 0x400 == 0x200 { &raw[0x200..] } else { raw };
    let rom = Rom::new(stripped.to_vec())?;
    let acts = read_acts_table(stripped, 0).unwrap_or_default();
    let level = Level::parse(&rom, level_num as u32)?;
    let mut records: Vec<String> = Vec::new();
    for obj in level.layer1.objects() {
        if let ObjectInstance::Extended(ExtendedInstance::Exit(e)) = obj {
            records.push(format!(
                "screen {} -> dest ${:03X}{}",
                e.screen_number(),
                e.destination_level(),
                if e.secondary_exit() { " (secondary)" } else { "" }
            ));
        }
    }
    let mut cpu = load_level_cpu(stripped, level_num)?;
    let g = level_geom_of(&mut cpu);
    let l2_active = g.level_mode == 0x01 && g.has_layer2;
    let l2_off = g.scr_len * g.scr_size;
    let (tw, th) = (g.width / 16, g.height / 16);
    let mut screens: HashMap<u8, Vec<u16>> = HashMap::new();
    for ty in 0..th {
        for tx in 0..tw {
            let id = block_at(&mut cpu, &g, tx, ty, BLOCK_MAP_BASE);
            let l1_on = id != 0 && is_exit_enabled(act_as_of(&acts, id), g.level_mode);
            let id2 = if l2_active { block_at(&mut cpu, &g, tx, ty, BLOCK_MAP_BASE + l2_off) } else { 0 };
            let l2_on = l2_active && id2 != 0 && is_exit_enabled(act_as_of(&acts, id2), g.level_mode);
            if l1_on {
                screens.entry(screen_at(&g, tx, ty)).or_default().push(id);
            }
            if l2_on {
                screens.entry(screen_at(&g, tx, ty)).or_default().push(id2 | 0x8000);
            }
        }
    }
    let mut screens: Vec<_> = screens.into_iter().collect();
    screens.sort();
    println!("level ${level_num:03X} (vertical={} mode=${:02X}):", g.vertical, g.level_mode);
    println!("  exit records: {}", if records.is_empty() { "(none)".to_string() } else { records.join(", ") });
    for (s, ids) in &screens {
        let mut uniq: Vec<u16> = ids.clone();
        uniq.sort_unstable();
        uniq.dedup();
        let ids: Vec<String> =
            uniq.iter().map(|i| format!("{}${:03X}", if i & 0x8000 != 0 { "L2 " } else { "" }, i & 0x7FFF)).collect();
        println!("  screen {s}: exit tiles {}", ids.join(" "));
    }
    Ok(())
}

fn main() -> anyhow::Result<()> {
    let rom_path = arg("--rom", "smw.smc");
    let out_path = arg("--out", "docs/screenshots/exit-scan.png");
    let report_only = std::env::args().any(|a| a == "--report");

    let raw = std::fs::read(&rom_path)?;

    // Fast single-level debug: --debug-level=11D prints exit records and
    // exit-tile screens for one level without running the full scan.
    if let Some(spec) = std::env::args().find_map(|a| a.strip_prefix("--debug-level=").map(str::to_string)) {
        let level = u16::from_str_radix(spec.trim_start_matches('$'), 16)?;
        return debug_one_level(&raw, level);
    }

    let t0 = Instant::now();
    let report = scan_undefined_exits(&raw)?;
    let elapsed = t0.elapsed();
    eprintln!(
        "scanned {} levels ({} skipped) in {:.1}s: {} findings",
        report.levels_scanned,
        report.levels_skipped,
        elapsed.as_secs_f32(),
        report.findings.len()
    );

    let rows: Vec<String> = report.findings.iter().map(finding_row).collect();

    if report_only {
        println!("{}", summary_line(&report));
        for r in &rows {
            println!("  {r}");
        }
        return Ok(());
    }

    // ── Honest mock of the results window ──
    let font = load_font(SANS_CANDIDATES)?;
    let font_bold = load_font(SANS_BOLD_CANDIDATES)?;
    let (w, h) = (860u32, 520u32);
    let mut img = RgbImage::new(w, h);
    let bg = Rgb([0x1B, 0x1D, 0x20]);
    let panel = Rgb([0x25, 0x28, 0x2C]);
    let titlebar = Rgb([0x12, 0x14, 0x16]);
    let ink = Rgb([0xE8, 0xE8, 0xE8]);
    let dim = Rgb([0xA8, 0xA8, 0xA8]);
    for p in img.pixels_mut() {
        *p = bg;
    }
    let (dx, dy, dw, dh) = (20u32, 20u32, 820u32, 480u32);
    fill_rect(&mut img, dx, dy, dw, dh, panel);
    rect_border(&mut img, dx, dy, dw, dh, Rgb([0x4A, 0x4E, 0x54]));
    fill_rect(&mut img, dx, dy, dw, 40, titlebar);
    draw_text(&mut img, &font_bold, "Scan for Undefined Exits", (dx + 16) as i32, (dy + 11) as i32, 17.0, ink);

    draw_text(&mut img, &font, &summary_line(&report), (dx + 16) as i32, (dy + 62) as i32, 14.0, ink);
    draw_text(
        &mut img,
        &font,
        "LM v1.50: exits whose destination was never set up point at the TEST levels ($000/$100).",
        (dx + 16) as i32,
        (dy + 86) as i32,
        12.0,
        dim,
    );

    let mut y = (dy + 118) as i32;
    for row in rows.iter().take(12) {
        draw_text(&mut img, &font, "⚠", (dx + 20) as i32, y, 13.0, Rgb([0xFF, 0xCD, 0x5A]));
        draw_text(&mut img, &font, row, (dx + 44) as i32, y, 13.0, ink);
        y += 24;
    }
    if rows.len() > 12 {
        draw_text(
            &mut img,
            &font,
            &format!("… and {} more (scroll in the editor)", rows.len() - 12),
            (dx + 44) as i32,
            y,
            12.0,
            dim,
        );
    }
    if rows.is_empty() {
        draw_text(&mut img, &font, "✓ No undefined exits found.", (dx + 20) as i32, y, 14.0, Rgb([0x8C, 0xDC, 0x8C]));
    }

    draw_button(&mut img, &font, dx + 16, dy + dh - 50, 110, "Re-scan", true);
    draw_button(&mut img, &font, dx + 136, dy + dh - 50, 110, "Close", true);

    img.save(&out_path)?;
    eprintln!("wrote {out_path}");
    Ok(())
}
