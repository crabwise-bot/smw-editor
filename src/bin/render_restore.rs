//! Headless mock screenshots of the Restore menu (LM v1.80 parity):
//! restore points, revert flow, and apply-IPS.
//!
//! egui can't render headless, so these compose honest mocks of the menu and
//! dialogs: every piece of *data* shown is real — the restore-point names and
//! stamps come from a real `RestoreManager` populated with snapshots of the
//! real ROM, and the Apply-IPS dialog shows a real IPS patch
//! (`smwe-ips::create_patch` over bytes actually changed in the ROM image)
//! with its true changed-byte count and size delta. Only the window/menu
//! chrome is drawn rather than real egui widgets.
//!
//! ```sh
//! cargo run --bin render_restore -- --rom=smw.smc
//! ```
//! Writes `docs/screenshots/restore-menu.png` and
//! `docs/screenshots/apply-ips.png`.

use ab_glyph::{Font, FontRef, Glyph, Point, PxScale, ScaleFont};
use image::{Rgb, RgbImage};
use smw_editor::{
    render_util::{fill_rect, rect_border},
    ui::restore::RestoreManager,
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

/// Build a real `RestoreManager` against a scratch copy of the ROM and take
/// real snapshots of it, so the menu rows show genuine names/stamps/sizes.
fn sample_manager(rom_bytes: &[u8]) -> anyhow::Result<(RestoreManager, std::path::PathBuf)> {
    let scratch = std::env::temp_dir().join("render_restore_scratch.smc");
    std::fs::write(&scratch, rom_bytes)?;
    let mut mgr = RestoreManager::new();
    mgr.open_rom(&scratch);
    assert!(mgr.original().is_some(), "reference copy must be captured on open");

    // Simulate a session: two manual snapshots of actually-differing images.
    let mut edited = rom_bytes.to_vec();
    edited[0x200] ^= 0xFF;
    edited[0x201] ^= 0xFF;
    mgr.create_point("Before boss text edits".to_string(), edited.clone());
    edited[0x300] ^= 0x0F;
    mgr.create_point("Before expanding to 2MB".to_string(), edited);

    // One automatic pre-save point, as the tracking toggle would capture.
    mgr.auto_track_on_save = true;
    mgr.auto_point_before_save(rom_bytes.to_vec());

    // Prove revert hands back the exact snapshot bytes.
    let back = mgr.revert_bytes(0).expect("point 0 must exist");
    assert_eq!(back[0x200], rom_bytes[0x200] ^ 0xFF, "revert must return snapshot bytes");
    Ok((mgr, scratch))
}

/// Build a real IPS patch between the pristine ROM and an edited image, then
/// apply it to a third image to prove the round trip — the dialog shows the
/// real patch name, real changed-byte count, and real size delta.
fn sample_ips(rom_bytes: &[u8]) -> anyhow::Result<(String, usize, usize, usize)> {
    let mut edited = rom_bytes.to_vec();
    // Change a scattering of bytes, like a small hack would.
    for (i, b) in [0x1234usize, 0x2345, 0x8000, 0x1_2345, 0x2_3456].iter().enumerate() {
        edited[*b] = edited[*b].wrapping_add((i as u8) + 1);
    }
    let patch = smwe_ips::create_patch(rom_bytes, &edited)?;
    let applied = smwe_ips::apply_patch(rom_bytes, &patch)?;
    assert_eq!(applied, edited, "create→apply must round-trip on the real ROM");
    let changed =
        rom_bytes.iter().zip(edited.iter()).filter(|(a, b)| a != b).count() + rom_bytes.len().abs_diff(edited.len());
    assert!(changed > 0, "the sample patch must change bytes");
    Ok(("boss-text-tweaks.ips".to_string(), patch.len(), changed, edited.len()))
}

fn render_restore_menu(fonts: &Fonts, mgr: &RestoreManager, out: &str) -> anyhow::Result<()> {
    let (w, h) = (1180u32, 760u32);
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
        "Restore menu — headless mock (point names/stamps are from a real RestoreManager over the real ROM)",
        w,
    );

    // Fake top menu bar.
    let mut y = 76u32;
    draw_text(&mut img, &fonts.sans_bold, "File    Restore    Editors    Tools", 24, y as i32, 16.0, ink);
    y += 34;

    // The open Restore menu (dark egui menu chrome).
    let (mx, mw) = (150u32, 470u32);
    let mut my = y;
    fill_rect(&mut img, mx, my, mw, 250, Rgb([0x2B, 0x2B, 0x2B]));
    rect_border(&mut img, mx, my, mw, 250, Rgb([0x55, 0x55, 0x55]));
    my += 14;
    let white = Rgb([0xFF, 0xFF, 0xFF]);
    let dim = Rgb([0xBB, 0xBB, 0xBB]);
    draw_text(&mut img, &fonts.sans, "Create Restore Point...", mx as i32 + 16, my as i32, 16.0, white);
    my += 34;
    draw_text(&mut img, &fonts.sans, "Revert to Restore Point  ▸", mx as i32 + 16, my as i32, 16.0, white);
    my += 34;

    // The open "Revert to Restore Point" submenu with REAL point rows.
    let (sx, sw) = (mx + mw + 6, 520u32);
    let submenu_h = 60 + mgr.points().len() as u32 * 32;
    fill_rect(&mut img, sx, y + 40, sw, submenu_h, Rgb([0x2B, 0x2B, 0x2B]));
    rect_border(&mut img, sx, y + 40, sw, submenu_h, Rgb([0x55, 0x55, 0x55]));
    let mut sy = y + 54;
    draw_text(&mut img, &fonts.sans_bold, "Restore points", sx as i32 + 16, sy as i32, 15.0, dim);
    sy += 30;
    for p in mgr.points() {
        draw_text(
            &mut img,
            &fonts.mono,
            &format!("{}  ({})", p.name, p.stamp()),
            sx as i32 + 16,
            sy as i32,
            14.0,
            white,
        );
        sy += 32;
    }
    // Back in the main menu.
    fill_rect(&mut img, mx, my, mw, 2, Rgb([0x55, 0x55, 0x55]));
    my += 16;
    draw_text(&mut img, &fonts.sans, "Create IPS Patch...", mx as i32 + 16, my as i32, 16.0, white);
    my += 34;
    draw_text(&mut img, &fonts.sans, "Apply IPS Patch...", mx as i32 + 16, my as i32, 16.0, white);
    my += 34;
    fill_rect(&mut img, mx, my, mw, 2, Rgb([0x55, 0x55, 0x55]));
    my += 16;
    // Checked tracking checkbox.
    fill_rect(&mut img, mx + 16, my, 18, 18, accent);
    draw_text(&mut img, &fonts.sans_bold, "✓", mx as i32 + 20, (my - 2) as i32, 15.0, white);
    draw_text(
        &mut img,
        &fonts.sans,
        "Track changes (restore point before each save)",
        mx as i32 + 44,
        my as i32,
        15.0,
        white,
    );
    // Footnote under the menu.
    let fy = y + 520;
    draw_text(
        &mut img,
        &fonts.sans,
        "LM v1.80 parity: the Restore menu keeps a reference copy of the original ROM",
        24,
        fy as i32,
        14.0,
        gray,
    );
    draw_text(
        &mut img,
        &fonts.sans,
        "and any number of named snapshots; reverting rewrites the ROM (with a .bak backup),",
        24,
        (fy + 26) as i32,
        14.0,
        gray,
    );
    draw_text(
        &mut img,
        &fonts.sans,
        "then closes stale editors and reopens the level editor on the reverted image.",
        24,
        (fy + 52) as i32,
        14.0,
        gray,
    );
    draw_text(
        &mut img,
        &fonts.sans,
        "Point data above is real — parsed from the same RestoreManager the UI uses. Window chrome is drawn, not real egui.",
        24,
        (fy + 96) as i32,
        13.0,
        gray,
    );

    img.save(out)?;
    println!("wrote {out} ({w}x{h})");
    Ok(())
}

