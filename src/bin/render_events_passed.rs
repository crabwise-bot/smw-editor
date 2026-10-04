//! Screenshot: the "Change Events Passed" dialog (LM overworld Edit-menu
//! dialog; toolbar button added in LM v3.70) backed by the real ROM.
//!
//! egui can't render headless, so the dialog chrome is drawn with real data:
//! every event label + tile offset comes from `OverworldEvents::parse` of the
//! real ROM (same labels the real dialog shows), and the two preview renders
//! run the exact emulated path the editor's preview uses — `load_overworld`
//! with the `$1F02-$1F60` passed-events bits set, submap 0. The binary scans
//! every event whose tile offset lands on submap 0 and picks the one whose
//! reveal changes the most rendered pixels, so the before/after difference is
//! real and visible; the changed region gets a red outline on both previews.
//!
//! Usage: `cargo run --bin render_events_passed -- --rom=smw.smc
//! --out=docs/screenshots/change-events-passed.png`
//!
//! Never commits or copies the ROM; it is only read for rendering.

use std::{env, path::Path, sync::Arc};

use ab_glyph::{Font, FontRef, Glyph, Point, PxScale, ScaleFont};
use image::{imageops::FilterType, ImageBuffer, Rgb};
use smw_editor::render_util::{fill_rect, rect_border, render_tile};
use smwe_emu::{emu::CheckedMem, rom::Rom as EmuRom, Cpu};
use smwe_rom::overworld::{OverworldEvents, OW_EVENT_COUNT};

const VRAM_L1_TILEMAP_BASE: usize = 0x2000 * 2;
const VRAM_L2_TILEMAP_BASE: usize = 0x3000 * 2;
const MAP_PX: u32 = 512;
const PREVIEW_PX: u32 = 300;

const W: u32 = 1088;
const DIALOG_W: u32 = 400;
const ROW_H: u32 = 25;
const LIST_ROWS: usize = 16;

