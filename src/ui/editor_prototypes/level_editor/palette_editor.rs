use anyhow::Context as _;
use egui::{vec2, Color32, Context, Rect, Sense, Ui, Vec2};
use egui_phosphor::regular as icon;

use super::UiLevelEditor;
use crate::{
    palette_files::{LevelPalette36, SharedPaletteTables, SHARED_PALETTE_BYTES},
    undo::Undo,
};

const CELL_SIZE: f32 = 20.0;
const COLS: usize = 12;

/// The palette editor's undoable state: the 36 colors on screen (BG, FG,
/// sprite) plus the full shared palette tables behind them, so
/// "Insert Shared Palette from File" is a single undo step like every
/// other palette edit.
///
/// Invariant: while the custom palette is off, `bg`/`fg`/`sprite` equal
/// `shared`'s rows at the level's palette indices. Color edits update both
/// sides in one undo step; disabling the custom palette copies the private
/// colors into the shared rows (the adopt behavior); the save path writes
/// only rows flagged in `UiLevelEditor::shared_rows_dirty`.
#[derive(Clone, Debug, Default)]
pub(super) struct EditablePalettes {
    pub bg:     [u16; 12],
    pub fg:     [u16; 12],
    pub sprite: [u16; 12],
    pub shared: SharedPaletteTables,
}

/// Serialized undo size: 72 bytes of on-screen colors + 576 bytes of
/// shared tables.
pub(super) const EDITABLE_PALETTES_BYTES: usize = 72 + crate::palette_files::SHARED_PALETTE_BYTES;

impl EditablePalettes {
    fn group(&self, group: usize) -> &[u16; 12] {
        match group {
            0 => &self.bg,
            1 => &self.fg,
            _ => &self.sprite,
        }
    }

    fn group_mut(&mut self, group: usize) -> &mut [u16; 12] {
        match group {
            0 => &mut self.bg,
            1 => &mut self.fg,
            _ => &mut self.sprite,
        }
    }

    /// Shared-table rows for a group (0=BG, 1=FG, 2=sprite): 8 rows of 12
    /// colors each.
    fn shared_group_mut(&mut self, group: usize) -> &mut [[u16; 12]; 8] {
        match group {
            0 => &mut self.shared.bg,
            1 => &mut self.shared.fg,
            _ => &mut self.shared.sprite,
        }
    }
}

impl Undo for EditablePalettes {
    fn from_bytes(bytes: Vec<u8>) -> Self {
        let mut palettes = Self::default();
        let mut words = bytes.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]]));
        for i in 0..36 {
            if let Some(w) = words.next() {
                palettes.group_mut(i / 12)[i % 12] = w;
            }
        }
        // The remaining 576 bytes are the shared tables (BG, FG, sprite).
        // A short/truncated tail (never produced by `to_bytes`) keeps the
        // default tables rather than failing the whole undo step.
        let rest: Vec<u8> = words.flat_map(|w| w.to_le_bytes()).collect();
        if let Ok(shared) = SharedPaletteTables::from_bytes(&rest) {
            palettes.shared = shared;
        }
        palettes
    }

    fn to_bytes(&self) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(EDITABLE_PALETTES_BYTES);
        for &v in self.bg.iter().chain(self.fg.iter()).chain(self.sprite.iter()) {
            bytes.extend_from_slice(&v.to_le_bytes());
        }
        bytes.extend_from_slice(&self.shared.to_bytes());
        bytes
    }

    fn size_bytes(&self) -> usize {
        EDITABLE_PALETTES_BYTES
    }
}

/// Parse an RGB hex color string into 8-bit sRGB components (Lunar Magic
/// v3.50 parity: "a way to enter RGB colors in hex format"). Accepts
/// `RRGGBB` with an optional leading `#`, case-insensitive. Anything else
/// (wrong length, non-hex digits) is rejected.
pub(super) fn parse_hex_rgb(s: &str) -> Option<(u8, u8, u8)> {
    let s = s.strip_prefix('#').unwrap_or(s);
    if s.len() != 6 || !s.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let v = u32::from_str_radix(s, 16).ok()?;
    Some(((v >> 16) as u8, ((v >> 8) & 0xFF) as u8, (v & 0xFF) as u8))
}

