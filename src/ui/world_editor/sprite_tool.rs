//! Overworld sprite editing tool — Lunar Magic overworld sprite mode parity.
//!
//! Covers the vanilla sprite table (`$04F625`, 13 fixed slots) including the
//! per-sprite-number map visibility bytes (`$04F828`, indexed by sprite
//! number — a set bit means *inactive*, see `smwe_rom::overworld::sprites`),
//! plus custom sprite insertion/storage through the `$0EF55D` pointer using
//! the LM v2.50 entry concepts (7 submap lists, 7-bit number, 6-bit 8px X/Y,
//! 5-bit height, variable extra bytes) inside a smw-editor-owned RATS block —
//! see `smwe_rom::overworld::sprites` for the exact on-disk story. Custom
//! sprite *behavior* needs a third-party runtime patch (e.g. PIXI);
//! smw-editor only edits and stores the data.

use egui::{vec2, Color32, Pos2, Rect, RichText, Stroke, Ui};
use smwe_rom::overworld::sprites as ow_sprites;

use super::{visible_map_crop, OwSpriteRef, UiWorldEditor};
use crate::ui::style::toggle_button;

/// Sprite marker radius, screen pixels.
const MARKER_R: f32 = 9.0;
/// Marker hit-test radius, screen pixels.
const HIT_R: f32 = 14.0;

impl UiWorldEditor {
    /// Toggle button for the sprite tool, drawn in the left panel's mode
    /// toolbar row.
    pub(super) fn ow_sprite_tool_toggle(&mut self, ui: &mut Ui) {
        if toggle_button(ui, "Sprites [4]", self.ow_sprite_tool) {
            self.ow_sprite_tool = !self.ow_sprite_tool;
            if self.ow_sprite_tool {
                // Tile interactions are inert while the sprite tool is up.
                self.ow_sprite_error = None;
            } else {
                self.ow_sprite_selection = None;
                self.ow_sprite_drag = None;
                self.ow_extra_hex.clear();
            }
        }
    }

    /// `(ref, x_px, y_px)` of every sprite shown on the current submap, in
    /// game pixels relative to the full 512×512 map. Vanilla sprites appear
    /// only when active on this submap (per-sprite-number visibility).
    fn ow_sprites_on_submap(&self) -> Vec<(OwSpriteRef, i32, i32)> {
        let submap = self.submap;
        self.edit_state.read(|s| {
            let mut out = Vec::new();
            for (slot, sprite) in s.vanilla_sprites.sprites.iter().enumerate() {
                if s.vanilla_sprites.is_active_on(sprite.number, submap) {
                    out.push((OwSpriteRef::Vanilla(slot), sprite.x_px() as i32, sprite.y_px() as i32));
                }
            }
            for (index, entry) in s.custom_sprites.submaps[submap as usize].iter().enumerate() {
                out.push((
                    OwSpriteRef::Custom { submap: submap as usize, index },
                    entry.x_px() as i32,
                    entry.y_px() as i32,
                ));
            }
            out
        })
    }

    /// Map-pixel position of a sprite ref, or `None` when the ref is stale
    /// (e.g. the sprite was deleted, then the selection survived an undo).
    fn ow_sprite_pos(&self, sprite_ref: OwSpriteRef) -> Option<(i32, i32)> {
        self.edit_state.read(|s| match sprite_ref {
            OwSpriteRef::Vanilla(slot) => {
                s.vanilla_sprites.sprites.get(slot).map(|sp| (sp.x_px() as i32, sp.y_px() as i32))
            }
            OwSpriteRef::Custom { submap, index } => {
                s.custom_sprites.submaps[submap].get(index).map(|e| (e.x_px() as i32, e.y_px() as i32))
            }
        })
    }

