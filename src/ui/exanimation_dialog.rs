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
///
/// LM v3.33 extends the same mode to palette frames: with a palette frame
/// selected, the target is a color field instead, filled by
/// Ctrl+Left-Clicking a color in the palette editor (or by pasting a
/// palette row, LM v3.61).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SelectTarget {
    Dest,
    Slot(usize, usize),
    /// Palette-frame color field: (step, unit). PaletteRotate frames store
    /// a single ring, so step is always 0 for them.
    ColorSlot(usize, usize),
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
    /// LM v3.61: a "Paste row" click asked the integration for the system
    /// clipboard; the answer arrives as `Event::Paste` on the next frame.
    paste_pending: bool,
    /// LM v3.61: outcome of the last palette-row paste.
    paste_status:  Option<String>,
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
            paste_pending: false,
            paste_status: None,
        }
    }

    /// Drop the cached tile-browser atlas (call after the host reloads VRAM).
    pub fn reset_atlas(&mut self) {
        self.atlas_tex = None;
        self.atlas_for = None;
    }

    /// Clear any armed 8x8/Palette Select target (the host calls this when
    /// the window closes, so a stale arm can't swallow palette clicks).
    pub fn disarm_select(&mut self) {
        self.select_target = None;
    }

    /// Select a frame in the list (LM v3.33: Ctrl+Shift+Left-Click in the
    /// palette editor on an ExAnimated color destination).
    pub fn select_frame(&mut self, idx: usize) {
        self.selected = idx;
        self.pick_slot = None;
    }

    /// LM v3.33: is Palette Select armed — the window's point-and-click
    /// mode is on, the armed target is a color field, and the selected
    /// frame is a palette kind? The palette editor uses this to route
    /// Ctrl+Left-Clicks into the armed color field.
    pub fn palette_select_armed(&self, data: &ExAnimationData, level_num: Option<u16>) -> bool {
        if !matches!(self.select_target, Some(SelectTarget::ColorSlot(_, _))) {
            return false;
        }
        let frames = self.view_frames(data, level_num);
        self.selected < frames.len() && frames[self.selected].kind.is_palette()
    }

    /// LM v3.33: fill the armed Palette-Select color field with a SNES
    /// RGB555 color (from a Ctrl+Left-Click in the palette editor) and
    /// auto-advance the arm to the next color field. Returns true when a
    /// slot was filled, so the host marks its save-dirty flags.
    pub fn fill_armed_color(&mut self, data: &mut ExAnimationData, level_num: Option<u16>, color: u16) -> bool {
        if !self.palette_select_armed(data, level_num) {
            return false;
        }
        let sel = self.selected;
        let Some(SelectTarget::ColorSlot(f, u)) = self.select_target else { return false };
        let idx = {
            let frame = &self.view_frames(data, level_num)[sel];
            let units = frame.units_per_frame as usize;
            let steps = if frame.kind == ExAnimFrameKind::PaletteRotate { 1 } else { frame.frames as usize };
            if f >= steps || u >= units {
                return false;
            }
            f * units + u
        };
        let filled = self.anim_mut(data, level_num).frames[sel].payload.get_mut(idx).is_some_and(|p| {
            *p = color;
            true
        });
        if filled {
            self.select_target = Some(self.next_color_target(data, level_num, sel, f, u));
        }
        filled
    }

    /// (frame index, CGRAM dest word, units written per step) for every
    /// palette-kind frame in the dialog's current list — used by the
    /// palette editor to mark ExAnimated color destinations (LM v3.33).
    pub fn palette_frame_dests(&self, data: &ExAnimationData, level_num: Option<u16>) -> Vec<(usize, u16, usize)> {
        self.view_frames(data, level_num)
            .iter()
            .enumerate()
            .filter(|(_, f)| f.kind.is_palette())
            .map(|(i, f)| (i, f.dest, f.units_per_frame as usize))
            .collect()
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
            // Palette frames never arm tile targets; a stale one wraps to
            // the first color slot.
            SelectTarget::ColorSlot(_, _) => SelectTarget::ColorSlot(0, 0),
        }
    }

    /// The palette-select field after `(f, u)`: next color in step/unit
    /// order, wrapping back to the first slot (LM v3.33 auto-advance).
    /// PaletteRotate frames store a single ring, so the step is always 0.
    fn next_color_target(
        &self, data: &ExAnimationData, level_num: Option<u16>, sel: usize, f: usize, u: usize,
    ) -> SelectTarget {
        let frame = &self.view_frames(data, level_num)[sel];
        let units = frame.units_per_frame as usize;
        let steps = if frame.kind == ExAnimFrameKind::PaletteRotate { 1 } else { frame.frames as usize };
        if u + 1 < units {
            SelectTarget::ColorSlot(f, u + 1)
        } else if f + 1 < steps {
            SelectTarget::ColorSlot(f + 1, 0)
        } else {
            SelectTarget::ColorSlot(0, 0)
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
            Some(SelectTarget::ColorSlot(f, u)) => format!("step {f} color {u}"),
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
            // A kind switch invalidates an armed 8x8/Palette Select target
            // of the other flavor (tile fields vs color fields).
            let keep = match self.select_target {
                Some(SelectTarget::ColorSlot(_, _)) => e.kind.is_palette(),
                Some(_) => e.kind.is_line(),
                None => true,
            };
            if !keep {
                self.select_target = None;
            }
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
                                // A color target can't be filled by a tile
                                // click — the palette editor does that (LM
                                // v3.33). Leave the arm alone.
                                SelectTarget::ColorSlot(_, _) => {}
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

    /// One palette color slot. With Palette Select armed (LM v3.33) it is a
    /// plain swatch whose click retargets the armed field; otherwise the
    /// usual color picker. Returns true when the payload value changed.
    fn color_slot(
        &mut self, ui: &mut egui::Ui, data: &mut ExAnimationData, level_num: Option<u16>, sel: usize, step: usize,
        unit: usize, palette_select: bool,
    ) -> bool {
        let mut changed = false;
        let units = self.view_frames(data, level_num)[sel].units_per_frame as usize;
        let idx = step * units + unit;
        let raw = self.view_frames(data, level_num)[sel].payload[idx];
        let c0 = snes555_to_color32(raw);
        if palette_select {
            let armed = self.select_target == Some(SelectTarget::ColorSlot(step, unit));
            let (rect, resp) = ui.allocate_exact_size(egui::vec2(22.0, 22.0), egui::Sense::click());
            ui.painter().rect_filled(rect, 2.0, c0);
            ui.painter().rect_stroke(
                rect,
                2.0,
                egui::Stroke::new(1.0_f32, egui::Color32::from_gray(80)),
                egui::StrokeKind::Outside,
            );
            if armed {
                ui.painter().rect_stroke(
                    rect,
                    2.0,
                    egui::Stroke::new(2.0_f32, egui::Color32::WHITE),
                    egui::StrokeKind::Outside,
                );
            }
            let clicked = resp.clicked();
            resp.on_hover_text(format!(
                "Color ${raw:04X} — click to make this the Palette Select target (step {step}, color {unit})"
            ));
            if clicked {
                self.select_target = Some(SelectTarget::ColorSlot(step, unit));
            }
        } else {
            let mut rgb = [c0.r(), c0.g(), c0.b()];
            if ui.color_edit_button_srgb(&mut rgb).changed() {
                let frame = &mut self.anim_mut(data, level_num).frames[sel];
                if let Some(p) = frame.payload.get_mut(idx) {
                    *p = color32_to_snes555(Color32::from_rgb(rgb[0], rgb[1], rgb[2]));
                    changed = true;
                }
            }
        }
        changed
    }

    /// LM v3.61: paste a clipboard palette row into the frame's color slots,
    /// starting at the armed Palette-Select field (or step 0 / the ring
    /// start when nothing is armed). Fills as many slots as the row covers;
    /// returns true when any slot changed.
    fn apply_pasted_palette_row(
        &mut self, text: &str, data: &mut ExAnimationData, level_num: Option<u16>, sel: usize,
    ) -> bool {
        let decoded = crate::ui::clipboard::ClipboardPayload::decode(text);
        let Some(crate::ui::clipboard::ClipboardPayload::PaletteRow { colors }) = decoded else {
            self.paste_status = Some(
                "Clipboard has no palette row — copy one with \"Copy row\" in the palette editor first.".to_owned(),
            );
            return false;
        };
        let (start_f, start_u) = match self.select_target {
            Some(SelectTarget::ColorSlot(f, u)) => (f, u),
            _ => (0, 0),
        };
        let (units, steps) = {
            let frame = &self.view_frames(data, level_num)[sel];
            let units = frame.units_per_frame as usize;
            let steps = if frame.kind == ExAnimFrameKind::PaletteRotate { 1 } else { frame.frames as usize };
            (units, steps)
        };
        let mut changed = 0usize;
        let mut ci = 0usize;
        'fill: for f in start_f..steps {
            for u in (if f == start_f { start_u } else { 0 })..units {
                if ci >= colors.len() {
                    break 'fill;
                }
                let idx = f * units + u;
                if let Some(p) = self.anim_mut(data, level_num).frames[sel].payload.get_mut(idx) {
                    if *p != colors[ci] {
                        *p = colors[ci];
                        changed += 1;
                    }
                }
                ci += 1;
            }
        }
        self.paste_status = Some(if changed > 0 {
            format!("Pasted {changed} color(s) from the clipboard row into frame #{sel}.")
        } else {
            "Clipboard row applied — every slot already had that color.".to_owned()
        });
        changed > 0
    }

    /// Per-step color editing for palette frames / the rotate ring. Returns
    /// true if the frame was modified.
    fn palette_payload_editor(
        &mut self, ui: &mut egui::Ui, data: &mut ExAnimationData, level_num: Option<u16>, sel: usize,
    ) -> bool {
        let mut changed = false;
        let kind = self.view_frames(data, level_num)[sel].kind;

        // ── LM v3.33 Palette Select + LM v3.61 row paste ──────────────────
        ui.horizontal(|ui| {
            let active = matches!(self.select_target, Some(SelectTarget::ColorSlot(_, _)));
            if ui
                .add(egui::Button::new(if active { "■ Palette Select" } else { "Palette Select" }).selected(active))
                .on_hover_text(
                    "LM v3.33: point-and-click color fill — arm a color field below, then \
                     Ctrl+Left-Click a color in the palette editor to fill it (auto-advances). \
                     Click any color slot to retarget it.",
                )
                .clicked()
            {
                self.select_target = if active { None } else { Some(SelectTarget::ColorSlot(0, 0)) };
            }
            if ui
                .button("Paste row")
                .on_hover_text(
                    "LM v3.61: paste a whole row of palette colors from the clipboard (copied \
                     with \"Copy row\" in the palette editor) into the color slots, starting at \
                     the armed field",
                )
                .clicked()
            {
                crate::ui::clipboard::request_paste(ui.ctx());
                self.paste_pending = true;
            }
        });
        // The integration answers the paste request as Event::Paste on the
        // next frame; drain it only while our own request is pending so a
        // text widget's Ctrl+V is never stolen.
        if self.paste_pending {
            if let Some(text) = crate::ui::clipboard::take_paste_text(ui.ctx()) {
                self.paste_pending = false;
                changed |= self.apply_pasted_palette_row(&text, data, level_num, sel);
            }
        }
        if let Some(s) = self.paste_status.clone() {
            ui.small(s);
        }
        let palette_select = matches!(self.select_target, Some(SelectTarget::ColorSlot(_, _)));
        if palette_select {
            ui.small(format!(
                "Palette Select → filling {}; Ctrl+Left-Click a color in the palette editor to fill it \
                 (advances automatically), or click any slot below to retarget.",
                self.target_label(data, level_num, sel)
            ));
        }
        ui.separator();

        if kind == ExAnimFrameKind::PaletteRotate {
            ui.label("Color ring (rotates one step per tick):");
            let units = self.view_frames(data, level_num)[sel].units_per_frame as usize;
            ui.horizontal(|ui| {
                for u in 0..units {
                    changed |= self.color_slot(ui, data, level_num, sel, 0, u, palette_select);
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
                        changed |= self.color_slot(ui, data, level_num, sel, f, u, palette_select);
                    }
                });
            }
        });
        changed
    }
}

