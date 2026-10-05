use anyhow::Context as _;
use egui::{vec2, Color32, Context, Rect, Sense, Ui, Vec2};
use egui_phosphor::regular as icon;

use super::UiLevelEditor;
use crate::{
    palette_files::{LevelPalette36, SharedPaletteTables, SHARED_PALETTE_BYTES},
    snes9x_state::{self, Snes9xCgram},
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

/// Pending Snes9x-savestate palette import dialog (Lunar Magic v3.40
/// parity: "added support for importing palettes from Snes9x save state
/// files"). The parsed CGRAM is shown as a 16×16 preview and the user
/// picks the destination before anything is applied, so the savestate
/// parse can never half-apply.
#[derive(Clone)]
pub(super) struct Snes9xImportDialog {
    /// The savestate's live 256-color CGRAM plus its snapshot version.
    pub cgram:                Snes9xCgram,
    /// Display name of the source file.
    pub file_name:            String,
    /// Destination: full shared palette tables, or just this level's
    /// BG/FG/sprite rows.
    pub import_shared_tables: bool,
}

/// Convert a SNES 15-bit BGR color word to an egui color (same conversion
/// the swatch grid uses).
fn snes_to_egui(raw: u16) -> Color32 {
    let r = ((raw & 0x1F) as f32 / 31.0 * 255.0) as u8;
    let g = (((raw >> 5) & 0x1F) as f32 / 31.0 * 255.0) as u8;
    let b = (((raw >> 10) & 0x1F) as f32 / 31.0 * 255.0) as u8;
    Color32::from_rgb(r, g, b)
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

/// Build a linear gradient between two ABGR1555 SNES colors, endpoints
/// included, with `between` intermediate colors (Lunar Magic palette-editor
/// gradients: v1.50 introduced them on Ctrl+Right-Click, v1.63 moved them to
/// Alt+Right-Click, v3.40 added the vertical variant on Alt+Shift+Right-Click).
///
/// Interpolation is per-channel in 5-bit space with round-to-nearest, so a
/// black→white midpoint lands on 0x4210 (the classic SNES gray) — the LM
/// v3.50 "gradient colors lightened" change, which replaced the older
/// floor-toward-dark behavior. LM's exact intermediate-cell semantics beyond
/// the documented gestures are not in the public docs, so this implements
/// the documented purpose: fill the cells between the selected color and the
/// clicked color.
pub(super) fn gradient_fill(start: u16, end: u16, between: usize) -> Vec<u16> {
    let (sr, sg, sb) = (start & 0x1F, (start >> 5) & 0x1F, (start >> 10) & 0x1F);
    let (er, eg, eb) = (end & 0x1F, (end >> 5) & 0x1F, (end >> 10) & 0x1F);
    (0..=(between + 1))
        .map(|i| {
            let t = i as f32 / (between + 1) as f32;
            let ch = |s: u16, e: u16| ((s as f32 + (e as f32 - s as f32) * t).round() as u16).min(0x1F);
            ch(sr, er) | (ch(sg, eg) << 5) | (ch(sb, eb) << 10)
        })
        .collect()
}

/// Find where the game's level-load palette upload placed one 12-color
/// palette-editor row in CGRAM: scan the 512-byte CGRAM for the exact
/// 12-word (24-byte) sequence and return its CGRAM *word* address.
///
/// The editor never guesses the game's dynamic palette-table layout —
/// this finds the row empirically, right after the emulator has run the
/// real upload code. Returns `None` when the row isn't in CGRAM (custom
/// palettes the game never uploaded, animated regions already rewritten
/// by a tick); the LM v3.33 destination features stay inert for it.
/// First match wins if a row's colors appear more than once.
pub(super) fn find_palette_cgram_base(cgram: &[u8], colors: &[u16; 12]) -> Option<u16> {
    if cgram.len() < 512 {
        return None;
    }
    let seq: Vec<u8> = colors.iter().flat_map(|c| c.to_le_bytes()).collect();
    (0..=(512 - 24)).step_by(2).find_map(|off| (cgram[off..off + 24] == seq[..]).then_some((off / 2) as u16))
}

impl UiLevelEditor {
    pub(super) fn palette_editor_window(&mut self, ctx: &Context) {
        if !self.show_palette_editor {
            return;
        }
        let mut open = self.show_palette_editor;
        let win = egui::Window::new("Palette Editor").open(&mut open).resizable(false).show(ctx, |ui| {
            ui.label("Click a color swatch to edit it. Changes save with Ctrl+S.");
            ui.small(
                "Alt+Right-Click a swatch: gradient from the selected color · \
                 Alt+Shift+Right-Click: vertical gradient (Lunar Magic v1.63 / v3.40)",
            );

            // ── Lunar Magic v3.33 ExAnimation link ────────────────────────
            // While the ExAnimated dialog has Palette Select armed, a
            // Ctrl+Left-Click here fills the armed color field instead of
            // selecting the swatch.
            if self.show_exanimation_editor
                && self.exanim_dialog.palette_select_armed(&self.exanimation, Some(self.level_num))
            {
                ui.colored_label(
                    egui::Color32::YELLOW,
                    "Palette Select armed — Ctrl+Left-Click a color to fill the ExAnimated field \
                     (Ctrl+Shift+Click a yellow-marked destination to select its slot).",
                );
            }
            if let Some(s) = self.palette_link_status.clone() {
                ui.small(egui::RichText::new(s).italics());
            }

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

        // ── Snes9x savestate import dialog (Lunar Magic v3.40) ─────────────
        // Drawn outside the palette window so it stays up after the file
        // picker closes; `take()`/restore inside handles its lifetime.
        if self.snes9x_import.is_some() {
            self.snes9x_import_dialog(ctx);
        }

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

    /// Lunar Magic palette-editor gradients (v1.50, gesture moved to
    /// Alt+Right-Click in v1.63; Alt+Shift+Right-Click for the vertical
    /// variant in v3.40): fill the cells between the currently selected
    /// color and the clicked cell with a linear gradient between the two
    /// colors. Horizontal fills the row within the clicked group; vertical
    /// fills the column down the three palette rows (BG → FG → sprite).
    /// The whole fill is a single undo step, like every other palette edit.
    fn apply_palette_gradient(&mut self, group: usize, col: usize, vertical: bool) {
        let (sg, sc) = (self.selected_palette_group as usize, self.selected_palette_idx);
        if sg >= 3 {
            self.palette_link_status =
                Some("Gradient: left-click a color first to pick the gradient start.".to_string());
            return;
        }
        let (axis_ok, lo, hi) =
            if vertical { (sc == col, sg.min(group), sg.max(group)) } else { (sg == group, sc.min(col), sc.max(col)) };
        if !axis_ok {
            self.palette_link_status = Some(
                if vertical {
                    "Vertical gradient: Alt+Shift+Right-Click a cell in the same column as the selected color."
                } else {
                    "Gradient: Alt+Right-Click a cell in the same palette row as the selected color."
                }
                .to_string(),
            );
            return;
        }
        if hi - lo < 1 {
            self.palette_link_status =
                Some("Gradient: pick two different colors — Alt+Right-Click another cell.".to_string());
            return;
        }
        // Cell coordinates from the selected cell to the clicked cell, so the
        // gradient endpoints land on the right cells regardless of direction.
        let coords: Vec<(usize, usize)> = if vertical {
            if sg <= group {
                (sg..=group).map(|g| (g, col)).collect()
            } else {
                (group..=sg).rev().map(|g| (g, col)).collect()
            }
        } else if sc <= col {
            (sc..=col).map(|c| (group, c)).collect()
        } else {
            (col..=sc).rev().map(|c| (group, c)).collect()
        };
        let (start_raw, end_raw) = self.palettes.read(|pal| (pal.group(sg)[sc], pal.group(group)[col]));
        let fills = gradient_fill(start_raw, end_raw, coords.len() - 2);
        self.commit_palette_gesture();
        let custom = self.custom_palette_enabled;
        // Shared-table row index per palette group (non-custom mode writes
        // into the shared tables, same as every other palette edit).
        let row_idx = [self.palette_row_index(0), self.palette_row_index(1), self.palette_row_index(2)];
        self.palettes.write(|pal| {
            for ((g, c), v) in coords.iter().zip(fills.iter()) {
                pal.group_mut(*g)[*c] = *v;
                if !custom {
                    pal.shared_group_mut(*g)[row_idx[*g]][*c] = *v;
                }
            }
        });
        if !custom {
            for (g, _) in &coords {
                self.shared_rows_dirty[g * 8 + row_idx[*g]] = true;
            }
        }
        self.palette_dirty = true;
        self.mark_edited();
        self.sync_custom_palette_entry();
        self.auto_enable_custom_palette_on_edit();
        self.palette_link_status = Some(format!(
            "{} gradient applied across {} colors (one undo step).",
            if vertical { "Vertical" } else { "Horizontal" },
            coords.len()
        ));
    }

    /// Commit an in-flight color-drag gesture as a single undo step, if any.
    fn commit_palette_gesture(&mut self) {
        if let Some(before) = self.palette_gesture_before.take() {
            self.palettes.commit_change(&before);
        }
    }

    /// Re-scan CGRAM for the three palette-editor rows after a level load.
    /// The scan runs after the emulator has uploaded the level's palettes
    /// through the real game code, so the addresses are the game's actual
    /// layout, not a guess (Lunar Magic v3.33 destination features).
    pub(super) fn scan_palette_cgram_bases(&mut self) {
        for g in 0..3 {
            let colors: [u16; 12] = self.palettes.read(|pal| *pal.group(g));
            self.palette_cgram_base[g] = find_palette_cgram_base(&self.cpu.mem.cgram, &colors);
        }
    }

    /// Lunar Magic v3.33: the ExAnimated frame whose palette destination
    /// covers this palette-editor swatch, if any. The swatch's CGRAM word
    /// address comes from the level-load scan (`palette_cgram_base`); a
    /// frame owns the swatch when the address falls in its per-step write
    /// range `[dest, dest + units)`.
    fn exanim_dest_at(&self, group: usize, col: usize) -> Option<usize> {
        let base = self.palette_cgram_base[group]?;
        let addr = base + col as u16;
        self.exanim_dialog
            .palette_frame_dests(&self.exanimation, Some(self.level_num))
            .into_iter()
            .find(|&(_, dest, units)| addr >= dest && addr - dest < units as u16)
            .map(|(idx, _, _)| idx)
    }

    /// Lunar Magic v3.33: Ctrl+Shift+Left-Click in the palette editor on an
    /// ExAnimated color destination selects that frame's slot in the
    /// ExAnimated dialog, opening the dialog if needed.
    fn select_exanim_dest(&mut self, group: usize, col: usize) {
        if let Some(idx) = self.exanim_dest_at(group, col) {
            self.exanim_dialog.select_frame(idx);
            self.exanim_dialog.disarm_select();
            self.show_exanimation_editor = true;
            self.palette_link_status =
                Some(format!("Selected ExAnimated frame #{idx} — its destination covers this color."));
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
        ui.horizontal(|ui| {
            if ui
                .button("Import Palette from Snes9x Savestate…")
                .on_hover_text(
                    "Load the live on-screen palette (CGRAM) from a Snes9x savestate file \
                     (Lunar Magic v3.40) — shows a preview and asks where to put it",
                )
                .clicked()
            {
                self.import_snes9x_savestate();
            }
        });
        if let Some(status) = self.palette_file_status.clone() {
            ui.label(egui::RichText::new(status).small().italics());
        }
        ui.separator();
        self.palmask_section(ui);
    }

    /// Lunar Magic v2.40 `.palmask` mask buttons: a same-name `.palmask`
    /// next to a palette file selects which of the file's colors an import
    /// applies. The mask is the 257-word selector (byte `i` ↔ palette word
    /// `i`; zero keeps the destination, nonzero takes the source); the
    /// default is everything selected, matching Lunar Magic's transient
    /// selector reset state.
    fn palmask_section(&mut self, ui: &mut Ui) {
        ui.label("Palette mask (.palmask)");
        ui.horizontal(|ui| {
            ui.checkbox(&mut self.palmask_edit_mode, "Edit mask").on_hover_text(
                "Mask-editing mode (Lunar Magic v2.40): clicking a palette swatch toggles whether that \
                 color is included in the next masked import, instead of selecting the swatch for editing. \
                 Masked colors show a green marker; dimmed colors are excluded.",
            );
            if ui.small_button("Select all").on_hover_text("Select every color in the mask").clicked() {
                self.palmask.select_all();
                self.palmask_status = Some("Mask: all 257 colors selected.".to_string());
            }
            if ui.small_button("Select none").on_hover_text("Deselect every color in the mask").clicked() {
                self.palmask.select_none();
                self.palmask_status =
                    Some("Mask: no colors selected — a masked import would change nothing.".to_string());
            }
            if ui.small_button("Invert").on_hover_text("Invert the mask selection").clicked() {
                self.palmask.invert();
                self.palmask_status =
                    Some(format!("Mask inverted: {} of 257 colors selected.", self.palmask.selected_count()));
            }
        });
        ui.horizontal(|ui| {
            if ui
                .button("Save mask (.palmask)…")
                .on_hover_text("Save the current 257-word import mask to a .palmask file")
                .clicked()
            {
                self.save_palmask();
            }
            if ui
                .button("Load mask (.palmask)…")
                .on_hover_text("Load a .palmask file as the current import mask (must be exactly 257 bytes)")
                .clicked()
            {
                self.load_palmask();
            }
        });
        let count = self.palmask.selected_count();
        let hint = if self.palmask_edit_mode {
            " — mask-editing mode is ON: click swatches to toggle their mask bits"
        } else {
            ""
        };
        ui.label(
            egui::RichText::new(format!(
                "{count} of 257 colors selected{hint}. Importing a palette file auto-discovers a \
                 same-name .palmask beside it; exporting republishes this mask beside the export."
            ))
            .small()
            .italics(),
        );
        if let Some(status) = self.palmask_status.clone() {
            ui.label(egui::RichText::new(status).small().italics());
        }
    }

    /// Save the current `.palmask` selector to a file (the v2.40
    /// "buttons to work with them": explicit mask save).
    fn save_palmask(&mut self) {
        let Some(path) = rfd::FileDialog::new()
            .add_filter("Lunar Magic palette mask", &["palmask"])
            .set_file_name("palette.palmask")
            .save_file()
        else {
            return;
        };
        let bytes = self.palmask.to_bytes();
        self.palmask_status = Some(match std::fs::write(&path, bytes) {
            Ok(()) => {
                let msg = format!(
                    "Saved palette mask (257 bytes, {} selected) → {}",
                    self.palmask.selected_count(),
                    path.display()
                );
                log::info!("{msg}");
                msg
            }
            Err(e) => format!("Palette-mask save failed: {e:#}"),
        });
    }

    /// Load a `.palmask` file as the current import mask. The file must be
    /// exactly 257 bytes; anything else is rejected without touching the
    /// current mask.
    fn load_palmask(&mut self) {
        let Some(path) = rfd::FileDialog::new().add_filter("Lunar Magic palette mask", &["palmask"]).pick_file() else {
            return;
        };
        let result = (|| -> anyhow::Result<String> {
            let bytes = std::fs::read(&path).with_context(|| format!("Failed to read {}", path.display()))?;
            let mask = crate::palmask::Palmask::from_bytes(&bytes).map_err(|e| anyhow::anyhow!("{e}"))?;
            let count = mask.selected_count();
            self.palmask = mask;
            Ok(format!("Loaded palette mask ← {} ({count} of 257 colors selected)", path.display()))
        })();
        self.palmask_status = Some(match result {
            Ok(msg) => {
                log::info!("{msg}");
                msg
            }
            Err(e) => format!("Palette-mask load failed: {e:#}"),
        });
    }

    /// Lunar Magic v3.40: "support for importing palettes from Snes9x save
    /// state files". Pick the savestate, parse its `PPU` block's `CGDATA`
    /// (the emulator's live CGRAM), and stage the import dialog — nothing
    /// is applied until the user confirms a destination there.
    fn import_snes9x_savestate(&mut self) {
        let Some(path) = rfd::FileDialog::new()
            .add_filter("Snes9x savestate", &[
                "000", "001", "002", "003", "004", "005", "006", "007", "008", "009", "o00", "o01", "o02", "o03",
                "o04", "o05", "o06", "o07", "o08", "o09", "sst", "s9x",
            ])
            .pick_file()
        else {
            return;
        };
        let result = (|| -> anyhow::Result<Snes9xImportDialog> {
            let bytes = std::fs::read(&path).with_context(|| format!("Failed to read {}", path.display()))?;
            let cgram = snes9x_state::parse_cgram(&bytes).map_err(|e| anyhow::anyhow!("{e}"))?;
            Ok(Snes9xImportDialog {
                cgram,
                file_name: path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(),
                import_shared_tables: false,
            })
        })();
        match result {
            Ok(dlg) => {
                log::info!("parsed Snes9x savestate CGRAM ← {}", path.display());
                self.snes9x_import = Some(dlg);
                self.palette_file_status = None;
            }
            Err(e) => {
                self.palette_file_status = Some(format!("Snes9x savestate import failed: {e:#}"));
            }
        }
    }

    /// The Snes9x import dialog: 16×16 preview of the savestate's CGRAM plus
    /// the destination choice. Import is a single undo step either way.
    fn snes9x_import_dialog(&mut self, ctx: &Context) {
        let Some(mut dlg) = self.snes9x_import.take() else { return };
        let mut apply = false;
        let mut cancel = false;
        // No `.open()` binding, so egui draws no title-bar close button —
        // the dialog lives until Import or Cancel is clicked.
        egui::Window::new("Import Palette from Snes9x Savestate").collapsible(false).show(ctx, |ui| {
            ui.label(format!("{} — savestate v{}, 256-color live CGRAM", dlg.file_name, dlg.cgram.version));
            // Preview grid: 16×16 swatches, CGRAM rows top to bottom.
            let (grid_rect, _) = ui.allocate_exact_size(vec2(16.0 * 12.0, 16.0 * 12.0), Sense::hover());
            for (i, &raw) in dlg.cgram.colors.iter().enumerate() {
                let cell_min = grid_rect.min + vec2((i % 16) as f32 * 12.0, (i / 16) as f32 * 12.0);
                let cell_rect = Rect::from_min_size(cell_min, Vec2::splat(12.0));
                ui.painter().rect_filled(cell_rect, egui::CornerRadius::ZERO, snes_to_egui(raw));
            }
            ui.radio_value(
                &mut dlg.import_shared_tables,
                false,
                "This level's rows (BG/FG/sprite at this level's palette indices)",
            )
            .on_hover_text(
                "Import the savestate's BG/FG/sprite rows at this level's palette indices \
                 into this level's palette — the custom palette is auto-enabled, like the \
                 .mw3 import",
            );
            ui.radio_value(&mut dlg.import_shared_tables, true, "Full shared palette tables (BG/FG/sprite groups)")
                .on_hover_text(
                    "Replace all 24 shared palette rows from the savestate's CGRAM \
                     (BG/FG rows 0–7, sprite rows 8–15) — affects every level that uses \
                     the shared tables",
                );
            ui.horizontal(|ui| {
                if ui.button("Import").clicked() {
                    apply = true;
                }
                if ui.button("Cancel").clicked() {
                    cancel = true;
                }
            });
        });
        if apply {
            self.apply_snes9x_import(&dlg.cgram.colors, dlg.import_shared_tables);
            self.palette_file_status = Some(format!("Imported palette ← {} (undo with Ctrl+Z)", dlg.file_name));
        } else if !cancel {
            self.snes9x_import = Some(dlg);
        }
    }

    /// Apply a staged Snes9x savestate import as one undo step.
    fn apply_snes9x_import(&mut self, cgram: &[u16; snes9x_state::CGRAM_COLORS], import_shared_tables: bool) {
        let (bg_idx, fg_idx, sp_idx) =
            (self.palette_row_index(0), self.palette_row_index(1), self.palette_row_index(2));
        if import_shared_tables {
            // Full shared-table replacement, same shape as
            // `insert_shared_palette`: one undo step over `EditablePalettes`,
            // on-screen colors re-synced when the custom palette is off.
            let custom = self.custom_palette_enabled;
            self.palettes.write(|pal| {
                for r in 0..8 {
                    pal.shared.bg[r] = snes9x_state::cgram_row(cgram, r);
                    pal.shared.fg[r] = snes9x_state::cgram_row(cgram, r);
                    pal.shared.sprite[r] = snes9x_state::cgram_row(cgram, 8 + r);
                }
                if !custom {
                    pal.bg = pal.shared.bg[bg_idx];
                    pal.fg = pal.shared.fg[fg_idx];
                    pal.sprite = pal.shared.sprite[sp_idx];
                }
            });
            self.shared_rows_dirty = [true; 24];
        } else {
            // Level rows: the savestate's CGRAM rows at this level's palette
            // indices. Per-level data, so the custom palette is auto-enabled
            // first — same semantics as the `.mw3` import.
            if !self.custom_palette_enabled {
                self.set_custom_palette_enabled(true);
            }
            let (bg, fg, sprite) = (
                snes9x_state::cgram_row(cgram, bg_idx),
                snes9x_state::cgram_row(cgram, fg_idx),
                snes9x_state::cgram_row(cgram, 8 + sp_idx),
            );
            self.palettes.write(|pal| {
                pal.bg = bg;
                pal.fg = fg;
                pal.sprite = sprite;
            });
        }
        self.palette_dirty = true;
        self.mark_edited();
        self.sync_custom_palette_entry();
        log::info!("imported Snes9x savestate palette (shared tables: {import_shared_tables})");
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
    ///
    /// Lunar Magic v2.40 republishes the palette editor's selector beside
    /// the export, so this also writes a same-name `.palmask` carrying the
    /// current mask.
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
        let mask_path = path.with_extension("palmask");
        let mask_bytes = self.palmask.to_bytes();
        self.palette_file_status = Some(match (std::fs::write(&path, bytes), std::fs::write(&mask_path, mask_bytes)) {
            (Ok(()), Ok(())) => {
                let msg = format!(
                    "Exported custom palette (514 bytes) → {} + mask → {}",
                    path.display(),
                    mask_path.display()
                );
                log::info!("{msg}");
                msg
            }
            (Ok(()), Err(e)) => {
                format!("Exported custom palette → {} but the .palmask republish failed: {e:#}", path.display())
            }
            (Err(e), _) => format!("Custom-palette export failed: {e:#}"),
        });
    }

    /// Import a Lunar Magic `.mw3` file into this level's palette as one
    /// undo step. A `.mw3` is a *custom* palette file, so the level's custom
    /// palette is auto-enabled first (Lunar Magic v3.30 auto-enable
    /// semantics) and the import lands in the private palette, never the
    /// shared tables. The file must be exactly 514 bytes or it is rejected
    /// without touching the palette.
    ///
    /// Lunar Magic v2.40: a same-name `.palmask` next to the `.mw3` is
    /// discovered automatically and only the masked colors are imported
    /// (zero byte keeps the destination word, nonzero takes the source
    /// word); selected row-zero words are cleared to the backdrop word like
    /// the ordinary loader. A malformed mask fails the whole import before
    /// anything changes.
    fn import_mw3(&mut self) {
        let Some(path) = rfd::FileDialog::new().add_filter("Lunar Magic custom palette", &["mw3"]).pick_file() else {
            return;
        };
        let result = (|| -> anyhow::Result<String> {
            let bytes = std::fs::read(&path).with_context(|| format!("Failed to read {}", path.display()))?;
            // Lunar Magic v2.40 mask discovery: an optional same-name
            // `.palmask` beside the palette file. Parsed strictly before
            // anything is applied, so a malformed mask fails the import
            // without touching the palette.
            let mask_path = path.with_extension("palmask");
            let mask = if mask_path.exists() {
                let mask_bytes =
                    std::fs::read(&mask_path).with_context(|| format!("Failed to read {}", mask_path.display()))?;
                Some(
                    crate::palmask::Palmask::from_bytes(&mask_bytes)
                        .map_err(|e| anyhow::anyhow!("{}: {e}", mask_path.display()))?,
                )
            } else {
                None
            };
            if !self.custom_palette_enabled {
                self.set_custom_palette_enabled(true);
            }
            let msg = match mask {
                Some(mask) => {
                    let src = crate::palette_files::read_mw3_words(&bytes).map_err(|e| anyhow::anyhow!("{e}"))?;
                    // Compose the 257-word window from the level's 36
                    // colors (words 36..257 are zero — the editor doesn't
                    // model them — so the backdrop word 256 reads as 0),
                    // apply the recovered loader semantics, write back
                    // words 0..36.
                    let mut dest = [0u16; crate::palmask::PALMASK_WORDS];
                    let current = self.palettes.read(|pal| [pal.bg, pal.fg, pal.sprite]);
                    for (i, row) in current.iter().enumerate() {
                        dest[i * 12..(i + 1) * 12].copy_from_slice(row);
                    }
                    crate::palmask::apply_masked_import(&mut dest, &src, &mask);
                    let (mut bg, mut fg, mut sprite) = ([0u16; 12], [0u16; 12], [0u16; 12]);
                    bg.copy_from_slice(&dest[0..12]);
                    fg.copy_from_slice(&dest[12..24]);
                    sprite.copy_from_slice(&dest[24..36]);
                    self.palettes.write(|p| {
                        p.bg = bg;
                        p.fg = fg;
                        p.sprite = sprite;
                    });
                    format!(
                        "Imported custom palette ← {} with mask ← {} ({} of 257 colors selected) (undo with Ctrl+Z)",
                        path.display(),
                        mask_path.display(),
                        mask.selected_count()
                    )
                }
                None => {
                    let pal = crate::palette_files::read_mw3(&bytes).map_err(|e| anyhow::anyhow!("{e}"))?;
                    let (bg, fg, sprite) = (pal.bg, pal.fg, pal.sprite);
                    self.palettes.write(|p| {
                        p.bg = bg;
                        p.fg = fg;
                        p.sprite = sprite;
                    });
                    format!("Imported custom palette ← {} (undo with Ctrl+Z)", path.display())
                }
            };
            self.palette_dirty = true;
            self.mark_edited();
            self.sync_custom_palette_entry();
            Ok(msg)
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
        ui.horizontal(|ui| {
            ui.label(format!("{label} ({source})"));
            // Lunar Magic v3.61: copy a whole row of palette colors for
            // pasting into the ExAnimation dialog.
            if ui
                .small_button("Copy row")
                .on_hover_text(
                    "Copy this palette row (12 colors) to the clipboard — paste it into an \
                     ExAnimated palette frame with \"Paste row\" (Lunar Magic v3.61)",
                )
                .clicked()
            {
                let colors: Vec<u16> = self.palettes.read(|pal| pal.group(group).to_vec());
                crate::ui::clipboard::copy_payload(ui.ctx(), &crate::ui::clipboard::ClipboardPayload::PaletteRow {
                    colors,
                });
                self.palette_link_status = Some(format!("Copied {label} row (12 colors) to the clipboard."));
            }
        });

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

            // Lunar Magic v2.40: in mask-editing mode the swatch shows its
            // `.palmask` selection bit instead of the color plainly —
            // selected words get a green marker, excluded words are dimmed.
            let masked = crate::palmask::level_color_word_index(group, col).is_some_and(|w| self.palmask.selected(w));
            let (r, g, b) = if self.palmask_edit_mode && !masked { (r / 3, g / 3, b / 3) } else { (r, g, b) };
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
            if self.palmask_edit_mode && masked {
                let bl = cell_rect.left_bottom();
                ui.painter().add(egui::Shape::convex_polygon(
                    vec![bl, bl + vec2(7.0, 0.0), bl + vec2(0.0, -7.0)],
                    egui::Color32::GREEN,
                    egui::Stroke::NONE,
                ));
            }

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

            // Lunar Magic v3.33: mark swatches that are ExAnimated color
            // destinations (their CGRAM address falls in a palette frame's
            // write range) so Ctrl+Shift+Click can find them.
            let exanim_dest = self.exanim_dest_at(group, col);
            if exanim_dest.is_some() {
                let tip = cell_rect.right_top();
                ui.painter().add(egui::Shape::convex_polygon(
                    vec![tip, tip + vec2(-7.0, 0.0), tip + vec2(0.0, 7.0)],
                    egui::Color32::YELLOW,
                    egui::Stroke::NONE,
                ));
            }

            // Detect click
            let mut resp = ui.interact(cell_rect, egui::Id::new(("pal_cell", group, col, index)), Sense::click());
            // Lunar Magic gradient gestures (v1.63 / v3.40): Alt+Right-Click
            // fills a gradient from the selected color to this cell,
            // Alt+Shift+Right-Click makes it vertical.
            let gradient_hint =
                "Alt+Right-Click: gradient from the selected color · Alt+Shift+Right-Click: vertical gradient";
            // Lunar Magic v2.40: in mask-editing mode the hover shows the
            // word's mask state instead of the other hints.
            if self.palmask_edit_mode {
                if let Some(word) = crate::palmask::level_color_word_index(group, col) {
                    resp = resp.on_hover_text(format!(
                        "Mask word {word} — {} in the import mask. Click to toggle.",
                        if masked { "included" } else { "excluded" }
                    ));
                }
            } else if exanim_dest.is_some() {
                resp = resp.on_hover_text(format!(
                    "ExAnimated color destination — Ctrl+Shift+Left-Click to select its slot in ExAnimated Frames\n{gradient_hint}"
                ));
            } else {
                resp = resp.on_hover_text(gradient_hint);
            }
            if resp.clicked() {
                let mods = ui.input(|i| i.modifiers);
                if self.palmask_edit_mode {
                    // Lunar Magic v2.40: mask-editing mode — toggle the
                    // word's `.palmask` selection bit instead of selecting
                    // the swatch for color editing.
                    if let Some(word) = crate::palmask::level_color_word_index(group, col) {
                        let selected = self.palmask.toggle(word);
                        self.palmask_status = Some(format!(
                            "Mask: word {word} {} ({} of 257 selected).",
                            if selected { "selected" } else { "excluded" },
                            self.palmask.selected_count()
                        ));
                    }
                } else if mods.ctrl && mods.shift {
                    // LM v3.33: select the ExAnimated slot whose destination
                    // is this color, opening the dialog if needed.
                    self.select_exanim_dest(group, col);
                } else if mods.ctrl
                    && self.show_exanimation_editor
                    && self.exanim_dialog.palette_select_armed(&self.exanimation, Some(self.level_num))
                {
                    // LM v3.33: fill the armed ExAnimated color field with
                    // this swatch instead of selecting the swatch.
                    if self.exanim_dialog.fill_armed_color(&mut self.exanimation, Some(self.level_num), raw) {
                        self.exanimation_dirty = true;
                        self.has_edits = true;
                    }
                } else {
                    self.selected_palette_group = group as u8;
                    self.selected_palette_idx = col;
                }
            }
            if resp.secondary_clicked() {
                let mods = ui.input(|i| i.modifiers);
                if mods.alt {
                    // Lunar Magic palette gradients: Alt+Right-Click (v1.63;
                    // Ctrl+Right-Click in v1.50) fills a gradient from the
                    // selected color to this cell; Alt+Shift+Right-Click
                    // (v3.40) makes it vertical.
                    self.apply_palette_gradient(group, col, mods.shift);
                }
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

    #[test]
    fn find_palette_cgram_base_locates_exact_row() {
        // Lunar Magic v3.33: the palette-editor rows are located in CGRAM
        // empirically, right after the real level-load palette upload.
        let colors: [u16; 12] =
            [0x7C00, 0x03E0, 0x001F, 0x7FFF, 0x0000, 0x4210, 0x6318, 0x7BDE, 0x1CE7, 0x7FE0, 0x7C1F, 0x03FF];
        let mut cgram = [0xAAu8; 512];
        // Plant the row at word address 0x20 (bytes 0x40..0x58).
        for (i, c) in colors.iter().enumerate() {
            let b = c.to_le_bytes();
            cgram[0x40 + i * 2] = b[0];
            cgram[0x40 + i * 2 + 1] = b[1];
        }
        assert_eq!(find_palette_cgram_base(&cgram, &colors), Some(0x20));
        // A row that isn't in CGRAM stays inert (custom palettes, animated
        // regions rewritten by a tick).
        let mut other = colors;
        other[11] ^= 0x7FFF;
        assert_eq!(find_palette_cgram_base(&cgram, &other), None);
        // Short/degenerate CGRAM never matches.
        assert_eq!(find_palette_cgram_base(&cgram[..100], &colors), None);
    }

    #[test]
    fn gradient_fill_keeps_endpoints_and_midpoint_gray() {
        // Lunar Magic palette gradients: endpoints are the two colors,
        // intermediates are per-channel round-to-nearest (the v3.50
        // "gradient colors lightened" behavior).
        let g = gradient_fill(0x0000, 0x7FFF, 1);
        assert_eq!(g.len(), 3);
        assert_eq!(g[0], 0x0000);
        assert_eq!(g[2], 0x7FFF);
        // 0 → 31 at t=0.5 rounds to 16, not 15: the classic SNES gray.
        assert_eq!(g[1], 0x4210);
    }

    #[test]
    fn gradient_fill_adjacent_colors_is_just_endpoints() {
        let g = gradient_fill(0x001F, 0x03E0, 0);
        assert_eq!(g, vec![0x001F, 0x03E0]);
    }

    #[test]
    fn gradient_fill_is_monotone_and_channel_exact() {
        let start = 0x001F; // full red
        let end = 0x7C00; // full blue
        let g = gradient_fill(start, end, 4);
        assert_eq!(g.len(), 6);
        assert_eq!(g[0], start);
        assert_eq!(g[5], end);
        // Red ramps down, blue ramps up, green stays zero throughout.
        let reds: Vec<u16> = g.iter().map(|v| v & 0x1F).collect();
        let blues: Vec<u16> = g.iter().map(|v| (v >> 10) & 0x1F).collect();
        assert!(reds.windows(2).all(|w| w[0] >= w[1]));
        assert!(blues.windows(2).all(|w| w[0] <= w[1]));
        assert!(g.iter().all(|v| (v >> 5) & 0x1F == 0));
        // Exact intermediate: i=2, t=2/5 → red 31*3/5=18.6→19, blue 31*2/5=12.4→12.
        assert_eq!(g[2], 19 | (12 << 10));
    }

    #[test]
    fn gradient_fill_single_color_range_is_constant() {
        let g = gradient_fill(0x1234, 0x1234, 3);
        assert!(g.iter().all(|&v| v == 0x1234));
    }
}