    fn ow_sprite_number(&self, sprite_ref: OwSpriteRef) -> Option<u8> {
        self.edit_state.read(|s| match sprite_ref {
            OwSpriteRef::Vanilla(slot) => s.vanilla_sprites.sprites.get(slot).map(|sp| sp.number),
            OwSpriteRef::Custom { submap, index } => s.custom_sprites.submaps[submap].get(index).map(|e| e.number),
        })
    }

    /// Move a sprite to a map-pixel position. Vanilla sprites keep pixel
    /// precision (negative values wrap, matching the vanilla table);
    /// custom sprites snap to 8×8 units. A single undo step; no-ops are
    /// skipped so plain clicks don't pollute the undo stack.
    fn ow_commit_sprite_pos(&mut self, sprite_ref: OwSpriteRef, x_px: i32, y_px: i32) {
        enum Target {
            Vanilla(u16, u16),
            Custom(u8, u8),
        }
        let target = match sprite_ref {
            OwSpriteRef::Vanilla(_) => {
                Target::Vanilla((x_px.clamp(-64, 576) as i16) as u16, (y_px.clamp(-64, 576) as i16) as u16)
            }
            OwSpriteRef::Custom { .. } => {
                Target::Custom(((x_px + 4) / 8).clamp(0, 63) as u8, ((y_px + 4) / 8).clamp(0, 63) as u8)
            }
        };
        let same = self.edit_state.read(|s| match (sprite_ref, &target) {
            (OwSpriteRef::Vanilla(slot), Target::Vanilla(nx, ny)) => {
                s.vanilla_sprites.sprites.get(slot).is_some_and(|sp| sp.x == *nx && sp.y == *ny)
            }
            (OwSpriteRef::Custom { submap, index }, Target::Custom(nx, ny)) => {
                s.custom_sprites.submaps[submap].get(index).is_some_and(|e| e.x == *nx && e.y == *ny)
            }
            _ => false,
        });
        if same {
            return;
        }
        self.edit_state.write(|s| match (sprite_ref, target) {
            (OwSpriteRef::Vanilla(slot), Target::Vanilla(nx, ny)) => {
                if let Some(sp) = s.vanilla_sprites.sprites.get_mut(slot) {
                    sp.x = nx;
                    sp.y = ny;
                }
            }
            (OwSpriteRef::Custom { submap, index }, Target::Custom(nx, ny)) => {
                if let Some(e) = s.custom_sprites.submaps[submap].get_mut(index) {
                    e.x = nx;
                    e.y = ny;
                }
            }
            _ => {}
        });
        self.has_edits = true;
    }

    /// Insert a custom sprite on the current submap at map center. Selects
    /// the new sprite. Capped at the native 24-sprite container limit
    /// ([`MAX_CUSTOM_SPRITES_PER_SUBMAP`]); the LM v3.51 sprite record-size
    /// table configures per-sprite *record sizes*, not per-submap capacity.
    fn ow_insert_custom_sprite(&mut self) {
        let submap = self.submap as usize;
        let counts = self.edit_state.read(|s| s.custom_extra_counts);
        let number = 0x10u8;
        let inserted = self.edit_state.write(|s| {
            let list = &mut s.custom_sprites.submaps[submap];
            if list.len() >= ow_sprites::MAX_CUSTOM_SPRITES_PER_SUBMAP {
                return None;
            }
            list.push(ow_sprites::CustomOwSprite {
                number,
                x: 32,
                y: 32,
                height: 0,
                extra: vec![0u8; counts[number as usize] as usize],
            });
            Some(list.len() - 1)
        });
        match inserted {
            Some(index) => {
                self.has_edits = true;
                self.ow_sprite_selection = Some(OwSpriteRef::Custom { submap, index });
                self.ow_sync_extra_hex();
                self.ow_sprite_error = None;
            }
            None => {
                self.ow_sprite_error =
                    Some(format!("Submap is full ({} custom sprites max)", ow_sprites::MAX_CUSTOM_SPRITES_PER_SUBMAP));
            }
        }
    }