/// Format an ABGR1555 SNES color as a 6-digit RGB hex string for the hex
/// entry field.
pub(super) fn snes_to_hex_rgb(raw: u16) -> String {
    let r = ((raw & 0x1F) * 255 / 31) as u8;
    let g = (((raw >> 5) & 0x1F) * 255 / 31) as u8;
    let b = (((raw >> 10) & 0x1F) * 255 / 31) as u8;
    format!("{r:02X}{g:02X}{b:02X}")
}

impl UiLevelEditor {
    pub(super) fn palette_editor_window(&mut self, ctx: &Context) {
        if !self.show_palette_editor {
            return;
        }
        let mut open = self.show_palette_editor;
        let win = egui::Window::new("Palette Editor").open(&mut open).resizable(false).show(ctx, |ui| {
            ui.label("Click a color swatch to edit it. Changes save with Ctrl+S.");

            // ── Undo/redo buttons (Lunar Magic v1.80 has these in the ──────
            // palette editors).
            ui.horizontal(|ui| {
                let can_undo = self.palettes.can_undo();
                if ui
                    .add_enabled(can_undo, egui::Button::new(format!("{} Undo", icon::ARROW_COUNTER_CLOCKWISE)))
                    .on_hover_text("Undo palette change (Ctrl+Z)")
                    .clicked()
                {
                    self.palette_undo();
                }
                let can_redo = self.palettes.can_redo();
                if ui
                    .add_enabled(can_redo, egui::Button::new(format!("{} Redo", icon::ARROW_CLOCKWISE)))
                    .on_hover_text("Redo palette change (Ctrl+Y)")
                    .clicked()
                {
                    self.palette_redo();
                }
            });
            ui.separator();

            // ── Custom palette (Lunar Magic v3.30) ───────────────────────
            // A level with a custom palette edits its own private BG/FG/
            // sprite palettes instead of the game's shared tables, so an
            // edit here stops recoloring every other level that shares the
            // table entry (LM: Level > Enable Custom Palette).
            ui.horizontal(|ui| {
                let mut enabled = self.custom_palette_enabled;
                if ui
                    .checkbox(&mut enabled, "Enable custom palette")
                    .on_hover_text(
                        "This level edits its own private palette instead of the shared palette \
                         tables (Lunar Magic: Level > Enable Custom Palette)",
                    )
                    .changed()
                {
                    self.set_custom_palette_enabled(enabled);
                }
                if self.custom_palette_enabled {
                    ui.label("edits stay private to this level");
                } else {
                    let p = &self.level_properties;
                    ui.label(format!(
                        "editing shared tables (BG {:X}, FG {:X}, sprite {:X})",
                        p.palette_bg, p.palette_fg, p.palette_sprite
                    ));
                }
            });
            ui.checkbox(&mut self.auto_enable_custom_palette, "Auto-enable custom palette on edit").on_hover_text(
                "Lunar Magic v3.30: automatically enable the custom palette the first time \
                     a palette edit is made, instead of editing the shared tables",
            );
            ui.separator();

            self.palette_group(ui, "BG Palette", 0);
            ui.separator();
            self.palette_group(ui, "FG Palette", 1);
            ui.separator();
            self.palette_group(ui, "Sprite Palette", 2);
            ui.separator();
            self.palette_files_section(ui);
        });
        self.show_palette_editor = open;

        // ── Ctrl+Z / Ctrl+Y while the pointer is over this window ───────────
        // (or while a color drag is in flight, since the picker popup floats
        // outside the window rect). The window is drawn before the central
        // panel, so consuming here wins over the level-canvas undo.
        let palette_active = win.is_some_and(|r| r.response.hovered()) || self.palette_gesture_before.is_some();
        if palette_active {
            if ctx
                .input_mut(|i| i.consume_shortcut(&egui::KeyboardShortcut::new(egui::Modifiers::COMMAND, egui::Key::Z)))
            {
                self.palette_undo();
            }
            if ctx
                .input_mut(|i| i.consume_shortcut(&egui::KeyboardShortcut::new(egui::Modifiers::COMMAND, egui::Key::Y)))
            {
                self.palette_redo();
            }
        }

        // ── Commit the undo step when a color-picker drag gesture ends ──────
        // (snapshot taken on the first change; the picker fires changed() on
        // every drag frame, so committing per-frame would make undo walk back
        // through every intermediate color).
        if self.palette_gesture_before.is_some() && ctx.input(|i| i.pointer.any_released()) {
            self.commit_palette_gesture();
        }
    }

