// Render the clipboard PR screenshot: a real-ROM level with two "selected"
// objects, a mock toolbar strip showing the new Cut/Copy/Paste buttons, and
// a status strip — plus a GIF that animates copy -> paste (the pasted
// footprints are pixel-copied, which is exactly what the editor stamps).
//   --rom=PATH --level=0x105 --out=docs/screenshots/clipboard.png  -> static
//   --out=docs/screenshots/clipboard.gif                          -> 2 frames
use std::sync::Arc;

use anyhow::Context;
use image::RgbImage;
use smwe_emu::{emu::CheckedMem, rom::Rom as EmuRom, Cpu};
use smwe_rom::{level::Level, objects::Object, snes_utils::rom::Rom};

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
/// Digits/slashes aren't in the message font; callers spell out numbers.
fn msg_encode(s: &str) -> Vec<u8> {
    let mut out = Vec::with_capacity(s.len());
    for c in s.chars() {
        let b = match c {
            'A'..='Z' => c as u8 - b'A',
            'a'..='z' => 0x40 + (c as u8 - b'a'),
            ' ' => 0x1F,
            '!' => 0x1A,
            '.' => 0x1B,
            ',' => 0x1D,
            '?' => 0x1E,
            '\'' => 0x5D,
            '"' => 0x1C,
            _ => 0x1F,
        };
        out.push(b);
    }
    // Bit 7 on the final byte ends the row (message_cells row fill).
    if let Some(last) = out.last_mut() {
        *last |= 0x80;
    }
    out
}

/// One line of SMW-font text: white glyphs on transparent black, scaled
/// `scale`x. Rasterized straight from the font tiles — in the message font,
/// color index 1 is the background and anything else is glyph.
fn text_line(s: &str, font: &[Box<[u8]>], scale: u32) -> RgbImage {
    let bytes = msg_encode(s);
    let w_px = bytes.len() as u32 * 8;
    let mut rgba = image::RgbaImage::new(w_px, 8);
    for (i, &b) in bytes.iter().enumerate() {
        let tile = &font[(b & 0x7F) as usize % font.len()];
        for y in 0..8 {
            for x in 0..8 {
                let v = tile[y * 8 + x];
                let g = if v == 1 { 0 } else { 255 };
                rgba.put_pixel(i as u32 * 8 + x as u32, y as u32, image::Rgba([g, g, g, g]));
            }
        }
    }
    let big = image::imageops::resize(&rgba, w_px * scale, 8 * scale, image::imageops::FilterType::Nearest);
    image::DynamicImage::ImageRgba8(big).to_rgb8()
}

