//! Lunar Magic **"Edit Manual"** dialog (v1.91 parity).
//!
//! Edit menu → "Edit Manual" (also Alt+Right-click on an object/sprite):
//! manually modify the selected object or sprite at the byte level, including
//! the extension fields of multibyte objects/sprites (v1.80).
//!
//! The byte codec lives in `crate::edit_manual` — the dialog and the headless
//! screenshot binary share it, so the two can never drift. Editing writes
//! through the undoable layer/sprite data (one undo step per Apply), exactly
//! like the other level-editor edits.

use egui::{Color32, Context, RichText};

use super::UiLevelEditor;
use crate::{custom_tooltips::ObjectKind, edit_manual};

/// Which selected item the Edit Manual dialog edits.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub(super) enum EditManualTarget {
    Object(usize),
    Sprite(usize),
}

impl UiLevelEditor {
    /// Open the Edit Manual dialog for the current single selection
    /// (one object or one sprite). Returns false when nothing suitable is
    /// selected.
    pub(super) fn open_edit_manual(&mut self) -> bool {
        let vertical = self.level_properties.is_vertical;
        let (target, bytes) = if self.edit_sprites {
            if self.selected_sprite_indices.len() != 1 {
                self.mwl_status = Some("Edit Manual needs exactly one selected sprite".to_string());
                return false;
            }
            let i = *self.selected_sprite_indices.iter().next().expect("len == 1");
            match self.sprites.read(|s| s.stream_bytes_for_index(i, vertical)) {
                Some(b) => (EditManualTarget::Sprite(i), b),
                None => {
                    self.mwl_status = Some("Couldn't read the sprite's stream bytes".to_string());
                    return false;
                }
            }
        } else {
            if self.selected_object_indices.len() != 1 {
                self.mwl_status = Some("Edit Manual needs exactly one selected object".to_string());
                return false;
            }
            let i = *self.selected_object_indices.iter().next().expect("len == 1");
            let Some(layer_data) = self.editing_objects() else {
                self.mwl_status = Some("Edit Manual needs the Layer 1 object list".to_string());
                return false;
            };
            match layer_data.read(|l| l.stream_bytes_for_index(i, vertical)) {
                Some(b) => (EditManualTarget::Object(i), b),
                None => {
                    self.mwl_status = Some("Couldn't read the object's stream bytes".to_string());
                    return false;
                }
            }
        };
        self.edit_manual_target = Some(target);
        self.edit_manual_bytes = bytes.map(|b| format!("{b:02X}"));
        self.edit_manual_error = None;
        self.show_edit_manual = true;
        true
    }

    /// Title for the dialog window, e.g. "Edit Manual — Object 0x2B".
    fn edit_manual_title(&self) -> String {
        match self.edit_manual_target {
            Some(EditManualTarget::Object(i)) => {
                let id = self
                    .editing_objects()
                    .and_then(|l| {
                        l.read(|l| l.objects.get(i).map(|o| if o.is_extended { o.extended_id } else { o.id }))
                    })
                    .unwrap_or(0);
                format!("✏️  Edit Manual — Object 0x{id:02X}")
            }
            Some(EditManualTarget::Sprite(i)) => {
                let id = self.sprites.read(|s| s.sprites.get(i).map(|s| s.sprite_id)).unwrap_or(0);
                format!("✏️  Edit Manual — Sprite 0x{id:02X}")
            }
            None => "✏️  Edit Manual".to_string(),
        }
    }

    /// Parse the three hex fields. The same function feeds the dialog, the
    /// Apply path, and the headless screenshot mock.
    fn edit_manual_parsed(&self) -> Result<[u8; 3], String> {
        Ok([
            edit_manual::parse_hex_byte(&self.edit_manual_bytes[0])?,
            edit_manual::parse_hex_byte(&self.edit_manual_bytes[1])?,
            edit_manual::parse_hex_byte(&self.edit_manual_bytes[2])?,
        ])
    }

