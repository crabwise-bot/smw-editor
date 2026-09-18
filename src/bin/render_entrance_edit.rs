// Render the entrance-editing PR screenshot: a real-ROM level crop around
// the main-entrance "M" marker in sprite editing mode, with the marker
// selected (orange outline, like selected sprites). Two frames:
//   1. M selected at its vanilla spot (0, 22) — "copied"
//   2. M pasted/moved to (5, 18) — "paste = move"
// The M glyph is rasterized from the real message font (red, like the
// editor); the selection style matches central_panel.rs.
//   --rom=PATH --level=0x105 --out=docs/screenshots/entrance-edit.gif
use std::sync::Arc;

use anyhow::Context;
use image::RgbImage;
use smwe_emu::{emu::CheckedMem, rom::Rom as EmuRom, Cpu};

fn arg(name: &str, default: &str) -> String {
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        if a == name {
            return args.next().unwrap_or_else(|| default.to_string());
        }
    }
    default.to_string()
}

use smw_editor::render_util::{fill_rect_raw, render_layer, stroke_rect};

/// Encode printable ASCII into SMW message bytes (see font_map.rs).
fn msg_encode(s: &str) -> Vec<u8> {
    let mut out = Vec::with_capacity(s.len());
    for c in s.chars() {
        let b = match c {
            'A'..='Z' => c as u8 - b'A',
            'a'..='z' => 0x40 + (c as u8 - b'a'),
            '0'..='9' => 0x20 + (c as u8 - b'0'),
            ' ' => 0x1F,
            '!' => 0x1A,
            '.' => 0x1B,
            ',' => 0x1D,
            '?' => 0x1E,
            '(' => 0x5B,
            ')' => 0x5C,
            '-' => 0x5E,
            _ => 0x1F,
        };
        out.push(b);
    }
    if let Some(last) = out.last_mut() {
        *last |= 0x80;
    }
    out
}

/// One line of SMW-font text in `rgb`, on transparent black, scaled `scale`x.
fn text_line(s: &str, font: &[Box<[u8]>], scale: u32, rgb: [u8; 3]) -> RgbImage {
    let bytes = msg_encode(s);
    let w_px = bytes.len() as u32 * 8;
    let mut rgba = image::RgbaImage::new(w_px, 8);
    for (i, &b) in bytes.iter().enumerate() {
        let tile = &font[(b & 0x7F) as usize % font.len()];
        for y in 0..8 {
            for x in 0..8 {
                let v = tile[y * 8 + x];
                let (r, g, bl) = if v == 1 { (0, 0, 0) } else { (rgb[0], rgb[1], rgb[2]) };
                rgba.put_pixel(i as u32 * 8 + x as u32, y as u32, image::Rgba([r, g, bl, 255]));
            }
        }
    }
    let big = image::imageops::resize(&rgba, w_px * scale, 8 * scale, image::imageops::FilterType::Nearest);
    image::DynamicImage::ImageRgba8(big).to_rgb8()
}

/// Blit `src` onto `dst` (RGB) at (dx, dy); near-black pixels are transparent.
/// Pixels past the row end are skipped (no wrapping into later rows).
fn blit_text(dst: &mut [u8], dst_w: u32, src: &RgbImage, dx: u32, dy: u32) {
    for (x, y, p) in src.enumerate_pixels() {
        if p[0] < 40 && p[1] < 40 && p[2] < 40 {
            continue;
        }
        let ox = dx + x;
        let oy = dy + y;
        if ox >= dst_w {
            continue;
        }
        let i = ((oy * dst_w + ox) * 3) as usize;
        if i + 2 < dst.len() {
            dst[i] = p[0];
            dst[i + 1] = p[1];
            dst[i + 2] = p[2];
        }
    }
}

/// Fill a whole buffer with an RGB color.
fn fill(buf: &mut [u8], rgb: [u8; 3]) {
    for px in buf.chunks_exact_mut(3) {
        px.copy_from_slice(&rgb);
    }
}

/// Draw the red "M" entrance marker at tile (tx, ty) inside `crop`
/// (whose top-left is tile (cx0, cy0)), plus the orange selection outline.
fn draw_marker(crop: &mut [u8], cw: u32, font: &[Box<[u8]>], tx: u32, ty: u32, cx0: u32, cy0: u32) {
    let px = (tx - cx0) * 16;
    let py = (ty - cy0) * 16;
    // Selection: translucent orange fill + orange outline (matches the
    // editor's selected-sprite style).
    fill_rect_raw(crop, cw, px, py, 16, 16, [255, 120, 0, 50]);
    stroke_rect(crop, cw, px, py, 16, 16, [255, 120, 0], 2);
    // Red "M" glyph, centered-ish in the tile (font is 8px, scale 2x).
    let m = text_line("M", font, 2, [255, 100, 100]);
    blit_text(crop, cw, &m, px + (16 - m.width()) / 2, py + (16 - m.height()) / 2);
}

