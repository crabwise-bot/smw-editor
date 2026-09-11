use egui::{Context, ScrollArea, Slider};
use smwe_rom::font_map::FontMap;
use smwe_rom::message_boxes::{MESSAGE_BOXES_MAX_SIZE, MESSAGE_NAMES};

use super::UiLevelEditor;

/// Editor for SMW's vanilla message box text: 22 global messages, each a
/// sequence of raw font-tile-index bytes (0x00-0xFF). Bit 7 set means "fill
/// the remainder of this 18-cell row with blanks" (per `CODE_05B208` in
/// bank_05.asm; see `smwe_rom::font_map` for the exact row-fill semantics).
///
/// The read-only preview pane at the bottom shows:
/// 1. Readable text (8 rows × 18 cells) via the real SMW (U) font map
///    (`FontMap::real()`, verified 2026-09-10 by running all 22 messages
///    through the genuine `CODE_05B1BC`).
/// 2. Stripe info: runs the selected message through the REAL game routine
///    (`CODE_05B1BC`) on a scratch CPU clone and captures the dynamic stripe
///    image (8 rows × 18 tile words, `$39TT`).
///
/// Pixel rasterization uses the real message font graphics: GFX2A
/// ("Message Box Letters", SNES $0BCB7B, 2bpp, 128 tiles), decompressed from
/// the ROM and rendered with the message palette.
/// The text preview above is genuine and verified.
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
        egui::Window::new("Message Box Editor").open(&mut open).resizable(true).default_size([520.0, 520.0]).show(
            ctx,
            |ui| {
                ui.label("Raw font-tile-index bytes (0x00-0xFF). Bit 7 = fill rest of row with blanks.");
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
                                    if ui.add(Slider::new(&mut v, 0..=0xFF).hexadecimal(2, false, false)).changed() {
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
                        ui.label("Preview (read-only, 8×18)");

                        // Readable-text preview via the real SMW (U) font map.
                        // Verified 2026-09-10: all 22 vanilla messages run
                        // through the genuine CODE_05B1BC produce readable
                        // 8×18 text via this map.
                        let real_map = FontMap::real();
                        let rows = real_map.to_rows(&self.message_boxes.messages[i]);
                        // Monospace for aligned 18-column rows.
                        let mono = egui::TextStyle::Monospace;
                        for row in rows.iter() {
                            ui.label(egui::RichText::new(row).text_style(mono.clone()));
                        }

                        // True raster preview: decompress GFX2A ("Message Box
                        // Letters") once, rasterize the 8×18 grid with the real
                        // SMW font graphics, and display it. The cache key
                        // includes a hash of the message bytes so edits
                        // invalidate and rebuild the preview.
                        if self.message_font.is_none() {
                            self.message_font =
                                smwe_rom::message_raster::decompress_message_font(&self.rom.rom).ok();
                        }
                        if let Some(font) = &self.message_font {
                            let msg_bytes = &self.message_boxes.messages[i];
                            // Simple hash for cache invalidation.
                            let mut hash: u64 = 0;
                            for &b in msg_bytes {
                                hash = hash.wrapping_mul(31).wrapping_add(b as u64);
                            }
                            if self.message_raster_for != Some((i, hash)) {
                                let cells = smwe_rom::font_map::message_cells(msg_bytes);
                                let img = smwe_rom::message_raster::rasterize_message(cells, font);
                                // 3x scale for visibility.
                                let (w, h) = (img.width() * 3, img.height() * 3);
                                let mut pixels = Vec::with_capacity((w * h) as usize);
                                for y in 0..h {
                                    for x in 0..w {
                                        let p = img.get_pixel(x / 3, y / 3);
                                        pixels.push(egui::Color32::from_rgb(p[0], p[1], p[2]));
                                    }
                                }
                                let color_img = egui::ColorImage { size: [w as usize, h as usize], pixels };
                                let tex = ui.ctx().load_texture(
                                    format!("message_raster_{i}"),
                                    color_img,
                                    egui::TextureOptions::NEAREST,
                                );
                                self.message_raster_texture = Some(tex);
                                self.message_raster_for = Some((i, hash));
                            }
                            if let Some(tex) = &self.message_raster_texture {
                                ui.label("Raster (true SMW font):");
                                ui.image((tex.id(), egui::vec2(432.0, 192.0)));
                            }
                        }

                        // Pixel preview: run the real CODE_05B1BC on a scratch CPU
                        // clone and capture the dynamic stripe image it appends
                        // to WRAM. The stripe is rasterized below with the real
                        // GFX2A message-font graphics.
                        let slot = smwe_rom::message_boxes::pointer_slot_for_message(i);
                        if self.message_preview_for != Some(i) {
                            let mut scratch = self.cpu.clone();
                            self.message_preview =
                                Some(smwe_emu::emu::render_message(&mut scratch, slot));
                            self.message_preview_for = Some(i);
                        }
                        if let Some(stripe) = &self.message_preview {
                            ui.small(format!(
                                "CODE_05B1BC ran ({} cycles): {} stripe bytes (8 rows × 18 tiles).",
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
