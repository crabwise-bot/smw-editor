//! Headless mock screenshots for the LM v3.00 dynamic level height PR.
//!
//! egui can't render headless, so these compose honest mocks: every VALUE
//! shown is real — parsed from the ROM by the same code the UI uses
//! (`SmwRom::from_file`, `LevelHeights`, `max_height_tiles`,
//! `ObjectLayer::parse`) — and every edit shown is verified by writing into
//! a scratch ROM copy and re-parsing. Only the window chrome (title bars,
//! sliders) is drawn, not real egui widgets. The canvas mock's tile pixels
//! for the top 27 rows are a real emulator render (`decompress_sublevel` +
//! `render_layer`, the same path as `render_level.rs`); rows below that are
//! the editor's authoring region (backdrop + grid + object overlays), which
//! is exactly what the editor canvas shows there.
//!
//! Two outputs:
//! - `--dialog-out=`: the Level Header window's new "Level Height" row,
//!   before (vanilla 27) and after (edited, verified via scratch ROM).
//! - `--canvas-out=`: a tall level canvas — real tiles for rows 0-26, the
//!   extended authoring region below, real object overlays, and demo objects
//!   placed at rows 28-30 (verified: the demo bytes are parsed back through
//!   `ObjectLayer::parse` and come back with their Y intact).
//!
//! ```sh
//! cargo run --bin render_level_height -- --rom=/path/to/smw.smc \
//!   --dialog-out=docs/screenshots/level-height-dialog.png \
//!   --canvas-out=docs/screenshots/level-height-canvas.png
//! ```

use std::sync::Arc;

use ab_glyph::{Font, FontRef, Glyph, Point, PxScale, ScaleFont};
use image::{Rgb, RgbImage};
use smw_editor::render_util::{read_color, render_layer};
use smwe_emu::{emu::CheckedMem, rom::Rom as EmuRom, Cpu};
use smwe_rom::{
    level::{
        dimensions::{max_height_tiles, LevelHeights, VANILLA_HORIZONTAL_HEIGHT_TILES},
        object_layer::{ObjectInstance, ObjectLayer},
    },
    SmwRom,
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
    #[allow(dead_code)]
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
        prev = Some(id);
        let glyph = Glyph { id, scale: PxScale::from(px), position: Point { x: caret_x, y: baseline } };
        if let Some(outlined) = scaled.outline_glyph(glyph) {
            let bb = outlined.px_bounds();
            outlined.draw(|gx, gy, v| {
                if v > 0.5 {
                    let (px_, py_) = (bb.min.x as i32 + gx as i32, bb.min.y as i32 + gy as i32);
                    if px_ >= 0 && py_ >= 0 && (px_ as u32) < img.width() && (py_ as u32) < img.height() {
                        img.put_pixel(px_ as u32, py_ as u32, color);
                    }
                }
            });
        }
        caret_x += scaled.h_advance(id);
    }
    (caret_x - x as f32) as i32
}

fn fill_rect(img: &mut RgbImage, x: u32, y: u32, w: u32, h: u32, color: Rgb<u8>) {
    let (iw, ih) = (img.width(), img.height());
    for yy in y..(y + h).min(ih) {
        for xx in x..(x + w).min(iw) {
            img.put_pixel(xx, yy, color);
        }
    }
}

fn hline(img: &mut RgbImage, x0: u32, x1: u32, y: u32, color: Rgb<u8>) {
    if y < img.height() {
        for x in x0..x1.min(img.width()) {
            img.put_pixel(x, y, color);
        }
    }
}

fn vline(img: &mut RgbImage, x: u32, y0: u32, y1: u32, color: Rgb<u8>) {
    if x < img.width() {
        for y in y0..y1.min(img.height()) {
            img.put_pixel(x, y, color);
        }
    }
}

/// Draw a mock slider track with a knob at `value` in `1..=max`.
fn draw_slider(img: &mut RgbImage, x: u32, y: u32, w: u32, value: u16, max: u16) {
    let track = Rgb([0xBB, 0xBB, 0xBB]);
    let knob = Rgb([0x33, 0x66, 0xCC]);
    fill_rect(img, x, y + 6, w, 4, track);
    let t = (value.saturating_sub(1) as f32) / (max.saturating_sub(1).max(1) as f32);
    let kx = x + (t * w as f32) as u32;
    fill_rect(img, kx.saturating_sub(6), y, 13, 16, knob);
}