    /// Undo one palette edit (Lunar Magic v1.80 palette-editor undo).
    /// An in-flight color drag is committed first, so Ctrl+Z mid-drag undoes
    /// the drag rather than the edit before it.
    fn palette_undo(&mut self) {
        self.commit_palette_gesture();
        if self.palettes.can_undo() {
            self.palettes.undo();
            self.palette_dirty = true;
            self.mark_edited();
            self.sync_custom_palette_entry();
        }
    }

    /// Redo one palette edit (Lunar Magic v1.80 palette-editor redo).
    fn palette_redo(&mut self) {
        self.commit_palette_gesture();
        if self.palettes.can_redo() {
            self.palettes.redo();
            self.palette_dirty = true;
            self.mark_edited();
            self.sync_custom_palette_entry();
        }
    }

    /// Enable/disable the per-level custom palette (Lunar Magic v3.30
    /// "Enable Custom Palette"). Enabling keeps the colors currently on
    /// screen — they are written to the level's private palette entry on
    /// save. Disabling drops the private entry; the on-screen colors are
    /// then saved to the shared tables, the vanilla behavior.
    fn set_custom_palette_enabled(&mut self, enabled: bool) {
        if self.custom_palette_enabled == enabled {
            return;
        }
        self.custom_palette_enabled = enabled;
        if enabled {
            // Seed the private entry from the colors on screen (a copy of
            // the shared tables, unless already edited this session).
            self.sync_custom_palette_entry();
        } else {
            self.custom_palettes.clear(self.level_num);
            // Adopt-on-disable (existing behavior): the private on-screen
            // colors move into the shared-table rows at this level's indices.
            // Direct mutation, no undo step — like the toggle itself.
            let (bg_idx, fg_idx, sp_idx) =
                (self.palette_row_index(0), self.palette_row_index(1), self.palette_row_index(2));
            let pal = self.palettes.data_mut();
            let (bg, fg, sprite) = (pal.bg, pal.fg, pal.sprite);
            pal.shared.bg[bg_idx] = bg;
            pal.shared.fg[fg_idx] = fg;
            pal.shared.sprite[sp_idx] = sprite;
            self.shared_rows_dirty[bg_idx] = true;
            self.shared_rows_dirty[8 + fg_idx] = true;
            self.shared_rows_dirty[16 + sp_idx] = true;
        }
        self.custom_palette_dirty = true;
        self.mark_edited();
    }

    /// Lunar Magic v3.30 "Auto-Enable custom palette on edit": the first
    /// palette edit of the session moves the level onto its own private
    /// palette instead of editing the shared tables.
    fn auto_enable_custom_palette_on_edit(&mut self) {
        if self.auto_enable_custom_palette && !self.custom_palette_enabled {
            self.set_custom_palette_enabled(true);
        }
    }

    /// Copy the live palette state into this level's custom-palette entry.
    /// Called after every palette commit while custom mode is on (hex
    /// apply, color drag frames, undo, redo), because `save_to_rom` is
    /// `&self` and can only merge the working copy — never `self.palettes`
    /// directly.
    fn sync_custom_palette_entry(&mut self) {
        if !self.custom_palette_enabled {
            return;
        }
        let (bg, fg, sprite) = self.palettes.read(|pal| (pal.bg, pal.fg, pal.sprite));
        self.custom_palettes.set(self.level_num, smwe_rom::level::custom_palette::CustomPalette { bg, fg, sprite });
    }

