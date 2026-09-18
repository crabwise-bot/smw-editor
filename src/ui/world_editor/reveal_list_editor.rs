//! LM v2.30 "Edit Reveal Tile List" dialog (overworld editor menu item).
//!
//! When a "destruction" event fires, the game swaps layer-1 tiles: for each
//! active event, if the tile at its offset (the `$04D85D` per-event offset
//! table) matches a "before" ID, it is replaced with the parallel "after" ID
//! (SMWDisX `bank_04.asm`, `CODE_04DA49`). The before/after ID lists live at
//! SNES `$04DA1D`/`$04DA33`, 22 bytes each, and are global — every event
//! consults the same list. This dialog edits those 22 pairs in place (no
//! relocation needed).
//!
//! Each row shows how many destruction events currently use it (events whose
//! tile at their per-event offset equals the row's "before" ID — the same
//! match the game performs). The last row is special in the vanilla game:
//! the switch-palace reveal also writes the tile *after* the event's offset
//! (see `smwe_rom::overworld::OverworldEvents::apply`).
//!
//! Edits are undoable (one step per changed row, like the sprite tool's
//! per-move commits) and the map preview re-applies the destruction events
//! with the edited table immediately, so the new reveals are visible without
//! saving.

use egui::{Context, DragValue, Grid, RichText, ScrollArea};

use super::{StartPlayer, UiWorldEditor};

impl UiWorldEditor {
    pub(super) fn reveal_list_editor_window(&mut self, ctx: &Context) {
        if !self.show_reveal_list_editor {
            return;
        }
        let mut open = self.show_reveal_list_editor;
        egui::Window::new("Edit Reveal Tile List").open(&mut open).resizable(true).default_size([520.0, 560.0]).show(
            ctx,
            |ui| {
                ui.label(
                    "Which layer-1 tiles an event reveals into which other tiles when it fires. \
                     Saved with Ctrl+S.",
                );
                ui.small("Row 21 (the switch-palace entry) also writes the tile after the event's offset.");
                ui.separator();

                let (tile_offsets, layer1_tiles, reveal) = (
                    self.rom.overworld_events.tile_offsets.clone(),
                    self.edit_state.read(|s| s.layer1_tiles.clone()),
                    self.edit_state.read(|s| s.reveal_list.clone()),
                );

                let mut changed_row: Option<usize> = None;
                let mut new_before = reveal.before.clone();
                let mut new_after = reveal.after.clone();
                ScrollArea::vertical().show(ui, |ui| {
                    Grid::new("reveal_list_grid").num_columns(4).spacing([10.0, 4.0]).striped(true).show(ui, |ui| {
                        ui.strong("Row");
                        ui.strong("Before");
                        ui.strong("After");
                        ui.strong("Used by");
                        ui.end_row();

                        for idx in 0..smwe_rom::overworld::reveal_list::REVEAL_COUNT {
                            let using = reveal.events_using_row(idx, &tile_offsets, &layer1_tiles);
                            ui.monospace(format!("{idx:2}"));
                            let mut b = new_before.get(idx).copied().unwrap_or(0);
                            if ui.add(DragValue::new(&mut b).range(0..=0xFFu8).hexadecimal(2, false, true)).changed() {
                                if let Some(slot) = new_before.get_mut(idx) {
                                    *slot = b;
                                }
                                changed_row = Some(idx);
                            }
                            let mut a = new_after.get(idx).copied().unwrap_or(0);
                            if ui.add(DragValue::new(&mut a).range(0..=0xFFu8).hexadecimal(2, false, true)).changed() {
                                if let Some(slot) = new_after.get_mut(idx) {
                                    *slot = a;
                                }
                                changed_row = Some(idx);
                            }
                            if using.is_empty() {
                                ui.label(RichText::new("unused").weak());
                            } else {
                                ui.label(format!("{} event{}", using.len(), if using.len() == 1 { "" } else { "s" }))
                                    .on_hover_text(format!(
                                        "Events whose tile matches {:#04X}: {}",
                                        new_before.get(idx).copied().unwrap_or(0),
                                        using.iter().map(|e| e.to_string()).collect::<Vec<_>>().join(", ")
                                    ));
                            }
                            ui.end_row();
                        }
                    });
                });

                if changed_row.is_some() {
                    self.edit_state.write(|s| {
                        s.reveal_list.before = new_before;
                        s.reveal_list.after = new_after;
                    });
                    self.reveal_list_dirty = true;
                    self.has_edits = true;
                    // Re-apply the destruction events with the edited table so
                    // the preview shows the new reveals immediately.
                    self.refresh_event_preview();
                }

                ui.separator();
                ui.weak(
                    "The lists are stored in place at $04DA1D/$04DA33 (22 bytes each). \
                     The preview above re-applies every active event with the edited table.",
                );
            },
        );
        self.show_reveal_list_editor = open;
    }

