use egui::{Context, ScrollArea, Slider};
use smwe_rom::message_boxes::{MESSAGE_BOXES_MAX_SIZE, MESSAGE_NAMES};

use super::UiLevelEditor;

/// Editor for SMW's vanilla "destruction event" message box text: 22 global
/// messages, each a sequence of raw font-tile-index bytes (0x00-0x7F; bit 7
/// is reserved by the game as a repeat/hold flag, so this editor doesn't let
/// users set it).
///
/// The read-only preview pane at the bottom runs the selected message through
/// the REAL game routine (`CODE_05B1BC`, verified in SMWDisX `bank_05.asm`)
/// on a scratch CPU clone and captures the dynamic stripe image it appends
/// to WRAM: 8 rows × 18 tile words (`$39TT` = tiles $100-$17F, palette 6).
/// Pixel rasterization of that stripe is pending real-ROM verification (the
/// routine has never executed here — no ROM). The readable-text line needs
/// the empirically derived font map (`smwe_rom::font_map`), also ROM-gated.
///
/// Edits are global (every level shares the same 22 messages) and size-
/// constrained: the vanilla ROM already uses the full byte budget, so making
/// one message longer requires shrinking another (see
/// `smwe_rom::message_boxes` module docs for why this data isn't repointable).
impl UiLevelEditor {
    pub(super) fn message_editor_window(&mut self, ctx: &Context) {
        if !self.show_message_editor {
            return;
        }
        let mut open = self.show_message_editor;
        egui::Window::new("Message Box Editor").open(&mut open).resizable(true).default_size([520.0, 420.0]).show(
            ctx,
            |ui| {
                ui.label("Raw font-tile-index bytes (0x00-0x7F) — no readable-text preview yet.");
                let total = self.message_boxes.total_size();
                let over_budget = total > MESSAGE_BOXES_MAX_SIZE;
                let color = if over_budget {
                    egui::Color32::from_rgb(220, 60, 60)
                } else if total == MESSAGE_BOXES_MAX_SIZE {
                    egui::Color32::from_rgb(220, 160, 60)
                } else {
                    ui.style().visuals.text_color()
                };
                ui.colored_label(color, format!("Total: {total} / {MESSAGE_BOXES_MAX_SIZE} bytes"));
                if total == MESSAGE_BOXES_MAX_SIZE {
                    ui.small(
                        "Vanilla already uses the full budget — lengthening one message requires shortening another.",
                    );
                }
                ui.separator();

                ui.horizontal(|ui| {
                    ScrollArea::vertical().max_height(300.0).id_salt("message_list").show(ui, |ui| {
                        for (i, name) in MESSAGE_NAMES.iter().enumerate() {
                            let label = format!("{name} ({} B)", self.message_boxes.messages[i].len());
                            ui.selectable_value(&mut self.message_editor_selected, i, label);
                        }
                    });

                    ui.separator();

                    ui.vertical(|ui| {
                        let i = self.message_editor_selected;
                        ui.label(format!("Editing: {}", MESSAGE_NAMES[i]));

                        ui.horizontal(|ui| {
                            if ui.button("+ Byte").clicked() {
                                self.message_boxes.messages[i].push(0x1F); // 0x1F = vanilla space code
                                self.message_boxes_dirty = true;
                                self.has_edits = true;
                            }
                            if ui.button("- Byte").clicked() && !self.message_boxes.messages[i].is_empty() {
                                self.message_boxes.messages[i].pop();
                                self.message_boxes_dirty = true;
                                self.has_edits = true;
                            }
                        });

                        ScrollArea::vertical().max_height(300.0).id_salt("message_bytes").show(ui, |ui| {
                            egui::Grid::new("message_byte_grid").num_columns(8).spacing([4.0, 4.0]).show(ui, |ui| {
                                let mut changed = false;
                                for (byte_i, byte) in self.message_boxes.messages[i].iter_mut().enumerate() {
                                    let mut v = *byte as i32;
                                    if ui.add(Slider::new(&mut v, 0..=0x7F).hexadecimal(2, false, false)).changed() {
                                        *byte = v as u8;
                                        changed = true;
                                    }
                                    if (byte_i + 1) % 8 == 0 {
                                        ui.end_row();
                                    }
                                }
                                if changed {
                                    self.message_boxes_dirty = true;
                                    self.has_edits = true;
                                }
                            });
                        });

                        ui.separator();
                        ui.label("Preview (read-only)");

                        // Readable-text preview via the empirically derived font map.
                        match &self.message_font_map {
                            Some(map) => {
                                // Control codes are identified during real-ROM
                                // derivation; none are known yet.
                                ui.label(format!(
                                    "Text: {}",
                                    map.to_text(&self.message_boxes.messages[i], &[])
                                ));
                            }
                            None => {
                                ui.small(
                                    "Readable-text preview needs the font map — derive it \
                                     with a ROM (see smwe_rom::font_map).",
                                );
                            }
                        }

                        // Pixel preview: run the real CODE_05B1BC on a scratch CPU
                        // clone and capture the dynamic stripe image it appends
                        // to WRAM. Rasterizing that stripe into pixels is pending
                        // real-ROM verification (see smwe_emu::emu::render_message).
                        let slot = smwe_rom::message_boxes::pointer_slot_for_message(i);
                        if self.message_preview_for != Some(i) {
                            let mut scratch = self.cpu.clone();
                            self.message_preview =
                                Some(smwe_emu::emu::render_message(&mut scratch, slot));
                            self.message_preview_for = Some(i);
                        }
                        if let Some(stripe) = &self.message_preview {
                            ui.small(format!(
                                "CODE_05B1BC ran ({} cycles): captured {} stripe bytes. \
                                 Tilemap rasterization pending real-ROM verification.",
                                stripe.cycles,
                                stripe.stripe.len()
                            ));
                        }
                    });
                });
            },
        );
        self.show_message_editor = open;
    }
}
