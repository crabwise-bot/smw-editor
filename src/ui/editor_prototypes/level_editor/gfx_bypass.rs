//! Super GFX Bypass dialog — Lunar Magic v1.60 parity.
//!
//! Per-level FG1/FG2/FG3/BG1 + SP1-SP4 file assignment. "Default" keeps the
//! level's FG/BG and sprite tileset tables; any other choice names a vanilla
//! GFX file (0x00-0x33) or an inserted ExGFX file (0x80+). Apply writes the
//! bypass table (dirty → saved by `save_to_rom`) and live-applies the slots
//! to the emulator VRAM so the level view, tile picker, and renderer update
//! immediately.

use egui::Context;
use smwe_rom::{
    exgfx::{self, BYPASS_DEFAULT, BYPASS_SLOT_COUNT, BYPASS_SLOT_NAMES, EXGFX_FIRST_INDEX},
    graphics::gfx_file,
};

use super::UiLevelEditor;

impl UiLevelEditor {
    /// Mirror LM's ExGFX ASM hack at level load for the current level: for
    /// every explicitly-assigned bypass slot, overwrite the slot's VRAM
    /// range. Vanilla files go through the game's own `UploadGFXFile`
    /// (bit-exact 3bpp→4bpp expansion); ExGFX files are 4bpp and their first
    /// [`exgfx::BYPASS_SLOT_BYTES`] bytes are copied straight in. Slots at
    /// [`BYPASS_DEFAULT`] — or referencing a missing ExGFX file — are left
    /// alone. Returns the number of slots overridden.
    pub(super) fn apply_bypass_to_vram(&mut self) -> usize {
        let level = self.level_num;
        let mut overridden = 0;
        for slot in 0..BYPASS_SLOT_COUNT {
            let value = self.bypass_data.slot(level, slot).unwrap_or(BYPASS_DEFAULT);
            if value == BYPASS_DEFAULT {
                continue;
            }
            let Some((byte_off, byte_len)) = exgfx::bypass_slot_vram_span(slot) else { continue };
            if value < gfx_file::gfx_file_count() as u16 {
                // Bit-exact game upload (handles 3bpp→4bpp etc.).
                smwe_emu::emu::upload_gfx_file_to_vram(&mut self.cpu, value as u8, (byte_off / 2) as u16);
                overridden += 1;
            } else if value >= EXGFX_FIRST_INDEX {
                if let Some(file) = self.exgfx_data.files.get(&value) {
                    let raw = file.raw_bytes();
                    let n = byte_len.min(raw.len());
                    if byte_off + n <= self.cpu.mem.vram.len() {
                        self.cpu.mem.vram[byte_off..byte_off + n].copy_from_slice(&raw[..n]);
                        overridden += 1;
                    }
                }
                // Missing ExGFX file: leave the slot's current graphics alone.
            }
            // Values in 0x34..0x80 are invalid; ignore them.
        }
        overridden
    }

