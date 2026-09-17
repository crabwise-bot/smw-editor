//! Headless mock screenshots of the expanded Secondary Entrances dialog and the
//! overworld Star/Pipe teleport table editor (LM v2.50/v3.00 parity).
//!
//! egui can't render headless, so this composes honest mocks of the editor
//! windows: the grid rows are REAL — parsed from the ROM by the same code the
//! UI uses (`SecondaryEntrance::read_from_rom` + the real byte accessors) —
//! and the extended options shown are a sample `SecondaryExitExtData` that is
//! first round-tripped through the real RATS codec. Only the window chrome
//! (title bar, sliders, checkboxes) is drawn rather than real egui widgets.
//!
//! ```sh
//! cargo run --bin render_secondary_entrances -- --rom=smw.smc
//! ```
//! Writes `docs/screenshots/secondary-entrances-expanded.png` and
//! `docs/screenshots/ow-teleport-table.png`.

use ab_glyph::{Font, FontRef, Glyph, Point, PxScale, ScaleFont};
use image::{Rgb, RgbImage};
use smw_editor::render_util::{fill_rect, rect_border};
use smwe_rom::{
    level::secondary_entrance::{
        OverworldExit,
        OwExitKind,
        OwPlayerSwitch,
        OwTeleportEntry,
        SecondaryEntrance,
        SecondaryExitExtData,
        SecondaryExitOptions,
        SECONDARY_ENTRANCE_COUNT_VANILLA,
    },
    overworld::SUBMAP_NAMES,
    snes_utils::rom::Rom,
};

const MONO_CANDIDATES: &[&str] = &["/usr/share/fonts/truetype/dejavu/DejaVuSansMono.ttf"];
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

struct Fonts {
    mono:      FontRef<'static>,
    sans:      FontRef<'static>,
    sans_bold: FontRef<'static>,
}

/// Draw one line of text; returns the advance width in px.
fn draw_text(img: &mut RgbImage, font: &FontRef, text: &str, x: i32, y: i32, px: f32, color: Rgb<u8>) -> i32 {
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
    (caret_x - x as f32) as i32
}

fn title_bar(img: &mut RgbImage, fonts: &Fonts, title: &str, w: u32) {
    fill_rect(img, 0, 0, w, 52, Rgb([0x2B, 0x2B, 0x2B]));
    draw_text(img, &fonts.sans_bold, title, 24, 15, 19.0, Rgb([0xFF, 0xFF, 0xFF]));
}

/// Build the sample extended data, round-tripped through the real RATS codec
/// to prove encode/decode before drawing.
fn sample_ext_data() -> SecondaryExitExtData {
    let mut data = SecondaryExitExtData::default();
    data.options.insert(0x001, SecondaryExitOptions { water_level: true, ..Default::default() });
    data.options.insert(0x002, SecondaryExitOptions {
        exit_to_overworld: Some(OverworldExit {
            exit_kind:  OwExitKind::Secret,
            player:     OwPlayerSwitch::Luigi,
            base_event: 0x2A,
            teleport:   0x07,
        }),
        ..Default::default()
    });
    data.options.insert(0x003, SecondaryExitOptions { midway_redirect: Some(0x105), ..Default::default() });
    data.extended_entries.insert(0x200, [0x10, 0x21, 0x43, 0x08]);
    data.teleport_table[0x07] = OwTeleportEntry { submap: 3, x: 12, y: 20, reserved: 0 };
    data.teleport_table[0x00] = OwTeleportEntry { submap: 0, x: 8, y: 24, reserved: 0 };

    // Prove the codec round-trips before anything is drawn.
    let mut rom = vec![0xFFu8; 0x40000];
    data.write_to_rom(&mut rom, 0).expect("encode sample");
    let back = SecondaryExitExtData::parse(&rom).expect("decode sample");
    assert_eq!(back, data, "sample extended data must survive the RATS codec");
    back
}

fn badges(data: &SecondaryExitExtData, idx: u16) -> String {
    let o = data.options_for(idx);
    let mut s = String::new();
    if o.water_level {
        s.push_str("W ");
    }
    if o.exit_to_overworld.is_some() {
        s.push_str("OW ");
    }
    if o.midway_redirect.is_some() {
        s.push_str("→M ");
    }
    s.pop();
    s
}

