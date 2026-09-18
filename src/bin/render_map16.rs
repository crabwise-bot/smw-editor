//! Headless screenshot of the expanded-Map16 feature set.
//!
//! egui can't render headless, so this composes an honest mock of the new
//! Map16 block editor UI: the FG/BG page selector covering expanded pages
//! 0x02-0x7F, the "Acts like" editor with its Remap dialog, and the modern
//! `.map16` export / export-ALL / import controls. Every data panel is real
//! program output:
//!
//! - The atlas renders expanded FG page 0x02 that was really written into
//!   a scratch ROM copy with `smwe_rom::map16_expanded::write_expanded_fg_page`
//!   (per-page RATS block) and read back through the exact
//!   `smwe_rom::map16_file::export_page_sel` function the UI calls,
//!   rasterized through the same emulator VRAM/CGRAM path the editor's own
//!   tile picker uses (`render_sub_tile`, copied verbatim below).
//! - The "Status" lines are the real strings the export/export-ALL code
//!   paths produce (page counts, act-as entry counts, byte sizes).
//! - The remap preview box shows the real output of the remap operation
//!   (`set_act_range_from_base` + `remap_act_refs`) on a real act-as table.
//! - The hex dump is the head of a real `serialize_modern_partial` file
//!   (`LM16` magic visible), and the section sizes are read back from a
//!   real `serialize_modern_full` export via `parse_modern_map16`.
//!
//! ```sh
//! cargo run --bin render_map16 -- --out=docs/screenshots/map16-expanded.png --rom=smw.smc
//! ```

use std::{collections::HashMap, sync::Arc};

use ab_glyph::{Font, FontRef, Glyph, Point, PxScale, ScaleFont};
use image::{Rgb, RgbImage};
use smwe_emu::{emu::CheckedMem, rom::Rom as EmuRom, Cpu};
use smwe_rom::{
    map16_expanded,
    map16_file::{self, PageSel, MAP16_PAGE_TILES},
    objects::tilesets::TILESETS_COUNT,
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
/// (`src/ui/editor_prototypes/level_editor/tile_picker.rs::render_sub_tile`):
/// decodes one 8x8 4bpp sub-tile from emulator VRAM with the CGRAM palette
/// the tile word selects, honoring flip bits. Transparent pixels are left
/// alone so the caller can pre-fill a background.
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

/// Render one 16x16 Map16 block (four tile words) into `pixels` at `scale`
/// with a checkerboard behind transparent pixels.
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

use smw_editor::render_util::{fill_rect, rect_border};

struct Ctx<'a> {
    img:   RgbImage,
    sans:  &'a FontRef<'a>,
    bold:  &'a FontRef<'a>,
    ink:   Rgb<u8>,
    gray:  Rgb<u8>,
    green: Rgb<u8>,
    dark:  Rgb<u8>,
}

impl<'a> Ctx<'a> {
    fn text(&mut self, t: &str, x: i32, y: i32, px: f32, c: Rgb<u8>) {
        draw_text(&mut self.img, self.sans, t, x, y, px, c);
    }

    fn heading(&mut self, t: &str, x: i32, y: i32) {
        draw_text(&mut self.img, self.bold, t, x, y, 17.0, self.ink);
    }

    fn button(&mut self, label: &str, x: u32, y: u32, w: u32) {
        fill_rect(&mut self.img, x, y, w, 32, Rgb([0xFF, 0xFF, 0xFF]));
        rect_border(&mut self.img, x, y, w, 32, Rgb([0x99, 0x99, 0x99]));
        self.text(label, (x + 10) as i32, (y + 7) as i32, 14.0, self.ink);
    }
}

fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let output = args.iter().find_map(|a| a.strip_prefix("--out=")).unwrap_or("docs/screenshots/map16-expanded.png");
    let rom_path = args
        .iter()
        .find_map(|a| a.strip_prefix("--rom="))
        .or_else(|| args.iter().skip(1).find(|a| !a.starts_with("--")).map(|a| a.as_str()))
        .unwrap_or("smw.smc");

    let sans = load_font(SANS_CANDIDATES)?;
    let sans_bold = load_font(SANS_BOLD_CANDIDATES)?;

    // Emulator VRAM/CGRAM for level 0x105: the same source the editor's
    // tile picker renders from.
    let rom_bytes = std::fs::read(rom_path)?;
    let mut emu_rom = EmuRom::new(rom_bytes.clone());
    emu_rom.load_symbols(include_str!("../../symbols/SMW_U.sym"));
    let mut cpu = Cpu::new(CheckedMem::new(Arc::new(emu_rom)));
    smwe_emu::emu::decompress_sublevel(&mut cpu, 0x105);
    let vram = cpu.mem.vram.clone();
    let cgram = cpu.mem.cgram.clone();

    // ── Real data path 1: expanded FG page 0x02 ──────────────────────────
    // Scratch ROM: expand to 1MB (guaranteed free space), write expanded
    // page 0x02 (FG page 0's tiles, palette-shifted so it's visibly new)
    // and an act-as entry, all through the real editor code paths.
    let rom = smwe_rom::SmwRom::from_file(rom_path)?;
    let page0 = map16_file::export_page(&rom, map16_file::PAGE_FG0, 0)?;
    let mut modified = page0.clone();
    for t in 0..256usize {
        for w in 0..4usize {
            let off = t * 8 + w * 2;
            let word = u16::from_le_bytes([modified[off], modified[off + 1]]);
            let word2 = (word & !(7 << 10)) | ((((word >> 10) + 3) % 8) << 10);
            modified[off..off + 2].copy_from_slice(&word2.to_le_bytes());
        }
    }
    let rom_for_expand = smwe_rom::snes_utils::rom::Rom::new(rom_bytes.clone())?;
    let expanded = smwe_rom::rom_expansion::expand_rom(&rom_for_expand, 0x100000)?;
    let mut scratch = expanded.bytes().to_vec();
    // (expand_rom strips any SMC header; work headerless from here on.)
    map16_expanded::write_expanded_fg_page(&mut scratch, 0, 0x02, &modified)?;
    let mut acts = HashMap::new();
    map16_expanded::set_act_as(&mut acts, 0x205, Some(0x25));
    map16_expanded::write_acts_table(&mut scratch, 0, &acts)?;
    // Read back exactly like the UI does.
    let scratch_rom = smwe_rom::SmwRom::from_rom(smwe_rom::snes_utils::rom::Rom::new(scratch.clone())?)?;
    let sel = PageSel { fg: true, page: 0x02 };
    let raw02 = map16_file::export_page_sel(&scratch_rom, sel, 0)?;
    assert_eq!(raw02, modified, "expanded page 0x02 must round-trip");
    let blocks02 = map16_file::parse_page(&raw02)?;
    assert_eq!(blocks02.len(), MAP16_PAGE_TILES);
    let stored_acts = map16_expanded::read_acts_table(&scratch, 0)?;
    assert_eq!(map16_expanded::act_as_of(&stored_acts, 0x205), 0x25);

    // ── Real data path 2: modern .map16 partial export ──────────────────
    let tiles02: Vec<[u8; 8]> = raw02.chunks_exact(8).map(|c| c.try_into().unwrap()).collect();
    let acts02: Vec<u16> =
        (0..256).map(|i| stored_acts.get(&(0x200 + i as u16)).copied().unwrap_or(0x200 + i as u16)).collect();
    let partial = map16_file::serialize_modern_partial(true, 0x02, 0x10, &tiles02, &acts02)
        .map_err(|e| anyhow::anyhow!("{e}"))?;
    let parsed_partial = map16_file::parse_modern_map16(&partial).map_err(|e| anyhow::anyhow!("{e}"))?;
    let partial_loc = map16_file::modern_partial_page(&parsed_partial).expect("partial page location");
    assert!(partial_loc.fg && partial_loc.page == 0x02);
    let status_export = format!("Exported {} ({} bytes) → map16-fg02.map16", sel.label(), partial.len());

    // ── Real data path 3: full "export ALL" ─────────────────────────────
    let fg_pages = vec![(0x02u8, modified.clone())];
    let bg0 = map16_file::export_page_sel(&scratch_rom, PageSel { fg: false, page: 0x00 }, 0)?;
    let bg_pages = vec![(0x00u8, bg0)];
    let mut ts_group_pages = [[0u8; 0x1000]; TILESETS_COUNT];
    for (ts, slot) in ts_group_pages.iter_mut().enumerate() {
        let p0 = map16_file::export_page(&scratch_rom, map16_file::PAGE_FG0, ts)?;
        let p1 = map16_file::export_page(&scratch_rom, map16_file::PAGE_FG1, ts)?;
        slot[..0x800].copy_from_slice(&p0);
        slot[0x800..].copy_from_slice(&p1);
    }
    let full = map16_file::serialize_modern_full(&map16_file::FullExportInput {
        fg_pages:       &fg_pages,
        bg_pages:       &bg_pages,
        acts:           &stored_acts,
        ts_group_pages: &ts_group_pages,
    });
    let parsed_full = map16_file::parse_modern_map16(&full).map_err(|e| anyhow::anyhow!("{e}"))?;
    let s0 = parsed_full.sections[0].size;
    let s1 = parsed_full.sections[1].size;
    let s5 = parsed_full.sections[5].size;
    let status_all = format!(
        "Exported ALL Map16 ({} FG pages, {} BG pages, {} act-as entries) → map16-all.map16 ({} bytes)",
        fg_pages.len(),
        bg_pages.len(),
        stored_acts.len(),
        full.len()
    );

    // ── Real data path 4: remap preview ─────────────────────────────────
    let mut remap_table = HashMap::new();
    map16_expanded::set_act_range_from_base(&mut remap_table, 0x200, 0x211, 0x25);
    let n = map16_expanded::remap_act_refs(&mut remap_table, 0x25, 0x26, 0x100);
    assert_eq!(n, 2);
    let remap_preview = format!(
        "2 tile(s) would change:\n  0200: 0025 → {:04X}\n  0201: 0026 → {:04X}",
        map16_expanded::act_as_of(&remap_table, 0x200),
        map16_expanded::act_as_of(&remap_table, 0x201),
    );

    // ── Compose the screenshot ──────────────────────────────────────────
    let (w, h) = (1280u32, 1470u32);
    let mut ctx = Ctx {
        img:   RgbImage::new(w, h),
        sans:  &sans,
        bold:  &sans_bold,
        ink:   Rgb([0x1A, 0x1A, 0x1A]),
        gray:  Rgb([0x66, 0x66, 0x66]),
        green: Rgb([0x1E, 0x7A, 0x1E]),
        dark:  Rgb([0x2B, 0x2B, 0x2B]),
    };
    for p in ctx.img.pixels_mut() {
        *p = Rgb([0xF2, 0xF2, 0xF2]);
    }
    fill_rect(&mut ctx.img, 0, 0, w, 52, ctx.dark);
    draw_text(
        &mut ctx.img,
        ctx.bold,
        "Map16 Block Editor — expanded pages (0x80 FG + 0x80 BG), acts-like remap, modern .map16 (headless mock; data is real ROM output)",
        24,
        15,
        17.0,
        Rgb([0xFF, 0xFF, 0xFF]),
    );

    // Section 1: page selector + acts-like + import/export (left), expanded atlas (right).
    let mut y = 70u32;
    ctx.heading("1 · Expanded page selector, acts-like, import/export", 24, y as i32);
    y += 30;
    let lx = 24u32;
    ctx.heading("Page import/export", lx as i32, y as i32);
    y += 28;
    ctx.text(
        "Modern .map16 files are Lunar Magic 1.90+ compatible; raw 0x800-byte pages still import.",
        lx as i32,
        y as i32,
        12.5,
        ctx.gray,
    );
    y += 30;
    ctx.text("Page:  (•) FG   ( ) BG    [02 ▾]  (expanded)", lx as i32, y as i32, 14.0, ctx.ink);
    y += 30;
    ctx.text("Acts like:  [0025 ▾]  [Reset]  [Remap…]", lx as i32, y as i32, 14.0, ctx.ink);
    y += 26;
    ctx.text(
        "Gameplay values are < 0x200 (LM enforces this in-game). Blank = acts as itself.",
        lx as i32,
        y as i32,
        12.0,
        ctx.gray,
    );
    y += 40;
    for (i, label) in ["Export page…", "Export ALL…", "Import…"].iter().enumerate() {
        ctx.button(label, lx + i as u32 * 160, y, 150);
    }
    y += 52;
    ctx.heading("Status", lx as i32, y as i32);
    y += 28;
    fill_rect(&mut ctx.img, lx, y, 590, 66, Rgb([0xFF, 0xFF, 0xFF]));
    rect_border(&mut ctx.img, lx, y, 590, 66, Rgb([0x99, 0x99, 0x99]));
    ctx.text(&status_export, (lx + 8) as i32, (y + 8) as i32, 11.0, ctx.green);
    ctx.text(&status_all, (lx + 8) as i32, (y + 30) as i32, 11.0, ctx.green);
    y += 96;
    let hexline: String = partial[..16].iter().map(|b| format!("{b:02x} ")).collect();
    ctx.heading("map16-fg02.map16 — modern container:", lx as i32, y as i32);
    y += 26;
    ctx.text(
        &format!("first 16 bytes: {hexline}  (\"LM16\" magic + version 0x100)"),
        lx as i32,
        y as i32,
        12.5,
        ctx.gray,
    );
    y += 24;
    ctx.text(
        &format!(
            "offset table: 8 entries · section 0 (tiles): {:#X} · section 1 (act-as): {:#X} · base=(0,2) FG-relative",
            parsed_partial.sections[0].size, parsed_partial.sections[1].size,
        ),
        lx as i32,
        y as i32,
        12.5,
        ctx.gray,
    );

    // Right: the real expanded-page atlas.
    let ax = 648u32;
    let mut ay = 100u32;
    ctx.heading("Exported: FG page 02 (expanded) — real round trip", ax as i32, ay as i32);
    ay += 26;
    ctx.text("write_expanded_fg_page → export_page_sel → emulator VRAM/CGRAM", ax as i32, ay as i32, 12.0, ctx.gray);
    ay += 24;
    const SCALE: u32 = 2;
    let atlas_px = 16 * 16 * SCALE;
    let mut atlas: Vec<u8> = vec![0u8; (atlas_px * atlas_px * 4) as usize];
    for (i, block) in blocks02.iter().enumerate() {
        let words = [block.upper_left.0, block.lower_left.0, block.upper_right.0, block.lower_right.0];
        render_block(
            &vram,
            &cgram,
            &words,
            (i % 16) as u32 * 16 * SCALE,
            (i / 16) as u32 * 16 * SCALE,
            SCALE,
            &mut atlas,
            atlas_px,
        );
    }
    for yy in 0..atlas_px {
        for xx in 0..atlas_px {
            let off = ((yy * atlas_px + xx) * 4) as usize;
            ctx.img.put_pixel(ax + xx, ay + yy, Rgb([atlas[off], atlas[off + 1], atlas[off + 2]]));
        }
    }
    rect_border(&mut ctx.img, ax, ay, atlas_px, atlas_px, Rgb([0x99, 0x99, 0x99]));

    // Section 2: remap dialog mock.
    y = 700u32;
    ctx.heading("2 · Remap act-as dialog (LM v1.91 / v3.01 parity)", 24, y as i32);
    y += 32;
    let dx = 24u32;
    let dw = 640u32;
    let dh = 300u32;
    fill_rect(&mut ctx.img, dx, y, dw, dh, Rgb([0xFF, 0xFF, 0xFF]));
    rect_border(&mut ctx.img, dx, y, dw, dh, Rgb([0x55, 0x55, 0x55]));
    fill_rect(&mut ctx.img, dx, y, dw, 30, Rgb([0x2B, 0x2B, 0x2B]));
    draw_text(
        &mut ctx.img,
        ctx.bold,
        "Remap act-as values",
        (dx + 12) as i32,
        (y + 7) as i32,
        14.0,
        Rgb([0xFF, 0xFF, 0xFF]),
    );
    let mut ry = y + 48;
    ctx.text(
        "Which tiles' act-as values point where. Values are hex; ranges like 100-1F3.",
        (dx + 12) as i32,
        ry as i32,
        12.5,
        ctx.ink,
    );
    ry += 30;
    ctx.text(
        "(•) Remap references (G)      ( ) Assign range from base (R)",
        (dx + 12) as i32,
        ry as i32,
        13.0,
        ctx.ink,
    );
    ry += 30;
    ctx.text("Source range:  [100-101]", (dx + 12) as i32, ry as i32, 13.0, ctx.ink);
    ry += 28;
    ctx.text("Reference:     [M125]", (dx + 12) as i32, ry as i32, 13.0, ctx.ink);
    ry += 26;
    ctx.text(
        "G100-101,+25 shifts by +0x25 · G100-101,M125 (or 125) remaps onto 0x125.",
        (dx + 12) as i32,
        ry as i32,
        11.5,
        ctx.gray,
    );
    ry += 34;
    ctx.button("Preview", dx + 12, ry, 110);
    ctx.button("Apply", dx + 132, ry, 110);
    ry += 44;
    for (i, line) in remap_preview.lines().enumerate() {
        ctx.text(line, (dx + 12) as i32, (ry + i as u32 * 20) as i32, 12.5, ctx.green);
    }

    // Section 3: full-export layout proof.
    let mut zy = y + dh + 36;
    ctx.heading("3 · Full \"export ALL\" container layout (parsed back from the real file)", 24, zy as i32);
    zy += 32;
    for line in [
        format!("section 0 (tiles, FG 0x00-0x7F + BG 0x80-0xFF): {:#X} bytes (0x10000 tiles × 8)", s0),
        format!("section 1 (FG act-as): {:#X} bytes (0x8000 × 2)", s1),
        "sections 2-4, 6, 7: 0 (aliases / optional pipe data, unused by parsers)".to_string(),
        format!("section 5 (tileset-group FG pages 0-1, 15 groups): {:#X} bytes", s5),
        "header: \"LM16\" · version 0x100 · game 1 (SMW) · flags 0x02 (full-game export)".to_string(),
        "FG pages 0x00/0x01 come from the tileset-group section (like LM's own exports); BG has no act-as.".to_string(),
    ] {
        ctx.text(&line, 24, zy as i32, 13.0, ctx.ink);
        zy += 26;
    }
    zy += 10;
    ctx.text(
        "Caveat: editor-owned expanded-page/act-as storage uses RATS blocks with smw-editor tags (documented in",
        24,
        zy as i32,
        12.0,
        ctx.gray,
    );
    zy += 22;
    ctx.text(
        "map16_expanded); in-game use of non-identity act-as values needs runtime support (LM's expanded-Map16 ASM).",
        24,
        zy as i32,
        12.0,
        ctx.gray,
    );

    // Crop to used height.
    let used = zy + 30;
    let mut final_img = RgbImage::new(w, used.min(h));
    for yy in 0..final_img.height() {
        for xx in 0..w {
            final_img.put_pixel(xx, yy, *ctx.img.get_pixel(xx, yy));
        }
    }
    final_img.save(output)?;
    println!("wrote {output}");
    Ok(())
}