    /// Re-apply the destruction events onto the composed map preview using
    /// the reveal list currently in the edit state (which may include
    /// unsaved dialog edits). Mirrors what the emulated game init does in
    /// `load_submap`, but with the edited table instead of the ROM bytes the
    /// emulator was constructed with — the emulated ROM image is immutable,
    /// so this is the only way to preview reveal-list edits live.
    pub(super) fn refresh_event_preview(&mut self) {
        use smwe_rom::overworld::OverworldEvents;
        let (tiles, reveal) = self.edit_state.read(|s| (s.layer1_tiles.clone(), s.reveal_list.clone()));
        let mut revealed = tiles;
        let events = OverworldEvents {
            tile_offsets:  self.rom.overworld_events.tile_offsets.clone(),
            reveal_before: reveal.before.clone(),
            reveal_after:  reveal.after.clone(),
        };
        events.apply(&mut revealed, &self.active_events);
        let offset = if self.submap == 0 { 0usize } else { 0x400 };
        let n = revealed.len().saturating_sub(offset).min(0x400);
        for idx in 0..n {
            let col = (idx % 32) as u32;
            let row = (idx / 32) as u32;
            self.write_source_l1_block_words(col, row, revealed[offset + idx]);
        }
        self.upload_tiles_from_vram();
    }

    /// Left-panel "Starting Positions" section: Mario's (LM v1.60) and
    /// Luigi's (v1.90) overworld starting positions from the `$009EF0` table.
    /// Tile X/Y are 16×16-tile coordinates on the main map; the pixel
    /// coordinates shown are derived (`pixel = tile * 16 + 8`, `tile = pixel
    /// >> 4`, the game's own invariant). "Place on map" arms click-to-place
    /// on the main-map canvas.
    pub(super) fn start_position_panel(&mut self, ui: &mut egui::Ui) {
        ui.collapsing("Starting Positions (Mario/Luigi)", |ui| {
            ui.small("Where each player appears on a new game ($009EF0). Saved with Ctrl+S.");
            for player in [StartPlayer::Mario, StartPlayer::Luigi] {
                self.start_position_row(ui, player);
            }
            if self.submap != 0 {
                ui.small("Markers and click-to-place are on the main map — switch the submap to 0 to use them.");
            }
        });
    }