#[cfg(test)]
mod tests {
    use smwe_rom::exanimation::{ExAnimFrame, ExAnimFrameKind, ExAnimTrigger, ExAnimation, ExAnimationData};

    use super::*;

    fn palette_frame(frames: u16, units: u8, payload: Vec<u16>) -> ExAnimFrame {
        ExAnimFrame {
            kind: ExAnimFrameKind::Palette,
            dest: 0x0002,
            speed: 1,
            trigger: ExAnimTrigger::Always,
            frames,
            units_per_frame: units,
            payload,
        }
    }

    fn dialog_with(frame: ExAnimFrame) -> (ExAnimDialog, ExAnimationData) {
        let mut dlg = ExAnimDialog::new(ExAnimList::Global);
        let mut data = ExAnimationData::default();
        data.global = ExAnimation { frames: vec![frame], disable_original: false };
        dlg.selected = 0;
        (dlg, data)
    }

    #[test]
    fn fill_armed_color_writes_and_advances() {
        // LM v3.33: Ctrl+Left-Click on a palette color fills the armed
        // field, then the arm auto-advances to the next color field.
        let (mut dlg, mut data) =
            dialog_with(palette_frame(2, 3, vec![0x0000, 0x0000, 0x0000, 0x0000, 0x0000, 0x0000]));
        dlg.select_target = Some(SelectTarget::ColorSlot(0, 1));
        assert!(dlg.fill_armed_color(&mut data, None, 0x7FFF));
        assert_eq!(data.global.frames[0].payload[1], 0x7FFF);
        assert_eq!(dlg.select_target, Some(SelectTarget::ColorSlot(0, 2)));
        // Arm wraps from the last slot back to the first.
        dlg.select_target = Some(SelectTarget::ColorSlot(1, 2));
        assert!(dlg.fill_armed_color(&mut data, None, 0x001F));
        assert_eq!(data.global.frames[0].payload[5], 0x001F);
        assert_eq!(dlg.select_target, Some(SelectTarget::ColorSlot(0, 0)));
    }

