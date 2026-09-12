// Render a real-ROM level with a selected object and its LM-style drag
// handles, for the drag-handles PR screenshot.
//   --rom=PATH --level=0x105 --out=out.png            -> static screenshot
//   --rom=PATH --level=0x105 --out=out.gif --frames=8 -> animated resize drag
use std::sync::Arc;

use anyhow::Context;
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

fn main() -> anyhow::Result<()> {
    let rom_path = arg("--rom", "smw.smc");
    let level_num: u32 =
        u32::from_str_radix(arg("--level", "0x105").trim_start_matches("0x"), 16).unwrap_or(0x105);
    let out_path = arg("--out", "docs/screenshots/drag-handles.png");
    let frames: usize = arg("--frames", "8").parse().unwrap_or(8);

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

    // ── Find a good demo object ──
    let rom = Rom::new(rom_bytes).map_err(|e| anyhow::anyhow!("Rom::new: {e:?}"))?;
    let level = Level::parse(&rom, level_num).map_err(|e| anyhow::anyhow!("Level::parse: {e:?}"))?;
    let objs = Object::parse_from_layer(level.layer1.as_bytes()).unwrap_or_default();
    let mut screen = 0u32;
    let mut screen_x = 0u32;
    let mut demo: Option<(u32, u32, u32, u32)> = None;
    let mut first: Option<(u32, u32, u32, u32)> = None;
    for obj in objs {
        if obj.is_new_screen() {
            // Screen-exit marker: subsequent objects belong to the next screen.
            // (screen_number() debug-asserts on some marker variants, so count instead.)
            screen += 1;
            screen_x = screen * 16;
            continue;
        }
        if obj.is_exit() || obj.is_screen_jump() {
            continue; // goal-tape / screen-jump markers: not drawable objects
        }
        let (w, h) = if obj.is_extended() {
            (1, 1)
        } else {
            ((obj.settings() & 0x0F) as u32 + 1, (obj.settings() >> 4) as u32 + 1)
        };
        let entry = (screen_x + obj.x() as u32, obj.y() as u32, w, h);
        // Only consider objects fully inside the rendered level for the demo.
        let fits = entry.0 + w <= width / 16 && entry.1 + h <= height / 16;
        if first.is_none() && fits {
            first = Some(entry);
        }
        // Prefer a sizable non-extended object: handles show better.
        if demo.is_none() && !obj.is_extended() && w >= 3 && h >= 3 && fits {
            demo = Some(entry);
        }
    }
    let (ox, oy, ow, oh) = demo.or(first).expect("level has no objects");
    eprintln!("demo object at ({ox},{oy}) {ow}x{oh} in level {level_num}");

    // ── Crop around the object ──
    let m = 48u32; // margin in px
    let cx0 = (ox * 16).saturating_sub(m).min(width - 1);
    let cy0 = (oy * 16).saturating_sub(m).min(height - 1);
    let cx1 = (ox * 16 + ow * 16 + m).min(width);
    let cy1 = (oy * 16 + oh * 16 + m).min(height);
    let cw = cx1 - cx0;
    let ch = cy1 - cy0;
    let crop: Vec<u8> = (cy0..cy1)
        .flat_map(|y| {
            let row = (y * width * 3) as usize;
            let col = (cx0 * 3) as usize;
            pixels[row + col..row + col + (cw * 3) as usize].to_vec()
        })
        .collect();
    let draw_overlay = |buf: &mut [u8], gw: u32, gh: u32, cursor: bool| {
        // Object rect relative to the crop.
        let rx = ox * 16 - cx0;
        let ry = oy * 16 - cy0;
        let rw = gw * 16;
        let rh = gh * 16;
        // Translucent blue fill.
        fill_rect(buf, cw, rx, ry, rw, rh, [80, 120, 255, 90]);
        // Yellow selection outline.
        stroke_rect(buf, cw, rx, ry, rw, rh, [255, 220, 0], 2);
        // 8 LM-style handles: white squares, black border.
        let hs = 7u32;
        let xs = [rx, rx + rw / 2, rx + rw - 1];
        let ys = [ry, ry + rh / 2, ry + rh - 1];
        for (hx, hy) in [(0, 0), (1, 0), (2, 0), (2, 1), (2, 2), (1, 2), (0, 2), (2 - 2, 1)] {
            let hcx = xs[hx];
            let hcy = ys[hy];
            let x0 = hcx.saturating_sub(hs / 2);
            let y0 = hcy.saturating_sub(hs / 2);
            fill_rect(buf, cw, x0, y0, hs, hs, [255, 255, 255, 255]);
            stroke_rect(buf, cw, x0, y0, hs, hs, [0, 0, 0], 1);
        }
        if cursor {
            // Little white arrow cursor grabbing the SE handle.
            let ax = rx + rw + 2;
            let ay = ry + rh + 2;
            for i in 0..11u32 {
                hline(buf, cw, ax, ay + i, i + 1, [255, 255, 255, 255]);
            }
            hline(buf, cw, ax + 1, ay + 11, 8, [255, 255, 255, 255]);
            hline(buf, cw, ax + 2, ay + 12, 5, [255, 255, 255, 255]);
            hline(buf, cw, ax + 3, ay + 13, 2, [255, 255, 255, 255]);
        }
    };

    if out_path.ends_with(".gif") {
        // Animate an SE-handle resize drag: the object grows from (ow,oh)
        // to (ow+4, oh+3) tile by tile, cursor on the SE handle.
        use image::codecs::gif::{GifEncoder, Repeat};
        use image::Delay;
        let file = std::fs::File::create(&out_path)?;
        let mut enc = GifEncoder::new(file);
        enc.set_repeat(Repeat::Infinite)?;
        let steps = frames.max(2);
        for f in 0..steps {
            let t = f as f32 / (steps - 1) as f32;
            let gw = ow + (4.0 * t).round() as u32;
            let gh = oh + (3.0 * t).round() as u32;
            let mut frame = crop.clone();
            draw_overlay(&mut frame, gw, gh, true);
            let img = image::RgbImage::from_raw(cw, ch, frame).expect("crop size");
            let rgba = image::DynamicImage::ImageRgb8(img).to_rgba8();
            enc.encode_frame(image::Frame::from_parts(rgba, 0, 0, Delay::from_numer_denom_ms(133, 1)))?;
        }
        eprintln!("wrote {out_path}");
    } else {
        let mut frame = crop.clone();
        draw_overlay(&mut frame, ow, oh, false);
        let img = image::RgbImage::from_raw(cw, ch, frame).expect("crop size");
        img.save(&out_path)?;
        eprintln!("wrote {out_path}");
    }
    Ok(())
}

