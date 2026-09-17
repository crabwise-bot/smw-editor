//! LM v3.00 "secondary exit teleport locations" editor (Star/Pipe table).
//!
//! Lunar Magic v3.00 added an overworld toolbar button that edits the
//! secondary-exit teleport locations: the 0x100-entry Star/Pipe table of
//! overworld destinations used when a secondary exit sends the player to the
//! overworld. There is no vanilla storage for this table (nothing matching in
//! SMWDisX), so the editor persists it in its own RATS-tagged free-space
//! block (`SMWESEX2`, shared with the level editor's per-entrance options).

use egui::{Context, Grid, ScrollArea, Slider};
use smwe_rom::{
    level::secondary_entrance::OW_TELEPORT_TABLE_LEN,
    overworld::{OW_HEIGHT_TILES, OW_WIDTH_TILES, SUBMAP_COUNT, SUBMAP_NAMES},
};

use super::UiWorldEditor;

impl UiWorldEditor {
    pub(super) fn se_teleport_editor_window(&mut self, ctx: &Context) {
        if !self.show_se_teleport_editor {
            return;
        }
        let mut open = self.show_se_teleport_editor;
        egui::Window::new("Secondary Exit Teleport Locations")
            .open(&mut open)
            .resizable(true)
            .default_size([560.0, 520.0])
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.label("Filter:");
                    ui.text_edit_singleline(&mut self.se_teleport_search);
                    if ui.small_button("Clear").clicked() {
                        self.se_teleport_search.clear();
                    }
                });
                ui.label(
                    "Where the player appears on the overworld when a secondary exit \
                       sends them there. Saved with Ctrl+S.",
                );
                ui.separator();

                let search = self.se_teleport_search.clone();
                let filter: Option<u8> = search
                    .trim()
                    .strip_prefix("0x")
                    .and_then(|s| u8::from_str_radix(s, 16).ok())
                    .or_else(|| search.trim().parse::<u8>().ok());

                let mut dirty = false;
                ScrollArea::vertical().show(ui, |ui| {
                    Grid::new("se_teleport_grid").num_columns(4).spacing([10.0, 4.0]).striped(true).show(ui, |ui| {
                        ui.strong("ID");
                        ui.strong("Submap");
                        ui.strong("X");
                        ui.strong("Y");
                        ui.end_row();

                        for idx in 0..OW_TELEPORT_TABLE_LEN {
                            if let Some(f) = filter {
                                if idx as u8 != f {
                                    continue;
                                }
                            }
                            let entry = self.se_teleports[idx];
                            ui.monospace(format!("{:02X}", idx));

                            // Submap
                            {
                                let mut v = entry.submap.min(SUBMAP_COUNT as u8 - 1);
                                let label = SUBMAP_NAMES.get(v as usize).copied().unwrap_or("???");
                                egui::ComboBox::from_id_salt(("se_tp_submap", idx))
                                    .selected_text(format!("{v} — {label}"))
                                    .show_ui(ui, |ui| {
                                        for (i, name) in SUBMAP_NAMES.iter().enumerate() {
                                            ui.selectable_value(&mut v, i as u8, format!("{i} — {name}"));
                                        }
                                    });
                                if v != entry.submap {
                                    self.se_teleports[idx].submap = v;
                                    dirty = true;
                                }
                            }

                            // X tile
                            {
                                let mut v = entry.x.min(OW_WIDTH_TILES as u8 - 1) as i32;
                                if ui.add(Slider::new(&mut v, 0..=OW_WIDTH_TILES as i32 - 1)).changed() {
                                    self.se_teleports[idx].x = v as u8;
                                    dirty = true;
                                }
                            }

                            // Y tile
                            {
                                let mut v = entry.y.min(OW_HEIGHT_TILES as u8 - 1) as i32;
                                if ui.add(Slider::new(&mut v, 0..=OW_HEIGHT_TILES as i32 - 1)).changed() {
                                    self.se_teleports[idx].y = v as u8;
                                    dirty = true;
                                }
                            }

                            ui.end_row();
                        }
                    });
                });

                if dirty {
                    self.se_teleports_dirty = true;
                    self.has_edits = true;
                }

                ui.separator();
                ui.weak(
                    "The teleport table is stored in the editor's RATS block (SMWESEX2). \
                 In-game playback needs Lunar Magic's ASM hacks, which this editor \
                 does not install.",
                );
            });
        self.show_se_teleport_editor = open;
    }
}