    #[test]
    fn fill_armed_color_ignores_tile_arms() {
        let (mut dlg, mut data) = dialog_with(palette_frame(1, 2, vec![0x0000, 0x0000]));
        dlg.select_target = Some(SelectTarget::Dest);
        assert!(!dlg.fill_armed_color(&mut data, None, 0x7FFF));
        assert_eq!(data.global.frames[0].payload, vec![0x0000, 0x0000]);
    }

    #[test]
    fn paste_row_fills_from_armed_field() {
        // LM v3.61: a clipboard palette row fills the slots starting at the
        // armed field; the count stops at the frame's end.
        let (mut dlg, mut data) = dialog_with(palette_frame(2, 2, vec![0x0000, 0x0000, 0x0000, 0x0000]));
        dlg.select_target = Some(SelectTarget::ColorSlot(0, 1));
        let row: Vec<u16> = (0..6).map(|i| 0x1000 + i).collect();
        let text = crate::ui::clipboard::ClipboardPayload::PaletteRow { colors: row }.encode();
        assert!(dlg.apply_pasted_palette_row(&text, &mut data, None, 0));
        // Slot (0,0) untouched; (0,1),(1,0),(1,1) filled; rest of row dropped.
        assert_eq!(data.global.frames[0].payload, vec![0x0000, 0x1000, 0x1001, 0x1002]);
        assert!(dlg.paste_status.as_ref().unwrap().contains("3 color"));
    }

    #[test]
    fn paste_row_rejects_non_palette_clipboard() {
        let (mut dlg, mut data) = dialog_with(palette_frame(1, 2, vec![0x1111, 0x2222]));
        assert!(!dlg.apply_pasted_palette_row("not a clipboard payload", &mut data, None, 0));
        assert_eq!(data.global.frames[0].payload, vec![0x1111, 0x2222]);
        assert!(dlg.paste_status.as_ref().unwrap().contains("no palette row"));
    }

    #[test]
    fn palette_frame_dests_reports_write_ranges() {
        let (dlg, data) = dialog_with(palette_frame(2, 3, vec![0; 6]));
        assert_eq!(dlg.palette_frame_dests(&data, None), vec![(0, 0x0002, 3)]);
    }
}
