//! "ExAnimated Frames" editor — Lunar Magic parity (LM v1.60/v1.70, OW v2.40).
//!
//! Per-level custom tile + palette animation authoring. Each level (plus one
//! global list that runs in every level) owns a list of frames; a frame is a
//! *line* frame (copy 8x8 tile graphics within VRAM, one source set per
//! animation step) or a *palette* frame (write/rotate CGRAM colors per step).
//!
//! The animation plays live in the level view — the central panel applies
//! [`smwe_rom::exanimation::apply_tick`] on the same ~133ms tick as the
//! vanilla animated tiles. Triggers other than `Always` are stored for the
//! game but ignored by the preview (the trigger picker says so).
//!
//! In-game playback needs Lunar Magic's ExAnimation ASM hack installed in
//! the ROM; the editor authors and previews the data but does not install
//! the hack (LM installs it itself when you use its ExAnimation dialog).

use egui::{Color32, Context, ScrollArea};
use smwe_rom::{
    exanimation::{
        ExAnimFrame,
        ExAnimFrameKind,
        ExAnimTrigger,
        ExAnimation,
        EXANIM_MAX_FRAMES,
        EXANIM_MAX_UNITS_PER_FRAME,
    },
    graphics::gfx_file::Tile,
};

use super::UiLevelEditor;

/// 8x8 tiles per row in the VRAM browser atlas (64 × 32 = 2048 tiles).
const ATLAS_COLS: usize = 64;
/// VRAM words per 4bpp 8x8 tile.
const TILE_WORDS: u16 = 16;

/// SNES RGB555 → egui color.
fn snes555_to_color32(c: u16) -> Color32 {
    let r = ((c & 0x1F) as u32 * 255 / 31) as u8;
    let g = (((c >> 5) & 0x1F) as u32 * 255 / 31) as u8;
    let b = (((c >> 10) & 0x1F) as u32 * 255 / 31) as u8;
    Color32::from_rgb(r, g, b)
}

/// egui color → SNES RGB555 (5-bit rounding).
fn color32_to_snes555(c: Color32) -> u16 {
    let r = (c.r() as u16 * 31 + 127) / 255;
    let g = (c.g() as u16 * 31 + 127) / 255;
    let b = (c.b() as u16 * 31 + 127) / 255;
    (b << 10) | (g << 5) | r
}

impl UiLevelEditor {
    fn exanimation_anim_mut(&mut self) -> &mut ExAnimation {
        if self.exanimation_global {
            &mut self.exanimation.global
        } else {
            let level = self.level_num;
            self.exanimation.level_mut(level)
        }
    }

    /// The animation list currently shown in the dialog, if any.
    fn exanimation_view(&self) -> Option<&ExAnimation> {
        if self.exanimation_global {
            Some(&self.exanimation.global)
        } else {
            self.exanimation.level(self.level_num)
        }
    }

    fn exanimation_view_frames(&self) -> &[ExAnimFrame] {
        self.exanimation_view().map(|a| a.frames.as_slice()).unwrap_or(&[])
    }

    /// Build (or reuse) the VRAM tile atlas texture from the clean
    /// post-load VRAM snapshot.
    fn exanimation_atlas(&mut self, ctx: &Context, pal_row: usize) -> egui::TextureHandle {
        let key = (self.level_num, pal_row);
        let dirty = self.exanimation_atlas_for != Some(key) || self.exanimation_atlas_tex.is_none();
        if dirty {
            let vram = &self.exanimation_base_vram;
            let tile_count = vram.len() / 32;
            let rows = tile_count.div_ceil(ATLAS_COLS);
            let (w, h) = (ATLAS_COLS * 8, rows * 8);
            let mut pixels = vec![Color32::BLACK; w * h];
            // Palette row from the live CGRAM (what the level actually uses).
            let mut pal = [Color32::BLACK; 16];
            for i in 0..16 {
                let off = pal_row * 32 + i * 2;
                let c = if off + 1 < self.cpu.mem.cgram.len() {
                    u16::from_le_bytes([self.cpu.mem.cgram[off], self.cpu.mem.cgram[off + 1]])
                } else {
                    0
                };
                pal[i] = snes555_to_color32(c);
            }
            for t in 0..tile_count {
                let bytes = &vram[t * 32..(t + 1) * 32];
                let Ok((_, tile)) = Tile::from_4bpp(bytes) else { continue };
                let (tx, ty) = ((t % ATLAS_COLS) * 8, (t / ATLAS_COLS) * 8);
                for (pi, &ci) in tile.color_indices.iter().enumerate() {
                    let (px, py) = (tx + pi % 8, ty + pi / 8);
                    pixels[py * w + px] = pal[(ci & 0xF) as usize];
                }
            }
            let img = egui::ColorImage { size: [w, h], pixels };
            self.exanimation_atlas_tex =
                Some(ctx.load_texture("exanim_vram_atlas", img, egui::TextureOptions::NEAREST));
            self.exanimation_atlas_for = Some(key);
        }
        self.exanimation_atlas_tex.as_ref().unwrap().clone()
    }

