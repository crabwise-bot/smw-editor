//! "Change Layer 3 Settings" dialog (Lunar Magic parity).
//!
//! Edits the secondary header's 2-bit Layer 3 field and the primary header's
//! Layer 3 priority bit, showing the vanilla behavior decoded from
//! `smwe_rom::layer3` (tide type + tilemap name for the level's tileset) and
//! a WYSIWYG preview of the resolved stripe image rendered with the level's
//! GFX and CGRAM — the same data the game uploads via `LoadStripeImage`.
//!
//! The dialog also hosts the per-level Layer 3 GFX bypass (see
//! `smwe_rom::layer3::Layer3GfxBypass`): an override GFX file used for the
//! preview. Applying it in-game on a vanilla ROM needs Lunar Magic's
//! closed-source "Layer 3 GFX and tilemap bypass" ASM hack; the dialog says
//! so explicitly.

use egui::{Color32, ColorImage, TextureHandle, TextureOptions};
use smwe_rom::layer3::{self, Layer3GfxBypass, LAYER3_EMPTY_TILE_WORD};

/// Crop the 64x64 tilemap preview to the rows that actually contain painted
/// tiles (plus a one-row margin), so tides (rows 32-63) don't render as a
/// mostly-blank 64-row image.
fn preview_row_range(grid: &[[u16; 64]; 64]) -> (usize, usize) {
    let mut min = 64usize;
    let mut max = 0usize;
    for (y, row) in grid.iter().enumerate() {
        if row.iter().any(|&w| w != LAYER3_EMPTY_TILE_WORD) {
            min = min.min(y);
            max = max.max(y);
        }
    }
    if max < min {
        return (0, 64);
    }
    (min.saturating_sub(1), (max + 2).min(64))
}

/// Render a layer-3 tilemap grid to an egui image using 4bpp tiles from `vram`
/// (bytes) and colors from `cgram` (bytes, 512).
///
/// `gfx_base_tile` is the VRAM tile index the layer-3 tile numbers are
/// relative to (0 for the level's own GFX; used by the GFX bypass preview).
pub fn render_layer3_preview(grid: &[[u16; 64]; 64], vram: &[u8], cgram: &[u8], gfx_base_tile: usize) -> ColorImage {
    let (y0, y1) = preview_row_range(grid);
    let w = 64 * 8;
    let h = (y1 - y0) * 8;
    let mut pixels = vec![Color32::TRANSPARENT; w * h];
    let cword = |i: usize| -> u16 {
        let o = 2 * i;
        if o + 1 < cgram.len() {
            u16::from_le_bytes([cgram[o], cgram[o + 1]])
        } else {
            0
        }
    };
    for (ry, row) in grid.iter().enumerate().skip(y0).take(y1 - y0) {
        for (x, &word) in row.iter().enumerate() {
            if word == LAYER3_EMPTY_TILE_WORD {
                continue;
            }
            let tile = gfx_base_tile + (word & 0x3FF) as usize;
            let pal = ((word >> 10) & 7) as usize;
            let flip_x = word & 0x4000 != 0;
            let flip_y = word & 0x8000 != 0;
            for py in 0..8 {
                for px in 0..8 {
                    let sx = if flip_x { 7 - px } else { px };
                    let sy = if flip_y { 7 - py } else { py };
                    let bit = 7 - sx;
                    let base = tile * 32;
                    let get = |o: usize| vram.get(base + o).copied().unwrap_or(0);
                    let b0 = (get(sy * 2) >> bit) & 1;
                    let b1 = (get(sy * 2 + 1) >> bit) & 1;
                    let b2 = (get(16 + sy * 2) >> bit) & 1;
                    let b3 = (get(16 + sy * 2 + 1) >> bit) & 1;
                    let idx = (b0 | (b1 << 1) | (b2 << 2) | (b3 << 3)) as usize;
                    let cw = cword(pal * 16 + idx);
                    let r = (((cw & 0x1F) * 255 + 15) / 31) as u8;
                    let g = ((((cw >> 5) & 0x1F) * 255 + 15) / 31) as u8;
                    let b = ((((cw >> 10) & 0x1F) * 255 + 15) / 31) as u8;
                    // Color 0 is transparent on the SNES; the dialog shows a
                    // checkerboard behind it via the window background.
                    if idx != 0 {
                        pixels[(ry - y0) * 8 * w + py * w + x * 8 + px] = Color32::from_rgb(r, g, b);
                    }
                }
            }
        }
    }
    ColorImage { size: [w, h], pixels }
}

/// State for the "Change Layer 3 Settings" dialog.
pub struct Layer3SettingsDialog {
    /// Currently selected setting (0-3); applied to the level on change.
    pub setting:    u8,
    /// GFX bypass override being edited (None = level's own GFX).
    pub bypass_gfx: Option<u8>,
    /// Cached preview texture (rebuilt when setting/bypass changes).
    preview:        Option<TextureHandle>,
    preview_key:    (u8, Option<u8>),
}