fn main() {
    let args: Vec<String> = env::args().collect();
    let rom_path = args
        .iter()
        .find_map(|a| a.strip_prefix("--rom="))
        .map(Path::new)
        .or_else(|| args.iter().skip(1).find(|a| !a.starts_with("--")).map(Path::new))
        .unwrap_or_else(|| Path::new("smw.smc"));
    let output = args.iter().find_map(|a| a.strip_prefix("--out=")).unwrap_or("events_passed.png");

    let raw = std::fs::read(rom_path).expect("cannot read smw.smc");
    let rom_bytes = if raw.len() % 0x400 == 0x200 { raw[0x200..].to_vec() } else { raw };

    // Real event data, identical to the UI's parse.
    let rom = smwe_rom::snes_utils::rom::Rom::new(rom_bytes.clone()).expect("rom parse");
    let events = OverworldEvents::parse(&rom).expect("events parse");

    let mut emu_rom = EmuRom::new(rom_bytes);
    emu_rom.load_symbols(include_str!("../../symbols/SMW_U.sym"));
    let mut cpu = Cpu::new(CheckedMem::new(Arc::new(emu_rom)));

    // Baseline: every event passed.
    let all_passed = render_with_events(&mut cpu, None);
    // Scan submap-0 events for the most visible reveal.
    let mut best = (0usize, 0usize, (0u32, 0u32, 0u32, 0u32));
    for (i, &off) in events.tile_offsets.iter().enumerate() {
        if off == 0 || off as usize >= 0x400 {
            continue; // unused slot, or not on submap 0
        }
        let img = render_with_events(&mut cpu, Some(i));
        let (diff, bbox) = diff_bbox(&all_passed, &img);
        if diff > best.1 {
            best = (i, diff, bbox);
        }
    }
    let (picked, diff, bbox) = best;
    assert!(diff > 0, "no event changed the submap-0 render");
    let offset = events.tile_offsets[picked];
    println!("picked event {picked} (tile offset {offset:#06X}): {diff} pixels differ");

    let before = render_with_events(&mut cpu, Some(picked)); // event unpassed
    let after = all_passed; // event passed

    let before_small = resize(&before);
    let after_small = resize(&after);
    let (bx0, by0, bx1, by1) = scale_bbox(bbox);

    // ── Compose ──────────────────────────────────────────────────────
    let list_y0 = 168u32;
    let h = list_y0 + LIST_ROWS as u32 * ROW_H + 64;
    let mut img = ImageBuffer::from_pixel(W, h, Rgb([27, 27, 30]));

    // Header band.
    fill_rect(&mut img, 0, 0, W, 48, Rgb([38, 38, 44]));
    draw_text(
        &mut img,
        "Change Events Passed — Lunar Magic overworld parity",
        12,
        30,
        16.0,
        true,
        Rgb([240, 240, 244]),
    );

    // ── Dialog mock (left) ──
    let dx = 12u32;
    rect_border(&mut img, 0, 48, DIALOG_W, h - 48, Rgb([70, 70, 80]));
    draw_text(&mut img, "Change Events Passed", dx as i32, 84, 15.0, true, Rgb([235, 235, 240]));
    draw_text(&mut img, "Current event:", dx as i32, 112, 13.0, false, Rgb([220, 220, 228]));
    // DragValue-style spinner box with the real picked value.
    fill_rect(&mut img, dx + 118, 94, 52, 22, Rgb([20, 20, 24]));
    rect_border(&mut img, dx + 118, 94, 52, 22, Rgb([80, 80, 90]));
    draw_text(&mut img, &format!("{picked}"), dx as i32 + 126, 111, 13.0, false, Rgb([230, 230, 236]));
    // All on / All off buttons.
    draw_button(&mut img, dx, 124, "All on");
    draw_button(&mut img, dx + 76, 124, "All off");

    // Checklist window: rows around the picked event, real labels.
    let start = picked.saturating_sub(LIST_ROWS / 2).min(OW_EVENT_COUNT.saturating_sub(LIST_ROWS));
    for r in 0..LIST_ROWS {
        let i = start + r;
        let y0 = list_y0 + r as u32 * ROW_H;
        if r % 2 == 1 {
            fill_rect(&mut img, 0, y0, DIALOG_W, ROW_H, Rgb([32, 32, 37]));
        }
        if i == picked {
            rect_border(&mut img, 2, y0 + 1, DIALOG_W - 4, ROW_H - 2, Rgb([120, 150, 220]));
        }
        let off = events.tile_offsets[i];
        let checked = i != picked; // dialog state matches the "before" preview
        draw_checkbox(&mut img, dx + 6, y0 + 5, checked, off != 0);
        let label =
            if off == 0 { format!("Event {i:3} (unused)") } else { format!("Event {i:3} (tile offset {off:#06X})") };
        let color = if off == 0 { Rgb([120, 120, 130]) } else { Rgb([220, 220, 228]) };
        draw_text(&mut img, &label, dx as i32 + 30, (y0 + 18) as i32, 12.0, false, color);
    }
    let fy = (list_y0 + LIST_ROWS as u32 * ROW_H + 22) as i32;
    draw_text(
        &mut img,
        "Preview only — these settings are not saved to the ROM.",
        dx as i32,
        fy,
        11.0,
        false,
        Rgb([150, 150, 160]),
    );

    // ── Preview renders (right) ──
    let px = DIALOG_W + 24;
    let py = 84u32;
    draw_text(
        &mut img,
        &format!("Event {picked} unpassed — preview"),
        px as i32,
        (py - 8) as i32,
        13.0,
        true,
        Rgb([235, 235, 240]),
    );
    blit(&mut img, &before_small, px, py);
    outline_bbox(&mut img, px, py, bx0, by0, bx1, by1);
    let px2 = px + PREVIEW_PX + 24;
    draw_text(
        &mut img,
        &format!("Event {picked} passed — preview"),
        px2 as i32,
        (py - 8) as i32,
        13.0,
        true,
        Rgb([235, 235, 240]),
    );
    blit(&mut img, &after_small, px2, py);
    outline_bbox(&mut img, px2, py, bx0, by0, bx1, by1);
    draw_text(
        &mut img,
        &format!("Real emulated load_overworld renders (submap 0); red outline = the {diff} changed pixels."),
        px as i32,
        (py + PREVIEW_PX + 26) as i32,
        11.0,
        false,
        Rgb([150, 150, 160]),
    );
    draw_text(
        &mut img,
        "Honest mock: dialog chrome drawn; labels/offsets from OverworldEvents::parse of smw.smc.",
        px as i32,
        (py + PREVIEW_PX + 44) as i32,
        11.0,
        false,
        Rgb([120, 120, 130]),
    );

    img.save(output).expect("save png");
    println!("wrote {output}");
}

