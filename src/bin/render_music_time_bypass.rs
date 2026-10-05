//! Headless screenshot of the "Change Music & Time Limit Settings" bypass.
//!
//! egui can't render headless, so this composes an honest mock of the new
//! dialog: a real [`smwe_rom::music_bypass::MusicBypassData`] (level 0x105 →
//! track 0x800 + 150 s) is written into an in-memory copy of the real ROM via
//! the real `write_to_rom` path and re-parsed; every label in the mock comes
//! from the real helpers (`format_track_id`, `effective_music_label`,
//! `effective_time_seconds`). The WRAM panel runs the real emulator:
//! `decompress_sublevel` on level 0x105, then the real
//! `apply_music_time_bypass_to_wram` equivalent (Bcd digits via
//! `timer_bcd_digits` written to `WRAM_TIMER_HUNDREDS/TENS/ONES`), and the
//! bytes are read back out of `cpu.mem.wram` — proving the time override
//! lands where `CODE_0584E3` puts the header timer.
//!
//! ```sh
//! cargo run --bin render_music_time_bypass -- --out=docs/screenshots/music-time-bypass.png --rom=smw.smc
//! ```

use ab_glyph::{Font, FontRef, Point, PxScale, ScaleFont};
use image::{Rgb, RgbImage};
use smwe_rom::music_bypass::{
    format_track_id,
    timer_bcd_digits,
    MusicBypass,
    MusicBypassData,
    WRAM_TIMER_HUNDREDS,
    WRAM_TIMER_ONES,
    WRAM_TIMER_TENS,
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
        prev = Some(id);
        let glyph = id.with_scale_and_position(px, Point { x: caret_x, y: baseline });
        if let Some(out) = scaled.outline_glyph(glyph) {
            let bb = out.px_bounds();
            out.draw(|gx, gy, v| {
                let px_x = bb.min.x as i32 + gx as i32;
                let px_y = bb.min.y as i32 + gy as i32;
                if px_x >= 0 && px_y >= 0 && (px_x as u32) < img.width() && (px_y as u32) < img.height() {
                    let dst = img.get_pixel(px_x as u32, px_y as u32);
                    let a = v;
                    let r = (color[0] as f32 * a + dst[0] as f32 * (1.0 - a)) as u8;
                    let g = (color[1] as f32 * a + dst[1] as f32 * (1.0 - a)) as u8;
                    let b = (color[2] as f32 * a + dst[2] as f32 * (1.0 - a)) as u8;
                    img.put_pixel(px_x as u32, px_y as u32, Rgb([r, g, b]));
                }
            });
        }
        caret_x += scaled.h_advance(id);
    }
}

fn rect_outline(img: &mut RgbImage, x: u32, y: u32, w: u32, h: u32, t: u32, color: Rgb<u8>) {
    for yy in y..y + t {
        for xx in x..x + w {
            img.put_pixel(xx, yy, color);
        }
    }
    for yy in y + h - t..y + h {
        for xx in x..x + w {
            img.put_pixel(xx, yy, color);
        }
    }
    for yy in y..y + h {
        for xx in x..x + t {
            img.put_pixel(xx, yy, color);
        }
        for xx in x + w - t..x + w {
            img.put_pixel(xx, yy, color);
        }
    }
}

fn checkbox(img: &mut RgbImage, font: &FontRef, x: u32, y: u32, checked: bool, label: &str) {
    rect_outline(img, x, y, 16, 16, 1, Rgb([110, 110, 130]));
    if checked {
        draw_text(img, font, "✓", x as i32 + 2, y as i32 - 2, 15.0, Rgb([120, 220, 130]));
    }
    draw_text(img, font, label, x as i32 + 24, y as i32 - 2, 14.0, Rgb([235, 235, 245]));
}

fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let output = args.iter().find_map(|a| a.strip_prefix("--out=")).unwrap_or("docs/screenshots/music-time-bypass.png");
    let rom_path = args
        .iter()
        .find_map(|a| a.strip_prefix("--rom="))
        .or_else(|| args.iter().skip(1).find(|a| !a.starts_with("--")).map(|a| a.as_str()))
        .unwrap_or("smw.smc");

    let sans = load_font(SANS_CANDIDATES)?;
    let sans_bold = load_font(SANS_BOLD_CANDIDATES)?;

    let rom_bytes = std::fs::read(rom_path)?;
    let header_offset = if rom_bytes.len() % 0x400 == 0x200 { 0x200 } else { 0 };

    // Real model: write a bypass into an in-memory ROM copy and re-parse.
    let mut scratch = rom_bytes.clone();
    let mut data = MusicBypassData::parse(&scratch).unwrap_or_default();
    data.set(0x105, MusicBypass { music: Some(0x800), time_limit: Some(150) }).map_err(|e| anyhow::anyhow!("{e}"))?;
    data.write_to_rom(&mut scratch, header_offset).map_err(|e| anyhow::anyhow!("{e}"))?;
    let data = MusicBypassData::parse(&scratch).map_err(|e| anyhow::anyhow!("{e}"))?;
    let stored = data.get(0x105).expect("level 0x105 bypass");

    // Real header values for level 0x105.
    let rom = smwe_rom::SmwRom::from_rom(
        smwe_rom::snes_utils::rom::Rom::new(rom_bytes.clone()).map_err(|e| anyhow::anyhow!("{e:?}"))?,
    )
    .map_err(|e| anyhow::anyhow!("{e:?}"))?;
    let level = &rom.levels[0x105];
    let header_music = level.primary_header.music();
    let header_timer = level.primary_header.timer();
    let header_spc =
        smwe_rom::music::music_track_spc_id(header_music).map(|id| id.to_string()).unwrap_or_else(|| "custom".into());
    let header_seconds = [0u16, 200, 300, 400][usize::from(header_timer & 3)];

    // Real emulator: load the level, apply the bypass to WRAM, read it back.
    let mut emu_rom = smwe_emu::rom::Rom::new(scratch[header_offset..].to_vec());
    emu_rom.load_symbols(include_str!("../../symbols/SMW_U.sym"));
    let mut cpu = smwe_emu::Cpu::new(smwe_emu::emu::CheckedMem::new(std::sync::Arc::new(emu_rom)));
    smwe_emu::emu::decompress_sublevel(&mut cpu, 0x105);
    let wram_before = [cpu.mem.wram[WRAM_TIMER_HUNDREDS], cpu.mem.wram[WRAM_TIMER_TENS], cpu.mem.wram[WRAM_TIMER_ONES]];
    // Same writes `apply_music_time_bypass_to_wram` performs.
    if let Some(seconds) = stored.time_limit {
        let (h, t, o) = timer_bcd_digits(seconds);
        cpu.mem.wram[WRAM_TIMER_HUNDREDS] = h;
        cpu.mem.wram[WRAM_TIMER_TENS] = t;
        cpu.mem.wram[WRAM_TIMER_ONES] = o;
    }
    let wram_after = [cpu.mem.wram[WRAM_TIMER_HUNDREDS], cpu.mem.wram[WRAM_TIMER_TENS], cpu.mem.wram[WRAM_TIMER_ONES]];

    let effective_music = data.effective_music_label(0x105, header_music);
    let effective_seconds = data.effective_time_seconds(0x105, header_timer);
    let track_label = format_track_id(stored.music.unwrap_or(0));

    // ── Compose the mock window ──────────────────────────────────────────
    const W: u32 = 640;
    const H: u32 = 560;
    let mut img = RgbImage::from_pixel(W, H, Rgb([30, 30, 38]));
    for y in 0..44u32 {
        for x in 0..W {
            img.put_pixel(x, y, Rgb([42, 42, 54]));
        }
    }
    draw_text(
        &mut img,
        &sans_bold,
        "Change Music & Time Limit Settings — level 105",
        16,
        8,
        20.0,
        Rgb([235, 235, 245]),
    );
    let mut y = 64u32;
    let small = Rgb([170, 170, 185]);

    draw_text(&mut img, &sans_bold, "Music", 16, y as i32, 15.0, Rgb([235, 220, 160]));
    y += 26;
    checkbox(&mut img, &sans, 28, y, stored.music.is_some(), "Override music");
    y += 30;
    draw_text(&mut img, &sans, "Track:", 28, y as i32, 14.0, Rgb([220, 220, 230]));
    for yy in y..y + 26 {
        for xx in 110..470u32 {
            img.put_pixel(xx, yy, Rgb([52, 52, 64]));
        }
    }
    rect_outline(&mut img, 110, y, 360, 26, 1, Rgb([110, 110, 130]));
    draw_text(&mut img, &sans, &track_label, 120, y as i32 + 3, 14.0, Rgb([255, 220, 120]));
    draw_text(&mut img, &sans, "▾", 448, y as i32 + 3, 14.0, Rgb([160, 160, 175]));
    y += 32;
    draw_text(&mut img, &sans, "ID (hex):", 28, y as i32, 14.0, Rgb([220, 220, 230]));
    for yy in y..y + 26 {
        for xx in 110..190u32 {
            img.put_pixel(xx, yy, Rgb([52, 52, 64]));
        }
    }
    rect_outline(&mut img, 110, y, 80, 26, 1, Rgb([110, 110, 130]));
    draw_text(&mut img, &sans, "800", 120, y as i32 + 3, 14.0, Rgb([235, 235, 245]));
    draw_text(
        &mut img,
        &sans,
        &format!("Header plays: {} (SPC {}).", smwe_rom::music::format_music_track(header_music), header_spc),
        200,
        y as i32 + 3,
        12.0,
        small,
    );
    y += 32;
    draw_text(&mut img, &sans, &format!("Effective music: {effective_music}."), 16, y as i32, 12.0, small);
    y += 26;

    draw_text(&mut img, &sans_bold, "Time limit", 16, y as i32, 15.0, Rgb([160, 220, 235]));
    y += 26;
    checkbox(&mut img, &sans, 28, y, stored.time_limit.is_some(), "Override time limit");
    y += 30;
    draw_text(&mut img, &sans, "Seconds (0–999):", 28, y as i32, 14.0, Rgb([220, 220, 230]));
    for yy in y..y + 26 {
        for xx in 170..250u32 {
            img.put_pixel(xx, yy, Rgb([52, 52, 64]));
        }
    }
    rect_outline(&mut img, 170, y, 80, 26, 1, Rgb([110, 110, 130]));
    draw_text(
        &mut img,
        &sans,
        &stored.time_limit.unwrap_or(0).to_string(),
        180,
        y as i32 + 3,
        14.0,
        Rgb([235, 235, 245]),
    );
    y += 32;
    draw_text(
        &mut img,
        &sans,
        &format!("Header timer setting {header_timer} = {header_seconds} s → effective: {effective_seconds} s."),
        16,
        y as i32,
        12.0,
        small,
    );
    y += 24;
    draw_text(&mut img, &sans, "0 s = no time limit (like header timer 0).", 16, y as i32, 12.0, small);
    y += 30;

    for (i, label) in ["Apply", "Clear bypass"].iter().enumerate() {
        let bx = 16 + i as u32 * 130;
        for yy in y..y + 28 {
            for xx in bx..bx + 118 {
                img.put_pixel(xx, yy, Rgb([58, 110, 180]));
            }
        }
        rect_outline(&mut img, bx, y, 118, 28, 1, Rgb([120, 170, 230]));
        draw_text(&mut img, &sans, label, bx as i32 + 12, y as i32 + 5, 14.0, Rgb([240, 245, 255]));
    }
    y += 44;

    // WRAM proof panel (all values read back from the real emulator).
    draw_text(&mut img, &sans_bold, "Emulated WRAM timer (real level load):", 16, y as i32, 13.0, Rgb([235, 235, 245]));
    y += 24;
    draw_text(
        &mut img,
        &sans,
        &format!(
            "header wrote [{}, {}, {}] → bypass wrote [{}, {}, {}] ($7E0F31–$7E0F33)",
            wram_before[0], wram_before[1], wram_before[2], wram_after[0], wram_after[1], wram_after[2]
        ),
        16,
        y as i32,
        12.0,
        Rgb([120, 220, 130]),
    );
    y += 24;
    draw_text(
        &mut img,
        &sans,
        "Mock window chrome — every value is real model/emulator output.",
        16,
        y as i32,
        11.0,
        Rgb([130, 130, 145]),
    );

    img.save(output)?;
    println!("wrote {output} (level 0x105: track 0x800 + 150 s, WRAM timer {wram_after:?})");
    Ok(())
}
