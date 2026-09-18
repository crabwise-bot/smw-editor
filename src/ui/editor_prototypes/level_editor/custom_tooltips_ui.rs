//! Lunar Magic **custom tooltips for objects** (v3.60 parity) — the manager
//! window.
//!
//! LM 3.60 lets the user attach their own description text to objects; the
//! text then shows as the object's tooltip. This window is the smw-editor
//! equivalent: pick the object kind (Standard / Extended), pick an ID, type
//! the tooltip, save. Everything persists to the per-user JSON store
//! (`crate::custom_tooltips`) — the ROM is never touched, since tooltips
//! are pure editor metadata.

use egui::{Context, RichText};

use super::UiLevelEditor;
use crate::custom_tooltips::ObjectKind;

/// The manager window, rendered from `mod.rs`'s window list next to the
/// other level-editor windows.
impl UiLevelEditor {
    pub(super) fn custom_tooltips_window(&mut self, ctx: &Context) {
        if !self.show_custom_tooltips {
            return;
        }
        let mut open = self.show_custom_tooltips;
        egui::Window::new("💬 Custom Object Tooltips")
            .open(&mut open)
            .resizable(true)
            .default_size([460.0, 520.0])
            .show(ctx, |ui| {
                ui.small(
                    "User-settable tooltip text for level objects (Lunar Magic v3.60). \
                     Hovering an object on the canvas shows its tooltip. \
                     Stored per-user — never written to the ROM.",
                );
                ui.separator();

                // ── Kind tabs ──────────────────────────────────
                ui.horizontal(|ui| {
                    for kind in [ObjectKind::Standard, ObjectKind::Extended] {
                        if ui.selectable_label(self.tooltip_kind == kind, kind.label()).clicked() {
                            self.tooltip_kind = kind;
                            self.sync_tooltip_edit_buffer();
                        }
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.small(format!("{} custom tooltip(s)", self.custom_tooltips.len()));
                    });
                });
                ui.separator();

                ui.horizontal(|ui| {
                    ui.label("Search:");
                    ui.add(
                        egui::TextEdit::singleline(&mut self.tooltip_search)
                            .hint_text("hex id or tooltip text")
                            .desired_width(220.0),
                    );
                    if ui.small_button("✕").on_hover_text("Clear search").clicked() {
                        self.tooltip_search.clear();
                    }
                });

                // ── ID list ────────────────────────────────────
                let search = self.tooltip_search.trim().to_lowercase();
                let mut ids: Vec<u8> = (0u8..=0xFF)
                    .filter(|&id| {
                        if search.is_empty() {
                            return true;
                        }
                        if format!("{id:02x}").contains(&search) {
                            return true;
                        }
                        self.custom_tooltips
                            .get(self.tooltip_kind, id)
                            .is_some_and(|t| t.to_lowercase().contains(&search))
                    })
                    .collect();
                ids.sort_unstable();

                egui::ScrollArea::vertical().max_height(300.0).show(ui, |ui| {
                    egui::Grid::new("custom_tooltip_id_grid").num_columns(2).spacing([8.0, 2.0]).show(ui, |ui| {
                        for id in ids {
                            let tip = self.custom_tooltips.get(self.tooltip_kind, id);
                            let has_tip = tip.is_some();
                            ui.monospace(format!("0x{id:02X}"));
                            let label = tip.unwrap_or("—");
                            let resp = ui.selectable_label(
                                self.tooltip_selected_id == id,
                                RichText::new(label).color(if has_tip {
                                    egui::Color32::WHITE
                                } else {
                                    egui::Color32::GRAY
                                }),
                            );
                            if resp.clicked() {
                                self.tooltip_selected_id = id;
                                self.sync_tooltip_edit_buffer();
                            }
                            ui.end_row();
                        }
                    });
                });
                ui.separator();

                // ── Edit the selected ID's tooltip ─────────────
                ui.horizontal(|ui| {
                    ui.strong(format!("{} object 0x{:02X}:", self.tooltip_kind.label(), self.tooltip_selected_id));
                });
                let id = self.tooltip_selected_id;
                let kind = self.tooltip_kind;
                let saved = self.custom_tooltips.get(kind, id).unwrap_or("").to_string();
                ui.add(
                    egui::TextEdit::multiline(&mut self.tooltip_edit_text)
                        .hint_text("Type the custom tooltip text… (empty clears it)")
                        .desired_rows(2)
                        .desired_width(f32::INFINITY),
                );
                let changed = self.tooltip_edit_text.trim() != saved.trim();
                ui.horizontal(|ui| {
                    if ui.add_enabled(changed, egui::Button::new("💾 Save tooltip")).clicked() {
                        self.custom_tooltips.set(kind, id, &self.tooltip_edit_text.clone());
                        self.custom_tooltips.save();
                        self.sync_tooltip_edit_buffer();
                    }
                    if ui
                        .add_enabled(!saved.is_empty(), egui::Button::new("🗑 Clear"))
                        .on_hover_text("Remove this object's custom tooltip")
                        .clicked()
                    {
                        self.custom_tooltips.set(kind, id, "");
                        self.custom_tooltips.save();
                        self.sync_tooltip_edit_buffer();
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.small("max 256 chars");
                    });
                });
            });
        self.show_custom_tooltips = open;
    }

    /// Re-sync the edit buffer with the stored tooltip for the currently
    /// selected (kind, id).
    pub(super) fn sync_tooltip_edit_buffer(&mut self) {
        self.tooltip_edit_text =
            self.custom_tooltips.get(self.tooltip_kind, self.tooltip_selected_id).unwrap_or("").to_string();
    }
}