    /// Decoded-field summary shown live under the byte fields, plus
    /// validation. For objects the bytes must still decode as the same entry
    /// kind (standard ↔ extended); exit and screen-jump byte patterns are
    /// refused (exits are edited via the Secondary Entrances editor).
    fn edit_manual_decoded_summary(&self, bytes: [u8; 3]) -> Result<String, String> {
        match self.edit_manual_target {
            Some(EditManualTarget::Object(i)) => {
                let is_extended =
                    self.editing_objects().and_then(|l| l.read(|l| l.objects.get(i).map(|o| o.is_extended)));
                let Some(is_extended) = is_extended else {
                    return Err("The selected object is gone".to_string());
                };
                edit_manual::object_decoded_summary(bytes, is_extended)
            }
            Some(EditManualTarget::Sprite(_)) => Ok(edit_manual::sprite_decoded_summary(bytes)),
            None => Err("No selection".to_string()),
        }
    }

    /// Write the parsed bytes into the selection: one undoable write to the
    /// object layer or sprite list, like every other level-editor edit.
    fn edit_manual_apply(&mut self, bytes: [u8; 3]) -> Result<String, String> {
        let vertical = self.level_properties.is_vertical;
        match self.edit_manual_target {
            Some(EditManualTarget::Object(i)) => {
                // Decode + validate first, outside the write.
                let summary = self.edit_manual_decoded_summary(bytes)?;
                let d = edit_manual::decode_object_bytes(bytes);
                let Some(layer_data) = self.editing_objects() else {
                    return Err("No editable object layer".to_string());
                };
                let (screen, is_extended) = layer_data
                    .read(|l| {
                        l.objects
                            .get(i)
                            .and_then(|o| o.screen_and_local_coords(vertical).ok().map(|(s, _, _)| (s, o.is_extended)))
                    })
                    .ok_or_else(|| "The selected object is gone".to_string())?;
                // The decoded fields are local to the object's screen; the
                // screen itself comes from the object's current position.
                let (x, y) = edit_manual::object_absolute_coords(&d, u32::from(screen), vertical);
                self.editing_objects_mut().expect("editable object layer missing").write(|l| {
                    if let Some(obj) = l.objects.get_mut(i) {
                        obj.x = x;
                        obj.y = y;
                        if is_extended {
                            obj.extended_id = d.id;
                        } else {
                            obj.id = d.id;
                            obj.settings = d.settings;
                        }
                    }
                });
                self.mark_edited();
                self.rebuild_tiles();
                Ok(format!("Object bytes → {summary}"))
            }
            Some(EditManualTarget::Sprite(i)) => {
                let d = edit_manual::decode_sprite_bytes(bytes);
                let (x, y) = edit_manual::sprite_absolute_coords(d.screen, d.x_tile, d.y_tile, vertical);
                self.sprites.write(|s| {
                    if let Some(spr) = s.sprites.get_mut(i) {
                        spr.x = x;
                        spr.y = y;
                        spr.sprite_id = d.sprite_id;
                        spr.extra_bits = d.extra_bits;
                    }
                });
                self.mark_edited();
                self.rebuild_sprite_tiles();
                Ok(format!("Sprite bytes → 0x{:02X} at tile ({x}, {y})", d.sprite_id))
            }
            None => Err("No selection".to_string()),
        }
    }

    /// Reset the byte fields to the selection's current stream bytes.
    fn edit_manual_reset(&mut self) {
        let vertical = self.level_properties.is_vertical;
        let bytes = match self.edit_manual_target {
            Some(EditManualTarget::Object(i)) => {
                self.editing_objects().and_then(|l| l.read(|l| l.stream_bytes_for_index(i, vertical)))
            }
            Some(EditManualTarget::Sprite(i)) => self.sprites.read(|s| s.stream_bytes_for_index(i, vertical)),
            None => None,
        };
        if let Some(b) = bytes {
            self.edit_manual_bytes = b.map(|b| format!("{b:02X}"));
            self.edit_manual_error = None;
        }
    }

