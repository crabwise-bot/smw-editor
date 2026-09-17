//! Shared "ExAnimated Frames" editor window — Lunar Magic parity (LM v1.60/v1.70,
//! overworld variant v2.40).
//!
//! Used by both the level editor (per-level + global lists) and the world-map
//! editor (the single overworld list). A frame is a *line* frame (copy 8x8
//! tile graphics within VRAM, one source set per animation step) or a
//! *palette* frame (write/rotate CGRAM colors per step).
//!
//! The host owns the [`smwe_rom::exanimation::ExAnimationData`] and a clean
//! post-load VRAM snapshot for the tile browser; this dialog owns only its
//! UI state. `show` returns true when the data was modified so the host can
//! mark its save-dirty flags.

use egui::{Color32, Context, ScrollArea};
use smwe_rom::{
    exanimation::{
        ExAnimFrame,
        ExAnimFrameKind,
        ExAnimTrigger,
        ExAnimation,
        ExAnimationData,
        EXANIM_MAX_FRAMES,
        EXANIM_MAX_UNITS_PER_FRAME,
    },
    graphics::gfx_file::Tile,
};

/// 8x8 tiles per row in the VRAM browser atlas (64 × 32 = 2048 tiles).
const ATLAS_COLS: usize = 64;
/// VRAM words per 4bpp 8x8 tile.
const TILE_WORDS: u16 = 16;

/// Which animation list the dialog edits.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ExAnimList {
    /// The overworld list (world-map editor; runs on the world maps).
    #[default]
    Overworld,
    /// The global list (level editor; runs in every level).
    Global,
    /// The current level's list (level editor; the host supplies the number).
    Level,
}

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

/// Dialog UI state for the shared "ExAnimated Frames" window.
pub struct ExAnimDialog {
    /// Which list is being edited. The world-map editor pins this to
    /// [`ExAnimList::Overworld`]; the level editor offers Level/Global tabs.
    pub list:  ExAnimList,
    selected:  usize,
    pick_slot: Option<(usize, usize)>,
    pal_row:   usize,
    atlas_tex: Option<egui::TextureHandle>,
    atlas_for: Option<(u64, usize)>,
}

impl ExAnimDialog {
    pub fn new(list: ExAnimList) -> Self {
        Self { list, selected: 0, pick_slot: None, pal_row: 0, atlas_tex: None, atlas_for: None }
    }

    /// Drop the cached tile-browser atlas (call after the host reloads VRAM).
    pub fn reset_atlas(&mut self) {
        self.atlas_tex = None;
        self.atlas_for = None;
    }

