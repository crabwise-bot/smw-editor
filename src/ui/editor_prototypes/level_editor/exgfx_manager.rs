//! ExGFX Manager — Lunar Magic-style extra graphics file management.
//!
//! LM v1.10/v1.60 parity: insert/extract/delete extra GFX files (indices
//! 0x80+). Files are stored as RATS-tagged free-space blocks in the ROM;
//! the "Levels using" column cross-references the Super GFX Bypass table.
//! Note: making the game itself *use* ExGFX still requires Lunar Magic's
//! ExGFX ASM hack — this editor authors and previews the data
//! (see `smwe_rom::exgfx` docs).

use egui::{Context, Slider};
use rfd::{MessageButtons, MessageDialog, MessageDialogResult};
use smwe_rom::exgfx::{BYPASS_DEFAULT, EXGFX_FILE_BYTES, EXGFX_FIRST_INDEX, EXGFX_MAX_INDEX, EXGFX_TILES_PER_FILE};

use super::UiLevelEditor;

impl UiLevelEditor {
    pub(super) fn exgfx_manager_window(&mut self, ctx: &Context) {
        if !self.show_exgfx_manager {
            return;
        }
        let mut open = self.show_exgfx_manager;
        // Snapshot everything the window body needs so the egui closure
        // doesn't fight the borrow checker with `self`.
        let mut insert_index = self.exgfx_insert_pending.as_ref().map(|&(_, idx)| idx);
        let insert_label = self.exgfx_insert_pending.as_ref().map(|(bytes, _)| bytes.len());

        let mut files: Vec<(u16, usize)> = self.exgfx_data.files.iter().map(|(&idx, f)| (idx, f.tiles.len())).collect();
        files.sort_unstable_by_key(|&(idx, _)| idx);
        let used_by: Vec<(u16, Vec<u16>)> = files
            .iter()
            .map(|&(idx, _)| {
                let mut levels: Vec<u16> = self
                    .bypass_data
                    .levels
                    .iter()
                    .filter(|(_, slots)| slots.contains(&idx))
                    .map(|(&level, _)| level)
                    .collect();
                levels.sort_unstable();
                (idx, levels)
            })
            .collect();

        let mut delete_idx: Option<u16> = None;
        let mut edit_idx: Option<u16> = None;
        let mut extract_idx: Option<u16> = None;
        let mut pick_insert = false;
        let mut confirm_insert = false;
        let mut cancel_insert = false;

        egui::Window::new("ExGFX Manager").open(&mut open).resizable(true).default_size([600.0, 420.0]).show(
            ctx,
            |ui| {
                ui.label(
                    "Extra graphics files (LM v1.10/v1.60). Inserted files live in ROM free space; \
                     assign them to a level's FG/BG or sprite slots in Super GFX Bypass.",
                );
                ui.small(
                    "In-game use needs Lunar Magic's ExGFX ASM hack — the editor itself doesn't patch the game engine.",
                );
                ui.separator();

                if files.is_empty() {
                    ui.label("No ExGFX files inserted yet.");
                } else {
                    egui::ScrollArea::vertical().max_height(260.0).show(ui, |ui| {
                        egui::Grid::new("exgfx_manager_grid").num_columns(5).spacing([12.0, 6.0]).striped(true).show(
                            ui,
                            |ui| {
                                ui.label("File");
                                ui.label("Tiles");
                                ui.label("Levels using");
                                ui.label("");
                                ui.label("");
                                ui.end_row();
                                for ((idx, tile_count), &(_, ref levels)) in files.iter().zip(used_by.iter()) {
                                    ui.monospace(format!("ExGFX{idx:03X}"));
                                    ui.label(format!("{tile_count}"));
                                    ui.label(if levels.is_empty() {
                                        "—".to_owned()
                                    } else {
                                        levels.iter().map(|l| format!("{l:03X}")).collect::<Vec<_>>().join(", ")
                                    });
                                    if ui.button("Edit").clicked() {
                                        edit_idx = Some(*idx);
                                    }
                                    ui.horizontal(|ui| {
                                        if ui.button("Extract").clicked() {
                                            extract_idx = Some(*idx);
                                        }
                                        if ui.button("Delete").clicked() {
                                            delete_idx = Some(*idx);
                                        }
                                    });
                                    ui.end_row();
                                }
                            },
                        );
                    });
                }
                ui.separator();

                ui.horizontal(|ui| {
                    if ui.button("Insert ExGFX file…").clicked() {
                        pick_insert = true;
                    }
                    if ui.button("Super GFX Bypass…").clicked() {
                        // Open the bypass dialog for the current level.
                        ui.data_mut(|data| data.insert_temp(egui::Id::new("exgfx_open_bypass"), true));
                    }
                });

                // Pending insert: pick the file index before committing.
                if let (Some(_), Some(bytes_len), Some(idx)) =
                    (&self.exgfx_insert_pending, insert_label, insert_index.as_mut())
                {
                    let _ = bytes_len;
                    ui.separator();
                    ui.label(format!("Insert {bytes_len} bytes as ExGFX file index:"));
                    ui.horizontal(|ui| {
                        ui.add(
                            Slider::new(idx, EXGFX_FIRST_INDEX..=EXGFX_MAX_INDEX)
                                .custom_formatter(|n, _| format!("ExGFX{:03X}", n as u32))
                                .custom_parser(|s| {
                                    let s = s.trim_start_matches("ExGFX").trim_start_matches("exgfx");
                                    let s = s.trim_start_matches("0x");
                                    u32::from_str_radix(s, 16).ok().map(|v| v as f64).or_else(|| s.parse::<f64>().ok())
                                }),
                        );
                        if self.exgfx_data.files.contains_key(idx) {
                            ui.colored_label(egui::Color32::RED, "index in use");
                        }
                        if ui.button("Insert").clicked() {
                            confirm_insert = true;
                        }
                        if ui.button("Cancel").clicked() {
                            cancel_insert = true;
                        }
                    });
                }

                if let Some(status) = &self.exgfx_manager_status {
                    ui.separator();
                    ui.label(status);
                }
            },
        );
        self.show_exgfx_manager = open;

        if pick_insert {
            self.pick_exgfx_insert();
        }
        if cancel_insert {
            self.exgfx_insert_pending = None;
            self.exgfx_manager_status = Some("Insert cancelled.".to_owned());
        } else if let (Some(idx), true) = (insert_index, confirm_insert) {
            // The slider above wrote the chosen index back into `insert_index`.
            if let Some((bytes, _)) = self.exgfx_insert_pending.take() {
                self.commit_exgfx_insert(bytes, idx);
            }
        } else if let Some(idx) = insert_index {
            // Keep the slider's index choice in the pending state.
            if let Some(pending) = self.exgfx_insert_pending.as_mut() {
                pending.1 = idx;
            }
        }

        if let Some(idx) = delete_idx {
            self.delete_exgfx_file(idx);
        }
        if let Some(idx) = extract_idx {
            self.extract_exgfx_file(idx);
        }
        if let Some(idx) = edit_idx {
            let palette = self.level_properties.palette_fg as usize;
            self.open_tile_editor_at(usize::from(idx), 0, palette);
        }
        if ctx.data(|data| data.get_temp::<bool>(egui::Id::new("exgfx_open_bypass")).unwrap_or(false)) {
            ctx.data_mut(|data| data.remove::<bool>(egui::Id::new("exgfx_open_bypass")));
            self.show_gfx_bypass = true;
        }
    }