    /// Delete the selected custom sprite. Vanilla slots are fixed — like in
    /// Lunar Magic they can't be deleted.
    pub(super) fn ow_delete_selected(&mut self) {
        match self.ow_sprite_selection {
            Some(OwSpriteRef::Vanilla(_)) => {
                self.ow_sprite_error = Some(
                    "Vanilla sprites can't be deleted — the 13 slots are fixed, like in Lunar Magic. \
                     Set the number to 00 or move it off-map instead."
                        .to_string(),
                );
            }
            Some(OwSpriteRef::Custom { submap, index }) => {
                self.edit_state.write(|s| {
                    let list = &mut s.custom_sprites.submaps[submap];
                    if index < list.len() {
                        list.remove(index);
                    }
                });
                self.has_edits = true;
                self.ow_sprite_selection = None;
                self.ow_extra_hex.clear();
                self.ow_sprite_error = None;
            }
            None => {}
        }
    }

    /// Refresh the extra-bytes hex buffer from the current selection.
    fn ow_sync_extra_hex(&mut self) {
        self.ow_extra_hex = match self.ow_sprite_selection {
            Some(OwSpriteRef::Custom { submap, index }) => self.edit_state.read(|s| {
                s.custom_sprites.submaps[submap]
                    .get(index)
                    .map(|e| e.extra.iter().map(|b| format!("{b:02X}")).collect::<Vec<_>>().join(" "))
                    .unwrap_or_default()
            }),
            _ => String::new(),
        };
    }

    /// Parse the hex buffer into the selected custom sprite's extra bytes.
    /// Returns an error message on invalid input.
    fn ow_apply_extra_hex(&mut self) -> Option<String> {
        let sprite_ref = match self.ow_sprite_selection {
            Some(r @ OwSpriteRef::Custom { .. }) => r,
            _ => return Some("No custom sprite selected".to_string()),
        };
        let hex: String = self.ow_extra_hex.chars().filter(|c| !c.is_whitespace()).collect();
        if hex.len() % 2 != 0 {
            return Some("Hex needs an even number of digits".to_string());
        }
        let mut bytes = Vec::with_capacity(hex.len() / 2);
        for pair in hex.as_bytes().chunks(2) {
            let text = std::str::from_utf8(pair).unwrap_or("??");
            match u8::from_str_radix(text, 16) {
                Ok(b) => bytes.push(b),
                Err(_) => return Some(format!("Invalid hex byte: {text}")),
            }
        }
        // The byte count is fixed by the ROM's extra-byte table for this
        // sprite number (see the note under the field); anything else would
        // not round-trip through the undo payload.
        let number = self.ow_sprite_number(sprite_ref).unwrap_or(0);
        let want = self.edit_state.read(|s| s.custom_extra_counts[number as usize] as usize);
        if bytes.len() != want {
            return Some(format!("Need exactly {want} extra byte(s) for sprite {number:02X} (ROM extra-byte table)"));
        }
        let applied = self.edit_state.write(|s| match sprite_ref {
            OwSpriteRef::Custom { submap, index } => {
                if let Some(e) = s.custom_sprites.submaps[submap].get_mut(index) {
                    e.extra = bytes;
                    true
                } else {
                    false
                }
            }
            _ => false,
        });
        if applied {
            self.has_edits = true;
        }
        self.ow_sync_extra_hex();
        None
    }

    fn ow_sprite_list_label(&self, sprite_ref: OwSpriteRef, x: i32, y: i32) -> String {
        match sprite_ref {
            OwSpriteRef::Vanilla(_) => {
                let number = self.ow_sprite_number(sprite_ref).unwrap_or(0);
                format!("{number:02X} {} @ ({x}, {y})", ow_sprites::sprite_type_name(number))
            }
            OwSpriteRef::Custom { .. } => {
                let (number, height) = self
                    .edit_state
                    .read(|s| match sprite_ref {
                        OwSpriteRef::Custom { submap, index } => {
                            s.custom_sprites.submaps[submap].get(index).map(|e| (e.number, e.height))
                        }
                        _ => None,
                    })
                    .unwrap_or((0, 0));
                format!("custom {number:02X} h{height} @ ({x}, {y})")
            }
        }
    }

