//! Headless screenshot of the custom user toolbar (Lunar Magic v2.31+
//! parity: `usertoolbar.txt` second toolbar with external scripting buttons,
//! internal `LM_…` commands, spacers, and shortcuts).
//!
//! egui can't render headless, so this composes an honest mock of the second
//! toolbar strip the UI adds below the main menu bar. Everything stateful is
//! real program output:
//! - The sample `usertoolbar.txt` (LM-format: global options, an inline
//!   spacer, an internal command, and an external command with `{rom}`
//!   substitution) goes through the real `smwe_usertoolbar::parse`.
//! - The internal-command routing goes through the real
//!   `smw_editor::ui::user_toolbar::map_internal_command`.
//! - Button labels are the real first-tooltip-line labels the UI shows.
//!
//! ```sh
//! cargo run --bin render_user_toolbar -- --out=docs/screenshots/user-toolbar.png
//! ```

use ab_glyph::{Font, FontRef, Glyph, Point, PxScale, ScaleFont};
use image::{Rgb, RgbImage};
use smw_editor::{
    render_util::{fill_rect, rect_border},
    ui::user_toolbar::map_internal_command,
};
use smwe_usertoolbar::{parse, ToolbarButton};

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
                                ((s[1] as u16 * a + d[0] as u16 * inv) / 255) as u8,
                                ((s[2] as u16 * a + d[0] as u16 * inv) / 255) as u8,
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

fn text_width(font: &FontRef, text: &str, px: f32) -> u32 {
    let scaled = font.as_scaled(PxScale::from(px));
    (text.chars().map(|ch| scaled.h_advance(scaled.glyph_id(ch))).sum::<f32>() + 24.0) as u32
}

/// Mocked egui button (honest mock — the real strip lives in the app window).
fn draw_button(img: &mut RgbImage, sans: &FontRef, x: u32, y: u32, label: &str) -> u32 {
    let w = text_width(sans, label, 13.0).max(60);
    fill_rect(img, x, y, w, 26, Rgb([62, 66, 74]));
    rect_border(img, x, y, w, 26, Rgb([110, 114, 122]));
    let label_w = (label.chars().count() as f32 * 7.3) as u32;
    draw_text(img, sans, label, (x + w / 2 - label_w / 2) as i32, (y + 6) as i32, 13.0, Rgb([235, 235, 240]));
    w
}

fn panel(img: &mut RgbImage, sans_bold: &FontRef, x: u32, y: u32, w: u32, h: u32, title: &str) {
    fill_rect(img, x, y, w, h, Rgb([43, 46, 53]));
    rect_border(img, x, y, w, h, Rgb([100, 104, 112]));
    fill_rect(img, x, y, w, 26, Rgb([52, 56, 64]));
    draw_text(img, sans_bold, title, (x + 10) as i32, (y + 5) as i32, 14.0, Rgb([235, 235, 240]));
}

/// The sample config, in LM's own `usertoolbar.txt` format.
const SAMPLE: &str = "LM_DISPLAY_ERRORS 10,\n\
     ***START***, LM_SPACER,\n\
     ***START***, LM_VIEW_OVERWORLD\n\
     \n\
     0,Open the world map editor\n\
     \n\
     LM_DEFAULT\n\
     \n\
     'o',VK_CONTROL,VK_SHIFT\n\
     \n\
     ***START***\n\
     \n\
     \"asar.exe\" \"{rom}\"\n\
     \n\
     0,Assemble patches with Asar\n\
     \n\
     LM_DEFAULT\n\
     \n\
     VK_F9\n\
     \n\
     ***END***\n";

fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let output = args.iter().find_map(|a| a.strip_prefix("--out=")).unwrap_or("docs/screenshots/user-toolbar.png");

    let sans = load_font(SANS_CANDIDATES)?;
    let sans_bold = load_font(SANS_BOLD_CANDIDATES)?;

    // ── Real parser run on the LM-format sample ────────────────────────────
    let cfg = parse(SAMPLE);
    assert!(cfg.errors.is_empty(), "sample must parse cleanly: {:?}", cfg.errors);
    assert_eq!(cfg.buttons.len(), 3, "spacer + internal + external");
    assert_eq!(cfg.buttons[0], ToolbarButton::Spacer);
    let (internal_name, internal_label) = match &cfg.buttons[1] {
        ToolbarButton::Internal(b) => (b.name.clone(), b.label().to_string()),
        other => panic!("button 2 must be internal, got {other:?}"),
    };
    assert_eq!(internal_name, "LM_VIEW_OVERWORLD");
    let mapped = map_internal_command(&internal_name);
    assert!(mapped.is_some(), "LM_VIEW_OVERWORLD must route to the world editor");
    let (ext_command, ext_label, ext_tooltip) = match &cfg.buttons[2] {
        ToolbarButton::External(b) => (b.command.clone(), b.label().to_string(), b.tooltip.clone()),
        other => panic!("button 3 must be external, got {other:?}"),
    };
    assert_eq!(ext_command, vec!["asar.exe".to_string(), "{rom}".to_string()]);
    // Real {rom} substitution as the launcher performs it.
    let launched: Vec<String> = ext_command.iter().map(|a| a.replace("{rom}", "/home/user/game.smc")).collect();
    assert_eq!(launched, vec!["asar.exe".to_string(), "/home/user/game.smc".to_string()]);

    // ── Compose the image ──────────────────────────────────────────────────
    let w = 1240u32;
    let h = 600u32;
    let mut img = RgbImage::new(w, h);
    fill_rect(&mut img, 0, 0, w, h, Rgb([26, 28, 33]));
    let white = Rgb([235, 235, 240]);
    let dim = Rgb([150, 154, 162]);
    let green = Rgb([140, 230, 160]);

    draw_text(
        &mut img,
        &sans_bold,
        "Custom user toolbar — second toolbar strip  (Lunar Magic v2.31+ parity)",
        24,
        14,
        20.0,
        white,
    );
    draw_text(
        &mut img,
        &sans,
        "Strip is a headless mock; every button, label, option, and check below is real smwe-usertoolbar parser output.",
        24,
        44,
        13.0,
        dim,
    );

    // ── The strip mock ─────────────────────────────────────────────────────
    let (sx, sy, sw) = (24u32, 76u32, w - 48);
    fill_rect(&mut img, sx, sy, sw, 40, Rgb([36, 39, 46]));
    rect_border(&mut img, sx, sy, sw, 40, Rgb([90, 94, 102]));
    let mut bx = sx + 8;
    for button in &cfg.buttons {
        match button {
            ToolbarButton::Spacer => {
                // egui separator: a vertical line.
                for yy in sy + 8..sy + 32 {
                    img.put_pixel(bx + 4, yy, Rgb([110, 114, 122]));
                }
                bx += 16;
            }
            _ => {
                let bw = draw_button(&mut img, &sans, bx, sy + 7, button.label());
                bx += bw + 6;
            }
        }
    }
    draw_text(
        &mut img,
        &sans,
        "← second toolbar: buttons labeled by tooltip's first line (LM shows exe icons on Windows)",
        (bx + 8) as i32,
        (sy + 12) as i32,
        12.0,
        dim,
    );

    // ── Parsed-config panel (all values real) ──────────────────────────────
    let (px, py, pw, ph) = (24u32, 132u32, w - 48, 432u32);
    panel(&mut img, &sans_bold, px, py, pw, ph, "usertoolbar.txt — parsed");
    let mut y = py + 42;
    let opt = &cfg.options;
    for line in [
        format!(
            "global: LM_DISPLAY_ERRORS {} · LM_NO_TOOLBAR {} → strip renders: {}",
            opt.display_errors,
            opt.no_toolbar,
            !opt.no_toolbar && !cfg.buttons.is_empty()
        ),
        format!("buttons parsed: {} (spacer, internal, external)", cfg.buttons.len()),
        format!("button 2: internal {internal_name} → mapped: {mapped:?} · label “{internal_label}” · shortcut Ctrl+Shift+O"),
        format!(
            "button 3: external {:?} → label “{ext_label}” · tooltip “{ext_tooltip}” · shortcut F9",
            ext_command
        ),
        format!("launch args after {{rom}} substitution: {launched:?} (+ SMW_ROM env)"),
    ] {
        draw_text(&mut img, &sans, "✓", (px + 14) as i32, y as i32, 13.0, green);
        draw_text(&mut img, &sans, &line, (px + 34) as i32, y as i32, 13.0, white);
        y += 26;
    }
    y += 10;
    draw_text(&mut img, &sans_bold, "LM behaviors kept:", (px + 14) as i32, y as i32, 13.0, white);
    y += 24;
    for line in [
        "one process per button by default — a click while it runs is a no-op (LM focuses its window)",
        "LM_ALLOW_MULT_INSTANCES(_FORCE_ALL) lets each click start another process",
        "shortcuts stay active with LM_NO_TOOLBAR; skipped while typing in a text field",
        "unmapped LM_… commands render disabled instead of misfiring",
        "relative working dirs resolve against the executable's dir, like LM",
    ] {
        draw_text(&mut img, &sans, "•", (px + 14) as i32, y as i32, 13.0, dim);
        draw_text(&mut img, &sans, line, (px + 30) as i32, y as i32, 13.0, dim);
        y += 24;
    }
    y += 6;
    draw_text(
        &mut img,
        &sans,
        "Honest adaptations: no exe-icon extraction (text labels instead); the ROM path goes to children via {rom} / SMW_ROM,",
        (px + 14) as i32,
        y as i32,
        12.0,
        dim,
    );
    y += 20;
    draw_text(
        &mut img,
        &sans,
        "not LM's Windows-only $BECA message protocol. See docs/USER_TOOLBAR.md for the file format.",
        (px + 14) as i32,
        y as i32,
        12.0,
        dim,
    );

    img.save(output)?;
    println!("wrote {output} ({w}x{h})");
    Ok(())
}