    pub(super) fn edit_manual_window(&mut self, ctx: &Context) {
        if !self.show_edit_manual {
            return;
        }
        let mut open = self.show_edit_manual;
        let title = self.edit_manual_title();
        egui::Window::new(title).open(&mut open).resizable(true).default_size([520.0, 320.0]).show(ctx, |ui| {
            ui.label(
                "Edit the selected entry's raw level-data bytes, Lunar Magic style. \
                          Type 1–2 hex digits per byte ($ or 0x prefix accepted).",
            );
            // The object's custom tooltip (Lunar Magic v3.60), if the user
            // set one — managed in the 💬 Custom Object Tooltips window.
            if let Some(EditManualTarget::Object(i)) = self.edit_manual_target {
                let kind_id = self.editing_objects().and_then(|l| {
                    l.read(|l| {
                        l.objects.get(i).map(|o| {
                            let kind = if o.is_extended { ObjectKind::Extended } else { ObjectKind::Standard };
                            (kind, if o.is_extended { o.extended_id } else { o.id })
                        })
                    })
                });
                if let Some((kind, id)) = kind_id {
                    if let Some(tip) = self.custom_tooltips.get(kind, id) {
                        ui.small(format!("💬 {tip}"))
                            .on_hover_text("Custom tooltip — edit it in the 💬 Custom Object Tooltips window");
                    }
                }
            }
            ui.small(
                "The new-screen flag (bit 7 of byte 0) is stream layout — \
                          it is recomputed when the level is saved.",
            );
            ui.separator();

            // ── Byte fields ──
            ui.horizontal(|ui| {
                for (i, label) in ["Byte 0", "Byte 1", "Byte 2"].iter().enumerate() {
                    ui.vertical(|ui| {
                        ui.small(*label);
                        let resp = ui.add(
                            egui::TextEdit::singleline(&mut self.edit_manual_bytes[i])
                                .font(egui::TextStyle::Monospace)
                                .desired_width(64.0),
                        );
                        if resp.changed() {
                            self.edit_manual_error = None;
                        }
                    });
                }
            });

            // ── Live decoded readout ──
            ui.separator();
            match self.edit_manual_parsed() {
                Ok(bytes) => match self.edit_manual_decoded_summary(bytes) {
                    Ok(summary) => {
                        ui.label(RichText::new(format!("Decodes as: {summary}")).monospace());
                    }
                    Err(e) => {
                        ui.label(RichText::new(e).color(Color32::from_rgb(255, 120, 120)));
                    }
                },
                Err(e) => {
                    ui.label(RichText::new(format!("Waiting for valid bytes: {e}")).color(Color32::GRAY));
                }
            }

            if let Some(err) = &self.edit_manual_error {
                ui.label(RichText::new(err).color(Color32::from_rgb(255, 120, 120)));
            }

            ui.separator();
            ui.horizontal(|ui| {
                // Apply is only enabled while the bytes parse AND decode
                // to the right entry kind.
                let can_apply = self.edit_manual_parsed().is_ok_and(|b| self.edit_manual_decoded_summary(b).is_ok());
                if ui.add_enabled(can_apply, egui::Button::new("✅ Apply")).clicked() {
                    match self.edit_manual_parsed().and_then(|b| self.edit_manual_apply(b)) {
                        Ok(msg) => {
                            self.mwl_status = Some(msg);
                            self.show_edit_manual = false;
                        }
                        Err(e) => self.edit_manual_error = Some(e),
                    }
                }
                if ui.button("↺ Reset").clicked() {
                    self.edit_manual_reset();
                }
                if ui.button("Close").clicked() {
                    self.show_edit_manual = false;
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.small("(LM v1.91 — Alt+Right-click also opens this)");
                });
            });
        });
        self.show_edit_manual = open;
    }
}