    fn start_position_row(&mut self, ui: &mut egui::Ui, player: StartPlayer) {
        let name = match player {
            StartPlayer::Mario => "Mario",
            StartPlayer::Luigi => "Luigi",
        };
        let cur = self.edit_state.read(|s| match player {
            StartPlayer::Mario => s.start_positions.mario,
            StartPlayer::Luigi => s.start_positions.luigi,
        });
        ui.separator();
        ui.label(RichText::new(name).strong());

        let mut submap = cur.submap.min(smwe_rom::overworld::SUBMAP_COUNT as u8 - 1);
        let label = smwe_rom::overworld::SUBMAP_NAMES.get(submap as usize).copied().unwrap_or("???");
        egui::ComboBox::from_id_salt(("start_pos_submap", name)).selected_text(format!("{submap} — {label}")).show_ui(
            ui,
            |ui| {
                for (i, sub_name) in smwe_rom::overworld::SUBMAP_NAMES.iter().enumerate() {
                    ui.selectable_value(&mut submap, i as u8, format!("{i} — {sub_name}"));
                }
            },
        );
        let mut tile_x = cur.tile_x.min(31) as i32;
        let mut tile_y = cur.tile_y.min(31) as i32;
        let mut dirty = false;
        ui.horizontal(|ui| {
            ui.label("Tile X");
            if ui.add(DragValue::new(&mut tile_x).range(0..=31)).changed() {
                dirty = true;
            }
            ui.label("Tile Y");
            if ui.add(DragValue::new(&mut tile_y).range(0..=31)).changed() {
                dirty = true;
            }
        });
        ui.monospace(format!("Pixel: ({}, {})", cur.pixel_x, cur.pixel_y));
        if submap != cur.submap || dirty {
            self.edit_state.write(|s| {
                let slot = match player {
                    StartPlayer::Mario => &mut s.start_positions.mario,
                    StartPlayer::Luigi => &mut s.start_positions.luigi,
                };
                slot.submap = submap;
                if dirty {
                    slot.set_tile(tile_x as u16, tile_y as u16);
                }
            });
            self.start_positions_dirty = true;
            self.has_edits = true;
        }

        // Click-to-place on the main map.
        let armed = self.place_start_target == Some(player);
        let can_place = self.submap == 0;
        let place_label = if armed { format!("Placing {name}… (click map)") } else { format!("Place {name} on map") };
        let mut btn = egui::Button::new(&place_label);
        if armed {
            btn = btn.fill(egui::Color32::from_rgb(70, 130, 200));
        }
        if ui.add_enabled(can_place, btn).clicked() {
            self.place_start_target = if armed { None } else { Some(player) };
        }
        if !can_place {
            ui.small("needs the main map (submap 0)");
        }
    }

    /// Draw the Mario (red "M") and Luigi (green "L") start markers on the
    /// main map, using the same canvas basis as the sprite markers
    /// (`origin`, `z`, `visible_map_crop`).
    pub(super) fn ow_draw_start_markers(&self, painter: &egui::Painter, origin: egui::Pos2, z: f32) {
        if self.submap != 0 {
            return;
        }
        let (crop_x, crop_y) = super::visible_map_crop(self.submap);
        let font = egui::FontId::monospace(11.0);
        let positions = self
            .edit_state
            .read(|s| [(StartPlayer::Mario, s.start_positions.mario), (StartPlayer::Luigi, s.start_positions.luigi)]);
        for (player, pos) in positions {
            let center =
                origin + egui::vec2((pos.pixel_x as f32 - crop_x as f32) * z, (pos.pixel_y as f32 - crop_y as f32) * z);
            let (letter, color) = match player {
                StartPlayer::Mario => ("M", egui::Color32::from_rgb(255, 90, 90)),
                StartPlayer::Luigi => ("L", egui::Color32::from_rgb(90, 230, 120)),
            };
            let armed = self.place_start_target == Some(player);
            painter.circle(
                center,
                10.0,
                egui::Color32::from_black_alpha(150),
                egui::Stroke::new(if armed { 3.0_f32 } else { 2.0 }, color),
            );
            painter.text(center, egui::Align2::CENTER_CENTER, letter, font.clone(), color);
        }
    }

    /// Click-to-place handling for start markers: while armed and on the main
    /// map in Select mode, a canvas click moves that player's start to the
    /// clicked 16×16 tile instead of selecting a tile. Returns true when the
    /// click was consumed.
    pub(super) fn ow_place_start_on_click(&mut self, map16_x: u32, map16_y: u32) -> bool {
        let Some(player) = self.place_start_target else { return false };
        if self.submap != 0 || map16_x > 31 || map16_y > 31 {
            return false;
        }
        self.edit_state.write(|s| {
            let slot = match player {
                StartPlayer::Mario => &mut s.start_positions.mario,
                StartPlayer::Luigi => &mut s.start_positions.luigi,
            };
            slot.set_tile(map16_x as u16, map16_y as u16);
        });
        self.start_positions_dirty = true;
        self.has_edits = true;
        self.place_start_target = None;
        true
    }
}