    /// Lowest free ExGFX index (0x80+), or the top of the range if full.
    fn lowest_free_exgfx_index(&self) -> u16 {
        (u32::from(EXGFX_FIRST_INDEX)..=u32::from(EXGFX_MAX_INDEX))
            .map(|i| i as u16)
            .find(|i| !self.exgfx_data.files.contains_key(i))
            .unwrap_or(EXGFX_MAX_INDEX)
    }

    fn pick_exgfx_insert(&mut self) {
        let Some(path) = rfd::FileDialog::new()
            .add_filter("ExGFX graphics file", &["bin"])
            .set_title("Insert ExGFX file (must be exactly 32 KiB, like LM's ExGFXnn.bin)")
            .pick_file()
        else {
            return;
        };
        match std::fs::read(&path) {
            Ok(bytes) if bytes.len() == EXGFX_FILE_BYTES => {
                let idx = self.lowest_free_exgfx_index();
                self.exgfx_insert_pending = Some((bytes, idx));
                self.exgfx_manager_status = Some(format!(
                    "Read {} ({} bytes, {} tiles). Pick the file index, then Insert.",
                    path.display(),
                    EXGFX_FILE_BYTES,
                    EXGFX_TILES_PER_FILE
                ));
            }
            Ok(bytes) => {
                self.exgfx_manager_status = Some(format!(
                    "Rejected {}: expected exactly {} bytes (4bpp, {} tiles), got {}.",
                    path.display(),
                    EXGFX_FILE_BYTES,
                    EXGFX_TILES_PER_FILE,
                    bytes.len()
                ));
            }
            Err(e) => {
                self.exgfx_manager_status = Some(format!("Cannot read {}: {e}", path.display()));
            }
        }
    }