// ── tiny raster helpers ──

fn blend(buf: &mut [u8], idx: usize, rgba: [u8; 4]) {
    let a = rgba[3] as u32;
    for c in 0..3 {
        let dst = buf[idx + c] as u32;
        buf[idx + c] = ((dst * (255 - a) + rgba[c] as u32 * a) / 255) as u8;
    }
}

fn fill_rect(buf: &mut [u8], stride: u32, x: u32, y: u32, w: u32, h: u32, rgba: [u8; 4]) {
    for yy in y..y + h {
        for xx in x..x + w {
            let idx = ((yy * stride + xx) * 3) as usize;
            if idx + 2 < buf.len() {
                blend(buf, idx, rgba);
            }
        }
    }
}

fn stroke_rect(buf: &mut [u8], stride: u32, x: u32, y: u32, w: u32, h: u32, rgb: [u8; 3], t: u32) {
    let c = [rgb[0], rgb[1], rgb[2], 255];
    for i in 0..t {
        hline(buf, stride, x, y + i, w, c);
        hline(buf, stride, x, y + h - 1 - i, w, c);
        vline(buf, stride, x + i, y, h, c);
        vline(buf, stride, x + w - 1 - i, y, h, c);
    }
}

fn hline(buf: &mut [u8], stride: u32, x: u32, y: u32, w: u32, rgba: [u8; 4]) {
    for xx in x..x + w {
        let idx = ((y * stride + xx) * 3) as usize;
        if idx + 2 < buf.len() {
            blend(buf, idx, rgba);
        }
    }
}

fn vline(buf: &mut [u8], stride: u32, x: u32, y: u32, h: u32, rgba: [u8; 4]) {
    for yy in y..y + h {
        let idx = ((yy * stride + x) * 3) as usize;
        if idx + 2 < buf.len() {
            blend(buf, idx, rgba);
        }
    }
}

// ── level rendering (copied from render_level.rs) ──