/// Render submap 0 with all events passed except `except` (cleared bit).
fn render_with_events(cpu: &mut Cpu, except: Option<usize>) -> Vec<u8> {
    for addr in 0x1F02u32..=0x1F60 {
        cpu.mem.store_u8(addr, 0xFF);
    }
    if let Some(n) = except {
        let addr = 0x1F02 + (n / 8) as u32;
        let cur = cpu.mem.load_u8(addr);
        cpu.mem.store_u8(addr, cur & !(0x80 >> (n % 8)));
    }
    smwe_emu::emu::load_overworld(cpu, 0);
    let (w, h) = (MAP_PX, MAP_PX);
    let mut pixels = vec![0u8; (w * h * 3) as usize];
    render_bg_full(&cpu.mem.vram, VRAM_L2_TILEMAP_BASE, w, &cpu.mem.cgram, &mut pixels);
    render_bg_full(&cpu.mem.vram, VRAM_L1_TILEMAP_BASE, w, &cpu.mem.cgram, &mut pixels);
    pixels
}

fn tilemap_vram_addr(base: usize, col: u32, row: u32) -> usize {
    let quadrant = ((row / 32) * 2) + (col / 32);
    let sub_row = row % 32;
    let sub_col = col % 32;
    let quadrant_offset = quadrant * 32 * 32 * 2;
    let idx = quadrant_offset + ((sub_row * 32 + sub_col) * 2);
    base + idx as usize
}

fn render_bg_full(vram: &[u8], tilemap_base: usize, width: u32, cgram: &[u8], pixels: &mut [u8]) {
    for row in 0..64u32 {
        for col in 0..64u32 {
            let addr = tilemap_vram_addr(tilemap_base, col, row);
            let t0 = vram[addr] as u16;
            let t1 = vram[addr + 1] as u16;
            render_tile(
                vram,
                cgram,
                (t0 | ((t1 & 3) << 8)) as usize,
                ((t1 >> 2) & 7) as usize,
                (t1 & 0x40) != 0,
                (t1 & 0x80) != 0,
                col * 8,
                row * 8,
                width,
                pixels,
            );
        }
    }
}

/// Count differing pixels between two 512×512 RGB buffers + their bbox.
fn diff_bbox(a: &[u8], b: &[u8]) -> (usize, (u32, u32, u32, u32)) {
    let (mut n, mut x0, mut y0, mut x1, mut y1) = (0usize, MAP_PX, MAP_PX, 0u32, 0u32);
    for y in 0..MAP_PX {
        for x in 0..MAP_PX {
            let o = ((y * MAP_PX + x) * 3) as usize;
            if a[o..o + 3] != b[o..o + 3] {
                n += 1;
                x0 = x0.min(x);
                y0 = y0.min(y);
                x1 = x1.max(x);
                y1 = y1.max(y);
            }
        }
    }
    (n, (x0, y0, x1, y1))
}

fn resize(px: &[u8]) -> ImageBuffer<Rgb<u8>, Vec<u8>> {
    let full = ImageBuffer::<Rgb<u8>, _>::from_raw(MAP_PX, MAP_PX, px.to_vec()).expect("image buffer");
    image::imageops::resize(&full, PREVIEW_PX, PREVIEW_PX, FilterType::Triangle)
}

fn scale_bbox((x0, y0, x1, y1): (u32, u32, u32, u32)) -> (u32, u32, u32, u32) {
    let s = |v: u32| v * PREVIEW_PX / MAP_PX;
    (s(x0), s(y0), s(x1), s(y1))
}

fn blit(img: &mut ImageBuffer<Rgb<u8>, Vec<u8>>, src: &ImageBuffer<Rgb<u8>, Vec<u8>>, x0: u32, y0: u32) {
    for (x, y, p) in src.enumerate_pixels() {
        img.put_pixel(x0 + x, y0 + y, *p);
    }
}