fn render_dialog(fonts: &Fonts, rom: &Rom, data: &SecondaryExitExtData, out: &str) -> anyhow::Result<()> {
    let (w, h) = (1480u32, 800u32);
    let mut img = RgbImage::new(w, h);
    let bg = Rgb([0xF2, 0xF2, 0xF2]);
    let ink = Rgb([0x1A, 0x1A, 0x1A]);
    let gray = Rgb([0x66, 0x66, 0x66]);
    let accent = Rgb([0x1A, 0x5A, 0x9A]);
    for p in img.pixels_mut() {
        *p = bg;
    }
    title_bar(
        &mut img,
        fonts,
        "Secondary Entrances — headless mock (grid rows are real ROM data; window chrome is drawn)",
        w,
    );

    // Real entrances 0x000..0x007 + the stored extended entry 0x200.
    let mut y = 76u32;
    draw_text(
        &mut img,
        &fonts.sans_bold,
        "Filter: [        ] [Clear]    Go to: [0x___] [Go]   (LM v2.50: type full index values)",
        24,
        y as i32,
        15.0,
        ink,
    );
    y += 34;
    draw_text(&mut img, &fonts.sans, "Editing entrances will be saved with Ctrl+S.", 24, y as i32, 14.0, gray);
    y += 30;

    let headers = ["ID", "Dest Level", "Screen", "X", "Y", "FG Pos", "BG Pos", "Flags", "Jump"];
    let col_x = [24u32, 110, 240, 340, 400, 450, 540, 630, 760];
    for (hi, hx) in headers.iter().zip(col_x.iter()) {
        draw_text(&mut img, &fonts.sans_bold, hi, *hx as i32, y as i32, 15.0, ink);
    }
    y += 28;

    let mut indices: Vec<u16> = (0..8u16).collect();
    indices.push(0x200);
    for (ri, idx) in indices.iter().enumerate() {
        let b = if (*idx as usize) < SECONDARY_ENTRANCE_COUNT_VANILLA {
            SecondaryEntrance::read_from_rom(rom, *idx as usize).expect("read entrance").bytes()
        } else {
            data.extended_entry(*idx).expect("sample extended entry")
        };
        let dest = {
            let hi = (b[3] as u16 & 0b1000) << 5;
            hi | b[0] as u16
        };
        if ri % 2 == 1 {
            fill_rect(&mut img, 12, y - 4, w - 24, 26, Rgb([0xE8, 0xE8, 0xE8]));
        }
        if *idx == 0x002 {
            rect_border(&mut img, 12, y - 4, w - 24, 26, accent);
        }
        let vals = [
            format!("{:03X}", idx),
            format!("0x{dest:03X}"),
            format!("{}", b[2] & 0x1F),
            format!("{}", b[2] >> 5),
            format!("{}", b[1] & 0x0F),
            format!("{}", (b[1] >> 4) & 0x3),
            format!("{}", b[1] >> 6),
            badges(data, *idx),
            format!("→ {dest:03X}"),
        ];
        for (v, hx) in vals.iter().zip(col_x.iter()) {
            let color = if hx == &col_x[7] && !v.is_empty() { accent } else { ink };
            draw_text(&mut img, &fonts.mono, v, *hx as i32, y as i32, 14.0, color);
        }
        y += 26;
    }

    // ── Extended options panel for entrance 0x002 ──
    y += 18;
    draw_text(&mut img, &fonts.sans_bold, "Entrance 0x002 — extended options (LM v3.00)", 24, y as i32, 17.0, ink);
    y += 32;
    // Water checkbox (unchecked).
    rect_border(&mut img, 24, y, 18, 18, Rgb([0x99, 0x99, 0x99]));
    draw_text(&mut img, &fonts.sans, "Water level — the destination plays as a water level", 52, y as i32, 15.0, ink);
    y += 30;
    // Exit-to-OW checkbox (checked).
    fill_rect(&mut img, 24, y, 18, 18, Rgb([0x1A, 0x5A, 0x9A]));
    draw_text(&mut img, &fonts.sans_bold, "✓", 28, (y - 2) as i32, 15.0, Rgb([0xFF, 0xFF, 0xFF]));
    draw_text(&mut img, &fonts.sans, "Exit to overworld instead of entering a level", 52, y as i32, 15.0, ink);
    y += 30;
    draw_text(&mut img, &fonts.sans, "Exit:  ( ) Normal   (●) Secret      Player: [Luigi ▾]", 52, y as i32, 15.0, ink);
    y += 28;
    draw_text(
        &mut img,
        &fonts.sans,
        "Base event: [0x2A]      Teleport: [0x07]  →  0x07 — Forest of Illusion (12, 20)",
        52,
        y as i32,
        15.0,
        ink,
    );
    y += 26;
    draw_text(
        &mut img,
        &fonts.sans,
        "Edit teleport locations from the overworld editor toolbar.",
        52,
        y as i32,
        13.0,
        gray,
    );
    y += 30;
    // Midway redirect (unchecked).
    rect_border(&mut img, 24, y, 18, 18, Rgb([0x99, 0x99, 0x99]));
    draw_text(
        &mut img,
        &fonts.sans,
        "Midway entrance redirects to another level's midway entrance   [Redirect to level: 0x105]",
        52,
        y as i32,
        15.0,
        gray,
    );

    // Honesty note.
    let cy = h - 64;
    draw_text(
        &mut img,
        &fonts.sans,
        "Mock window chrome — grid values come from the real ROM; the W/OW/→M badges and the 0x200 row come from",
        24,
        cy as i32,
        13.0,
        gray,
    );
    draw_text(
        &mut img,
        &fonts.sans,
        "a sample extended-data block that was round-tripped through the real RATS codec. In-game playback needs LM's ASM hacks.",
        24,
        (cy + 22) as i32,
        13.0,
        gray,
    );

    img.save(out)?;
    println!("wrote {out} ({w}x{h})");
    Ok(())
}