    pub(super) fn gfx_bypass_window(&mut self, ctx: &Context) {
        if !self.show_gfx_bypass {
            return;
        }
        // Resync the working copy when the level changed under the dialog.
        if self.bypass_edit_level != self.level_num {
            self.bypass_edit_slots =
                self.bypass_data.levels.get(&self.level_num).copied().unwrap_or([BYPASS_DEFAULT; BYPASS_SLOT_COUNT]);
            self.bypass_edit_level = self.level_num;
        }

        // Snapshot immutable inputs so the egui closure never captures `self`.
        let level_num = self.level_num;
        let fg_tileset = usize::from(self.level_properties.fg_bg_gfx).min(25);
        let sp_tileset = usize::from(self.level_properties.sprite_gfx).min(25);
        let fg_files = self.rom.gfx.object_gfx_list.files_for_object_tileset(fg_tileset);
        let sp_files = self.rom.gfx.sprite_gfx_list.files_for_sprite_tileset(sp_tileset);
        let vanilla_count = gfx_file::gfx_file_count();
        let mut exgfx_indices: Vec<u16> = self.exgfx_data.files.keys().copied().collect();
        exgfx_indices.sort_unstable();
        let missing: Vec<u16> = self
            .bypass_edit_slots
            .iter()
            .copied()
            .filter(|&v| v >= EXGFX_FIRST_INDEX && !self.exgfx_data.files.contains_key(&v))
            .collect();

        let mut open = self.show_gfx_bypass;
        let mut apply_clicked = false;
        let mut reset_clicked = false;
        let mut slots = self.bypass_edit_slots;

        egui::Window::new(format!("Super GFX Bypass — level {level_num:03X}"))
            .open(&mut open)
            .resizable(true)
            .default_size([540.0, 430.0])
            .show(ctx, |ui| {
                ui.label(
                    "Pick the GFX file each upload slot uses. \"Default\" keeps the level's \
                     FG/BG and sprite tileset tables; anything else overrides the slot — \
                     like Lunar Magic's Super GFX Bypass dialog.",
                );
                if !missing.is_empty() {
                    ui.colored_label(
                        egui::Color32::YELLOW,
                        format!(
                            "Referenced ExGFX file(s) not inserted: {}. Those slots keep \
                             their current graphics until the file is inserted.",
                            missing.iter().map(|v| format!("{v:03X}")).collect::<Vec<_>>().join(", ")
                        ),
                    );
                }
                ui.separator();
                egui::Grid::new("gfx_bypass_grid").num_columns(2).spacing([12.0, 6.0]).show(ui, |ui| {
                    for slot in 0..BYPASS_SLOT_COUNT {
                        let default_file = if slot < 4 { fg_files[slot] } else { sp_files[slot - 4] };
                        ui.label(BYPASS_SLOT_NAMES[slot]);
                        egui::ComboBox::from_id_salt(format!("gfx_bypass_slot_{slot}"))
                            .selected_text(slot_value_label(slots[slot], default_file))
                            .show_ui(ui, |ui| {
                                ui.selectable_value(
                                    &mut slots[slot],
                                    BYPASS_DEFAULT,
                                    format!("Default (tileset → GFX file {default_file:02X})"),
                                );
                                ui.separator();
                                for f in 0..vanilla_count {
                                    ui.selectable_value(&mut slots[slot], f as u16, format!("GFX file {f:02X}"));
                                }
                                if !exgfx_indices.is_empty() {
                                    ui.separator();
                                    for &idx in &exgfx_indices {
                                        ui.selectable_value(&mut slots[slot], idx, format!("ExGFX {idx:03X}"));
                                    }
                                }
                            });
                        ui.end_row();
                    }
                });
                ui.separator();
                ui.horizontal(|ui| {
                    if ui.button("Apply").clicked() {
                        apply_clicked = true;
                    }
                    if ui.button("Reset to defaults").clicked() {
                        reset_clicked = true;
                    }
                    ui.small("Apply updates the level view immediately.");
                });
            });
        self.show_gfx_bypass = open;
        self.bypass_edit_slots = slots;
        if reset_clicked {
            self.bypass_edit_slots = [BYPASS_DEFAULT; BYPASS_SLOT_COUNT];
        }
        if apply_clicked {
            self.commit_bypass_edits();
        }
    }

    /// Write the dialog's working copy into the bypass table and live-apply.
    fn commit_bypass_edits(&mut self) {
        let mut changed = false;
        for slot in 0..BYPASS_SLOT_COUNT {
            let current = self.bypass_data.slot(self.level_num, slot).unwrap_or(BYPASS_DEFAULT);
            if current != self.bypass_edit_slots[slot]
                && self.bypass_data.set_slot(self.level_num, slot, self.bypass_edit_slots[slot]).is_ok()
            {
                changed = true;
            }
        }
        if changed {
            self.bypass_dirty = true;
            self.mark_edited();
        }
        // Live-apply even when nothing changed: VRAM may hold stale graphics
        // from an earlier preview state.
        let n = self.apply_bypass_to_vram();
        if n > 0 || changed {
            let renderer = self.level_renderer.lock().expect("Cannot lock level_renderer");
            renderer.upload_gfx(&self.gl, &self.cpu.mem.vram);
            drop(renderer);
            self.tile_picker.rebuild(&mut self.cpu);
            self.bg_tile_picker.rebuild(&mut self.cpu);
        }
        // Force the working copy to resync from the stored table next frame.
        self.bypass_edit_level = 0xFFFF;
    }
}

/// Dialog label for a slot value: "Default (GFX file 0C)" or the
/// [`exgfx::slot_source_label`] ("GFX file 0C" / "ExGFX 080").
fn slot_value_label(value: u16, default_file: usize) -> String {
    if value == BYPASS_DEFAULT {
        format!("Default (GFX file {default_file:02X})")
    } else {
        exgfx::slot_source_label(value)
    }
}