    /// Nearest marker within hit radius of a map-pixel point.
    fn ow_hit_test(&self, mx: f32, my: f32, origin: Pos2, z: f32) -> Option<OwSpriteRef> {
        let (crop_x, crop_y) = visible_map_crop(self.submap);
        let px = origin.x + (mx - crop_x as f32) * z;
        let py = origin.y + (my - crop_y as f32) * z;
        let mut best: Option<(f32, OwSpriteRef)> = None;
        for (sprite_ref, x_px, y_px) in self.ow_sprites_on_submap() {
            let sx = origin.x + (x_px as f32 - crop_x as f32) * z;
            let sy = origin.y + (y_px as f32 - crop_y as f32) * z;
            let d = ((sx - px).powi(2) + (sy - py).powi(2)).sqrt();
            if d <= HIT_R && best.is_none_or(|(bd, _)| d < bd) {
                best = Some((d, sprite_ref));
            }
        }
        best.map(|(_, r)| r)
    }

    /// Canvas pointer handling for the sprite tool: click selects, drag
    /// moves (one undo step on release).
    pub(super) fn ow_handle_canvas(&mut self, resp: &egui::Response, origin: Pos2, z: f32) {
        use egui::PointerButton::Primary;
        let (crop_x, crop_y) = visible_map_crop(self.submap);
        let pointer_map = resp.hover_pos().or_else(|| resp.interact_pointer_pos()).map(|p| {
            let rel = (p - origin) / z;
            (rel.x + crop_x as f32, rel.y + crop_y as f32)
        });

        if resp.drag_started_by(Primary) {
            if let Some((mx, my)) = pointer_map {
                if let Some(hit) = self.ow_hit_test(mx, my, origin, z) {
                    self.ow_sprite_selection = Some(hit);
                    self.ow_sync_extra_hex();
                    self.ow_sprite_error = None;
                    if let Some((sx, sy)) = self.ow_sprite_pos(hit) {
                        self.ow_sprite_drag = Some((hit, vec2(mx - sx as f32, my - sy as f32)));
                    }
                } else {
                    self.ow_sprite_selection = None;
                    self.ow_extra_hex.clear();
                }
            }
        } else if resp.clicked_by(Primary) {
            if let Some((mx, my)) = pointer_map {
                self.ow_sprite_selection = self.ow_hit_test(mx, my, origin, z);
                self.ow_sync_extra_hex();
                self.ow_sprite_error = None;
            }
        }

        if resp.drag_stopped_by(Primary) {
            if let Some((drag_ref, grab)) = self.ow_sprite_drag.take() {
                if let Some((mx, my)) = pointer_map {
                    self.ow_commit_sprite_pos(drag_ref, (mx - grab.x).round() as i32, (my - grab.y).round() as i32);
                }
            }
        }
    }