fn render_teleport_table(fonts: &Fonts, data: &SecondaryExitExtData, out: &str) -> anyhow::Result<()> {
    let (w, h) = (980u32, 760u32);
    let mut img = RgbImage::new(w, h);
    let bg = Rgb([0xF2, 0xF2, 0xF2]);
    let ink = Rgb([0x1A, 0x1A, 0x1A]);
    let gray = Rgb([0x66, 0x66, 0x66]);
    let accent = Rgb([0x1A, 0x5A, 0x9A]);
    for p in img.pixels_mut() {
        *p = bg;
    }
    title_bar(&mut img, fonts, "Secondary Exit Teleport Locations — headless mock (Star/Pipe table, 0x100 entries)", w);

    let mut y = 76u32;
    draw_text(&mut img, &fonts.sans, "Filter: [        ] [Clear]", 24, y as i32, 15.0, ink);
    y += 32;
    draw_text(
        &mut img,
        &fonts.sans,
        "Where the player appears on the overworld when a secondary exit sends them there. Saved with Ctrl+S.",
        24,
        y as i32,
        14.0,
        gray,
    );
    y += 34;

    let headers = ["ID", "Submap", "X", "Y"];
    let col_x = [24u32, 120, 480, 580];
    for (hi, hx) in headers.iter().zip(col_x.iter()) {
        draw_text(&mut img, &fonts.sans_bold, hi, *hx as i32, y as i32, 15.0, ink);
    }
    y += 28;

    for idx in 0..16u16 {
        let e = data.teleport_table[idx as usize];
        if idx % 2 == 1 {
            fill_rect(&mut img, 12, y - 4, w - 24, 26, Rgb([0xE8, 0xE8, 0xE8]));
        }
        if idx == 0x07 {
            rect_border(&mut img, 12, y - 4, w - 24, 26, accent);
        }
        let sub = SUBMAP_NAMES.get(e.submap as usize).copied().unwrap_or("???");
        let vals = [format!("{idx:02X}"), format!("{} — {sub}", e.submap), format!("{}", e.x), format!("{}", e.y)];
        for (v, hx) in vals.iter().zip(col_x.iter()) {
            draw_text(&mut img, &fonts.mono, v, *hx as i32, y as i32, 14.0, ink);
        }
        y += 26;
    }
    draw_text(&mut img, &fonts.sans, "…", 24, y as i32, 15.0, gray);
    y += 30;
    draw_text(
        &mut img,
        &fonts.sans,
        "Rows 0x00 and 0x07 are the sample entries (round-tripped through the real RATS codec);",
        24,
        y as i32,
        13.0,
        gray,
    );
    draw_text(
        &mut img,
        &fonts.sans,
        "the rest are defaults, exactly as a fresh ROM parses. Window chrome is drawn, not real egui.",
        24,
        (y + 22) as i32,
        13.0,
        gray,
    );

    img.save(out)?;
    println!("wrote {out} ({w}x{h})");
    Ok(())
}

fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let rom_path = args
        .iter()
        .find_map(|a| a.strip_prefix("--rom="))
        .or_else(|| args.iter().skip(1).find(|a| !a.starts_with("--")).map(|a| a.as_str()))
        .unwrap_or("smw.smc");
    let out_dir = args.iter().find_map(|a| a.strip_prefix("--out-dir=")).unwrap_or("docs/screenshots");

    let fonts = Fonts {
        mono:      load_font(MONO_CANDIDATES)?,
        sans:      load_font(SANS_CANDIDATES)?,
        sans_bold: load_font(SANS_BOLD_CANDIDATES)?,
    };

    let raw = std::fs::read(rom_path)?;
    let rom = Rom::new(raw)?;
    let data = sample_ext_data();

    render_dialog(&fonts, &rom, &data, &format!("{out_dir}/secondary-entrances-expanded.png"))?;
    render_teleport_table(&fonts, &data, &format!("{out_dir}/ow-teleport-table.png"))?;
    Ok(())
}
