//! Headless screenshot of palette/Map16 editor undo+redo (LM v1.80/v1.91 parity).
//!
//! egui can't render headless, so this composes an honest mock of the two
//! editors' new Undo/Redo buttons. Everything stateful is real program
//! output:
//! - The undo engine is the real `smw_editor::undo::UndoableData` (same delta
//!   compression, same undo/redo semantics the UI calls). The data structs
//!   below mirror the editor's private `EditablePalettes` /
//!   `EditableMap16Edits` byte-for-byte (72-byte LE u16 palette blob;
//!   sorted 10-byte block records); their real counterparts are covered by
//!   unit tests in the editor modules.
//! - Palette colors are read from the real ROM for level 0x105 through the
//!   same `0x00B0B0/0x00B190/0x00B318 + index*0x18` address math the editor's
//!   load path uses.
//! - The Map16 block (0x101) tile words come from the real ROM via the same
//!   `smwe_rom::map16_file::export_page` path the editor's import/export
//!   uses, and the block is rasterized through the same emulator VRAM/CGRAM
//!   sub-tile path the Map16 Block Editor previews with.
//!
//! The three panels per editor show: before edit → after a real `write()`
//! edit → after a real `undo()` (which must be pixel-identical to "before").
//!
//! ```sh
//! cargo run --bin render_undo_editors -- --out=docs/screenshots/palette-map16-undo.png --rom=smw.smc
//! ```

use std::{collections::BTreeMap, sync::Arc};

use ab_glyph::{Font, FontRef, Glyph, Point, PxScale, ScaleFont};
use image::{Rgb, RgbImage};
use smw_editor::{
    render_util::{fill_rect, rect_border},
    undo::{Undo, UndoableData},
};
use smwe_emu::{emu::CheckedMem, rom::Rom as EmuRom, Cpu};
use smwe_rom::snes_utils::addr::{AddrPc, AddrSnes};

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

/// Verbatim copy of the editor tile picker's sub-tile renderer
/// (`src/ui/editor_prototypes/level_editor/tile_picker.rs::render_sub_tile`).
fn render_sub_tile(vram: &[u8], cgram: &[u8], t: u16, x0: u32, y0: u32, pixels: &mut [u8], stride: usize) {
    let tile_num = (t & 0x3FF) as usize;
    let pal = ((t >> 10) & 0x7) as usize;
    let flip_x = (t & 0x4000) != 0;
    let flip_y = (t & 0x8000) != 0;

    let tile_base = tile_num * 32;
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

            let pal_idx = pal * 16 + color_idx;
            let off_color = pal_idx * 2;
            if off_color + 1 >= cgram.len() {
                continue;
            }
            let lo = cgram[off_color] as u16;
            let hi = cgram[off_color + 1] as u16;
            let rgb = lo | (hi << 8);

            let r = ((rgb & 0x1F) << 3) as u8;
            let g = (((rgb >> 5) & 0x1F) << 3) as u8;
            let b = (((rgb >> 10) & 0x1F) << 3) as u8;

            let px_abs = x0 + tx;
            let py_abs = y0 + ty;
            let off = ((py_abs as usize) * stride + px_abs as usize) * 4;
            if off + 3 < pixels.len() {
                pixels[off] = r;
                pixels[off + 1] = g;
                pixels[off + 2] = b;
                pixels[off + 3] = 255;
            }
        }
    }
}