    /// Apply the hex RGB entry field to the selected color (Lunar Magic v3.50
    /// parity). A successful apply is one undo step (`write()`), matching
    /// the discrete-edit semantics of the rest of the window; an in-flight
    /// color-picker drag is committed first so the two edits stay separate.
    /// Returns true when a color was applied.
    fn apply_palette_hex(&mut self, group: usize, col: usize) -> bool {
        let Some((r, g, b)) = parse_hex_rgb(&self.palette_hex_input) else {
            self.palette_hex_invalid = true;
            return false;
        };
        self.palette_hex_invalid = false;
        self.commit_palette_gesture();
        // Same 8-bit → 5-bit conversion as the color picker above.
        let r5 = (r as u16 * 31 / 255) & 0x1F;
        let g5 = (g as u16 * 31 / 255) & 0x1F;
        let b5 = (b as u16 * 31 / 255) & 0x1F;
        let new_raw = r5 | (g5 << 5) | (b5 << 10);
        let custom = self.custom_palette_enabled;
        let row_idx = self.palette_row_index(group);
        self.palettes.write(|pal| {
            pal.group_mut(group)[col] = new_raw;
            if !custom {
                // Non-custom mode edits the shared tables: keep the shared
                // row (and the save-path dirty flag) in sync, in the same
                // undo step.
                pal.shared_group_mut(group)[row_idx][col] = new_raw;
            }
        });
        if !custom {
            self.shared_rows_dirty[group * 8 + row_idx] = true;
        }
        self.palette_dirty = true;
        self.mark_edited();
        self.sync_custom_palette_entry();
        self.auto_enable_custom_palette_on_edit();
        true
    }

    /// Commit an in-flight color-drag gesture as a single undo step, if any.
    fn commit_palette_gesture(&mut self) {
        if let Some(before) = self.palette_gesture_before.take() {
            self.palettes.commit_change(&before);
        }
    }

    /// This level's shared-table row index for a palette group (0=BG, 1=FG,
    /// 2=sprite): the level header's palette indices into the shared tables.
    fn palette_row_index(&self, group: usize) -> usize {
        match group {
            0 => self.level_properties.palette_bg as usize,
            1 => self.level_properties.palette_fg as usize,
            _ => self.level_properties.palette_sprite as usize,
        }
    }

    /// Palette file interchange (Lunar Magic's Palette Editor file buttons):
    /// shared-palette extract/insert plus `.mw3` custom-palette
    /// export/import.
    fn palette_files_section(&mut self, ui: &mut Ui) {
        ui.label("Palette files");
        ui.horizontal(|ui| {
            if ui
                .button("Extract Shared Palette…")
                .on_hover_text(
                    "Save the shared palette tables (BG/FG/sprite groups, 8 rows × 12 colors each) \
                     to a .spal file — byte-identical to the ROM",
                )
                .clicked()
            {
                self.extract_shared_palette();
            }
            if ui
                .button("Insert Shared Palette…")
                .on_hover_text(
                    "Load a .spal file into the shared palette tables as one undo step (affects \
                     every level that uses the shared tables)",
                )
                .clicked()
            {
                self.insert_shared_palette();
            }
        });
        ui.horizontal(|ui| {
            if ui
                .button("Export Custom Palette (.mw3)…")
                .on_hover_text("Save this level's 36 palette colors to a Lunar Magic .mw3 file (514 bytes)")
                .clicked()
            {
                self.export_mw3();
            }
            if ui
                .button("Import Custom Palette (.mw3)…")
                .on_hover_text(
                    "Load a Lunar Magic .mw3 file into this level's palette as one undo step \
                     (auto-enables the custom palette, like LM)",
                )
                .clicked()
            {
                self.import_mw3();
            }
        });
        if let Some(status) = self.palette_file_status.clone() {
            ui.label(egui::RichText::new(status).small().italics());
        }
    }

    /// Lunar Magic "Extract Shared Palette to File": save the shared palette
    /// tables (the editor's working state, so unsaved edits are included) to
    /// a `.spal` file — 576 bytes, byte-identical to the ROM ranges.
    fn extract_shared_palette(&mut self) {
        let Some(path) =
            rfd::FileDialog::new().add_filter("Shared palette", &["spal"]).set_file_name("shared.spal").save_file()
        else {
            return;
        };
        let bytes = self.palettes.read(|pal| pal.shared.to_bytes());
        self.palette_file_status = Some(match std::fs::write(&path, bytes) {
            Ok(()) => {
                let msg = format!("Extracted shared palette ({} bytes) → {}", SHARED_PALETTE_BYTES, path.display());
                log::info!("{msg}");
                msg
            }
            Err(e) => format!("Shared-palette extract failed: {e:#}"),
        });
    }