fn render_layer(cpu: &mut Cpu, bg: bool, width: u32, pixels: &mut [u8]) {
    let mut scratch = cpu.clone();
    let map16_bank = cpu.mem.cart.resolve("Map16Common").expect("Cannot resolve Map16Common") & 0xFF0000;
    let map16_bg = smwe_emu::emu::lm_bg_map16_base(&mut scratch)
        .unwrap_or_else(|| cpu.mem.cart.resolve("Map16BGTiles").expect("Cannot resolve Map16BGTiles"));
    let vertical = cpu.mem.load_u8(0x5B) & if bg { 2 } else { 1 } != 0;
    let mode = cpu.mem.load_u8(0x1925);
    let renderer_table = cpu.mem.cart.resolve("CODE_058955").unwrap() + 9;
    let renderer = cpu.mem.load_u24(renderer_table + (mode as u32) * 3);
    let l2_renderers = [cpu.mem.cart.resolve("CODE_058B8D"), cpu.mem.cart.resolve("CODE_058C71")];
    let has_layer2 = l2_renderers.contains(&Some(renderer));
    let scr_len = match (vertical, has_layer2) {
        (false, false) => 0x20,
        (true, false) => 0x1C,
        (false, true) => 0x10,
        (true, true) => 0x0E,
    };
    let scr_size = if vertical { 16 * 32 } else { 16 * 27 };
    let (blocks_lo_addr, blocks_hi_addr) = match (bg, has_layer2) {
        (true, true) => {
            let offset = scr_len * scr_size;
            (0x7EC800 + offset, 0x7FC800 + offset)
        }
        (true, false) => (0x7EB900, 0x7EBD00),
        (false, _) => (0x7EC800, 0x7FC800),
    };
    let len = if has_layer2 { 256 * 27 } else { 512 * 27 };
    let mut ext_cache: std::collections::HashMap<u16, u32> = std::collections::HashMap::new();

    for idx in 0..len {
        let (block_x, block_y) = if vertical {
            let (screen, sidx) = (idx / (16 * 16), idx % (16 * 16));
            let (row, column) = (sidx / 16, sidx % 16);
            let (sub_y, sub_x) = (screen / 2, screen % 2);
            (column * 16 + sub_x * 256, row * 16 + sub_y * 256)
        } else {
            let (screen, sidx) = (idx / (16 * 27), idx % (16 * 27));
            let (row, column) = (sidx / 16, sidx % 16);
            (column * 16 + screen * 256, row * 16)
        };

        let idx_adj = if bg && !has_layer2 { idx % (16 * 27 * 2) } else { idx };
        let block_id = cpu.mem.load_u8(blocks_lo_addr + idx_adj) as u16
            | (((cpu.mem.load_u8(blocks_hi_addr + idx_adj) as u16) & 0x3F) << 8);
        if block_id == 0 {
            continue;
        }
        let block_ptr = if bg && !has_layer2 {
            block_id as u32 * 8 + map16_bg
        } else if block_id >= 0x200 {
            *ext_cache
                .entry(block_id)
                .or_insert_with(|| smwe_emu::emu::lm_ext_map16_data_addr(&mut scratch, block_id).unwrap_or(0))
        } else {
            cpu.mem.load_u16(0x0FBE + block_id as u32 * 2) as u32 + map16_bank
        };
        if block_ptr == 0 {
            continue;
        }

        for (sub, (off_x, off_y)) in (0..4).zip([(0u32, 0u32), (0, 8), (8, 0), (8, 8)]) {
            let t = cpu.mem.load_u16(block_ptr + sub * 2);
            render_bg_tile(&cpu.mem.vram, &cpu.mem.cgram, block_x + off_x, block_y + off_y, t, width, pixels);
        }
    }
}

fn render_bg_tile(vram: &[u8], cgram: &[u8], x: u32, y: u32, t: u16, width: u32, pixels: &mut [u8]) {
    let tile = (t & 0x3FF) as usize;
    let pal = ((t >> 10) & 0x7) as usize;
    render_tile(vram, cgram, tile, pal, (t & 0x4000) != 0, (t & 0x8000) != 0, x, y, width, pixels);
}

#[allow(clippy::too_many_arguments)]
fn render_tile(
    vram: &[u8], cgram: &[u8], tile_id: usize, palette: usize, flip_x: bool, flip_y: bool, x0: u32, y0: u32,
    width: u32, pixels: &mut [u8],
) {
    let tile_base = tile_id * 32;
    for ty in 0..8u32 {
        for tx in 0..8u32 {
            let px = if flip_x { 7 - tx } else { tx };
            let py = if flip_y { 7 - ty } else { ty };
            let row_off = tile_base + (py as usize) * 2;
            if row_off + 17 >= vram.len() {
                continue;
            }
            let b0 = vram[row_off];
            let b1 = vram[row_off + 1];
            let b2 = vram[row_off + 16];
            let b3 = vram[row_off + 17];
            let bit = 7 - px as usize;
            let color_idx =
                (((b0 >> bit) & 1) | (((b1 >> bit) & 1) << 1) | (((b2 >> bit) & 1) << 2) | (((b3 >> bit) & 1) << 3))
                    as usize;
            if color_idx == 0 {
                continue;
            }
            let rgb = read_color(cgram, palette * 16 + color_idx);
            let off = (((y0 + ty) * width + x0 + tx) * 3) as usize;
            if off + 2 < pixels.len() {
                pixels[off] = rgb[0];
                pixels[off + 1] = rgb[1];
                pixels[off + 2] = rgb[2];
            }
        }
    }
}

fn read_color(cgram: &[u8], idx: usize) -> [u8; 3] {
    let off = idx * 2;
    if off + 1 >= cgram.len() {
        return [0, 0, 0];
    }
    let lo = cgram[off] as u16;
    let hi = cgram[off + 1] as u16;
    let rgb = lo | (hi << 8);
    [((rgb & 0x1F) << 3) as u8, (((rgb >> 5) & 0x1F) << 3) as u8, (((rgb >> 10) & 0x1F) << 3) as u8]
}