/// Render one 16x16 Map16 block (four tile words) at `scale` with a
/// checkerboard behind transparent pixels (same as the Map16 Block Editor).
fn render_block(
    vram: &[u8], cgram: &[u8], words: &[u16; 4], x0: u32, y0: u32, scale: u32, pixels: &mut [u8], stride: u32,
) {
    for y in 0..16 * scale {
        for x in 0..16 * scale {
            let checker = ((x / 4 + y / 4) % 2) == 0;
            let shade = if checker { 52u8 } else { 84u8 };
            let off = (((y0 + y) * stride + (x0 + x)) * 4) as usize;
            pixels[off] = shade;
            pixels[off + 1] = shade;
            pixels[off + 2] = shade;
            pixels[off + 3] = 255;
        }
    }
    let mut small = vec![0u8; 16 * 16 * 4];
    let quads = [(0u32, 0u32), (0, 8), (8, 0), (8, 8)];
    for (i, &(qx, qy)) in quads.iter().enumerate() {
        render_sub_tile(vram, cgram, words[i], qx, qy, &mut small, 16);
    }
    for sy in 0..16u32 {
        for sx in 0..16u32 {
            let s = ((sy * 16 + sx) * 4) as usize;
            if small[s + 3] == 0 {
                continue;
            }
            for dy in 0..scale {
                for dx in 0..scale {
                    let d = (((y0 + sy * scale + dy) * stride + (x0 + sx * scale + dx)) * 4) as usize;
                    pixels[d..d + 4].copy_from_slice(&small[s..s + 4]);
                }
            }
        }
    }
}

// ── Mirror data structs ──────────────────────────────────────────────────────
// Byte-for-byte mirrors of the editor's private `EditablePalettes` and
// `EditableMap16Edits` (same field order, same serialization), so the
// screenshot drives the real public `UndoableData` engine with identical
// bytes.

#[derive(Clone, Debug, Default)]
struct MirrorPalettes {
    bg:     [u16; 12],
    fg:     [u16; 12],
    sprite: [u16; 12],
}

impl Undo for MirrorPalettes {
    fn from_bytes(bytes: Vec<u8>) -> Self {
        let mut p = Self::default();
        for (i, chunk) in bytes.chunks_exact(2).enumerate().take(36) {
            let v = u16::from_le_bytes([chunk[0], chunk[1]]);
            let arr = match i / 12 {
                0 => &mut p.bg,
                1 => &mut p.fg,
                _ => &mut p.sprite,
            };
            arr[i % 12] = v;
        }
        p
    }

    fn to_bytes(&self) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(72);
        for &v in self.bg.iter().chain(self.fg.iter()).chain(self.sprite.iter()) {
            bytes.extend_from_slice(&v.to_le_bytes());
        }
        bytes
    }

    fn size_bytes(&self) -> usize {
        72
    }
}

#[derive(Clone, Debug, Default)]
struct MirrorMap16Edits {
    edits: BTreeMap<u16, [u16; 4]>,
}

impl Undo for MirrorMap16Edits {
    fn from_bytes(bytes: Vec<u8>) -> Self {
        let mut edits = BTreeMap::new();
        for chunk in bytes.chunks_exact(10) {
            let block_id = u16::from_le_bytes([chunk[0], chunk[1]]);
            let mut words = [0u16; 4];
            for (i, w) in words.iter_mut().enumerate() {
                *w = u16::from_le_bytes([chunk[2 + i * 2], chunk[3 + i * 2]]);
            }
            edits.insert(block_id, words);
        }
        Self { edits }
    }

    fn to_bytes(&self) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(self.edits.len() * 10);
        for (&block_id, words) in &self.edits {
            bytes.extend_from_slice(&block_id.to_le_bytes());
            for &w in words {
                bytes.extend_from_slice(&w.to_le_bytes());
            }
        }
        bytes
    }

    fn size_bytes(&self) -> usize {
        self.edits.len() * 10
    }
}

fn abgr1555_to_rgb(v: u16) -> Rgb<u8> {
    let r = ((v & 0x1F) as u32 * 255 / 31) as u8;
    let g = (((v >> 5) & 0x1F) as u32 * 255 / 31) as u8;
    let b = (((v >> 10) & 0x1F) as u32 * 255 / 31) as u8;
    Rgb([r, g, b])
}

/// Same address math as the editor's palette load path
/// (`src/ui/editor_prototypes/level_editor/mod.rs`).
fn read_palette(rom_bytes: &[u8], snes_addr: u32) -> [u16; 12] {
    let mut colors = [0u16; 12];
    if let Ok(pc) = AddrPc::try_from_lorom(AddrSnes(snes_addr)) {
        let base = pc.as_index();
        for (i, c) in colors.iter_mut().enumerate() {
            let off = base + i * 2;
            if off + 1 < rom_bytes.len() {
                *c = rom_bytes[off] as u16 | ((rom_bytes[off + 1] as u16) << 8);
            }
        }
    }
    colors
}