    /// Lunar Magic "Insert Shared Palette from File": load a `.spal` file
    /// into the shared palette tables as one undo step. The file must be
    /// exactly 576 bytes — anything else is rejected without touching the
    /// palette, so a truncated or foreign file can never half-apply. In
    /// custom-palette mode the on-screen colors are the level's private
    /// palette, so they are left alone while the shared tables update
    /// underneath.
    fn insert_shared_palette(&mut self) {
        let Some(path) = rfd::FileDialog::new().add_filter("Shared palette", &["spal", "bin"]).pick_file() else {
            return;
        };
        let result = (|| -> anyhow::Result<String> {
            let bytes = std::fs::read(&path).with_context(|| format!("Failed to read {}", path.display()))?;
            let tables = SharedPaletteTables::from_bytes(&bytes).map_err(|e| anyhow::anyhow!("{e}"))?;
            let custom = self.custom_palette_enabled;
            let (bg_idx, fg_idx, sp_idx) =
                (self.palette_row_index(0), self.palette_row_index(1), self.palette_row_index(2));
            self.palettes.write(|pal| {
                pal.shared = tables;
                if !custom {
                    // Keep the on-screen colors (and the shared invariant) in sync.
                    pal.bg = pal.shared.bg[bg_idx];
                    pal.fg = pal.shared.fg[fg_idx];
                    pal.sprite = pal.shared.sprite[sp_idx];
                }
            });
            self.shared_rows_dirty = [true; 24];
            self.palette_dirty = true;
            self.mark_edited();
            Ok(format!("Inserted shared palette ← {} (undo with Ctrl+Z)", path.display()))
        })();
        self.palette_file_status = Some(match result {
            Ok(msg) => {
                log::info!("{msg}");
                msg
            }
            Err(e) => format!("Shared-palette insert failed: {e:#}"),
        });
    }

    /// Export this level's 36 palette colors to a Lunar Magic `.mw3`
    /// custom-palette file (514 bytes; Lunar Magic v1.40 File-menu parity —
    /// here in the palette window next to the other file buttons).
    fn export_mw3(&mut self) {
        let Some(path) = rfd::FileDialog::new()
            .add_filter("Lunar Magic custom palette", &["mw3"])
            .set_file_name(format!("level_{:03X}.mw3", self.level_num))
            .save_file()
        else {
            return;
        };
        let bytes = self.palettes.read(|pal| {
            crate::palette_files::write_mw3(&LevelPalette36 { bg: pal.bg, fg: pal.fg, sprite: pal.sprite })
        });
        self.palette_file_status = Some(match std::fs::write(&path, bytes) {
            Ok(()) => {
                let msg = format!("Exported custom palette (514 bytes) → {}", path.display());
                log::info!("{msg}");
                msg
            }
            Err(e) => format!("Custom-palette export failed: {e:#}"),
        });
    }

    /// Import a Lunar Magic `.mw3` file into this level's palette as one
    /// undo step. A `.mw3` is a *custom* palette file, so the level's custom
    /// palette is auto-enabled first (Lunar Magic v3.30 auto-enable
    /// semantics) and the import lands in the private palette, never the
    /// shared tables. The file must be exactly 514 bytes or it is rejected
    /// without touching the palette.
    fn import_mw3(&mut self) {
        let Some(path) = rfd::FileDialog::new().add_filter("Lunar Magic custom palette", &["mw3"]).pick_file() else {
            return;
        };
        let result = (|| -> anyhow::Result<String> {
            let bytes = std::fs::read(&path).with_context(|| format!("Failed to read {}", path.display()))?;
            let pal = crate::palette_files::read_mw3(&bytes).map_err(|e| anyhow::anyhow!("{e}"))?;
            if !self.custom_palette_enabled {
                self.set_custom_palette_enabled(true);
            }
            let (bg, fg, sprite) = (pal.bg, pal.fg, pal.sprite);
            self.palettes.write(|p| {
                p.bg = bg;
                p.fg = fg;
                p.sprite = sprite;
            });
            self.palette_dirty = true;
            self.mark_edited();
            self.sync_custom_palette_entry();
            Ok(format!("Imported custom palette ← {} (undo with Ctrl+Z)", path.display()))
        })();
        self.palette_file_status = Some(match result {
            Ok(msg) => {
                log::info!("{msg}");
                msg
            }
            Err(e) => format!("Custom-palette import failed: {e:#}"),
        });
    }