/// Editor's object-overlay fill color for a standard object id
/// (central_panel.rs `obj_color`).
fn obj_color(id: u8) -> (u8, u8, u8) {
    let r = 40 + (id as u32 * 53 % 180) as u8;
    let g = 40 + (id as u32 * 97 % 180) as u8;
    let b = 40 + (id as u32 * 151 % 180) as u8;
    (r, g, b)
}

fn alpha_blend(dst: Rgb<u8>, src: (u8, u8, u8), alpha: u8) -> Rgb<u8> {
    let a = alpha as u32;
    Rgb([
        ((src.0 as u32 * a + dst[0] as u32 * (255 - a)) / 255) as u8,
        ((src.1 as u32 * a + dst[1] as u32 * (255 - a)) / 255) as u8,
        ((src.2 as u32 * a + dst[2] as u32 * (255 - a)) / 255) as u8,
    ])
}

fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let dialog_out =
        args.iter().find_map(|a| a.strip_prefix("--dialog-out=")).unwrap_or("docs/screenshots/level-height-dialog.png");
    let canvas_out =
        args.iter().find_map(|a| a.strip_prefix("--canvas-out=")).unwrap_or("docs/screenshots/level-height-canvas.png");
    let rom_path = args
        .iter()
        .find_map(|a| a.strip_prefix("--rom="))
        .or_else(|| args.iter().skip(1).find(|a| !a.starts_with("--")).map(|a| a.as_str()))
        .unwrap_or("smw.smc");

    let fonts = Fonts {
        mono:      load_font(MONO_CANDIDATES)?,
        sans:      load_font(SANS_CANDIDATES)?,
        sans_bold: load_font(SANS_BOLD_CANDIDATES)?,
    };

    // ---- Real data: level 0x105 (the repo's canonical example level) ----
    let rom = SmwRom::from_file(rom_path)?;
    let level = &rom.levels[0x105];
    anyhow::ensure!(!level.secondary_header.vertical_level(), "level 105 must be horizontal");
    let screens = level.primary_header.level_length() as u32 + 1;
    let max_h = max_height_tiles(screens);
    let before_h = rom.level_heights.get(0x105);
    anyhow::ensure!(before_h == VANILLA_HORIZONTAL_HEIGHT_TILES, "expected vanilla height on a fresh ROM");

    // AFTER: the edit the dialog screenshot shows — write a taller height
    // into a scratch ROM copy (exactly what save_to_rom does) and re-parse.
    let after_h = max_h.min(40).max(28);
    anyhow::ensure!(after_h > before_h && after_h <= max_h, "no room above vanilla for this level's screen count");
    let mut scratch = rom.rom.bytes().to_vec();
    let header_offset = if scratch.len() % 0x400 == 0x200 { 0x200 } else { 0 };
    {
        let mut heights = LevelHeights::default();
        heights.set(0x105, after_h, screens)?;
        heights.write_to_rom(&mut scratch, header_offset)?;
        let back = LevelHeights::parse(&scratch)?;
        anyhow::ensure!(back.get(0x105) == after_h, "height did not round-trip through the scratch ROM");
    }

    render_dialog(&fonts, dialog_out, screens, max_h, before_h, after_h)?;
    render_canvas(&fonts, canvas_out, rom_path, after_h)?;

    println!("wrote {dialog_out}");
    println!("wrote {canvas_out}");
    Ok(())
}