    /// Draw sprite markers over the map. Uses the same canvas basis as the
    /// GL render (`origin`, `z`, `visible_map_crop`).
    pub(super) fn ow_draw_markers(
        &self, painter: &egui::Painter, resp: &egui::Response, origin: Pos2, z: f32, view_rect: Rect,
    ) {
        let (crop_x, crop_y) = visible_map_crop(self.submap);
        let cull = view_rect.expand(24.0);
        let font = egui::FontId::monospace(10.0);
        for (sprite_ref, x_px, y_px) in self.ow_sprites_on_submap() {
            // Live drag preview: the marker follows the pointer.
            let (mut fx, mut fy) = (x_px as f32, y_px as f32);
            if let Some((drag_ref, grab)) = self.ow_sprite_drag {
                if drag_ref == sprite_ref {
                    if let Some(p) = resp.hover_pos().or_else(|| resp.interact_pointer_pos()) {
                        let rel = (p - origin) / z;
                        fx = rel.x + crop_x as f32 - grab.x;
                        fy = rel.y + crop_y as f32 - grab.y;
                    }
                }
            }
            let center = origin + vec2((fx - crop_x as f32) * z, (fy - crop_y as f32) * z);
            if !cull.contains(center) {
                continue;
            }
            let is_custom = matches!(sprite_ref, OwSpriteRef::Custom { .. });
            let selected = self.ow_sprite_selection == Some(sprite_ref);
            let base = if is_custom { Color32::from_rgb(80, 220, 255) } else { Color32::from_rgb(255, 214, 64) };
            painter.circle(
                center,
                MARKER_R,
                Color32::from_black_alpha(140),
                Stroke::new(if selected { 3.0_f32 } else { 2.0 }, if selected { Color32::WHITE } else { base }),
            );
            painter.circle_filled(center, 2.5, base);
            if let Some(number) = self.ow_sprite_number(sprite_ref) {
                painter.text(
                    center + vec2(MARKER_R + 3.0, -7.0),
                    egui::Align2::LEFT_TOP,
                    format!("{number:02X}"),
                    font.clone(),
                    if selected { Color32::WHITE } else { base },
                );
            }
        }
    }

    /// Left-panel sprite section: sprite list, insert, and the selected
    /// sprite's property editor.
    pub(super) fn ow_sprite_panel(&mut self, ui: &mut Ui) {
        let submap_name = smwe_rom::overworld::SUBMAP_NAMES.get(self.submap as usize).copied().unwrap_or("Submap");
        ui.heading(format!("Sprites — {submap_name}"));
        ui.add_space(2.0);
        ui.label(
            RichText::new(
                "Custom sprites need a third-party runtime patch (e.g. PIXI) to move and draw in-game — \
                 smw-editor stores the table, it doesn't run the sprites.",
            )
            .small()
            .italics(),
        );

        let foreign = self.edit_state.read(|s| s.foreign_custom_table);
        if foreign {
            ui.add_space(4.0);
            ui.colored_label(
                Color32::from_rgb(255, 170, 80),
                "This ROM's custom sprite table was written by another tool. Custom sprite editing is \
                 disabled so it isn't corrupted — vanilla sprites below still work.",
            );
        }

        ui.add_space(4.0);
        ui.label(RichText::new("On this submap:").strong());
        let entries = self.ow_sprites_on_submap();
        egui::ScrollArea::vertical().max_height(150.0).show(ui, |ui| {
            for (sprite_ref, x, y) in &entries {
                let label = self.ow_sprite_list_label(*sprite_ref, *x, *y);
                let selected = self.ow_sprite_selection == Some(*sprite_ref);
                if ui.selectable_label(selected, label).clicked() {
                    self.ow_sprite_selection = Some(*sprite_ref);
                    self.ow_sync_extra_hex();
                    self.ow_sprite_error = None;
                }
            }
            if entries.is_empty() {
                ui.label(RichText::new("No sprites on this submap.").weak());
            }
        });
        // Drop selections that no longer resolve (e.g. a custom sprite
        // removed, then the deletion undone/redone out from under it).
        if self.ow_sprite_selection.is_some_and(|r| self.ow_sprite_pos(r).is_none()) {
            self.ow_sprite_selection = None;
            self.ow_extra_hex.clear();
        }

        ui.add_space(4.0);
        ui.horizontal(|ui| {
            let custom_count = entries.iter().filter(|(r, _, _)| matches!(r, OwSpriteRef::Custom { .. })).count();
            let cap = ow_sprites::MAX_CUSTOM_SPRITES_PER_SUBMAP;
            let can_insert = !foreign && custom_count < cap;
            if ui.add_enabled(can_insert, egui::Button::new("Insert custom sprite")).clicked() {
                self.ow_insert_custom_sprite();
            }
            if foreign {
                ui.small("disabled: foreign table");
            } else if custom_count >= cap {
                ui.small(format!("max {cap} per submap"));
            } else {
                ui.small("adds at map center — drag it into place");
            }
        });
        ui.horizontal(|ui| {
            let custom_count = entries.iter().filter(|(r, _, _)| matches!(r, OwSpriteRef::Custom { .. })).count();
            ui.small(format!(
                "Custom sprites: {custom_count}/{} on this submap",
                ow_sprites::MAX_CUSTOM_SPRITES_PER_SUBMAP
            ));
            let btn = ui.add_enabled(!foreign, egui::Button::new("Sprite Sizes…"));
            if btn.clicked() {
                // Sync the draft from the ROM's size table (defaults when
                // the ROM has none), then show the dialog.
                self.ow_size_table_draft = self.edit_state.read(|s| {
                    s.sprite_size_table
                        .map(|t| t.sizes)
                        .unwrap_or([ow_sprites::DEFAULT_SPRITE_RECORD_SIZE; ow_sprites::SIZE_TABLE_LEN])
                });
                self.ow_size_table_open = true;
            }
            if foreign {
                ui.small("disabled: foreign table");
            } else {
                ui.small("record sizes (LM v3.51)");
            }
        });
        self.ow_sprite_size_table_dialog(ui.ctx());

        // ── Selected sprite editor ────────────────────────────────────
        let selection = self.ow_sprite_selection;
        match selection {
            Some(OwSpriteRef::Vanilla(slot)) => self.ow_vanilla_editor(ui, slot),
            Some(OwSpriteRef::Custom { submap, index }) => self.ow_custom_editor(ui, submap, index),
            None => {}
        }

        if let Some(err) = self.ow_sprite_error.clone() {
            ui.add_space(4.0);
            ui.colored_label(Color32::from_rgb(255, 120, 120), err);
        }
    }