    pub(super) fn exanimation_editor_window(&mut self, ctx: &Context) {
        if !self.show_exanimation_editor {
            return;
        }
        let mut open = self.show_exanimation_editor;
        egui::Window::new("ExAnimated Frames").open(&mut open).resizable(true).default_size([860.0, 640.0]).show(
            ctx,
            |ui| {
                ui.horizontal(|ui| {
                    ui.selectable_value(&mut self.exanimation_global, false, format!("Level {:03X}", self.level_num));
                    ui.selectable_value(&mut self.exanimation_global, true, "Global (all levels)");
                });
                ui.small(
                    "Line frames copy 8x8 tile graphics within VRAM (destination = VRAM word address). \
                     Palette frames write SNES RGB555 colors to CGRAM. The animation plays live in the level view.",
                );
                ui.separator();

                ui.horizontal(|ui| {
                    // ── Frame list ──
                    ui.vertical(|ui| {
                        ui.label("Frames");
                        let frame_count = self.exanimation_view_frames().len();
                        ScrollArea::vertical().max_height(420.0).id_salt("exanim_frames").show(ui, |ui| {
                            for i in 0..frame_count {
                                let Some(f) = self.exanimation_view_frames().get(i) else { continue };
                                let (kind, dest, frames) = (f.kind, f.dest, f.frames);
                                let label = format!("#{i} {} @ ${dest:04X} · {frames}f", kind.label(),);
                                if ui.selectable_value(&mut self.exanimation_selected, i, label).clicked() {
                                    self.exanimation_pick_slot = None;
                                }
                            }
                        });
                        ui.horizontal(|ui| {
                            if ui.button("+ Add").clicked() {
                                let anim = self.exanimation_anim_mut();
                                if anim.frames.len() < 0x100 {
                                    anim.frames.push(ExAnimFrame::default());
                                    self.exanimation_selected = anim.frames.len() - 1;
                                    self.exanimation_dirty = true;
                                    self.has_edits = true;
                                }
                            }
                            if ui.button("− Del").clicked() {
                                let sel = self.exanimation_selected;
                                let anim = self.exanimation_anim_mut();
                                if sel < anim.frames.len() {
                                    anim.frames.remove(sel);
                                    self.exanimation_selected = sel.saturating_sub(1);
                                    self.exanimation_pick_slot = None;
                                    self.exanimation_dirty = true;
                                    self.has_edits = true;
                                }
                            }
                        });
                        ui.horizontal(|ui| {
                            let sel = self.exanimation_selected;
                            if ui.button("▲").clicked() && sel > 0 {
                                let anim = self.exanimation_anim_mut();
                                if sel < anim.frames.len() {
                                    anim.frames.swap(sel - 1, sel);
                                    self.exanimation_selected = sel - 1;
                                    self.exanimation_dirty = true;
                                    self.has_edits = true;
                                }
                            }
                            if ui.button("▼").clicked() {
                                let anim = self.exanimation_anim_mut();
                                if sel + 1 < anim.frames.len() {
                                    anim.frames.swap(sel, sel + 1);
                                    self.exanimation_selected += 1;
                                    self.exanimation_dirty = true;
                                    self.has_edits = true;
                                }
                            }
                        });
                        ui.separator();
                        let mut disable = self.exanimation_view().is_some_and(|a| a.disable_original);
                        if ui.checkbox(&mut disable, "Disable original animations").changed() {
                            self.exanimation_anim_mut().disable_original = disable;
                            self.exanimation_dirty = true;
                            self.has_edits = true;
                        }
                        ui.small("Skips the game's own animated tiles in the preview while this list plays.");
                    });

                    ui.separator();

                    // ── Selected frame editor ──
                    ui.vertical(|ui| {
                        let sel = self.exanimation_selected;
                        let has = sel < self.exanimation_view_frames().len();
                        if !has {
                            ui.label("Add a frame to begin.");
                        } else {
                            self.exanimation_frame_editor(ui, sel);
                        }
                    });
                });

                ui.separator();
                ui.small(
                    "Preview ignores triggers (everything plays as Always). In-game playback requires \
                     Lunar Magic's ExAnimation ASM hack — the editor does not install it.",
                );
            },
        );
        self.show_exanimation_editor = open;
    }

