//! "Custom Collections of Objects" manager window + draw-mode picker
//! (Lunar Magic v3.60 parity).
//!
//! LM 3.60 re-added the **"Custom Collections of Objects"** category to the
//! **"Add Objects"** window: per-user named groups of custom extended
//! objects, useful for storing the 3-byte extended-object definitions that
//! control various level settings. The data model lives in
//! [`crate::custom_collections`]; this file is the egui front end:
//!
//! * a toolbar-toggled manager window (add/rename/delete collections and
//!   entries, with a hex ID field);
//! * a picker section in the draw-mode left panel — arming an entry makes
//!   the next canvas click in Draw mode place that extended object instead
//!   of painting the Map16 block (see `editing.rs::place_custom_object_at`).

use egui::{Context, RichText, Ui};

use super::UiLevelEditor;
use crate::custom_collections::{format_extended_id, parse_extended_id, CustomCollections};

impl UiLevelEditor {
    /// Persist the store; surfaces failures as a manager-window status line
    /// instead of panicking.
    fn save_custom_collections(&mut self) {
        if let Err(e) = self.custom_collections.save() {
            self.cc_status = Some(format!("Could not save custom collections: {e}"));
        }
    }

    /// Lookup the armed entry, if the collections still contain it.
    /// Returns `(label, extended_id)` with owned data so callers can mutate
    /// the editor afterwards.
    pub(super) fn armed_custom_entry(&self) -> Option<(String, u8)> {
        let (ci, ei) = self.draw_custom_entry?;
        let c = self.custom_collections.collections.get(ci)?;
        let e = c.entries.get(ei)?;
        Some((format!("{}/{}", c.name, e.name), e.extended_id))
    }