fn main() -> anyhow::Result<()> {
    let rom_path = arg("--rom", "smw.smc");
    let level_num: u32 = u32::from_str_radix(arg("--level", "0x105").trim_start_matches("0x"), 16).unwrap_or(0x105);
    let out_path = arg("--out", "docs/screenshots/entrance-edit.gif");

    let raw = std::fs::read(&rom_path).with_context(|| format!("read {rom_path}"))?;
    let rom_bytes: Vec<u8> = if raw.len() % 0x400 == 0x200 { raw[0x200..].to_vec() } else { raw };

    // ── Load + render the level (same path as render_level.rs) ──
    let mut emu_rom = EmuRom::new(rom_bytes.clone());
    emu_rom.load_symbols(include_str!("../../symbols/SMW_U.sym"));
    let mut cpu = Cpu::new(CheckedMem::new(Arc::new(emu_rom)));
    smwe_emu::emu::decompress_sublevel(&mut cpu, level_num as u16);

    let vertical = cpu.mem.load_u8(0x5B) & 1 != 0;
    anyhow::ensure!(!vertical, "demo level must be horizontal");
    let width = 0x20 * 16 * 16;
    let height = 27 * 16;
    let mut pixels = vec![0u8; (width * height * 3) as usize];
    render_layer(&mut cpu, false, width, &mut pixels);
    render_layer(&mut cpu, true, width, &mut pixels);

    // ── Crop around the entrance: level 0x105's vanilla entrance is tile
    // (0, 22) (half-res (0,11) screen 0). Crop tiles x 0..16, y 13..27. ──
    let (cx0, cy0) = (0u32, 13u32);
    let (cw, ch) = (16u32 * 16, 14u32 * 16);
    let crop: Vec<u8> = (cy0 * 16..cy0 * 16 + ch)
        .flat_map(|y| {
            let row = (y * width * 3) as usize;
            let col = (cx0 * 16 * 3) as usize;
            pixels[row + col..row + col + (cw * 3) as usize].to_vec()
        })
        .collect();

    let smw_rom = smwe_rom::SmwRom::from_file(&rom_path)?;
    let font = smwe_rom::message_raster::decompress_message_font(&smw_rom.rom)?;

    // ── Layout: toolbar strip + crop + status strip ──
    let toolbar_h = 84u32;
    let status_h = 160u32;
    let fw = cw.max(720);
    let fh = toolbar_h + ch + status_h;
    let mut frame = vec![0u8; (fw * fh * 3) as usize];
    let crop_dx = (fw - cw) / 2;

    // Whole-frame backdrop so the area around the centered crop isn't pure
    // black.
    fill(&mut frame, [18, 18, 26]);
    fill(&mut frame[..(fw * toolbar_h * 3) as usize], [30, 30, 44]);
    let btn_labels = ["CUT", "COPY", "PASTE"];
    let btn_w = 120u32;
    let btn_h = 44u32;
    let mut bx = 16u32;
    for label in btn_labels {
        for y in 20..20 + btn_h {
            for x in bx..bx + btn_w {
                let i = ((y * fw + x) * 3) as usize;
                let edge = x == bx || x + 1 == bx + btn_w || y == 20 || y + 1 == 20 + btn_h;
                let c = if edge { [120, 120, 150] } else { [58, 58, 78] };
                frame[i..i + 3].copy_from_slice(&c);
            }
        }
        let t = text_line(label, &font, 3, [255, 255, 255]);
        blit_text(&mut frame, fw, &t, bx + (btn_w - t.width()) / 2, 20 + (btn_h - t.height()) / 2);
        bx += btn_w + 12;
    }
    let cap = text_line("SPRITE MODE", &font, 2, [255, 255, 255]);
    blit_text(&mut frame, fw, &cap, bx + 8, 20 + (btn_h - cap.height()) / 2);

    let blit_crop = |frame: &mut [u8], px: &[u8]| {
        let dst_y = toolbar_h;
        for row in 0..ch {
            let d = ((dst_y + row) * fw + crop_dx) as usize * 3;
            let s = (row * cw) as usize * 3;
            frame[d..d + (cw as usize) * 3].copy_from_slice(&px[s..s + (cw as usize) * 3]);
        }
    };

    let paint_status = |frame: &mut [u8], lines: &[&str]| {
        let sy = toolbar_h + ch;
        fill(&mut frame[(sy * fw * 3) as usize..((sy + status_h) * fw * 3) as usize], [22, 22, 32]);
        let mut y = sy + 10;
        for l in lines {
            let t = text_line(l, &font, 2, [255, 255, 255]);
            blit_text(frame, fw, &t, 16, y);
            y += t.height() + 10;
        }
    };

    // ── Frame 1: entrance selected at its vanilla spot (copied) ──
    let mut f1 = frame.clone();
    let mut c1 = crop.clone();
    draw_marker(&mut c1, cw, &font, 0, 22, cx0, cy0);
    blit_crop(&mut f1, &c1);
    paint_status(&mut f1, &[
        "Copied entrance position.",
        "Ctrl C copies. Ctrl X cuts. Delete resets.",
        "Ctrl Z undoes the entrance move.",
    ]);

    // ── Frame 2: pasted — the entrance moved to (5, 18) ──
    let mut f2 = frame.clone();
    let mut c2 = crop.clone();
    draw_marker(&mut c2, cw, &font, 5, 18, cx0, cy0);
    blit_crop(&mut f2, &c2);
    paint_status(&mut f2, &[
        "Pasted. The entrance moved.",
        "Drag the M marker in sprite mode to move it.",
        "Delete resets to the vanilla default spot.",
    ]);

    if out_path.ends_with(".gif") {
        use image::{
            codecs::gif::{GifEncoder, Repeat},
            Delay,
        };
        let file = std::fs::File::create(&out_path)?;
        let mut enc = GifEncoder::new(file);
        enc.set_repeat(Repeat::Infinite)?;
        for f in [&f1, &f2] {
            let img = RgbImage::from_raw(fw, fh, f.clone()).expect("frame size");
            let rgba = image::DynamicImage::ImageRgb8(img).to_rgba8();
            enc.encode_frame(image::Frame::from_parts(rgba, 0, 0, Delay::from_numer_denom_ms(1100, 1)))?;
        }
        eprintln!("wrote {out_path}");
    } else {
        let img = RgbImage::from_raw(fw, fh, f1).expect("frame size");
        img.save(&out_path)?;
        eprintln!("wrote {out_path}");
    }
    Ok(())
}