fn outline_bbox(img: &mut ImageBuffer<Rgb<u8>, Vec<u8>>, ox: u32, oy: u32, x0: u32, y0: u32, x1: u32, y1: u32) {
    let c = Rgb([255, 60, 60]);
    let (x0, y0, x1, y1) = (ox + x0.saturating_sub(3), oy + y0.saturating_sub(3), ox + x1 + 3, oy + y1 + 3);
    for x in x0..=x1.min(img.width() - 1) {
        for dy in 0..2 {
            if y0 + dy < img.height() {
                img.put_pixel(x, y0 + dy, c);
            }
            if y1 >= dy && y1 - dy < img.height() {
                img.put_pixel(x, y1 - dy, c);
            }
        }
    }
    for y in y0..=y1.min(img.height() - 1) {
        for dx in 0..2 {
            if x0 + dx < img.width() {
                img.put_pixel(x0 + dx, y, c);
            }
            if x1 >= dx && x1 - dx < img.width() {
                img.put_pixel(x1 - dx, y, c);
            }
        }
    }
}

fn draw_button(img: &mut ImageBuffer<Rgb<u8>, Vec<u8>>, x: u32, y: u32, label: &str) {
    let w = 68u32;
    fill_rect(img, x, y, w, 24, Rgb([48, 48, 56]));
    rect_border(img, x, y, w, 24, Rgb([90, 90, 100]));
    draw_text(img, label, x as i32 + 10, (y + 17) as i32, 12.0, false, Rgb([225, 225, 232]));
}

fn draw_checkbox(img: &mut ImageBuffer<Rgb<u8>, Vec<u8>>, x: u32, y: u32, checked: bool, enabled: bool) {
    fill_rect(img, x, y, 14, 14, Rgb([20, 20, 24]));
    rect_border(img, x, y, 14, 14, Rgb([90, 90, 100]));
    if checked {
        let c = if enabled { Rgb([110, 220, 130]) } else { Rgb([90, 90, 95]) };
        // Check mark: (3,7)->(6,10)->(11,4), 2px thick.
        for (x0, y0, x1, y1) in [(3i32, 7i32, 6, 10), (6, 10, 11, 4)] {
            let steps = (x1 - x0).abs().max((y1 - y0).abs());
            for s in 0..=steps {
                let px = x + (x0 + (x1 - x0) * s / steps) as u32;
                let py = y + (y0 + (y1 - y0) * s / steps) as u32;
                for (dx, dy) in [(0u32, 0u32), (1, 0), (0, 1)] {
                    img.put_pixel(px + dx, py + dy, c);
                }
            }
        }
    }
}

fn font(bold: bool) -> FontRef<'static> {
    use std::sync::OnceLock;
    static REGULAR: OnceLock<FontRef<'static>> = OnceLock::new();
    static BOLD: OnceLock<FontRef<'static>> = OnceLock::new();
    let (slot, path) = if bold {
        (&BOLD, "/usr/share/fonts/truetype/dejavu/DejaVuSans-Bold.ttf")
    } else {
        (&REGULAR, "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf")
    };
    slot.get_or_init(|| {
        let data = std::fs::read(path).expect("font");
        let leaked: &'static [u8] = Box::leak(data.into_boxed_slice());
        FontRef::try_from_slice(leaked).expect("font parse")
    })
    .clone()
}

fn draw_text(
    img: &mut ImageBuffer<Rgb<u8>, Vec<u8>>, text: &str, x: i32, baseline_y: i32, px: f32, bold: bool, color: Rgb<u8>,
) {
    let font = font(bold);
    let scaled = font.as_scaled(PxScale::from(px));
    let mut caret_x = x as f32;
    let mut prev = None;
    for ch in text.chars() {
        let id = font.glyph_id(ch);
        if let Some(p) = prev {
            caret_x += scaled.kern(p, id);
        }
        let glyph = Glyph { id, scale: PxScale::from(px), position: Point { x: caret_x, y: baseline_y as f32 } };
        if let Some(o) = scaled.outline_glyph(glyph) {
            let bb = o.px_bounds();
            o.draw(|gx, gy, v| {
                let px_x = bb.min.x as i32 + gx as i32;
                let px_y = bb.min.y as i32 + gy as i32;
                if px_x >= 0 && px_y >= 0 {
                    let (px_x, px_y) = (px_x as u32, px_y as u32);
                    if px_x < img.width() && px_y < img.height() && v > 0.4 {
                        img.put_pixel(px_x, px_y, color);
                    }
                }
            });
        }
        prev = Some(id);
        caret_x += scaled.h_advance(id);
    }
}