fn render_dialog(
    fonts: &Fonts, out: &str, screens: u32, max_h: u16, before_h: u16, after_h: u16,
) -> anyhow::Result<()> {
    let (w, h) = (1200u32, 660u32);
    let mut img = RgbImage::new(w, h);
    let bg = Rgb([0xF2, 0xF2, 0xF2]);
    let ink = Rgb([0x1A, 0x1A, 0x1A]);
    let gray = Rgb([0x5A, 0x5A, 0x5A]);
    let green = Rgb([0x1E, 0x7A, 0x1E]);
    for p in img.pixels_mut() {
        *p = bg;
    }

    // Title bar.
    fill_rect(&mut img, 0, 0, w, 52, Rgb([0x2B, 0x2B, 0x2B]));
    draw_text(
        &mut img,
        &fonts.sans_bold,
        "Level Editor \u{2014} Level Header window, dynamic level height (headless mock; values are real ROM output)",
        24,
        15,
        19.0,
        Rgb([0xFF, 0xFF, 0xFF]),
    );

    let panels = [
        ("BEFORE \u{2014} vanilla height (level 0x105 as parsed from the ROM)", before_h, false),
        ("AFTER \u{2014} height edited, written to a scratch ROM copy and re-parsed", after_h, true),
    ];
    for (i, (heading, hv, highlight)) in panels.iter().enumerate() {
        let px = 24 + i as u32 * 592;
        let pw = 560u32;
        fill_rect(&mut img, px, 70, pw, 470, Rgb([0xFF, 0xFF, 0xFF]));
        // Border.
        hline(&mut img, px, px + pw, 70, gray);
        hline(&mut img, px, px + pw, 539, gray);
        vline(&mut img, px, 70, 540, gray);
        vline(&mut img, px + pw - 1, 70, 540, gray);

        draw_text(&mut img, &fonts.sans_bold, heading, (px + 16) as i32, 88, 16.0, ink);
        hline(&mut img, px + 16, px + pw - 16, 118, Rgb([0xCC, 0xCC, 0xCC]));

        let mut y = 140;
        draw_text(&mut img, &fonts.sans, "Primary Header", (px + 16) as i32, y, 16.0, ink);
        y += 34;
        draw_text(&mut img, &fonts.sans, &format!("Level Length:  {screens} screens"), (px + 16) as i32, y, 15.0, ink);
        y += 36;

        // The new row.
        if *highlight {
            fill_rect(&mut img, px + 8, (y - 8) as u32, pw - 16, 44, Rgb([0xE8, 0xF5, 0xE9]));
        }
        draw_text(&mut img, &fonts.sans_bold, "Level Height:", (px + 16) as i32, y, 15.0, ink);
        draw_slider(&mut img, px + 170, y as u32, 220, *hv, max_h);
        draw_text(
            &mut img,
            &fonts.sans,
            &format!("{hv} tiles  (max {max_h})"),
            (px + 404) as i32,
            y,
            15.0,
            if *highlight { green } else { ink },
        );
        y += 40;

        for line in [
            "Tilemap RAM budget: screens \u{00D7} height \u{2264} 896",
            "(LM v3.00; 32 screens \u{2192} 27 tiles, 6 screens \u{2192} 149, 1 screen \u{2192} 896).",
            "Stored in a RATS block (\"SMWLVLH1\"), not the vanilla header.",
            "In-game playback needs LM's dynamic-dimensions engine;",
            "without it the ROM plays 27 tiles.",
        ] {
            draw_text(&mut img, &fonts.sans, line, (px + 16) as i32, y, 13.0, gray);
            y += 24;
        }
    }

    draw_text(
        &mut img,
        &fonts.sans,
        "Full bottom tile row: the canvas always renders every row fully \u{2014} LM 3.00's companion view option is the default here.",
        24,
        566,
        14.0,
        gray,
    );
    draw_text(
        &mut img,
        &fonts.sans,
        "Rows 32+ need LM's 32-row band objects (ext 01/03); the vanilla object stream places rows 0\u{2013}31.",
        24,
        592,
        14.0,
        gray,
    );

    img.save(out)?;
    Ok(())
}

/// Draw one object overlay rect the way the editor does (translucent fill +
/// brighter stroke), alpha-blended by hand.
fn draw_obj_rect(img: &mut RgbImage, x: u32, y: u32, w: u32, h: u32, color: (u8, u8, u8), selected: bool) {
    let (iw, ih) = (img.width(), img.height());
    for yy in y..(y + h).min(ih) {
        for xx in x..(x + w).min(iw) {
            let dst = *img.get_pixel(xx, yy);
            img.put_pixel(xx, yy, alpha_blend(dst, color, 70));
        }
    }
    let stroke = (color.0.min(255), color.1.min(255), color.2.min(255));
    for xx in x..(x + w).min(iw) {
        if y < ih {
            img.put_pixel(xx, y, Rgb([stroke.0, stroke.1, stroke.2]));
        }
        if y + h - 1 < ih {
            img.put_pixel(xx, y + h - 1, Rgb([stroke.0, stroke.1, stroke.2]));
        }
    }
    for yy in y..(y + h).min(ih) {
        if x < iw {
            img.put_pixel(x, yy, Rgb([stroke.0, stroke.1, stroke.2]));
        }
        if x + w - 1 < iw {
            img.put_pixel(x + w - 1, yy, Rgb([stroke.0, stroke.1, stroke.2]));
        }
    }
    if selected {
        let sel = Rgb([0xFF, 0xDC, 0x00]);
        for xx in x.saturating_sub(1)..(x + w + 1).min(iw) {
            for &yy in &[y.saturating_sub(1), y + h] {
                if yy < ih {
                    img.put_pixel(xx, yy, sel);
                }
            }
        }
        for yy in y.saturating_sub(1)..(y + h + 1).min(ih) {
            for &xx in &[x.saturating_sub(1), x + w] {
                if xx < iw {
                    img.put_pixel(xx, yy, sel);
                }
            }
        }
    }
}