    /// Property editor for one vanilla table slot.
    fn ow_vanilla_editor(&mut self, ui: &mut Ui, slot: usize) {
        let cur = self.edit_state.read(|s| s.vanilla_sprites.sprites[slot]);
        ui.separator();
        ui.label(RichText::new(format!("Vanilla sprite — slot {slot}")).strong());
        ui.small("13 fixed slots, like Lunar Magic: pick the sprite number, position, and which submaps show it.");

        let mut number = cur.number;
        let mut x = cur.x_px() as i32;
        let mut y = cur.y_px() as i32;
        let mut dirty = false;

        egui::ComboBox::from_label("Sprite")
            .selected_text(format!("{:02X} {}", number, ow_sprites::sprite_type_name(number)))
            .show_ui(ui, |ui| {
                for n in 0..=ow_sprites::MAX_VANILLA_SPRITE_NUMBER {
                    if ui
                        .selectable_value(&mut number, n, format!("{n:02X} {}", ow_sprites::sprite_type_name(n)))
                        .changed()
                    {
                        dirty = true;
                    }
                }
            });

        ui.horizontal(|ui| {
            ui.label("X");
            if ui.add(egui::DragValue::new(&mut x).range(-64..=576)).changed() {
                dirty = true;
            }
            ui.label("Y");
            if ui.add(egui::DragValue::new(&mut y).range(-64..=576)).changed() {
                dirty = true;
            }
        });

        if dirty {
            self.edit_state.write(|s| {
                if let Some(sp) = s.vanilla_sprites.sprites.get_mut(slot) {
                    sp.number = number;
                    sp.x = (x.clamp(-64, 576) as i16) as u16;
                    sp.y = (y.clamp(-64, 576) as i16) as u16;
                }
            });
            self.has_edits = true;
            // A number change can move the sprite between visibility groups.
            if number != cur.number {
                self.ow_sprite_error = None;
            }
        }

        ui.add_space(4.0);
        ui.label(RichText::new("Visible on:").strong());
        let number_now = self.edit_state.read(|s| s.vanilla_sprites.sprites[slot].number);
        if number_now == 0 {
            ui.small("Sprite 00 is always visible — the game reads its visibility byte from code, not the table.");
        }
        for map in 0..7u8 {
            let name = smwe_rom::overworld::SUBMAP_NAMES[map as usize];
            let mut on = self.edit_state.read(|s| s.vanilla_sprites.is_active_on(number_now, map));
            let resp = ui.add_enabled(number_now != 0, egui::Checkbox::new(&mut on, name));
            if resp.changed() {
                self.edit_state.write(|s| {
                    // Number 0 has no editable visibility byte; the checkbox
                    // is disabled above, so this is just a backstop.
                    let _ = s.vanilla_sprites.set_active_on(number_now, map, on);
                });
                self.has_edits = true;
            }
        }
    }

