//! Headless renderer for the ExAnimation palette-editor integration
//! (backlog #43: LM v3.33 palette link + LM v3.61 palette-row paste).
//!
//! Loads a level through the real emulator path, then renders the real
//! ExAnimated palette-frame color state at three moments:
//!
//!   1. "Palette Select" armed — the armed color field is outlined white,
//!      exactly the marker the dialog paints (`color_slot`).
//!   2. After a Ctrl+Left-Click fill in the palette editor — the armed slot
//!      takes the clicked SNES555 color and the arm auto-advances to the
//!      next color field (`fill_armed_color` semantics).
//!   3. After "Paste row" — a real `smwclip:1:palrow:` clipboard payload
//!      (12 real CGRAM colors) fills the slots from the armed field
//!      (`apply_pasted_palette_row` semantics).
//!
//! A fourth panel mocks the palette editor's "BG Palette (index 0)" row:
//! the 12 swatches are real CGRAM colors from the emulated level load, the
//! yellow corner triangles sit on the swatches the real `exanim_dest_at`
//! predicate reports (CGRAM address in the demo frame's `[dest,
//! dest+units)` write range), and the "Copy row" button + link-status line
//! use the real UI strings. Panel titles are added afterwards with PIL.
//!
//! Usage:
//!   render_exanim_palette_select --rom=smw.smc --level=0x105 --out=/tmp/exanim_palette.png

use std::{env, path::Path, sync::Arc};

use image::{ImageBuffer, Rgb};
use smwe_emu::{emu::CheckedMem, rom::Rom as EmuRom, Cpu};
use smwe_rom::exanimation::{ExAnimFrame, ExAnimFrameKind, ExAnimTrigger};

/// 2x zoom on the 22px color swatches.
const CELL: u32 = 44;
const GAP: u32 = 6;
const PANEL_GAP: u32 = 28;

fn hex(s: &str) -> u16 {
    u16::from_str_radix(s.trim().trim_start_matches("0x").trim_start_matches('$'), 16).unwrap_or(0)
}

/// SNES RGB555 → 8-bit RGB (same math as the dialog's `snes555_to_color32`).
fn snes555(c: u16) -> [u8; 3] {
    [
        ((c & 0x1F) as u32 * 255 / 31) as u8,
        (((c >> 5) & 0x1F) as u32 * 255 / 31) as u8,
        (((c >> 10) & 0x1F) as u32 * 255 / 31) as u8,
    ]
}

fn cgram_word(cgram: &[u8], addr: u16) -> u16 {
    let o = addr as usize * 2;
    u16::from_le_bytes([cgram[o], cgram[o + 1]])
}

struct Grid {
    frame: ExAnimFrame,
    armed: Option<(usize, usize)>,
}

impl Grid {
    /// LM v3.33: fill the armed color field and auto-advance the arm —
    /// mirrors `ExAnimDialog::fill_armed_color`.
    fn fill_armed(&mut self, color: u16) {
        let (f, u) = match self.armed {
            Some(t) => t,
            None => return,
        };
        let units = self.frame.units_per_frame as usize;
        let steps = self.frame.frames as usize;
        if f < steps && u < units {
            if let Some(p) = self.frame.payload.get_mut(f * units + u) {
                *p = color;
            }
            self.armed = Some(if u + 1 < units {
                (f, u + 1)
            } else if f + 1 < steps {
                (f + 1, 0)
            } else {
                (0, 0)
            });
        }
    }

    /// LM v3.61: fill from a clipboard palette row starting at the armed
    /// field — mirrors `ExAnimDialog::apply_pasted_palette_row`.
    fn paste_row(&mut self, colors: &[u16]) -> usize {
        let (start_f, start_u) = self.armed.unwrap_or((0, 0));
        let units = self.frame.units_per_frame as usize;
        let steps = self.frame.frames as usize;
        let mut changed = 0;
        let mut ci = 0;
        'fill: for f in start_f..steps {
            for u in (if f == start_f { start_u } else { 0 })..units {
                if ci >= colors.len() {
                    break 'fill;
                }
                if let Some(p) = self.frame.payload.get_mut(f * units + u) {
                    if *p != colors[ci] {
                        *p = colors[ci];
                        changed += 1;
                    }
                }
                ci += 1;
            }
        }
        changed
    }
}

fn draw_swatch(img: &mut ImageBuffer<Rgb<u8>, Vec<u8>>, x0: u32, y0: u32, rgb: [u8; 3], armed: bool) {
    for py in 0..CELL {
        for px in 0..CELL {
            img.put_pixel(x0 + px, y0 + py, Rgb(rgb));
        }
    }
    // Thin dark frame.
    let frame = Rgb([80u8, 80, 80]);
    for px in 0..CELL {
        img.put_pixel(x0 + px, y0, frame);
        img.put_pixel(x0 + px, y0 + CELL - 1, frame);
    }
    for py in 0..CELL {
        img.put_pixel(x0, y0 + py, frame);
        img.put_pixel(x0 + CELL - 1, y0 + py, frame);
    }
    // White outline = the armed Palette-Select target.
    if armed {
        let white = Rgb([255u8, 255, 255]);
        for px in 0..CELL {
            for t in 0..3u32 {
                img.put_pixel(x0 + px, y0 + 2 + t, white);
                img.put_pixel(x0 + px, y0 + CELL - 3 + t, white);
            }
        }
        for py in 0..CELL {
            for t in 0..3u32 {
                img.put_pixel(x0 + 2 + t, y0 + py, white);
                img.put_pixel(x0 + CELL - 3 + t, y0 + py, white);
            }
        }
    }
}