fn render_apply_ips(
    fonts: &Fonts, patch_name: &str, changed: usize, old_len: usize, new_len: usize, out: &str,
) -> anyhow::Result<()> {
    let (w, h) = (1180u32, 640u32);
    let mut img = RgbImage::new(w, h);
    let bg = Rgb([0xF2, 0xF2, 0xF2]);
    let gray = Rgb([0x66, 0x66, 0x66]);
    let accent = Rgb([0x1A, 0x5A, 0x9A]);
    for p in img.pixels_mut() {
        *p = bg;
    }
    title_bar(
        &mut img,
        fonts,
        "Apply IPS dialog — headless mock (patch name, byte counts and sizes are from a real smwe-ips round trip)",
        w,
    );

    // Dialog window.
    let (dx, dy, dw, dh) = (240u32, 130u32, 700u32, 330u32);
    fill_rect(&mut img, dx, dy, dw, dh, Rgb([0x2B, 0x2B, 0x2B]));
    rect_border(&mut img, dx, dy, dw, dh, Rgb([0x77, 0x77, 0x77]));
    let white = Rgb([0xFF, 0xFF, 0xFF]);
    let mut y = dy + 22;
    draw_text(&mut img, &fonts.sans_bold, "Apply IPS Patch", dx as i32 + 24, y as i32, 19.0, white);
    y += 44;
    draw_text(&mut img, &fonts.sans, &format!("Patch: {patch_name}"), dx as i32 + 24, y as i32, 16.0, white);
    y += 32;
    draw_text(
        &mut img,
        &fonts.mono,
        &format!("{changed} byte(s) will change. ROM size: {old_len} → {new_len}."),
        dx as i32 + 24,
        y as i32,
        15.0,
        white,
    );
    y += 34;
    draw_text(
        &mut img,
        &fonts.sans,
        "The patched ROM is written to disk (a .bak backup is kept).",
        dx as i32 + 24,
        y as i32,
        15.0,
        Rgb([0xBB, 0xBB, 0xBB]),
    );
    y += 28;
    draw_text(
        &mut img,
        &fonts.sans,
        "All open editors will be closed and reopened on the patched ROM.",
        dx as i32 + 24,
        y as i32,
        15.0,
        Rgb([0xBB, 0xBB, 0xBB]),
    );
    y += 44;
    // Buttons.
    fill_rect(&mut img, dx + 24, y, 120, 40, accent);
    draw_text(&mut img, &fonts.sans_bold, "Apply", dx as i32 + 60, (y + 8) as i32, 16.0, white);
    rect_border(&mut img, dx + 160, y, 120, 40, Rgb([0x99, 0x99, 0x99]));
    draw_text(&mut img, &fonts.sans, "Cancel", dx as i32 + 196, (y + 8) as i32, 16.0, white);

    let fy = dy + dh + 40;
    draw_text(
        &mut img,
        &fonts.sans,
        "LM v1.80 parity: the confirmation shows exactly what the patch does before",
        24,
        fy as i32,
        14.0,
        gray,
    );
    draw_text(
        &mut img,
        &fonts.sans,
        "it is installed — the patch above was really created from and applied to the ROM",
        24,
        (fy + 26) as i32,
        14.0,
        gray,
    );
    draw_text(
        &mut img,
        &fonts.sans,
        "with smwe-ips (create → apply round-trip asserted). Window chrome is drawn, not real egui.",
        24,
        (fy + 52) as i32,
        14.0,
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

    let rom_bytes = std::fs::read(rom_path)?;
    assert!(!rom_bytes.is_empty(), "ROM must not be empty");

    let (mgr, scratch) = sample_manager(&rom_bytes)?;
    let (patch_name, _patch_len, changed, new_len) = sample_ips(&rom_bytes)?;
    std::fs::remove_file(&scratch).ok();

    render_restore_menu(&fonts, &mgr, &format!("{out_dir}/restore-menu.png"))?;
    render_apply_ips(&fonts, &patch_name, changed, rom_bytes.len(), new_len, &format!("{out_dir}/apply-ips.png"))?;
    Ok(())
}