/// Blit `src` onto `dst` (RGB) at (dx, dy); magenta (255,0,255) is transparent.
fn blit_text(dst: &mut [u8], dst_w: u32, src: &RgbImage, dx: u32, dy: u32, transparent_black: bool) {
    for (x, y, p) in src.enumerate_pixels() {
        if transparent_black && p[0] < 40 && p[1] < 40 && p[2] < 40 {
            continue;
        }
        let ox = dx + x;
        let oy = dy + y;
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

fn main() -> anyhow::Result<()> {
    let rom_path = arg("--rom", "smw.smc");
    let level_num: u32 = u32::from_str_radix(arg("--level", "0x105").trim_start_matches("0x"), 16).unwrap_or(0x105);
    let out_path = arg("--out", "docs/screenshots/clipboard.png");

    let raw = std::fs::read(&rom_path).with_context(|| format!("read {rom_path}"))?;
    let rom_bytes: Vec<u8> = if raw.len() % 0x400 == 0x200 { raw[0x200..].to_vec() } else { raw };

    // ── Load + render the level (same path as render_level.rs) ──
    let mut emu_rom = EmuRom::new(rom_bytes.clone());
    emu_rom.load_symbols(include_str!("../../symbols/SMW_U.sym"));
    let mut cpu = Cpu::new(CheckedMem::new(Arc::new(emu_rom)));
    smwe_emu::emu::decompress_sublevel(&mut cpu, level_num as u16);

    let vertical = cpu.mem.load_u8(0x5B) & 1 != 0;
    let level_mode = cpu.mem.load_u8(0x1925);
    let renderer_table = cpu.mem.cart.resolve("CODE_058955").unwrap() + 9;
    let renderer = cpu.mem.load_u24(renderer_table + (level_mode as u32) * 3);
    let l2_renderers = [cpu.mem.cart.resolve("CODE_058B8D"), cpu.mem.cart.resolve("CODE_058C71")];
    let has_layer2 = l2_renderers.contains(&Some(renderer));
    let scr_len = match (vertical, has_layer2) {
        (false, false) => 0x20,
        (true, false) => 0x1C,
        (false, true) => 0x10,
        (true, true) => 0x0E,
    } as u32;
    let (width, height) = if vertical { (32 * 16, scr_len * 16 * 16) } else { (scr_len * 16 * 16, 27 * 16) };
    let mut pixels = vec![0u8; (width * height * 3) as usize];
    render_layer(&mut cpu, false, width, &mut pixels);
    render_layer(&mut cpu, true, width, &mut pixels);

    // ── Pick two demo objects ──
    let rom = Rom::new(rom_bytes).map_err(|e| anyhow::anyhow!("Rom::new: {e:?}"))?;
    let level = Level::parse(&rom, level_num).map_err(|e| anyhow::anyhow!("Level::parse: {e:?}"))?;
    let objs = Object::parse_from_layer(level.layer1.as_bytes()).unwrap_or_default();
    let mut screen = 0u32;
    let mut screen_x = 0u32;
    let mut cands: Vec<(u32, u32, u32, u32)> = Vec::new();
    for obj in objs {
        if obj.is_new_screen() {
            screen += 1;
            screen_x = screen * 16;
            continue;
        }
        if obj.is_exit() || obj.is_screen_jump() || obj.is_extended() {
            continue;
        }
        let w = (obj.settings() & 0x0F) as u32 + 1;
        let h = (obj.settings() >> 4) as u32 + 1;
        let (ox, oy) = (screen_x + obj.x() as u32, obj.y() as u32);
        if w >= 2 && h >= 2 && ox + w <= width / 16 && oy + h <= height / 16 {
            cands.push((ox, oy, w, h));
        }
    }
    anyhow::ensure!(cands.len() >= 2, "level has fewer than 2 demo objects");
    // Two objects a few tiles apart: compact crop, and the paste offset
    // (+3/+2 tiles) won't overlap the originals.
    let (a, b) = cands
        .iter()
        .enumerate()
        .flat_map(|(i, &o1)| cands[i + 1..].iter().map(move |&o2| (o1, o2)))
        .filter(|&(o1, o2)| {
            let d = o1.0.abs_diff(o2.0) + o1.1.abs_diff(o2.1);
            (6..=30).contains(&d)
        })
        .next()
        .unwrap_or((cands[0], cands[1]));
    eprintln!("demo objects at ({},{}) {}x{} and ({},{}) {}x{}", a.0, a.1, a.2, a.3, b.0, b.1, b.2, b.3);

    // ── Crop around both ──
    let m = 40u32;
    let x0 = a.0.min(b.0) * 16;
    let y0 = a.1.min(b.1) * 16;
    let x1 = (a.0 + a.2).max(b.0 + b.2) * 16;
    let y1 = (a.1 + a.3).max(b.1 + b.3) * 16;
    let cx0 = x0.saturating_sub(m);
    let cy0 = y0.saturating_sub(m);
    let cx1 = (x1 + m).min(width);
    let cy1 = (y1 + m).min(height);
    let cw = cx1 - cx0;
    let ch = cy1 - cy0;
    let crop: Vec<u8> = (cy0..cy1)
        .flat_map(|y| {
            let row = (y * width * 3) as usize;
            let col = (cx0 * 3) as usize;
            pixels[row + col..row + col + (cw * 3) as usize].to_vec()
        })
        .collect();

    // ── SMW message font for chrome text ──
    let smw_rom = smwe_rom::SmwRom::from_file(&rom_path)?;
    let font = smwe_rom::message_raster::decompress_message_font(&smw_rom.rom)?;

    // ── Layout ──
    let toolbar_h = 84u32;
    let status_h = 128u32;
    let fw = cw.max(720);
    let fh = toolbar_h + ch + status_h;
    let mut frame = vec![0u8; (fw * fh * 3) as usize];
    // Center the level crop horizontally if the frame is wider.
    let crop_dx = (fw - cw) / 2;

    // Toolbar strip: dark slate with three labeled buttons.
    fill(&mut frame[..(fw * toolbar_h * 3) as usize], [30, 30, 44]);
    let btn_labels = ["CUT", "COPY", "PASTE"];
    let btn_w = 120u32;
    let btn_h = 44u32;
    let mut bx = 16u32;
    for label in btn_labels {
        // Button face.
        for y in 20..20 + btn_h {
            for x in bx..bx + btn_w {
                let i = ((y * fw + x) * 3) as usize;
                let edge = x == bx || x + 1 == bx + btn_w || y == 20 || y + 1 == 20 + btn_h;
                let c = if edge { [120, 120, 150] } else { [58, 58, 78] };
                frame[i..i + 3].copy_from_slice(&c);
            }
        }
        let t = text_line(label, &font, 3);
        blit_text(&mut frame, fw, &t, bx + (btn_w - t.width()) / 2, 20 + (btn_h - t.height()) / 2, true);
        bx += btn_w + 12;
    }
    let cap = text_line("CLIPBOARD", &font, 3);
    blit_text(&mut frame, fw, &cap, bx + 8, 20 + (btn_h - cap.height()) / 2, true);

    // Selection overlay helper: translucent blue fill + yellow outline.
    let select = |buf: &mut [u8], stride: u32, ox: u32, oy: u32, ow: u32, oh: u32, rgb: [u8; 3]| {
        let rx = ox * 16 - cx0;
        let ry = oy * 16 - cy0;
        let rw = ow * 16;
        let rh = oh * 16;
        fill_rect_raw(buf, stride, rx, ry, rw, rh, [80, 120, 255, 90]);
        stroke_rect(buf, stride, rx, ry, rw, rh, rgb, 2);
    };

    // Status strip painter (three lines, all 2x so they fit 720px).
    let paint_status = |frame: &mut [u8], l1: &str, l2: &str, l3: &str| {
        let sy = toolbar_h + ch;
        fill(&mut frame[(sy * fw * 3) as usize..((sy + status_h) * fw * 3) as usize], [22, 22, 32]);
        let t1 = text_line(l1, &font, 2);
        let t2 = text_line(l2, &font, 2);
        let t3 = text_line(l3, &font, 2);
        blit_text(frame, fw, &t1, 16, sy + 10, true);
        blit_text(frame, fw, &t2, 16, sy + 10 + t1.height() + 8, true);
        blit_text(frame, fw, &t3, 16, sy + 10 + t1.height() + 8 + t2.height() + 8, true);
    };

    // Blit the level crop centered.
    let blit_crop = |frame: &mut [u8], px: &[u8]| {
        let dst_y = toolbar_h;
        for row in 0..ch {
            let d = ((dst_y + row) * fw + crop_dx) as usize * 3;
            let s = (row * cw) as usize * 3;
            frame[d..d + (cw as usize) * 3].copy_from_slice(&px[s..s + (cw as usize) * 3]);
        }
    };

    // ── Frame 1: copied ──
    let mut f1 = frame.clone();
    blit_crop(&mut f1, &crop);
    {
        let mut level_part = crop.clone();
        select(&mut level_part, cw, a.0, a.1, a.2, a.3, [255, 220, 0]);
        select(&mut level_part, cw, b.0, b.1, b.2, b.3, [255, 220, 0]);
        blit_crop(&mut f1, &level_part);
    }
    paint_status(
        &mut f1,
        "Copied two objects to clipboard.",
        "Ctrl X cut. Ctrl C copy. Ctrl V paste.",
        "App buttons are icon only.",
    );

    if out_path.ends_with(".gif") {
        // ── Frame 2: pasted at +3/+2 tiles ──
        let mut f2 = frame.clone();
        let mut level_px = crop.clone();
        let (dx, dy) = (3u32 * 16, 2u32 * 16);
        // Snapshot both footprints first: one object's stamp can overlap
        // another object's source rect.
        let mut snaps: Vec<(u32, u32, u32, u32, Vec<u8>)> = Vec::new();
        for &(ox, oy, ow, oh) in &[a, b] {
            let sx = ox * 16 - cx0;
            let sy = oy * 16 - cy0;
            let pw = ow * 16;
            let ph = oh * 16;
            let mut tmp: Vec<u8> = Vec::with_capacity((pw * ph * 3) as usize);
            for row in 0..ph {
                let s = ((sy + row) * cw + sx) as usize * 3;
                tmp.extend_from_slice(&level_px[s..s + (pw as usize) * 3]);
            }
            snaps.push((sx, sy, pw, ph, tmp));
        }
        for (n, &(ox, oy, _ow, _oh)) in [a, b].iter().enumerate() {
            let (sx, sy, pw, ph, tmp) = &snaps[n];
            let (pw, ph) = (*pw, *ph);
            let tx = sx + dx;
            let ty = sy + dy;
            // Stamp the footprint pixels (what the editor's paste does).
            for row in 0..ph {
                let d = ((ty + row) * cw + tx) as usize * 3;
                if d + (pw as usize) * 3 <= level_px.len() {
                    let s = (row * pw) as usize * 3;
                    level_px[d..d + (pw as usize) * 3].copy_from_slice(&tmp[s..s + (pw as usize) * 3]);
                }
            }
            // Pasted selection in green at the new spot.
            let rx = ox * 16 - cx0 + dx;
            let ry = oy * 16 - cy0 + dy;
            fill_rect_raw(&mut level_px, cw, rx, ry, pw, ph, [80, 200, 120, 90]);
            stroke_rect(&mut level_px, cw, rx, ry, pw, ph, [120, 255, 150], 2);
        }
        blit_crop(&mut f2, &level_px);
        paint_status(
            &mut f2,
            "Pasted two objects at cursor.",
            "Ctrl Z undoes the paste.",
            "Footprint tiles stamp with the objects.",
        );

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
            enc.encode_frame(image::Frame::from_parts(rgba, 0, 0, Delay::from_numer_denom_ms(900, 1)))?;
        }
        eprintln!("wrote {out_path}");
    } else {
        let img = RgbImage::from_raw(fw, fh, f1).expect("frame size");
        img.save(&out_path)?;
        eprintln!("wrote {out_path}");
    }
    Ok(())
}