    fn palette_group(&mut self, ui: &mut Ui, label: &str, group: usize) {
        let p = &self.level_properties;
        let index = match group {
            0 => p.palette_bg,
            1 => p.palette_fg,
            _ => p.palette_sprite,
        };
        // In custom-palette mode the shared-table index no longer applies:
        // the level edits its own private palette.
        let source = if self.custom_palette_enabled { "custom".to_string() } else { format!("index {index:X}") };
        ui.label(format!("{label} ({source})"));

        let colors: [u16; 12] = self.palettes.read(|pal| *pal.group(group));

        // Draw grid of colored cells
        let total_w = CELL_SIZE * COLS as f32;
        let (grid_rect, _) = ui.allocate_exact_size(vec2(total_w, CELL_SIZE), Sense::hover());

        let mut changed = false;
        for col in 0..COLS {
            let raw = colors[col];
            let cell_min = grid_rect.min + vec2(col as f32 * CELL_SIZE, 0.0);
            let cell_rect = Rect::from_min_size(cell_min, Vec2::splat(CELL_SIZE));

            let r = ((raw & 0x1F) as f32 / 31.0 * 255.0) as u8;
            let g = (((raw >> 5) & 0x1F) as f32 / 31.0 * 255.0) as u8;
            let b = (((raw >> 10) & 0x1F) as f32 / 31.0 * 255.0) as u8;
            let c32 = Color32::from_rgb(r, g, b);

            // Fill the cell
            ui.painter().rect_filled(cell_rect, egui::CornerRadius::ZERO, c32);
            // Thin border
            ui.painter().rect_stroke(
                cell_rect,
                egui::CornerRadius::ZERO,
                egui::Stroke::new(1.0_f32, Color32::from_gray(80)),
                egui::StrokeKind::Outside,
            );

            // Highlight selected cell
            let selected = self.selected_palette_group == group as u8
                && self.selected_palette_idx == col
                && self.selected_palette_group < 3;
            if selected {
                ui.painter().rect_stroke(
                    cell_rect,
                    egui::CornerRadius::ZERO,
                    egui::Stroke::new(2.0_f32, Color32::WHITE),
                    egui::StrokeKind::Outside,
                );
            }

            // Detect click
            let resp = ui.interact(cell_rect, egui::Id::new(("pal_cell", group, col, index)), Sense::click());
            if resp.clicked() {
                self.selected_palette_group = group as u8;
                self.selected_palette_idx = col;
            }
        }

        // If this group has a selected cell, show a color picker below
        if self.selected_palette_group == group as u8 && self.selected_palette_idx < COLS {
            let col = self.selected_palette_idx;
            let raw = colors[col];
            let mut c32 = Color32::from_rgb(
                ((raw & 0x1F) as f32 / 31.0 * 255.0) as u8,
                (((raw >> 5) & 0x1F) as f32 / 31.0 * 255.0) as u8,
                (((raw >> 10) & 0x1F) as f32 / 31.0 * 255.0) as u8,
            );
            ui.horizontal(|ui| {
                ui.label(format!("Color {}:", col));
                if ui.color_edit_button_srgba(&mut c32).changed() {
                    // Convert sRGBA back to ABGR1555
                    let r5 = (c32.r() as u16 * 31 / 255) & 0x1F;
                    let g5 = (c32.g() as u16 * 31 / 255) & 0x1F;
                    let b5 = (c32.b() as u16 * 31 / 255) & 0x1F;
                    let new_raw = r5 | (g5 << 5) | (b5 << 10);
                    // Gesture-style edit: snapshot once on the first change,
                    // mutate directly; a single undo step is committed when
                    // the drag ends (see palette_editor_window), so undo
                    // restores the pre-drag color instead of walking back
                    // through every intermediate drag frame.
                    if self.palette_gesture_before.is_none() {
                        self.palette_gesture_before = Some(self.palettes.read(|pal| pal.clone()));
                    }
                    let custom = self.custom_palette_enabled;
                    let row_idx = self.palette_row_index(group);
                    {
                        let pal = self.palettes.data_mut();
                        pal.group_mut(group)[col] = new_raw;
                        if !custom {
                            // Non-custom mode edits the shared tables: keep
                            // the shared row (and the save-path dirty flag)
                            // in sync. The gesture commits as one undo step.
                            pal.shared_group_mut(group)[row_idx][col] = new_raw;
                        }
                    }
                    if !custom {
                        self.shared_rows_dirty[group * 8 + row_idx] = true;
                    }
                    changed = true;
                }
                let raw2 = self.palettes.read(|pal| pal.group(group)[col]);
                ui.monospace(format!("{:04X}", raw2));

                // ── RGB hex entry (Lunar Magic v3.50 parity) ──────────────
                ui.label("RGB #");
                let hex_resp = ui.add(
                    egui::TextEdit::singleline(&mut self.palette_hex_input).hint_text("RRGGBB").desired_width(64.0),
                );
                let apply_clicked =
                    ui.small_button("Apply").on_hover_text("Apply the hex RGB value to this color (Enter)").clicked();
                let enter_pressed = hex_resp.has_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                if apply_clicked || enter_pressed {
                    if self.apply_palette_hex(group, col) {
                        changed = true;
                    }
                }
                if self.palette_hex_invalid {
                    ui.colored_label(Color32::from_rgb(255, 120, 120), "invalid hex");
                }
                // Keep the field showing the selected color's value while the
                // user isn't typing in it (e.g. after a color-picker drag).
                if !hex_resp.has_focus() {
                    self.palette_hex_input = snes_to_hex_rgb(raw2);
                    self.palette_hex_invalid = false;
                }
            });
        }

        if changed {
            self.palette_dirty = true;
            self.mark_edited();
            self.sync_custom_palette_entry();
            self.auto_enable_custom_palette_on_edit();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::undo::UndoableData;

    #[test]
    fn palette_undo_redo_round_trip() {
        let mut palettes = UndoableData::new(EditablePalettes::default());
        // One write() = one undo step, like a single committed edit.
        palettes.write(|p| p.bg[3] = 0x7FFF);
        palettes.write(|p| p.sprite[0] = 0x001F);
        assert!(palettes.can_undo());
        assert!(!palettes.can_redo());

        palettes.undo();
        assert_eq!(palettes.read(|p| p.sprite[0]), 0);
        assert_eq!(palettes.read(|p| p.bg[3]), 0x7FFF);
        assert!(palettes.can_redo());

        palettes.undo();
        assert_eq!(palettes.read(|p| p.bg[3]), 0);
        assert!(!palettes.can_undo());

        palettes.redo();
        palettes.redo();
        assert_eq!(palettes.read(|p| p.bg[3]), 0x7FFF);
        assert_eq!(palettes.read(|p| p.sprite[0]), 0x001F);
        assert!(!palettes.can_redo());
    }

    #[test]
    fn palette_gesture_commit_is_single_step() {
        let mut palettes = UndoableData::new(EditablePalettes::default());
        // Gesture-style edit: snapshot, mutate directly across several
        // "frames", commit once — the color-picker drag path.
        let before = palettes.read(|p| p.clone());
        for v in [0x1111u16, 0x2222, 0x3333] {
            palettes.data_mut().fg[7] = v;
        }
        palettes.commit_change(&before);
        assert!(palettes.can_undo());
        palettes.undo();
        assert_eq!(palettes.read(|p| p.fg[7]), 0);
        // A second undo must not exist: the whole drag was one step.
        assert!(!palettes.can_undo());
    }

    #[test]
    fn palette_serialization_is_stable() {
        let mut p = EditablePalettes::default();
        p.bg[0] = 0x1234;
        p.sprite[11] = 0x7FFF;
        p.shared.bg[3][5] = 0x03E0;
        p.shared.sprite[7][11] = 0x001F;
        let bytes = p.to_bytes();
        assert_eq!(bytes.len(), EDITABLE_PALETTES_BYTES);
        let back = EditablePalettes::from_bytes(bytes);
        assert_eq!(back.to_bytes(), p.to_bytes());
        assert_eq!(back.shared.bg[3][5], 0x03E0);
        assert_eq!(back.shared.sprite[7][11], 0x001F);
    }

    #[test]
    fn palette_from_bytes_keeps_shared_tables_after_undo_round_trip() {
        // An insert-shared-palette undo step must restore both the visible
        // colors and the shared tables.
        let mut palettes = UndoableData::new(EditablePalettes::default());
        let mut tables = SharedPaletteTables::default();
        tables.bg[0] = [0x001F; 12];
        tables.fg[1] = [0x03E0; 12];
        palettes.write(|p| {
            p.shared = tables.clone();
            p.bg = tables.bg[0];
        });
        palettes.undo();
        let (bg0, shared_bg0) = palettes.read(|p| (p.bg[0], p.shared.bg[0][0]));
        assert_eq!(bg0, 0);
        assert_eq!(shared_bg0, 0);
        palettes.redo();
        let (bg0, shared_bg0) = palettes.read(|p| (p.bg[0], p.shared.bg[0][0]));
        assert_eq!(bg0, 0x001F);
        assert_eq!(shared_bg0, 0x001F);
    }

    #[test]
    fn palette_from_bytes_tolerates_truncated_tail() {
        // Defensive: a 72-byte undo record (pre-shared-table era) still
        // restores the colors; the shared tables stay defaulted.
        let mut p = EditablePalettes::default();
        p.bg[0] = 0x1234;
        let mut bytes = p.to_bytes();
        bytes.truncate(72);
        let back = EditablePalettes::from_bytes(bytes);
        assert_eq!(back.bg[0], 0x1234);
        assert_eq!(back.shared.bg[0][0], 0);
    }

    #[test]
    fn hex_rgb_parses_six_digits_with_optional_hash() {
        // Lunar Magic v3.50: "a way to enter RGB colors in hex format".
        assert_eq!(parse_hex_rgb("FF0000"), Some((255, 0, 0)));
        assert_eq!(parse_hex_rgb("00ff00"), Some((0, 255, 0)));
        assert_eq!(parse_hex_rgb("#0000FF"), Some((0, 0, 255)));
        assert_eq!(parse_hex_rgb("#aBcDeF"), Some((0xAB, 0xCD, 0xEF)));
    }

    #[test]
    fn hex_rgb_rejects_anything_else() {
        for bad in ["", "#", "FFF", "#FFF", "FFFFFFF", "GGGGGG", "FF00 0", "0xFF0000", "#FF000G", "red"] {
            assert_eq!(parse_hex_rgb(bad), None, "must reject {bad:?}");
        }
    }

    #[test]
    fn snes_to_hex_rgb_formats_known_colors() {
        assert_eq!(snes_to_hex_rgb(0x0000), "000000");
        assert_eq!(snes_to_hex_rgb(0x7FFF), "FFFFFF");
        // Full red / green / blue 5-bit primaries.
        assert_eq!(snes_to_hex_rgb(0x001F), "FF0000");
        assert_eq!(snes_to_hex_rgb(0x03E0), "00FF00");
        assert_eq!(snes_to_hex_rgb(0x7C00), "0000FF");
    }

    #[test]
    fn hex_entry_apply_is_one_undo_step() {
        // Exercise the apply path's data flow without the egui shell: the
        // same `write()` + 8-to-5-bit conversion the UI calls.
        let mut palettes = UndoableData::new(EditablePalettes::default());
        let (r, g, b) = parse_hex_rgb("#FF0000").unwrap();
        let new_raw = ((r as u16 * 31 / 255) & 0x1F)
            | (((g as u16 * 31 / 255) & 0x1F) << 5)
            | (((b as u16 * 31 / 255) & 0x1F) << 10);
        palettes.write(|p| p.bg[3] = new_raw);
        assert_eq!(palettes.read(|p| p.bg[3]), 0x001F);
        palettes.undo();
        assert_eq!(palettes.read(|p| p.bg[3]), 0);
        assert!(!palettes.can_undo(), "hex apply must be a single undo step");
    }
}