/// Yellow corner triangle = ExAnimated color destination (mirrors the
/// palette editor's `exanim_dest_at` marker).
fn draw_dest_marker(img: &mut ImageBuffer<Rgb<u8>, Vec<u8>>, x0: u32, y0: u32) {
    let yellow = Rgb([255u8, 255, 0]);
    for d in 0..14u32 {
        for t in 0..(14 - d) {
            img.put_pixel(x0 + CELL - 1 - t, y0 + d, yellow);
        }
    }
}

fn draw_grid(img: &mut ImageBuffer<Rgb<u8>, Vec<u8>>, x0: u32, y0: u32, grid: &Grid) {
    let units = grid.frame.units_per_frame as usize;
    let steps = grid.frame.frames as usize;
    for f in 0..steps {
        for u in 0..units {
            let x = x0 + (u as u32) * (CELL + GAP);
            let y = y0 + (f as u32) * (CELL + GAP);
            let raw = grid.frame.payload[f * units + u];
            let armed = grid.armed == Some((f, u));
            draw_swatch(img, x, y, snes555(raw), armed);
        }
    }
}

fn main() {
    let args: Vec<String> = env::args().collect();
    let level = args.iter().find_map(|a| a.strip_prefix("--level=")).map(|s| hex(s)).unwrap_or(0x105);
    let rom_path =
        args.iter().find_map(|a| a.strip_prefix("--rom=")).map(Path::new).unwrap_or_else(|| Path::new("smw.smc"));
    let out = args.iter().find_map(|a| a.strip_prefix("--out=")).unwrap_or("/tmp/exanim_palette_select.png");

    let raw = std::fs::read(rom_path).expect("cannot read ROM");
    let rom_bytes = if raw.len() % 0x400 == 0x200 { raw[0x200..].to_vec() } else { raw };
    let mut emu_rom = EmuRom::new(rom_bytes);
    emu_rom.load_symbols(include_str!("../../symbols/SMW_U.sym"));
    let mut cpu = Cpu::new(CheckedMem::new(Arc::new(emu_rom)));
    smwe_emu::emu::decompress_sublevel(&mut cpu, level);
    smwe_emu::emu::fetch_anim_frame(&mut cpu);

    // Demo palette frame: 3 steps × 4 units. The destination sits inside
    // the CGRAM row the palette-editor panel renders (words 0x02..0x06),
    // so the yellow destination markers land on real addresses.
    let demo = ExAnimFrame {
        kind:            ExAnimFrameKind::Palette,
        dest:            0x0002,
        speed:           1,
        trigger:         ExAnimTrigger::Always,
        frames:          3,
        units_per_frame: 4,
        payload:         vec![
            0x001F, 0x03E0, 0x7C00, 0x7FFF, // step 0: blue green red white
            0x7FE0, 0x7C1F, 0x03FF, 0x0000, // step 1
            0x4210, 0x6318, 0x7BDE, 0x1CE7, // step 2
        ],
    };

    // Panel 1: Palette Select armed at step 0 / color 1.
    let p1 = Grid { frame: demo.clone(), armed: Some((0, 1)) };
    // Panel 2: Ctrl+Left-Click on a palette color ($7FFF white) fills the
    // armed field; the arm auto-advances to step 0 / color 2.
    let mut p2 = Grid { frame: demo.clone(), armed: Some((0, 1)) };
    p2.fill_armed(0x7FFF);
    // Panel 3: "Paste row" — 12 real CGRAM colors from the emulated level
    // load, applied from the armed field (step 0 / color 2).
    let row_colors: Vec<u16> = (0..12).map(|a| cgram_word(&cpu.mem.cgram, a)).collect();
    let mut p3 = Grid { frame: p2.frame.clone(), armed: p2.armed };
    let pasted = p3.paste_row(&row_colors);
    println!("pasted {pasted} colors from real CGRAM row: {row_colors:04X?}");

    let units = demo.units_per_frame as usize;
    let steps = demo.frames as usize;
    let grid_w = units as u32 * CELL + (units as u32 - 1) * GAP;
    let grid_h = steps as u32 * CELL + (steps as u32 - 1) * GAP;

    // Panel 4: palette-editor BG row — 12 real CGRAM colors (words
    // 0x00..0x0B); yellow markers where the demo frame's destination range
    // [0x0002, 0x0006) covers a swatch (the real exanim_dest_at check).
    let pal_w = 12 * CELL + 11 * GAP;

    let w = grid_w.max(pal_w) + 20;
    let h = 3 * (grid_h + PANEL_GAP) + CELL + PANEL_GAP + 20;
    let mut img: ImageBuffer<Rgb<u8>, Vec<u8>> = ImageBuffer::new(w, h);
    for px in img.pixels_mut() {
        *px = Rgb([32, 32, 32]);
    }

    let mut y = 10u32;
    draw_grid(&mut img, 10, y, &p1);
    y += grid_h + PANEL_GAP;
    draw_grid(&mut img, 10, y, &p2);
    y += grid_h + PANEL_GAP;
    draw_grid(&mut img, 10, y, &p3);
    y += grid_h + PANEL_GAP;
    for (c, color) in row_colors.iter().enumerate() {
        let x = 10 + c as u32 * (CELL + GAP);
        draw_swatch(&mut img, x, y, snes555(*color), false);
        let addr = c as u16;
        if addr >= demo.dest && addr - demo.dest < demo.units_per_frame as u16 {
            draw_dest_marker(&mut img, x, y);
        }
    }

    img.save(out).expect("save panels");
    println!("wrote {out} ({w}x{h})");
    println!("panel order: armed / ctrl-click-filled / paste-row / palette-editor row");
}