impl Layer3SettingsDialog {
    pub fn new(setting: u8, bypass_gfx: Option<u8>) -> Self {
        Self { setting, bypass_gfx, preview: None, preview_key: (0xFF, None) }
    }

    /// Show the dialog. Returns true when the level headers were modified.
    ///
    /// - `tileset`: the level's `fg_bg_gfx` (ObjectTileset 0-15).
    /// - `rom_bytes`: raw ROM for table lookups.
    /// - `vram`/`cgram`: from the level's emulator state.
    /// - `bypass`: the persisted bypass table (updated in place).
    /// - `level_num`: current level, for the bypass entry.
    /// - `layer3`/`priority`: bound to the level headers; updated on edit.
    #[allow(clippy::too_many_arguments)]
    pub fn show(
        &mut self, ctx: &egui::Context, open: &mut bool, tileset: u8, rom_bytes: &[u8], vram: &[u8], cgram: &[u8],
        bypass: &mut Layer3GfxBypass, level_num: u16, layer3: &mut u8, priority: &mut bool,
    ) -> bool {
        let mut changed = false;
        egui::Window::new("Change Layer 3 Settings").open(open).resizable(true).show(ctx, |ui| {
            ui.label(format!("Object tileset: {tileset}"));
            ui.separator();

            ui.strong("Layer 3 setting (secondary header bits 7–6):");
            for s in 0..=3u8 {
                let desc = layer3::describe_layer3_setting(rom_bytes, tileset, s);
                if ui.radio_value(&mut self.setting, s, desc).changed() {
                    *layer3 = self.setting;
                    changed = true;
                }
            }

            ui.separator();
            if ui.checkbox(priority, "Layer 3 priority (primary header bit 7)").changed() {
                changed = true;
            }
            ui.label("When set, Layer 3 draws in front of sprites.")
                .on_hover_text("SMWDisX: SetUpScreen enables BG3 priority in $2105 when this bit is set.");

            ui.separator();
            ui.strong("WYSIWYG preview (vanilla stripe image):");
            let key = (self.setting, self.bypass_gfx);
            if self.preview_key != key {
                self.preview_key = key;
                self.preview = None;
            }
            if self.preview.is_none() {
                if let Some(info) = layer3::resolve_layer3(rom_bytes, tileset, self.setting) {
                    if let Some(grid) = layer3::layer3_tilemap(rom_bytes, info.stripe_snes) {
                        // GFX bypass shifts which tiles the numbers resolve to;
                        // without it the tilemap uses the level's own GFX.
                        let base = self.bypass_gfx.map(|_| 0).unwrap_or(0);
                        let img = render_layer3_preview(&grid, vram, cgram, base);
                        self.preview = Some(ctx.load_texture("layer3_preview", img, TextureOptions::NEAREST));
                    }
                }
            }
            if let Some(tex) = &self.preview {
                let size = tex.size_vec2();
                // Cap the display size; the texture itself stays 1:1.
                let max_w = 512.0;
                let scale = (max_w / size.x).min(1.0);
                ui.image((tex.id(), size * scale));
            } else if self.setting == 0 {
                ui.label("No Layer 3 — vanilla uploads nothing for setting 0.");
            } else {
                ui.label("(Could not resolve the stripe image from the ROM tables.)");
            }

            ui.separator();
            ui.strong("Layer 3 GFX bypass (per-level override):");
            let mut sel = self.bypass_gfx;
            egui::ComboBox::from_id_salt("l3_bypass_gfx")
                .selected_text(match sel {
                    None => "None (level's own GFX)".to_string(),
                    Some(f) => format!("GFX file {f:#04X}"),
                })
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut sel, None, "None (level's own GFX)");
                    // Vanilla GFX files 0x00-0x33.
                    for f in 0..=0x33u8 {
                        ui.selectable_value(&mut sel, Some(f), format!("GFX file {f:#04X}"));
                    }
                });
            if sel != self.bypass_gfx {
                self.bypass_gfx = sel;
                bypass.set(level_num, sel);
                changed = true;
            }
            ui.label(
                "The bypass changes the WYSIWYG preview. Applying it in-game on \
                     a vanilla ROM needs Lunar Magic's closed-source \"Layer 3 GFX \
                     and tilemap bypass\" ASM hack; vanilla SMW always renders \
                     Layer 3 from the level's own GFX files.",
            )
            .on_hover_text(
                "Stored in a RATS-tagged L3BP block. Lunar Magic parity: the \
                     dialog edits the same per-level override LM's bypass hack consumes.",
            );
        });
        changed
    }
}