    /// Property editor for one custom sprite.
    fn ow_custom_editor(&mut self, ui: &mut Ui, submap: usize, index: usize) {
        let cur = match self.edit_state.read(|s| s.custom_sprites.submaps[submap].get(index).cloned()) {
            Some(e) => e,
            None => {
                self.ow_sprite_selection = None;
                return;
            }
        };
        ui.separator();
        ui.label(RichText::new(format!("Custom sprite #{index}")).strong());

        let mut number = cur.number;
        let mut x = cur.x;
        let mut y = cur.y;
        let mut height = cur.height;
        let mut dirty = false;

        ui.horizontal(|ui| {
            ui.label("Number");
            if ui.add(egui::DragValue::new(&mut number).range(0..=0x7Fu8).hexadecimal(2, false, true)).changed() {
                dirty = true;
            }
        });
        ui.horizontal(|ui| {
            ui.label("X (×8)");
            if ui.add(egui::DragValue::new(&mut x).range(0..=63u8)).changed() {
                dirty = true;
            }
            ui.label("Y (×8)");
            if ui.add(egui::DragValue::new(&mut y).range(0..=63u8)).changed() {
                dirty = true;
            }
            ui.label("Height");
            if ui.add(egui::DragValue::new(&mut height).range(0..=31u8)).changed() {
                dirty = true;
            }
        });
        if dirty {
            // The extra-byte count belongs to the sprite number: changing the
            // number resizes the extra bytes (like LM), keeping
            // `extra.len() == counts[number]` so the undo payload
            // (encode/decode with these counts) round-trips exactly.
            let counts = self.edit_state.read(|s| s.custom_extra_counts);
            self.edit_state.write(|s| {
                if let Some(e) = s.custom_sprites.submaps[submap].get_mut(index) {
                    if e.number != number {
                        e.number = number;
                        e.extra.resize(counts[number as usize] as usize, 0);
                    }
                    e.x = x;
                    e.y = y;
                    e.height = height;
                }
            });
            self.has_edits = true;
            self.ow_sync_extra_hex();
        }

        ui.add_space(2.0);
        ui.label("Extra bytes (hex):");
        let resp = ui.text_edit_singleline(&mut self.ow_extra_hex);
        if resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
            match self.ow_apply_extra_hex() {
                Some(err) => self.ow_sprite_error = Some(err),
                None => self.ow_sprite_error = None,
            }
        }
        ui.small("Applied on Enter. Byte count comes from the ROM's extra-byte table.");