    /// Editor for one frame. Split out so the borrow checker sees the
    /// `&mut self` frame edits independently from the `ui` usage.
    fn exanimation_frame_editor(&mut self, ui: &mut egui::Ui, sel: usize) {
        // Snapshot the fields the widgets edit; write back at the end so
        // the ui closure never holds a borrow into `self.exanimation`.
        struct FrameEdits {
            kind:            ExAnimFrameKind,
            dest:            u16,
            speed:           u8,
            trigger:         ExAnimTrigger,
            frames:          u16,
            units_per_frame: u8,
            payload:         Vec<u16>,
        }
        let mut e = {
            let f = &self.exanimation_view_frames()[sel];
            FrameEdits {
                kind:            f.kind,
                dest:            f.dest,
                speed:           f.speed,
                trigger:         f.trigger,
                frames:          f.frames,
                units_per_frame: f.units_per_frame,
                payload:         f.payload.clone(),
            }
        };
        let mut changed = false;

        ui.horizontal(|ui| {
            ui.label("Type");
            egui::ComboBox::from_id_salt("exanim_kind").selected_text(e.kind.label()).show_ui(ui, |ui| {
                for kind in [
                    ExAnimFrameKind::Line8x8,
                    ExAnimFrameKind::Line16x16,
                    ExAnimFrameKind::Palette,
                    ExAnimFrameKind::PaletteRotate,
                ] {
                    if ui.selectable_value(&mut e.kind, kind, kind.label()).changed() {
                        changed = true;
                    }
                }
            });
            ui.label("Trigger");
            egui::ComboBox::from_id_salt("exanim_trigger").selected_text(e.trigger.label()).show_ui(ui, |ui| {
                for t in [ExAnimTrigger::Always, ExAnimTrigger::OnOff, ExAnimTrigger::Manual, ExAnimTrigger::OneShot] {
                    if ui.selectable_value(&mut e.trigger, t, t.label()).changed() {
                        changed = true;
                    }
                }
            });
        });

        // Destination range depends on the kind: VRAM words for line frames,
        // CGRAM words for palette frames.
        let (dest_label, dest_max) = if e.kind.is_line() { ("VRAM dest", 0x7FFF) } else { ("CGRAM dest", 0xFF) };
        ui.horizontal(|ui| {
            ui.label(dest_label);
            let mut dest = e.dest.min(dest_max) as i32;
            if ui.add(egui::DragValue::new(&mut dest).hexadecimal(4, false, true).range(0..=dest_max as i32)).changed()
            {
                e.dest = dest as u16;
                changed = true;
            }
            ui.label("Speed (ticks/step)");
            let mut speed = e.speed as i32;
            if ui.add(egui::DragValue::new(&mut speed).range(0..=120)).changed() {
                e.speed = speed as u8;
                changed = true;
            }
        });
        ui.horizontal(|ui| {
            ui.label("Steps");
            let mut frames = e.frames as i32;
            if ui.add(egui::DragValue::new(&mut frames).range(1..=EXANIM_MAX_FRAMES as i32)).changed() {
                e.frames = frames as u16;
                changed = true;
            }
            let unit_label = if e.kind.is_line() { "Tiles/step" } else { "Colors/step" };
            ui.label(unit_label);
            let mut units = e.units_per_frame as i32;
            if ui.add(egui::DragValue::new(&mut units).range(1..=EXANIM_MAX_UNITS_PER_FRAME as i32)).changed() {
                e.units_per_frame = units as u8;
                changed = true;
            }
        });

        if changed {
            // Resize the payload to the new geometry, preserving overlap.
            let want = if e.kind == ExAnimFrameKind::PaletteRotate {
                e.units_per_frame as usize
            } else {
                e.frames as usize * e.units_per_frame as usize
            };
            e.payload.resize(want, 0);
            let f = &mut self.exanimation_anim_mut().frames[sel];
            f.kind = e.kind;
            f.dest = e.dest.min(dest_max);
            f.speed = e.speed;
            f.trigger = e.trigger;
            f.frames = e.frames;
            f.units_per_frame = e.units_per_frame;
            f.payload = e.payload.clone();
            self.exanimation_dirty = true;
            self.has_edits = true;
        }

        ui.separator();
        if e.kind.is_line() {
            self.exanimation_line_payload_editor(ui, sel);
        } else {
            self.exanimation_palette_payload_editor(ui, sel);
        }
    }