    /// The manager window: collections list on the left, entries on the right.
    pub(super) fn custom_collections_window(&mut self, ctx: &Context) {
        if !self.show_custom_collections {
            return;
        }
        let mut open = self.show_custom_collections;
        egui::Window::new("🧩 Custom Collections of Objects")
            .open(&mut open)
            .resizable(true)
            .default_size([560.0, 380.0])
            .show(ctx, |ui| {
                ui.label(RichText::new("Lunar Magic v3.60: named groups of custom extended objects.").weak());
                ui.label(
                    RichText::new(format!(
                        "Stored per-user at {} (not in the ROM).",
                        CustomCollections::default_path().display()
                    ))
                    .weak()
                    .small(),
                );
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    self.custom_collections_left(ui);
                    ui.separator();
                    self.custom_collections_right(ui);
                });
                if let Some(status) = &self.cc_status {
                    ui.add_space(4.0);
                    ui.label(RichText::new(status).small().color(egui::Color32::YELLOW));
                }
            });
        self.show_custom_collections = open;
    }

    /// Left column: collection list + add/rename/delete.
    fn custom_collections_left(&mut self, ui: &mut Ui) {
        ui.vertical(|ui| {
            ui.heading("Collections");
            egui::ScrollArea::vertical().max_height(260.0).show(ui, |ui| {
                for (i, c) in self.custom_collections.collections.iter().enumerate() {
                    let selected = self.cc_selected == Some(i);
                    let label = format!("{} ({})", c.name, c.entries.len());
                    if ui.selectable_label(selected, label).clicked() {
                        self.cc_selected = Some(i);
                        self.cc_rename_mode = false;
                        self.cc_edit_entry = None;
                        self.cc_status = None;
                    }
                }
            });
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                ui.label("New:");
                ui.text_edit_singleline(&mut self.cc_new_collection_name);
                if ui.button("Add").clicked() {
                    match self.custom_collections.add_collection(&self.cc_new_collection_name.clone()) {
                        Ok(i) => {
                            self.cc_selected = Some(i);
                            self.cc_new_collection_name.clear();
                            self.cc_status = None;
                            self.save_custom_collections();
                        }
                        Err(e) => self.cc_status = Some(e),
                    }
                }
            });
            if let Some(sel) = self.cc_selected {
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    if self.cc_rename_mode {
                        ui.text_edit_singleline(&mut self.cc_rename_buf);
                        if ui.button("OK").clicked() {
                            match self.custom_collections.rename_collection(sel, &self.cc_rename_buf.clone()) {
                                Ok(()) => {
                                    self.cc_rename_mode = false;
                                    self.cc_status = None;
                                    self.save_custom_collections();
                                }
                                Err(e) => self.cc_status = Some(e),
                            }
                        }
                        if ui.button("Cancel").clicked() {
                            self.cc_rename_mode = false;
                        }
                    } else {
                        if ui.button("Rename").clicked() {
                            self.cc_rename_buf = self
                                .custom_collections
                                .collections
                                .get(sel)
                                .map(|c| c.name.clone())
                                .unwrap_or_default();
                            self.cc_rename_mode = true;
                        }
                        if ui.button("Delete").clicked() {
                            self.custom_collections.remove_collection(sel);
                            self.cc_selected = None;
                            self.cc_rename_mode = false;
                            self.cc_edit_entry = None;
                            self.draw_custom_entry = None;
                            self.cc_status = None;
                            self.save_custom_collections();
                        }
                    }
                });
            }
        });
    }

    /// Right column: entries of the selected collection + add/edit/delete.
    fn custom_collections_right(&mut self, ui: &mut Ui) {
        ui.vertical(|ui| {
            let Some(sel) = self.cc_selected else {
                ui.label(RichText::new("Select a collection, or add one.").weak());
                return;
            };
            let name = self.custom_collections.collections.get(sel).map(|c| c.name.clone()).unwrap_or_default();
            ui.heading(format!("{name} — entries"));

            egui::ScrollArea::vertical().max_height(200.0).show(ui, |ui| {
                let entries: Vec<(usize, String, u8)> = self
                    .custom_collections
                    .collections
                    .get(sel)
                    .map(|c| c.entries.iter().enumerate().map(|(i, e)| (i, e.name.clone(), e.extended_id)).collect())
                    .unwrap_or_default();
                for (i, ename, eid) in entries {
                    ui.horizontal(|ui| {
                        if self.cc_edit_entry == Some((sel, i)) {
                            ui.text_edit_singleline(&mut self.cc_edit_name);
                            ui.add(egui::TextEdit::singleline(&mut self.cc_edit_id).desired_width(52.0));
                            if ui.button("OK").clicked() {
                                let id = parse_extended_id(&self.cc_edit_id.clone());
                                let res = id.and_then(|id| {
                                    self.custom_collections.update_entry(sel, i, &self.cc_edit_name.clone(), id)
                                });
                                match res {
                                    Ok(()) => {
                                        self.cc_edit_entry = None;
                                        self.cc_status = None;
                                        self.save_custom_collections();
                                    }
                                    Err(e) => self.cc_status = Some(e),
                                }
                            }
                            if ui.button("Cancel").clicked() {
                                self.cc_edit_entry = None;
                            }
                        } else {
                            ui.label(format!("{}  {}", ename, format_extended_id(eid)));
                            let armed = self.draw_custom_entry == Some((sel, i));
                            if ui.small_button(if armed { "Armed ✓" } else { "Place" }).clicked() {
                                self.draw_custom_entry = if armed { None } else { Some((sel, i)) };
                            }
                            if ui.small_button("Edit").clicked() {
                                self.cc_edit_entry = Some((sel, i));
                                self.cc_edit_name = ename.clone();
                                self.cc_edit_id = format!("{eid:02X}");
                            }
                            if ui.small_button("🗑").clicked() {
                                self.custom_collections.remove_entry(sel, i);
                                if self.draw_custom_entry == Some((sel, i)) {
                                    self.draw_custom_entry = None;
                                }
                                self.cc_edit_entry = None;
                                self.cc_status = None;
                                self.save_custom_collections();
                            }
                        }
                    });
                }
            });

            ui.add_space(4.0);
            ui.horizontal(|ui| {
                ui.label("Name:");
                ui.text_edit_singleline(&mut self.cc_new_entry_name);
                ui.label("ID:");
                ui.add(egui::TextEdit::singleline(&mut self.cc_new_entry_id).desired_width(52.0).hint_text("E0"))
                    .on_hover_text("Hex byte 00–FF (e.g. E0 or $E0). 00/01 are reserved: exit / screen jump.");
                if ui.button("Add entry").clicked() {
                    let id = parse_extended_id(&self.cc_new_entry_id.clone());
                    let res =
                        id.and_then(|id| self.custom_collections.add_entry(sel, &self.cc_new_entry_name.clone(), id));
                    match res {
                        Ok(()) => {
                            self.cc_new_entry_name.clear();
                            self.cc_new_entry_id.clear();
                            self.cc_status = None;
                            self.save_custom_collections();
                        }
                        Err(e) => self.cc_status = Some(e),
                    }
                }
            });
        });
    }

    /// Draw-mode left-panel section: pick a custom entry to place on the
    /// canvas (the "Custom Collections of Objects" category of LM's
    /// Add Objects window).
    pub(super) fn custom_collections_picker(&mut self, ui: &mut Ui) {
        ui.separator();
        // Keep a copy for the armed banner; the picker below mutates self.
        let armed_label = self.armed_custom_entry().map(|(label, _)| label);
        if let Some(label) = armed_label {
            ui.horizontal(|ui| {
                ui.label(RichText::new(format!("Placing custom object: {label}")).strong());
                if ui.small_button("✕ Cancel").clicked() {
                    self.draw_custom_entry = None;
                }
            });
            ui.label(RichText::new("Click the canvas in Draw mode to place it.").weak().small());
        }
        ui.collapsing("🧩 Custom Collections of Objects (LM 3.60)", |ui| {
            if self.custom_collections.collections.is_empty() {
                ui.label(RichText::new("No collections yet.").weak());
            }
            for (ci, c) in self.custom_collections.collections.iter().enumerate() {
                ui.collapsing(format!("{} ({})", c.name, c.entries.len()), |ui| {
                    for (ei, e) in c.entries.iter().enumerate() {
                        ui.horizontal(|ui| {
                            let armed = self.draw_custom_entry == Some((ci, ei));
                            ui.label(format!("{}  {}", e.name, format_extended_id(e.extended_id)));
                            if ui.small_button(if armed { "Armed ✓" } else { "Place" }).clicked() {
                                self.draw_custom_entry = if armed { None } else { Some((ci, ei)) };
                            }
                        });
                    }
                });
            }
            if ui.small_button("⚙ Manage collections…").clicked() {
                self.show_custom_collections = true;
            }
        });
    }
}