    fn anim<'a>(&self, data: &'a ExAnimationData, level_num: Option<u16>) -> Option<&'a ExAnimation> {
        match self.list {
            ExAnimList::Overworld => Some(&data.overworld),
            ExAnimList::Global => Some(&data.global),
            ExAnimList::Level => level_num.and_then(|n| data.level(n)),
        }
    }

    fn anim_mut<'a>(&self, data: &'a mut ExAnimationData, level_num: Option<u16>) -> &'a mut ExAnimation {
        match self.list {
            ExAnimList::Overworld => &mut data.overworld,
            ExAnimList::Global => &mut data.global,
            // The Level tab is only offered when the host passes a level
            // number; fall back to the global list rather than silently
            // editing level 0's list.
            ExAnimList::Level => match level_num {
                Some(n) => data.level_mut(n),
                None => &mut data.global,
            },
        }
    }

    fn view_frames<'a>(&self, data: &'a ExAnimationData, level_num: Option<u16>) -> &'a [ExAnimFrame] {
        self.anim(data, level_num).map(|a| a.frames.as_slice()).unwrap_or(&[])
    }

    /// Build (or reuse) the VRAM tile atlas texture from the host's clean
    /// post-load VRAM snapshot.
    fn atlas(&mut self, ctx: &Context, base_vram: &[u8], cgram: &[u8], vram_id: u64) -> egui::TextureHandle {
        let key = (vram_id, self.pal_row);
        let dirty = self.atlas_for != Some(key) || self.atlas_tex.is_none();
        if dirty {
            let tile_count = base_vram.len() / 32;
            let rows = tile_count.div_ceil(ATLAS_COLS);
            let (w, h) = (ATLAS_COLS * 8, rows * 8);
            let mut pixels = vec![Color32::BLACK; w * h];
            // Palette row from the live CGRAM (what the view actually uses).
            let mut pal = [Color32::BLACK; 16];
            for i in 0..16 {
                let off = self.pal_row * 32 + i * 2;
                let c = if off + 1 < cgram.len() { u16::from_le_bytes([cgram[off], cgram[off + 1]]) } else { 0 };
                pal[i] = snes555_to_color32(c);
            }
            for t in 0..tile_count {
                let bytes = &base_vram[t * 32..(t + 1) * 32];
                let Ok((_, tile)) = Tile::from_4bpp(bytes) else { continue };
                let (tx, ty) = ((t % ATLAS_COLS) * 8, (t / ATLAS_COLS) * 8);
                for (pi, &ci) in tile.color_indices.iter().enumerate() {
                    let (px, py) = (tx + pi % 8, ty + pi / 8);
                    pixels[py * w + px] = pal[(ci & 0xF) as usize];
                }
            }
            let img = egui::ColorImage { size: [w, h], pixels };
            self.atlas_tex = Some(ctx.load_texture("exanim_vram_atlas", img, egui::TextureOptions::NEAREST));
            self.atlas_for = Some(key);
        }
        self.atlas_tex.as_ref().unwrap().clone()
    }

    /// Show the window. `open` toggles visibility; `data` is the host's
    /// animation data; `base_vram` is the host's clean VRAM snapshot for the
    /// tile browser; `cgram` is the live CGRAM used to color it; `vram_id`
    /// identifies the VRAM snapshot (level number / submap generation) so the
    /// atlas rebuilds when it changes; `level_num` enables the Level/Global
    /// tabs (level editor) — with `None` the dialog edits the overworld list
    /// only. Returns true if the data was modified.
    #[allow(clippy::too_many_arguments)]
    pub fn show(
        &mut self, ctx: &Context, open: &mut bool, data: &mut ExAnimationData, base_vram: &[u8], cgram: &[u8],
        vram_id: u64, level_num: Option<u16>,
    ) -> bool {
        let mut changed = false;
        let title =
            if self.list == ExAnimList::Overworld { "ExAnimated Frames (Overworld)" } else { "ExAnimated Frames" };
        let mut open_flag = *open;
        egui::Window::new(title).open(&mut open_flag).resizable(true).default_size([860.0, 640.0]).show(ctx, |ui| {
            if let Some(n) = level_num {
                ui.horizontal(|ui| {
                    ui.selectable_value(&mut self.list, ExAnimList::Level, format!("Level {n:03X}"));
                    ui.selectable_value(&mut self.list, ExAnimList::Global, "Global (all levels)");
                });
            } else {
                ui.label("Overworld animation list — plays on the world maps.");
            }
            ui.small(
                "Line frames copy 8x8 tile graphics within VRAM (destination = VRAM word address). \
                     Palette frames write SNES RGB555 colors to CGRAM. The animation plays live in the view.",
            );
            ui.separator();

            ui.horizontal(|ui| {
                // ── Frame list ──
                ui.vertical(|ui| {
                    ui.label("Frames");
                    let frame_count = self.view_frames(data, level_num).len();
                    ScrollArea::vertical().max_height(420.0).id_salt("exanim_frames").show(ui, |ui| {
                        for i in 0..frame_count {
                            let Some(f) = self.view_frames(data, level_num).get(i) else { continue };
                            let (kind, dest, frames) = (f.kind, f.dest, f.frames);
                            let label = format!("#{i} {} @ ${dest:04X} · {frames}f", kind.label(),);
                            if ui.selectable_value(&mut self.selected, i, label).clicked() {
                                self.pick_slot = None;
                            }
                        }
                    });
                    ui.horizontal(|ui| {
                        if ui.button("+ Add").clicked() {
                            let anim = self.anim_mut(data, level_num);
                            if anim.frames.len() < 0x100 {
                                anim.frames.push(ExAnimFrame::default());
                                self.selected = anim.frames.len() - 1;
                                changed = true;
                            }
                        }
                        if ui.button("− Del").clicked() {
                            let sel = self.selected;
                            let anim = self.anim_mut(data, level_num);
                            if sel < anim.frames.len() {
                                anim.frames.remove(sel);
                                self.selected = sel.saturating_sub(1);
                                self.pick_slot = None;
                                changed = true;
                            }
                        }
                    });
                    ui.horizontal(|ui| {
                        let sel = self.selected;
                        if ui.button("▲").clicked() && sel > 0 {
                            let anim = self.anim_mut(data, level_num);
                            if sel < anim.frames.len() {
                                anim.frames.swap(sel - 1, sel);
                                self.selected = sel - 1;
                                changed = true;
                            }
                        }
                        if ui.button("▼").clicked() {
                            let sel = self.selected;
                            let anim = self.anim_mut(data, level_num);
                            if sel + 1 < anim.frames.len() {
                                anim.frames.swap(sel, sel + 1);
                                self.selected += 1;
                                changed = true;
                            }
                        }
                    });
                    ui.separator();
                    let mut disable = self.anim(data, level_num).is_some_and(|a| a.disable_original);
                    if ui.checkbox(&mut disable, "Disable original animations").changed() {
                        self.anim_mut(data, level_num).disable_original = disable;
                        changed = true;
                    }
                    ui.small("Skips the game's own animated tiles in the preview while this list plays.");
                });

                ui.separator();

                // ── Selected frame editor ──
                ui.vertical(|ui| {
                    let sel = self.selected;
                    let has = sel < self.view_frames(data, level_num).len();
                    if !has {
                        ui.label("Add a frame to begin.");
                    } else if self.frame_editor(ui, data, level_num, sel, base_vram, cgram, vram_id) {
                        changed = true;
                    }
                });
            });

            ui.separator();
            ui.small(
                "Preview ignores triggers (everything plays as Always). In-game playback requires \
                     Lunar Magic's ExAnimation ASM hack — the editor does not install it.",
            );
        });
        *open = open_flag;
        changed
    }

    /// Editor for one frame. Returns true if the frame was modified. The
    /// field snapshot / write-back split keeps the borrow checker happy: the
    /// widgets edit a local copy, then the changes are written back at once.
    fn frame_editor(
        &mut self, ui: &mut egui::Ui, data: &mut ExAnimationData, level_num: Option<u16>, sel: usize, base_vram: &[u8],
        cgram: &[u8], vram_id: u64,
    ) -> bool {
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
            let f = &self.view_frames(data, level_num)[sel];
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
            let f = &mut self.anim_mut(data, level_num).frames[sel];
            f.kind = e.kind;
            f.dest = e.dest.min(dest_max);
            f.speed = e.speed;
            f.trigger = e.trigger;
            f.frames = e.frames;
            f.units_per_frame = e.units_per_frame;
            f.payload = e.payload.clone();
        }

        ui.separator();
        if e.kind.is_line() {
            changed |= self.line_payload_editor(ui, data, level_num, sel, base_vram, cgram, vram_id);
        } else {
            changed |= self.palette_payload_editor(ui, data, level_num, sel);
        }
        changed
    }

    /// Per-step source-tile assignment for line frames, with the VRAM tile
    /// browser below. Returns true if the frame was modified.
    fn line_payload_editor(
        &mut self, ui: &mut egui::Ui, data: &mut ExAnimationData, level_num: Option<u16>, sel: usize, base_vram: &[u8],
        cgram: &[u8], vram_id: u64,
    ) -> bool {
        let mut changed = false;
        let (frames, units) = {
            let f = &self.view_frames(data, level_num)[sel];
            (f.frames as usize, f.units_per_frame as usize)
        };
        ui.label("Source tiles per step (click a slot, then a tile below):");
        ScrollArea::vertical().max_height(180.0).id_salt("exanim_line_payload").show(ui, |ui| {
            for f in 0..frames {
                ui.horizontal(|ui| {
                    ui.monospace(format!("step {f:3}:"));
                    for u in 0..units {
                        let src = self.view_frames(data, level_num)[sel].payload[f * units + u];
                        let armed = self.pick_slot == Some((f, u));
                        let btn =
                            egui::Button::new(format!("${src:04X}")).selected(armed).min_size([52.0, 18.0].into());
                        if ui.add(btn).clicked() {
                            self.pick_slot = if armed { None } else { Some((f, u)) };
                        }
                    }
                });
            }
        });

        ui.separator();
        ui.horizontal(|ui| {
            ui.label("Tile browser (view VRAM)");
            ui.label("palette row");
            let mut row = self.pal_row as i32;
            if ui.add(egui::DragValue::new(&mut row).range(0..=7)).changed() {
                self.pal_row = row as usize;
                self.atlas_for = None; // rebuild with the new row
            }
        });
        let tex = self.atlas(ui.ctx(), base_vram, cgram, vram_id);
        let (w, h) = (ATLAS_COLS * 8, (2048usize.div_ceil(ATLAS_COLS)) * 8);
        // Show at 1x; scroll both ways.
        ScrollArea::both().max_height(220.0).id_salt("exanim_atlas").show(ui, |ui| {
            let resp = ui.add(egui::Image::new((tex.id(), egui::vec2(w as f32, h as f32))).sense(egui::Sense::click()));
            if resp.clicked() {
                if let Some((slot_f, slot_u)) = self.pick_slot {
                    if let Some(pos) = resp.interact_pointer_pos() {
                        let rel = pos - resp.rect.min;
                        let col = (rel.x / 8.0) as usize;
                        let row = (rel.y / 8.0) as usize;
                        let tile = row * ATLAS_COLS + col;
                        if tile < 2048 && col < ATLAS_COLS {
                            let units = self.view_frames(data, level_num)[sel].units_per_frame as usize;
                            let frame = &mut self.anim_mut(data, level_num).frames[sel];
                            if let Some(p) = frame.payload.get_mut(slot_f * units + slot_u) {
                                *p = tile as u16 * TILE_WORDS;
                                changed = true;
                            }
                            self.pick_slot = None;
                        }
                    }
                }
            }
        });
        if self.pick_slot.is_some() {
            ui.small("Pick a source tile above — click any tile in the browser.");
        } else {
            ui.small("Tile numbers are VRAM word addresses ($0010 = 8x8 tile 1).");
        }
        changed
    }

    /// Per-step color editing for palette frames / the rotate ring. Returns
    /// true if the frame was modified.
    fn palette_payload_editor(
        &mut self, ui: &mut egui::Ui, data: &mut ExAnimationData, level_num: Option<u16>, sel: usize,
    ) -> bool {
        let mut changed = false;
        let kind = self.view_frames(data, level_num)[sel].kind;
        if kind == ExAnimFrameKind::PaletteRotate {
            ui.label("Color ring (rotates one step per tick):");
            let units = self.view_frames(data, level_num)[sel].units_per_frame as usize;
            ui.horizontal(|ui| {
                for u in 0..units {
                    let c0 = snes555_to_color32(self.view_frames(data, level_num)[sel].payload[u]);
                    let mut rgb = [c0.r(), c0.g(), c0.b()];
                    if ui.color_edit_button_srgb(&mut rgb).changed() {
                        let frame = &mut self.anim_mut(data, level_num).frames[sel];
                        frame.payload[u] = color32_to_snes555(Color32::from_rgb(rgb[0], rgb[1], rgb[2]));
                        changed = true;
                    }
                }
            });
            return changed;
        }
        let (frames, units) = {
            let f = &self.view_frames(data, level_num)[sel];
            (f.frames as usize, f.units_per_frame as usize)
        };
        ui.label("Colors per step:");
        ScrollArea::vertical().max_height(300.0).id_salt("exanim_pal_payload").show(ui, |ui| {
            for f in 0..frames {
                ui.horizontal(|ui| {
                    ui.monospace(format!("step {f:3}:"));
                    for u in 0..units {
                        let c0 = snes555_to_color32(self.view_frames(data, level_num)[sel].payload[f * units + u]);
                        let mut rgb = [c0.r(), c0.g(), c0.b()];
                        if ui.color_edit_button_srgb(&mut rgb).changed() {
                            let frame = &mut self.anim_mut(data, level_num).frames[sel];
                            frame.payload[f * units + u] =
                                color32_to_snes555(Color32::from_rgb(rgb[0], rgb[1], rgb[2]));
                            changed = true;
                        }
                    }
                });
            }
        });
        changed
    }
}
