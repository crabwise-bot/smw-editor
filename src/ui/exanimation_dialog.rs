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
        remap_addresses,
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
/// Tiles shown in the browser atlas.
const ATLAS_TILES: usize = 2048;
/// How long (seconds) a double-clicked tile stays highlighted in the browser.
const REVEAL_SECS: f64 = 2.0;

/// LM v3.32 "8x8 Select" point-and-click target: the field the next browser
/// click fills. After each click the target auto-advances to the next field
/// (destination → step 0/tile 0 → … → wraps back to the destination).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SelectTarget {
    Dest,
    Slot(usize, usize),
}

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

/// LM v3.50 parity: the Ctrl / Ctrl+Shift gestures on the frame-group
/// buttons, which move the selected frame's *values* instead of reordering
/// the frame list.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum FrameGesture {
    /// Ctrl+▼: shift values right one step, inserting a blank step at the
    /// start ("effectively inserting a frame at the start").
    InsertAtStart,
    /// Ctrl+▲: shift values left one step, dropping the first step
    /// ("effectively deleting a frame at the start").
    DeleteAtStart,
    /// Ctrl+Shift+▲: circular-rotate the values left by one step.
    RotateLeft,
    /// Ctrl+Shift+▼: circular-rotate the values right by one step.
    RotateRight,
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
    pub list:      ExAnimList,
    selected:      usize,
    pick_slot:     Option<(usize, usize)>,
    pal_row:       usize,
    atlas_tex:     Option<egui::TextureHandle>,
    atlas_for:     Option<(u64, usize)>,
    /// LM v3.32 "8x8 Select" point-and-click mode: the field the next tile-
    /// browser click fills (`None` = mode off).
    select_target: Option<SelectTarget>,
    /// LM v3.32 "Remap" window state.
    remap_open:    bool,
    remap_old:     u16,
    remap_new:     u16,
    remap_frames:  bool,
    remap_dests:   bool,
    remap_status:  Option<String>,
    /// Tile index the browser should scroll to and flash (double-click a
    /// frame/destination value to reveal its tile), with the egui timestamp
    /// of the request.
    reveal:        Option<(usize, f64)>,
}