        ui.add_space(4.0);
        if ui.button("Delete sprite").clicked() {
            self.ow_delete_selected();
        }
    }

    /// "Custom Overworld Sprite Record Sizes" dialog (LM v3.51 parity): one
    /// total record size per custom sprite number `01..=7F`, each `3..=15`
    /// (3 = the 3 fixed bytes only, no extra bytes; 15 = max; LM's default
    /// 4). Shows the derived extra-byte count next to each size.
    ///
    /// Apply is a single undo step: it stores the table, recomputes the
    /// extra-byte counts, and resizes every placed custom sprite's extra
    /// bytes to match (existing bytes are preserved). On a ROM without a
    /// size table, applying creates one (RATS-tagged free-space block,
    /// `$0DE18C` pointed at it, `$42` marker set).
    fn ow_sprite_size_table_dialog(&mut self, ctx: &egui::Context) {
        if !self.ow_size_table_open {
            return;
        }
        let has_table = self.edit_state.read(|s| s.sprite_size_table.is_some());
        let mut open = self.ow_size_table_open;
        let mut apply: Option<[u8; ow_sprites::SIZE_TABLE_LEN]> = None;
        let mut close = false;
        egui::Window::new("Custom Overworld Sprite Record Sizes")
            .open(&mut open)
            .resizable(true)
            .collapsible(false)
            .default_size([420.0, 480.0])
            .show(ctx, |ui| {
                ui.small(
                    "Total record size per custom sprite, like Lunar Magic v3.51. \
                    3 = fixed bytes only (no extra bytes), 15 = max, default 4. \
                    Entry N is sprite N (01–7F).",
                );
                ui.small(
                    "Lunar Magic only uses this table to parse the custom sprite list — \
                    like custom sprites themselves, the sizes do nothing in-game without \
                    a runtime patch.",
                );
                if !has_table {
                    ui.colored_label(
                        Color32::from_rgb(255, 200, 120),
                        "This ROM has no size table yet — Apply will create one.",
                    );
                }
                ui.add_space(4.0);
                egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                    for (i, size) in self.ow_size_table_draft.iter_mut().enumerate() {
                        let number = (i + 1) as u8;
                        ui.horizontal(|ui| {
                            ui.monospace(format!("Sprite {number:02X}"));
                            let mut v = *size as i32;
                            if ui
                                .add(
                                    egui::DragValue::new(&mut v)
                                        .range(
                                            ow_sprites::MIN_SPRITE_RECORD_SIZE as i32
                                                ..=ow_sprites::MAX_SPRITE_RECORD_SIZE as i32,
                                        )
                                        .prefix("total bytes: "),
                                )
                                .changed()
                            {
                                *size = (v as u8)
                                    .clamp(ow_sprites::MIN_SPRITE_RECORD_SIZE, ow_sprites::MAX_SPRITE_RECORD_SIZE);
                            }
                            let extra = size.saturating_sub(ow_sprites::MIN_SPRITE_RECORD_SIZE);
                            ui.small(format!("= {extra} extra {}", if extra == 1 { "byte" } else { "bytes" }));
                        });
                    }
                });
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    if ui.button("Apply").clicked() {
                        apply = Some(self.ow_size_table_draft);
                    }
                    if ui.button("Reset all to 4").clicked() {
                        self.ow_size_table_draft = [ow_sprites::DEFAULT_SPRITE_RECORD_SIZE; ow_sprites::SIZE_TABLE_LEN];
                    }
                    if ui.button("Close").clicked() {
                        close = true;
                    }
                });
            });
        self.ow_size_table_open = open && !close;
        if let Some(sizes) = apply {
            let table = ow_sprites::SpriteSizeTable { sizes };
            self.edit_state.write(|s| {
                s.sprite_size_table = Some(table);
                // Table entries are TOTAL record sizes; extra = size - 3.
                // Sprite 0 has no table entry and keeps the default.
                let mut counts = [ow_sprites::DEFAULT_EXTRA_BYTES as u8; 128];
                for (i, size) in sizes.iter().enumerate() {
                    counts[i + 1] = size.saturating_sub(ow_sprites::MIN_SPRITE_RECORD_SIZE);
                }
                s.custom_extra_counts = counts;
                // Resize every placed custom sprite's extra bytes to the new
                // counts, preserving existing bytes (pad with zero).
                for list in s.custom_sprites.submaps.iter_mut() {
                    for sprite in list.iter_mut() {
                        sprite.extra.resize(counts[(sprite.number & 0x7F) as usize] as usize, 0);
                    }
                }
            });
            self.has_edits = true;
            self.ow_size_table_open = false;
            self.ow_sprite_error = None;
            self.ow_sync_extra_hex();
        }
    }
}