fn render_canvas(fonts: &Fonts, out: &str, rom_path: &str, height: u16) -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    // Pick a level with few screens so a tall height shows well. Default: the
    // first horizontal level with 2-6 screens on this ROM (deterministic).
    let rom = SmwRom::from_file(rom_path)?;
    let level_num: u16 = args
        .iter()
        .find_map(|a| a.strip_prefix("--level="))
        .and_then(|s| u16::from_str_radix(s.trim_start_matches("0x"), 16).ok())
        .unwrap_or_else(|| {
            (0..0x200u16)
                .find(|&n| {
                    let l = &rom.levels[n as usize];
                    !l.secondary_header.vertical_level()
                        && (2..=6).contains(&(l.primary_header.level_length() as u32 + 1))
                })
                .expect("no small horizontal level on this ROM")
        });
    let level = &rom.levels[level_num as usize];
    anyhow::ensure!(!level.secondary_header.vertical_level(), "canvas mock needs a horizontal level");
    let screens = level.primary_header.level_length() as u32 + 1;
    let max_h = max_height_tiles(screens);
    let height =
        args.iter().find_map(|a| a.strip_prefix("--height=")).and_then(|s| s.parse::<u16>().ok()).unwrap_or(height);
    anyhow::ensure!(
        height >= VANILLA_HORIZONTAL_HEIGHT_TILES && height <= max_h,
        "height {height} out of range {VANILLA_HORIZONTAL_HEIGHT_TILES}..={max_h} for {screens} screens"
    );
    println!("canvas: level {level_num:#05X} ({screens} screens), height {height} tiles (max {max_h})");

    // ---- Real tile pixels for the top 27 rows (emulator render) ----
    let raw = std::fs::read(rom_path)?;
    let rom_bytes = if raw.len() % 0x400 == 0x200 { raw[0x200..].to_vec() } else { raw };
    let mut emu_rom = EmuRom::new(rom_bytes);
    emu_rom.load_symbols(include_str!("../../symbols/SMW_U.sym"));
    let mut cpu = Cpu::new(CheckedMem::new(Arc::new(emu_rom)));
    smwe_emu::emu::decompress_sublevel(&mut cpu, level_num);
    // Render at the full 32-screen tilemap width exactly like render_level.rs:
    // render_layer's `width` is the pixel-row stride, and blocks for screens
    // past the level's own would wrap into wrong rows of a narrower buffer.
    // Crop to the level's screens afterwards.
    let render_width = 32 * 256;
    let width = screens * 256;
    let vanilla_h = VANILLA_HORIZONTAL_HEIGHT_TILES as u32 * 16;
    let mut pixels = vec![0u8; (render_width * vanilla_h * 3) as usize];
    {
        let backdrop = read_color(&cpu.mem.cgram, 0);
        for px in pixels.chunks_exact_mut(3) {
            px.copy_from_slice(&backdrop);
        }
    }
    render_layer(&mut cpu, true, render_width, &mut pixels);
    render_layer(&mut cpu, false, render_width, &mut pixels);

    // ---- Compose the tall canvas ----
    let strip = 64u32; // caption strip
    let canvas_h = height as u32 * 16;
    let mut img = RgbImage::new(width, strip + canvas_h);
    let backdrop = Rgb([pixels[0], pixels[1], pixels[2]]);
    for p in img.pixels_mut() {
        *p = backdrop;
    }
    // Paste the real tile render at the top (cropped to the level's screens).
    for y in 0..vanilla_h {
        for x in 0..width {
            let i = ((y * render_width + x) * 3) as usize;
            img.put_pixel(x, strip + y, Rgb([pixels[i], pixels[i + 1], pixels[i + 2]]));
        }
    }

    let ink = Rgb([0x1A, 0x1A, 0x1A]);
    let gray = Rgb([0x66, 0x66, 0x66]);
    let grid_minor = Rgb([0xFF, 0xFF, 0xFF]);
    let grid_major = Rgb([0xFF, 0xFF, 0xFF]);

    // Caption strip.
    fill_rect(&mut img, 0, 0, width, strip, Rgb([0x2B, 0x2B, 0x2B]));
    draw_text(
        &mut img,
        &fonts.sans_bold,
        &format!(
            "Level {level_num:#05X} at {height} tiles tall (vanilla 27) \u{2014} headless mock; tiles are a real emulator render"
        ),
        16,
        12,
        17.0,
        Rgb([0xFF, 0xFF, 0xFF]),
    );
    draw_text(
        &mut img,
        &fonts.sans,
        "Tile pixels: emulator/WRAM (vanilla 27 rows). Rows 27+ are the editor authoring region: grid + object overlays.",
        16,
        36,
        13.0,
        Rgb([0xCC, 0xCC, 0xCC]),
    );

    // Grid across the whole canvas (editor: white alpha 25 minor / 100 major).
    for gx in (0..=width).step_by(16) {
        let major = gx % 256 == 0;
        for y in strip..strip + canvas_h {
            let dst = *img.get_pixel(gx.min(width - 1), y);
            img.put_pixel(gx.min(width - 1), y, alpha_blend(dst, (255, 255, 255), if major { 100 } else { 25 }));
        }
        let _ = (grid_minor, grid_major);
    }
    for gy in (0..=canvas_h).step_by(16) {
        let y = strip + gy;
        let major = gy % 256 == 0;
        for x in 0..width {
            let dst = *img.get_pixel(x, y.min(strip + canvas_h - 1));
            img.put_pixel(
                x,
                y.min(strip + canvas_h - 1),
                alpha_blend(dst, (255, 255, 255), if major { 100 } else { 25 }),
            );
        }
    }

    // Vanilla 27-row boundary.
    let by = strip + vanilla_h;
    for x in 0..width {
        for dy in 0..2 {
            if by + dy < img.height() {
                img.put_pixel(x, by + dy, Rgb([0xFF, 0xC8, 0x00]));
            }
        }
    }
    draw_text(&mut img, &fonts.sans_bold, "vanilla 27-row boundary", 10, by as i32 + 6, 13.0, Rgb([0xFF, 0xC8, 0x00]));

    // ---- Object overlays: the level's real objects ----
    // Same screen tracking as EditableObjectLayer::from_object_layer.
    let mut screen = 0u32;
    let mut drawn = 0u32;
    for inst in level.layer1.objects() {
        let (id, lx, ly, w, h) = match inst {
            ObjectInstance::Standard(o) => {
                if o.new_screen() {
                    screen += 1;
                }
                let (lx, ly) = o.xy_pos();
                let s = o.settings();
                (o.std_obj_num(), screen * 16 + lx as u32, ly as u32, (s & 0x0F) as u32 + 1, (s >> 4) as u32 + 1)
            }
            ObjectInstance::Extended(_) => continue, // exits/jumps carry no tile footprint
        };
        if ly * 16 >= strip + canvas_h {
            continue;
        }
        draw_obj_rect(&mut img, lx * 16, strip + ly * 16, w * 16, h * 16, obj_color(id), false);
        drawn += 1;
    }

    // ---- Demo objects in the extended region (rows 28-30) ----
    // Real standard-object bytes, parsed back through ObjectLayer::parse:
    // the 5-bit Y must survive the round trip. (0xFF terminates the stream.)
    let demo: [u8; 10] = [
        0x3C, 0x62, 0x00, // x=2,  y=28, id=0x16
        0x3E, 0x65, 0x00, // x=5,  y=30, id=0x16
        0x5D, 0xE8, 0x00, // x=8,  y=29, id=0x2E
        0xFF,
    ];
    let (_, (demo_layer, _)) =
        ObjectLayer::parse(&demo).map_err(|_| anyhow::anyhow!("demo objects failed to parse"))?;
    let mut demo_ys = Vec::new();
    for inst in demo_layer.objects() {
        let ObjectInstance::Standard(o) = inst else { anyhow::bail!("demo object parsed as non-standard") };
        let (lx, ly) = o.xy_pos();
        demo_ys.push(ly);
        draw_obj_rect(&mut img, lx as u32 * 16, strip + ly as u32 * 16, 16, 16, obj_color(o.std_obj_num()), true);
    }
    anyhow::ensure!(demo_ys == vec![28, 30, 29], "demo object Y did not round-trip: {demo_ys:?}");

    draw_text(
        &mut img,
        &fonts.sans,
        &format!("{drawn} real object overlays + 3 demo objects at rows 28\u{2013}30 (yellow outline; Y verified through the real parser)"),
        10,
        (strip + canvas_h) as i32 - 22,
        13.0,
        ink,
    );
    let _ = gray;

    img.save(out)?;
    Ok(())
}