fn draw_undo_redo_buttons(img: &mut RgbImage, font: &FontRef, x: u32, y: u32, can_undo: bool, can_redo: bool) {
    let draw_btn = |img: &mut RgbImage, x: u32, label: &str, enabled: bool| {
        let w = 64u32;
        let h = 24u32;
        let bg = if enabled { Rgb([62, 66, 74]) } else { Rgb([40, 42, 48]) };
        fill_rect(img, x, y, w, h, bg);
        rect_border(img, x, y, w, h, Rgb([110, 114, 122]));
        let fg = if enabled { Rgb([235, 235, 240]) } else { Rgb([120, 122, 130]) };
        draw_text(img, font, label, (x + 12) as i32, (y + 5) as i32, 13.0, fg);
    };
    draw_btn(img, x, "Undo", can_undo);
    draw_btn(img, x + 72, "Redo", can_redo);
}

fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let output =
        args.iter().find_map(|a| a.strip_prefix("--out=")).unwrap_or("docs/screenshots/palette-map16-undo.png");
    let rom_path = args
        .iter()
        .find_map(|a| a.strip_prefix("--rom="))
        .or_else(|| args.iter().skip(1).find(|a| !a.starts_with("--")).map(|a| a.as_str()))
        .unwrap_or("smw.smc");

    let sans = load_font(SANS_CANDIDATES)?;
    let sans_bold = load_font(SANS_BOLD_CANDIDATES)?;

    // ── Real ROM data ────────────────────────────────────────────────────
    let rom = smwe_rom::SmwRom::from_file(rom_path)?;
    let rom_bytes = rom.rom_bytes();
    let level = &rom.levels[0x105];
    let palette_bg = level.primary_header.palette_bg();
    let palette_fg = level.primary_header.palette_fg();
    let palette_sprite = level.primary_header.palette_sprite();
    let bg0 = read_palette(rom_bytes, 0x00B0B0 + palette_bg as u32 * 0x18);
    let fg0 = read_palette(rom_bytes, 0x00B190 + palette_fg as u32 * 0x18);
    let sp0 = read_palette(rom_bytes, 0x00B318 + palette_sprite as u32 * 0x18);

    // Block 0x101's real tile words (FG page 1) via the map16 export path.
    let page_data = smwe_rom::map16_file::export_page(&rom, smwe_rom::map16_file::PAGE_FG1, 0)?;
    let b = 0x01usize * 8; // block 0x101 is index 1 of FG page 1
    let words0 = [
        u16::from_le_bytes([page_data[b], page_data[b + 1]]),
        u16::from_le_bytes([page_data[b + 2], page_data[b + 3]]),
        u16::from_le_bytes([page_data[b + 4], page_data[b + 5]]),
        u16::from_le_bytes([page_data[b + 6], page_data[b + 7]]),
    ];

    // Emulator VRAM/CGRAM for level 0x105 — the source the editor's Map16
    // Block Editor previews render from.
    let file_bytes = std::fs::read(rom_path)?;
    let mut emu_rom = EmuRom::new(file_bytes);
    emu_rom.load_symbols(include_str!("../../symbols/SMW_U.sym"));
    let mut cpu = Cpu::new(CheckedMem::new(Arc::new(emu_rom)));
    smwe_emu::emu::decompress_sublevel(&mut cpu, 0x105);
    let vram = cpu.mem.vram.clone();
    let cgram = cpu.mem.cgram.clone();

    // ── Real undo-engine state transitions ───────────────────────────────
    let mut palettes = UndoableData::new(MirrorPalettes { bg: bg0, fg: fg0, sprite: sp0 });
    let pal_before = palettes.read(|p| p.to_bytes());
    // One committed edit (a single write() = a single undo step, like the
    // UI's gesture commit): BG color 3 -> full red, sprite color 5 -> full
    // green.
    palettes.write(|p| {
        p.bg[3] = 0x001F;
        p.sprite[5] = 0x03E0;
    });
    let pal_after = palettes.read(|p| p.to_bytes());
    assert_ne!(pal_before, pal_after, "edit must change the palette bytes");
    assert!(palettes.can_undo() && !palettes.can_redo());
    palettes.undo();
    let pal_undone = palettes.read(|p| p.to_bytes());
    assert_eq!(pal_undone, pal_before, "undo must restore the exact pre-edit bytes");
    assert!(!palettes.can_undo() && palettes.can_redo());
    palettes.redo();
    assert_eq!(palettes.read(|p| p.to_bytes()), pal_after, "redo must restore the edit");

    let mut map16 = UndoableData::new(MirrorMap16Edits::default());
    // One committed edit: retile block 0x101's upper-left 8x8 (+0x20 tiles
    // over) and bump word 2's palette row, keeping flip/priority bits.
    let words1 = [
        (words0[0] & !0x3FF) | ((words0[0] + 0x20) & 0x3FF),
        words0[1],
        (words0[2] & !(0x7 << 10)) | ((((words0[2] >> 10) + 1) & 0x7) << 10),
        words0[3],
    ];
    map16.write(|e| {
        e.edits.insert(0x101, words1);
    });
    assert!(map16.can_undo() && !map16.can_redo());
    map16.undo();
    assert!(map16.read(|e| e.edits.is_empty()), "undo must drop the block edit");
    assert!(!map16.can_undo() && map16.can_redo());
    map16.redo();
    assert_eq!(map16.read(|e| e.edits[&0x101]), words1, "redo must restore the edit");

    // Snapshots for the three panels of each editor.
    let pal_states = [pal_before.clone(), pal_after.clone(), pal_undone.clone()];
    let map16_states: [Option<[u16; 4]>; 3] = [None, Some(words1), None];

    // ── Compose the image ────────────────────────────────────────────────
    let w = 1240u32;
    let h = 1010u32;
    let mut img = RgbImage::new(w, h);
    fill_rect(&mut img, 0, 0, w, h, Rgb([26, 28, 33]));
    let white = Rgb([235, 235, 240]);
    let gray = Rgb([170, 174, 182]);
    let dim = Rgb([120, 124, 132]);

    draw_text(
        &mut img,
        &sans_bold,
        "Undo / redo in the Palette + Map16 editors  (Lunar Magic v1.80 / v1.91 parity)",
        24,
        18,
        20.0,
        white,
    );

    // ── Palette section ──────────────────────────────────────────────────
    draw_text(&mut img, &sans_bold, "Palette Editor — Undo/Redo buttons + Ctrl+Z / Ctrl+Y", 24, 58, 16.0, white);
    let captions = [
        "1. Before edit",
        "2. After edit — BG color 3 set to red, sprite color 5 to green",
        "3. After undo (Ctrl+Z) — identical to panel 1",
    ];
    let btn_states = [(false, false), (true, false), (false, true)];
    let group_names = ["BG Palette", "FG Palette", "Sprite Palette"];
    let group_idx = [palette_bg, palette_fg, palette_sprite];
    for (pi, caption) in captions.iter().enumerate() {
        let x0 = 24 + pi as u32 * 400;
        let y0 = 88u32;
        let ww = 376u32;
        // Window chrome.
        fill_rect(&mut img, x0, y0, ww, 300, Rgb([43, 46, 53]));
        rect_border(&mut img, x0, y0, ww, 300, Rgb([100, 104, 112]));
        fill_rect(&mut img, x0, y0, ww, 26, Rgb([52, 56, 64]));
        draw_text(&mut img, &sans_bold, "Palette Editor", (x0 + 10) as i32, (y0 + 5) as i32, 14.0, white);
        let (cu, cr) = btn_states[pi];
        draw_undo_redo_buttons(&mut img, &sans, x0 + 10, y0 + 34, cu, cr);

        let pal = MirrorPalettes::from_bytes(pal_states[pi].clone());
        let groups = [&pal.bg, &pal.fg, &pal.sprite];
        let mut gy = y0 + 68;
        for (gi, colors) in groups.iter().enumerate() {
            draw_text(
                &mut img,
                &sans,
                &format!("{} (index {:X})", group_names[gi], group_idx[gi]),
                (x0 + 10) as i32,
                gy as i32,
                13.0,
                gray,
            );
            gy += 20;
            for (ci, &c) in colors.iter().enumerate() {
                let sx = x0 + 10 + ci as u32 * 30;
                fill_rect(&mut img, sx, gy, 28, 22, abgr1555_to_rgb(c));
                rect_border(&mut img, sx, gy, 28, 22, Rgb([80, 80, 88]));
                // White selection ring on the edited swatches, like the UI.
                let edited = pi > 0 && ((gi == 0 && ci == 3) || (gi == 2 && ci == 5));
                if edited {
                    rect_border(&mut img, sx - 1, gy - 1, 30, 24, Rgb([255, 255, 255]));
                }
            }
            gy += 30;
        }
        draw_text(&mut img, &sans, caption, (x0 + 4) as i32, (y0 + 308) as i32, 13.0, dim);
    }

    // ── Map16 section ────────────────────────────────────────────────────
    let my = 450u32;
    draw_text(&mut img, &sans_bold, "Map16 Block Editor — Ctrl+Z / Ctrl+Y", 24, my as i32, 16.0, white);
    let m_captions = [
        "1. Before edit — block 0101 from the ROM",
        "2. After edit — tile words changed",
        "3. After undo (Ctrl+Z) — identical to panel 1",
    ];
    for (pi, caption) in m_captions.iter().enumerate() {
        let x0 = 24 + pi as u32 * 400;
        let y0 = my + 30;
        let ww = 376u32;
        let wh = 380u32;
        fill_rect(&mut img, x0, y0, ww, wh, Rgb([43, 46, 53]));
        rect_border(&mut img, x0, y0, ww, wh, Rgb([100, 104, 112]));
        fill_rect(&mut img, x0, y0, ww, 26, Rgb([52, 56, 64]));
        draw_text(&mut img, &sans_bold, "Map16 Block Editor", (x0 + 10) as i32, (y0 + 5) as i32, 14.0, white);
        let (cu, cr) = btn_states[pi];
        draw_undo_redo_buttons(&mut img, &sans, x0 + 10, y0 + 34, cu, cr);
        draw_text(&mut img, &sans, "Block: 0101", (x0 + 10) as i32, (y0 + 68) as i32, 13.0, gray);

        let words = map16_states[pi].unwrap_or(words0);
        // Render the block at 8x into a scratch buffer, then blit.
        let scale = 8u32;
        let mut buf = vec![0u8; (16 * scale * 16 * scale * 4) as usize];
        render_block(&vram, &cgram, &words, 0, 0, scale, &mut buf, 16 * scale);
        let bx = x0 + 10;
        let by = y0 + 92;
        for yy in 0..16 * scale {
            for xx in 0..16 * scale {
                let s = ((yy * 16 * scale + xx) * 4) as usize;
                img.put_pixel(bx + xx, by + yy, Rgb([buf[s], buf[s + 1], buf[s + 2]]));
            }
        }
        rect_border(&mut img, bx, by, 16 * scale, 16 * scale, Rgb([100, 104, 112]));

        let mut ty = by + 16 * scale + 12;
        for (i, word) in words.iter().enumerate() {
            let changed_word = pi == 1 && words0[i] != *word;
            draw_text(
                &mut img,
                &sans,
                &format!("tile{}: {:04X}{}", i, word, if changed_word { "  <- edited" } else { "" }),
                (x0 + 10) as i32,
                ty as i32,
                13.0,
                if changed_word { Rgb([255, 210, 120]) } else { gray },
            );
            ty += 20;
        }
        draw_text(&mut img, &sans, caption, (x0 + 4) as i32, (y0 + wh + 8) as i32, 13.0, dim);
    }

    draw_text(
        &mut img,
        &sans,
        "Real UndoableData engine: edit = write(), undo = undo(), redo = redo(). Palette colors + block 0101 read from the real ROM (level 0x105).",
        24,
        (h - 28) as i32,
        13.0,
        dim,
    );

    img.save(output)?;
    println!("wrote {output}");
    Ok(())
}