    fn commit_exgfx_insert(&mut self, bytes: Vec<u8>, index: u16) {
        if self.exgfx_data.files.contains_key(&index) {
            self.exgfx_manager_status = Some(format!("ExGFX{index:03X} is already in use — pick a free index."));
            self.exgfx_insert_pending = Some((bytes, self.lowest_free_exgfx_index()));
            return;
        }
        match self.exgfx_data.insert_raw(index, bytes) {
            Ok(()) => {
                self.exgfx_dirty = true;
                self.mark_edited();
                self.exgfx_manager_status = Some(format!(
                    "Inserted ExGFX{index:03X} ({} tiles). Assign it in Super GFX Bypass.",
                    EXGFX_TILES_PER_FILE
                ));
            }
            Err(e) => {
                self.exgfx_manager_status = Some(format!("Insert failed: {e}"));
            }
        }
    }

    fn delete_exgfx_file(&mut self, index: u16) {
        let used: Vec<u16> = self
            .bypass_data
            .levels
            .iter()
            .filter(|(_, slots)| slots.contains(&index))
            .map(|(&level, _)| level)
            .collect();
        let warning = if used.is_empty() {
            String::new()
        } else {
            format!(
                " Levels {} reference it; their bypass slots will fall back to defaults.",
                used.iter().map(|l| format!("{l:03X}")).collect::<Vec<_>>().join(", ")
            )
        };
        let confirmed = MessageDialog::new()
            .set_title("Delete ExGFX file")
            .set_description(format!("Delete ExGFX{index:03X} from the ROM?{warning}"))
            .set_buttons(MessageButtons::OkCancel)
            .show();
        if confirmed != MessageDialogResult::Ok {
            return;
        }
        if self.exgfx_data.remove(index) {
            self.exgfx_dirty = true;
            // The confirmation promised fallback to defaults: rewrite every
            // bypass slot that referenced the deleted file.
            let mut cleared: Vec<u16> = Vec::new();
            for (&level, slots) in self.bypass_data.levels.iter_mut() {
                for s in slots.iter_mut() {
                    if *s == index {
                        *s = BYPASS_DEFAULT;
                        if !cleared.contains(&level) {
                            cleared.push(level);
                        }
                    }
                }
            }
            if !cleared.is_empty() {
                self.bypass_dirty = true;
                self.exgfx_manager_status = Some(format!(
                    "Deleted ExGFX{index:03X}; bypass slots in levels {} reset to defaults.",
                    cleared.iter().map(|l| format!("{l:03X}")).collect::<Vec<_>>().join(", ")
                ));
            } else {
                self.exgfx_manager_status = Some(format!("Deleted ExGFX{index:03X}."));
            }
            self.mark_edited();
            if cleared.contains(&self.level_num) {
                // The current level lost an override: re-upload its slots so
                // the deleted file's tiles leave VRAM immediately.
                self.apply_bypass_to_vram();
            }
        }
    }

    fn extract_exgfx_file(&mut self, index: u16) {
        let Some(file) = self.exgfx_data.files.get(&index) else {
            self.exgfx_manager_status = Some(format!("ExGFX{index:03X} not found."));
            return;
        };
        let raw = file.raw_bytes();
        let Some(path) = rfd::FileDialog::new()
            .add_filter("ExGFX graphics file", &["bin"])
            .set_file_name(format!("ExGFX{index:03X}.bin"))
            .set_title("Extract ExGFX file")
            .save_file()
        else {
            return;
        };
        match std::fs::write(&path, raw) {
            Ok(()) => {
                self.exgfx_manager_status = Some(format!("Extracted ExGFX{index:03X} to {}.", path.display()));
            }
            Err(e) => {
                self.exgfx_manager_status = Some(format!("Cannot write {}: {e}", path.display()));
            }
        }
    }
}