    /// Per-step source-tile assignment for line frames, with the VRAM tile
    /// browser below.
    fn exanimation_line_payload_editor(&mut self, ui: &mut egui::Ui, sel: usize) {
        let (frames, units) = {
            let f = &self.exanimation_view_frames()[sel];
            (f.frames as usize, f.units_per_frame as usize)
        };
        ui.label("Source tiles per step (click a slot, then a tile below):");
        ScrollArea::vertical().max_height(180.0).id_salt("exanim_line_payload").show(ui, |ui| {
            for f in 0..frames {
                ui.horizontal(|ui| {
                    ui.monospace(format!("step {f:3}:"));
                    for u in 0..units {
                        let src = self.exanimation_view_frames()[sel].payload[f * units + u];
                        let armed = self.exanimation_pick_slot == Some((f, u));
                        let btn =
                            egui::Button::new(format!("${src:04X}")).selected(armed).min_size([52.0, 18.0].into());
                        if ui.add(btn).clicked() {
                            self.exanimation_pick_slot = if armed { None } else { Some((f, u)) };
                        }
                    }
                });
            }
        });

        ui.separator();
        ui.horizontal(|ui| {
            ui.label("Tile browser (level VRAM)");
            ui.label("palette row");
            let mut row = self.exanimation_pal_row as i32;
            if ui.add(egui::DragValue::new(&mut row).range(0..=7)).changed() {
                self.exanimation_pal_row = row as usize;
                self.exanimation_atlas_for = None; // rebuild with the new row
            }
        });
        let tex = self.exanimation_atlas(ui.ctx(), self.exanimation_pal_row);
        let (w, h) = (ATLAS_COLS * 8, (2048usize.div_ceil(ATLAS_COLS)) * 8);
        // Show at 1x; scroll both ways.
        ScrollArea::both().max_height(220.0).id_salt("exanim_atlas").show(ui, |ui| {
            let resp = ui.add(egui::Image::new((tex.id(), egui::vec2(w as f32, h as f32))).sense(egui::Sense::click()));
            if resp.clicked() {
                if let Some((slot_f, slot_u)) = self.exanimation_pick_slot {
                    if let Some(pos) = resp.interact_pointer_pos() {
                        let rel = pos - resp.rect.min;
                        let col = (rel.x / 8.0) as usize;
                        let row = (rel.y / 8.0) as usize;
                        let tile = row * ATLAS_COLS + col;
                        if tile < 2048 && col < ATLAS_COLS {
                            let units = self.exanimation_view_frames()[sel].units_per_frame as usize;
                            let frame = &mut self.exanimation_anim_mut().frames[sel];
                            if let Some(p) = frame.payload.get_mut(slot_f * units + slot_u) {
                                *p = tile as u16 * TILE_WORDS;
                                self.exanimation_dirty = true;
                                self.has_edits = true;
                            }
                            self.exanimation_pick_slot = None;
                        }
                    }
                }
            }
        });
        if self.exanimation_pick_slot.is_some() {
            ui.small("Pick a source tile above — click any tile in the browser.");
        } else {
            ui.small("Tile numbers are VRAM word addresses ($0010 = 8x8 tile 1).");
        }
    }

    /// Per-step color editing for palette frames / the rotate ring.
    fn exanimation_palette_payload_editor(&mut self, ui: &mut egui::Ui, sel: usize) {
        let kind = self.exanimation_view_frames()[sel].kind;
        if kind == ExAnimFrameKind::PaletteRotate {
            ui.label("Color ring (rotates one step per tick):");
            let units = self.exanimation_view_frames()[sel].units_per_frame as usize;
            ui.horizontal(|ui| {
                for u in 0..units {
                    let c0 = snes555_to_color32(self.exanimation_view_frames()[sel].payload[u]);
                    let mut rgb = [c0.r(), c0.g(), c0.b()];
                    if ui.color_edit_button_srgb(&mut rgb).changed() {
                        let frame = &mut self.exanimation_anim_mut().frames[sel];
                        frame.payload[u] = color32_to_snes555(Color32::from_rgb(rgb[0], rgb[1], rgb[2]));
                        self.exanimation_dirty = true;
                        self.has_edits = true;
                    }
                }
            });
            return;
        }
        let (frames, units) = {
            let f = &self.exanimation_view_frames()[sel];
            (f.frames as usize, f.units_per_frame as usize)
        };
        ui.label("Colors per step:");
        ScrollArea::vertical().max_height(300.0).id_salt("exanim_pal_payload").show(ui, |ui| {
            for f in 0..frames {
                ui.horizontal(|ui| {
                    ui.monospace(format!("step {f:3}:"));
                    for u in 0..units {
                        let c0 = snes555_to_color32(self.exanimation_view_frames()[sel].payload[f * units + u]);
                        let mut rgb = [c0.r(), c0.g(), c0.b()];
                        if ui.color_edit_button_srgb(&mut rgb).changed() {
                            let frame = &mut self.exanimation_anim_mut().frames[sel];
                            frame.payload[f * units + u] =
                                color32_to_snes555(Color32::from_rgb(rgb[0], rgb[1], rgb[2]));
                            self.exanimation_dirty = true;
                            self.has_edits = true;
                        }
                    }
                });
            }
        });
    }
}