impl ExAnimDialog {
    pub fn new(list: ExAnimList) -> Self {
        Self {
            list,
            selected: 0,
            pick_slot: None,
            pal_row: 0,
            atlas_tex: None,
            atlas_for: None,
            select_target: None,
            remap_open: false,
            remap_old: 0,
            remap_new: 0,
            remap_frames: true,
            remap_dests: true,
            remap_status: None,
            reveal: None,
        }
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

    /// The 8x8 Select field after `cur`: destination → slots in step/tile
    /// order → wraps back to the destination (LM v3.32 auto-advance).
    fn next_target(
        &self, data: &ExAnimationData, level_num: Option<u16>, sel: usize, cur: SelectTarget,
    ) -> SelectTarget {
        let (frames, units) = {
            let f = &self.view_frames(data, level_num)[sel];
            (f.frames as usize, f.units_per_frame as usize)
        };
        match cur {
            SelectTarget::Dest => {
                if frames > 0 && units > 0 {
                    SelectTarget::Slot(0, 0)
                } else {
                    SelectTarget::Dest
                }
            }
            SelectTarget::Slot(f, u) => {
                if u + 1 < units {
                    SelectTarget::Slot(f, u + 1)
                } else if f + 1 < frames {
                    SelectTarget::Slot(f + 1, 0)
                } else {
                    SelectTarget::Dest
                }
            }
        }
    }

    /// Human-readable label for the current 8x8 Select target.
    fn target_label(&self, data: &ExAnimationData, level_num: Option<u16>, sel: usize) -> String {
        match self.select_target {
            None => "off".to_owned(),
            Some(SelectTarget::Dest) => {
                let d = self.view_frames(data, level_num)[sel].dest;
                format!("VRAM dest (now ${d:04X})")
            }
            Some(SelectTarget::Slot(f, u)) => format!("step {f} tile {u}"),
        }
    }

    /// LM v3.32 "Remap" window: old→new VRAM word addresses across the
    /// current animation list. Returns true if the data was modified.
    fn remap_window(&mut self, ctx: &Context, data: &mut ExAnimationData, level_num: Option<u16>) -> bool {
        let mut changed = false;
        let mut open = self.remap_open;
        let mut apply = false;
        let mut cancel = false;
        egui::Window::new("Remap ExAnimation tiles").open(&mut open).show(ctx, |ui| {
            ui.small(
                "After moving tiles around in VRAM, re-point every frame source tile and destination \
                 at the new addresses (LM v3.32 \"Remap\"). Only exact address matches are rewritten.",
            );
            ui.horizontal(|ui| {
                ui.label("Old VRAM address");
                let mut old = self.remap_old as i32;
                ui.add(egui::DragValue::new(&mut old).hexadecimal(4, false, true).range(0..=0x7FFF));
                self.remap_old = old as u16;
                ui.label("New VRAM address");
                let mut new = self.remap_new as i32;
                ui.add(egui::DragValue::new(&mut new).hexadecimal(4, false, true).range(0..=0x7FFF));
                self.remap_new = new as u16;
            });
            ui.checkbox(&mut self.remap_frames, "Frame source tiles");
            ui.checkbox(&mut self.remap_dests, "Destinations");
            ui.small("Palette frames are never touched (colors and CGRAM addresses, not VRAM tiles).");
            ui.horizontal(|ui| {
                if ui.button("Apply remap").clicked() {
                    apply = true;
                }
                if ui.button("Cancel").clicked() {
                    cancel = true;
                }
            });
            if let Some(s) = &self.remap_status {
                ui.small(s);
            }
        });
        if cancel {
            open = false;
        }
        self.remap_open = open;
        if apply {
            let n = remap_addresses(
                self.anim_mut(data, level_num),
                self.remap_old,
                self.remap_new,
                self.remap_frames,
                self.remap_dests,
            );
            self.remap_status = Some(format!("Remapped {n} reference(s)."));
            changed = n > 0;
        }
        changed
    }

    /// LM v3.50 parity: apply a Ctrl / Ctrl+Shift frame-value gesture to the
    /// selected frame (the ▲/▼ buttons move *values* instead of reordering
    /// the list while a modifier is held). Returns true when the data
    /// changed, so the host marks its save-dirty flags.
    fn apply_frame_gesture(
        &mut self, data: &mut ExAnimationData, level_num: Option<u16>, sel: usize, gesture: FrameGesture,
    ) -> bool {
        let anim = self.anim_mut(data, level_num);
        let Some(frame) = anim.frames.get_mut(sel) else { return false };
        let before = (frame.frames, frame.units_per_frame, frame.payload.clone());
        match gesture {
            FrameGesture::InsertAtStart => frame.insert_frame_at_start(),
            FrameGesture::DeleteAtStart => frame.delete_frame_at_start(),
            FrameGesture::RotateLeft => frame.rotate_steps_left(),
            FrameGesture::RotateRight => frame.rotate_steps_right(),
        }
        let changed = (frame.frames, frame.units_per_frame, frame.payload.clone()) != before;
        if changed {
            // Payload indices shifted or wrapped — drop any armed
            // tile-picker slot, whose (step, unit) no longer points at the
            // same value.
            self.pick_slot = None;
        }
        changed
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
                        // LM v3.50 parity: with Ctrl / Ctrl+Shift held, the
                        // ▲/▼ frame-group buttons move the selected frame's
                        // *values* instead of reordering the frame list.
                        let mods = ui.ctx().input(|i| i.modifiers);
                        let up = ui.button("▲").on_hover_text(
                            "Move frame up — hold Ctrl to delete the first animation step (shift frame values \
                                 left), Ctrl+Shift to rotate the frame values left (Lunar Magic v3.50)",
                        );
                        if up.clicked() {
                            if mods.ctrl && mods.shift {
                                changed |= self.apply_frame_gesture(data, level_num, sel, FrameGesture::RotateLeft);
                            } else if mods.ctrl {
                                changed |= self.apply_frame_gesture(data, level_num, sel, FrameGesture::DeleteAtStart);
                            } else if sel > 0 {
                                let anim = self.anim_mut(data, level_num);
                                if sel < anim.frames.len() {
                                    anim.frames.swap(sel - 1, sel);
                                    self.selected = sel - 1;
                                    changed = true;
                                }
                            }
                        }
                        let down = ui.button("▼").on_hover_text(
                            "Move frame down — hold Ctrl to insert a blank animation step at the start (shift \
                                 frame values right), Ctrl+Shift to rotate the frame values right (Lunar Magic v3.50)",
                        );
                        if down.clicked() {
                            if mods.ctrl && mods.shift {
                                changed |= self.apply_frame_gesture(data, level_num, sel, FrameGesture::RotateRight);
                            } else if mods.ctrl {
                                changed |= self.apply_frame_gesture(data, level_num, sel, FrameGesture::InsertAtStart);
                            } else {
                                let anim = self.anim_mut(data, level_num);
                                if sel + 1 < anim.frames.len() {
                                    anim.frames.swap(sel, sel + 1);
                                    self.selected += 1;
                                    changed = true;
                                }
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
        if self.remap_open {
            changed |= self.remap_window(ctx, data, level_num);
        }
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
            let dest_resp =
                ui.add(egui::DragValue::new(&mut dest).hexadecimal(4, false, true).range(0..=dest_max as i32));
            if e.kind.is_line() && dest_resp.double_clicked() {
                // LM v3.32: double-clicking a destination value shows its
                // tile in the 8x8 selector.
                let tile = e.dest as usize / TILE_WORDS as usize;
                if tile < ATLAS_TILES {
                    self.reveal = Some((tile, ui.ctx().input(|i| i.time)));
                }
            }
            if dest_resp.changed() {
                e.dest = dest as u16;
                changed = true;
            }
            if e.kind.is_line() && self.select_target.is_some() {
                let is_target = self.select_target == Some(SelectTarget::Dest);
                if ui
                    .add(egui::Button::new("◎").selected(is_target))
                    .on_hover_text("8x8 Select: make the destination the click target")
                    .clicked()
                {
                    self.select_target = Some(SelectTarget::Dest);
                }
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
        ui.horizontal(|ui| {
            if ui
                .button("Remap…")
                .on_hover_text("LM v3.32: re-point frame tiles and destinations after moving tiles in VRAM")
                .clicked()
            {
                self.remap_open = true;
                self.remap_status = None;
            }
            let active = self.select_target.is_some();
            if ui
                .add(egui::Button::new(if active { "■ 8x8 Select" } else { "8x8 Select" }).selected(active))
                .on_hover_text(
                    "LM v3.32: click tiles in the browser to fill the target field — \
                     auto-advances to the next field. Double-click a value to find its tile.",
                )
                .clicked()
            {
                self.select_target = if active { None } else { Some(SelectTarget::Dest) };
                self.pick_slot = None;
            }
        });
        if self.select_target.is_some() {
            ui.small(format!(
                "8x8 Select → filling {}; click a tile below (advances automatically), or click any slot to retarget.",
                self.target_label(data, level_num, sel)
            ));
        }
        ScrollArea::vertical().max_height(180.0).id_salt("exanim_line_payload").show(ui, |ui| {
            for f in 0..frames {
                ui.horizontal(|ui| {
                    ui.monospace(format!("step {f:3}:"));
                    for u in 0..units {
                        let src = self.view_frames(data, level_num)[sel].payload[f * units + u];
                        let armed = self.pick_slot == Some((f, u));
                        let targeted = self.select_target == Some(SelectTarget::Slot(f, u));
                        let btn = egui::Button::new(format!("${src:04X}"))
                            .selected(armed || targeted)
                            .min_size([52.0, 18.0].into());
                        let resp = ui.add(btn);
                        if resp.double_clicked() {
                            // LM v3.32: double-clicking a frame value shows
                            // its tile in the 8x8 selector.
                            let tile = src as usize / TILE_WORDS as usize;
                            if tile < ATLAS_TILES {
                                self.reveal = Some((tile, ui.ctx().input(|i| i.time)));
                            }
                            self.pick_slot = None;
                        } else if resp.clicked() {
                            if self.select_target.is_some() {
                                self.select_target = Some(SelectTarget::Slot(f, u));
                            } else {
                                self.pick_slot = if armed { None } else { Some((f, u)) };
                            }
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
        let (w, h) = (ATLAS_COLS * 8, (ATLAS_TILES.div_ceil(ATLAS_COLS)) * 8);
        // Show at 1x; scroll both ways.
        ScrollArea::both().max_height(220.0).id_salt("exanim_atlas").show(ui, |ui| {
            let resp = ui.add(egui::Image::new((tex.id(), egui::vec2(w as f32, h as f32))).sense(egui::Sense::click()));
            if resp.clicked() {
                if let Some(pos) = resp.interact_pointer_pos() {
                    let rel = pos - resp.rect.min;
                    let col = (rel.x / 8.0) as usize;
                    let row = (rel.y / 8.0) as usize;
                    let tile = row * ATLAS_COLS + col;
                    if tile < ATLAS_TILES && col < ATLAS_COLS {
                        let word = tile as u16 * TILE_WORDS;
                        if let Some(target) = self.select_target {
                            // 8x8 Select: fill the target field, then
                            // auto-advance to the next one.
                            match target {
                                SelectTarget::Dest => {
                                    self.anim_mut(data, level_num).frames[sel].dest = word;
                                }
                                SelectTarget::Slot(sf, su) => {
                                    let units = self.view_frames(data, level_num)[sel].units_per_frame as usize;
                                    if let Some(p) =
                                        self.anim_mut(data, level_num).frames[sel].payload.get_mut(sf * units + su)
                                    {
                                        *p = word;
                                    }
                                }
                            }
                            self.select_target = Some(self.next_target(data, level_num, sel, target));
                            changed = true;
                        } else if let Some((slot_f, slot_u)) = self.pick_slot {
                            let units = self.view_frames(data, level_num)[sel].units_per_frame as usize;
                            let frame = &mut self.anim_mut(data, level_num).frames[sel];
                            if let Some(p) = frame.payload.get_mut(slot_f * units + slot_u) {
                                *p = word;
                                changed = true;
                            }
                            self.pick_slot = None;
                        }
                    }
                }
            }
            // Double-click reveal (LM v3.32): scroll the browser to the tile
            // and flash it so the user can see where the value points.
            if let Some((tile, t0)) = self.reveal {
                let now = ui.ctx().input(|i| i.time);
                if now - t0 < REVEAL_SECS && tile < ATLAS_TILES {
                    let r = egui::Rect::from_min_size(
                        resp.rect.min + egui::vec2((tile % ATLAS_COLS) as f32 * 8.0, (tile / ATLAS_COLS) as f32 * 8.0),
                        egui::vec2(8.0, 8.0),
                    );
                    ui.scroll_to_rect(r, Some(egui::Align::Center));
                    ui.painter().rect_stroke(
                        r.expand(2.0),
                        0.0,
                        egui::Stroke::new(2.0_f32, egui::Color32::YELLOW),
                        egui::StrokeKind::Middle,
                    );
                    ui.ctx().request_repaint();
                } else {
                    self.reveal = None;
                }
            }
        });
        if self.pick_slot.is_some() {
            ui.small("Pick a source tile above — click any tile in the browser.");
        } else if self.select_target.is_none() {
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
