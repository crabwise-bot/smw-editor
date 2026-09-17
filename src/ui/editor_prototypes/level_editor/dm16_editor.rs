//! Direct Map16 access (Lunar Magic v1.70–v1.90 parity).
//!
//! - "Add Objects / Direct Map16" window: drag-select a rectangular Map16
//!   pattern, arm placement, then click the canvas to drop it as one
//!   resizable level object (the pattern repeats on resize).
//! - Conditional Direct Map16: per-object RAM flag stored as inert metadata.
//! - Remap Direct Map16: old→new tile-ID mappings across the level's objects.
//! - Ctrl+Shift+RightClick: flood-fill an enclosed blank region with the
//!   current pattern.
//!
//! DM16 objects are undoable (`EditableDirectMap16`) and stamped into the
//! WRAM block map like the game's own Direct Map16 rendering would. They are
//! distinct from vanilla objects and persist in the editor-native RATS block.

use std::collections::VecDeque;

use egui::{vec2, Align2, Color32, CornerRadius, FontId, Pos2, Rect, RichText, Sense, Stroke, StrokeKind, Ui, Vec2};

use super::{object_layer::EditableDm16Object, UiLevelEditor};

// -------------------------------------------------------------------------------------------------

/// A Map16 pattern armed for canvas placement (from the Add Objects window).
#[derive(Clone, Debug)]
pub struct Dm16Placement {
    pub pw:    u32,
    pub ph:    u32,
    pub tiles: Vec<u16>,
}

/// 4-connected flood fill over `is_fillable`, decomposed into horizontal runs
/// that are merged vertically when adjacent rows share the same (x, w).
/// Returns merged (x, y, w, h) rects in tile coords.
pub fn flood_fill_runs(
    is_fillable: &mut dyn FnMut(u32, u32) -> bool, sx: u32, sy: u32, level_w: u32, level_h: u32,
) -> Vec<(u32, u32, u32, u32)> {
    if sx >= level_w || sy >= level_h || !is_fillable(sx, sy) {
        return Vec::new();
    }
    let mut visited = vec![false; (level_w as usize) * (level_h as usize)];
    let mut filled: Vec<(u32, u32)> = Vec::new();
    let mut queue = VecDeque::from([(sx, sy)]);
    visited[(sy * level_w + sx) as usize] = true;
    while let Some((x, y)) = queue.pop_front() {
        filled.push((x, y));
        for (nx, ny) in [(x.wrapping_sub(1), y), (x + 1, y), (x, y.wrapping_sub(1)), (x, y + 1)] {
            if nx < level_w && ny < level_h {
                let i = (ny * level_w + nx) as usize;
                if !visited[i] && is_fillable(nx, ny) {
                    visited[i] = true;
                    queue.push_back((nx, ny));
                }
            }
        }
    }
    // Horizontal runs per row (row-major order).
    filled.sort_by_key(|&(x, y)| (y, x));
    let mut runs: Vec<(u32, u32, u32)> = Vec::new();
    let mut i = 0;
    while i < filled.len() {
        let (x0, y) = filled[i];
        let mut x1 = x0;
        while i + 1 < filled.len() && filled[i + 1].1 == y && filled[i + 1].0 == x1 + 1 {
            i += 1;
            x1 = filled[i].0;
        }
        runs.push((x0, y, x1 - x0 + 1));
        i += 1;
    }
    // Merge vertically adjacent runs with identical (x, w).
    runs.sort_by_key(|&(x, y, w)| (x, w, y));
    let mut rects: Vec<(u32, u32, u32, u32)> = Vec::new();
    for (x, y, w) in runs {
        if let Some(last) = rects.last_mut() {
            if last.0 == x && last.2 == w && last.1 + last.3 == y {
                last.3 += 1;
                continue;
            }
        }
        rects.push((x, y, w, 1));
    }
    rects
}

// -------------------------------------------------------------------------------------------------

impl UiLevelEditor {
    /// Stamp every Direct Map16 object of the current level into the WRAM
    /// block map (clamped to the level bounds), reproducing what the game's
    /// own Direct Map16 rendering would draw.
    /// Stamp every Direct Map16 object into the WRAM block map, in order
    /// (later objects win overlaps). Uses the layer-1-forced write; callers
    /// that changed the model should prefer [`Self::rerasterize_dm16`].
    pub(super) fn stamp_dm16_tiles(&mut self) {
        let (level_w, level_h) = self.level_properties.level_dimensions_in_tiles();
        let objs = self.direct_map16.read(|d| d.objects.clone());
        for obj in &objs {
            for ly in 0..obj.h {
                for lx in 0..obj.w {
                    let tx = obj.x + lx;
                    let ty = obj.y + ly;
                    if tx < level_w && ty < level_h {
                        self.dm16_stamp_at(tx, ty, obj.tile_at(lx, ly));
                    }
                }
            }
        }
    }

    /// Record a Direct Map16 model change (dirty for save + undo routing).
    fn dm16_mark_dirty(&mut self) {
        self.dm16_dirty = true;
        self.last_undo_was_dm16 = false;
        self.mark_edited();
    }

    /// Drop the DM16 selection (called when the user edits anything else so
    /// Ctrl+Z/Ctrl+Y keep routing to the right layer).
    pub(super) fn disarm_dm16_selection(&mut self) {
        self.selected_dm16_indices.clear();
        self.last_undo_was_dm16 = false;
    }

    /// Topmost DM16 object covering absolute tile (tx, ty).
    fn dm16_at(&self, tx: u32, ty: u32) -> Option<usize> {
        self.direct_map16.read(|d| d.objects.iter().rposition(|o| o.covers(tx, ty)))
    }

    /// Click-select a DM16 object. Returns true when a DM16 object was hit
    /// (the caller should then clear the vanilla selections).
    pub(super) fn dm16_select_at(&mut self, pos: Pos2, origin: Pos2, tile_sz: f32) -> bool {
        let rel = (pos - origin) / tile_sz;
        let (tx, ty) = (rel.x.floor() as i32, rel.y.floor() as i32);
        let (level_w, level_h) = self.level_properties.level_dimensions_in_tiles();
        if tx < 0 || ty < 0 || tx as u32 >= level_w || ty as u32 >= level_h {
            return false;
        }
        match self.dm16_at(tx as u32, ty as u32) {
            Some(i) => {
                self.selected_dm16_indices.clear();
                self.selected_dm16_indices.insert(i);
                true
            }
            None => {
                self.disarm_dm16_selection();
                false
            }
        }
    }

    /// Add objects in a single undo step, select them, re-rasterize.
    pub(super) fn dm16_add_objects(&mut self, mut objs: Vec<EditableDm16Object>) {
        if objs.is_empty() {
            return;
        }
        for o in &mut objs {
            o.sanitize();
        }
        let n = objs.len();
        let base = self.direct_map16.write(|d| {
            let base = d.objects.len();
            d.objects.extend(objs);
            base
        });
        self.selected_object_indices.clear();
        self.selected_sprite_indices.clear();
        self.selected_dm16_indices.clear();
        self.selected_dm16_indices.extend(base..base + n);
        self.dm16_mark_dirty();
        self.rerasterize_dm16();
    }

    /// Drop the armed pattern at absolute tile (tx, ty), clamped to the level.
    pub(super) fn dm16_place_at(&mut self, tx: u32, ty: u32) {
        let Some(placement) = self.dm16_placing.take() else { return };
        let (level_w, level_h) = self.level_properties.level_dimensions_in_tiles();
        if tx >= level_w || ty >= level_h {
            return;
        }
        let w = placement.pw.min(level_w - tx).max(1);
        let h = placement.ph.min(level_h - ty).max(1);
        let obj = EditableDm16Object {
            x: tx,
            y: ty,
            w,
            h,
            pw: placement.pw,
            ph: placement.ph,
            tiles: placement.tiles,
            condition: None,
        };
        self.dm16_add_objects(vec![obj]);
        self.dm16_status =
            Some(format!("Placed Direct Map16 object {w}×{h} at ({tx}, {ty}) — resize repeats the pattern"));
    }

    /// Delete the selected DM16 objects (one undo step) and re-rasterize,
    /// restoring the vanilla tiles their footprints covered.
    pub(super) fn dm16_delete_selected(&mut self) {
        if self.selected_dm16_indices.is_empty() {
            return;
        }
        let indices: Vec<usize> = self.selected_dm16_indices.iter().copied().collect();
        let n_deleted =
            self.direct_map16.read(|d| d.objects.iter().enumerate().filter(|(i, _)| indices.contains(i)).count());
        self.direct_map16.write(|d| {
            let mut keep = Vec::with_capacity(d.objects.len());
            for (i, obj) in d.objects.drain(..).enumerate() {
                if !indices.contains(&i) {
                    keep.push(obj);
                }
            }
            d.objects = keep;
        });
        self.disarm_dm16_selection();
        self.dm16_mark_dirty();
        self.rerasterize_dm16();
        self.dm16_status = Some(format!("Deleted {n_deleted} Direct Map16 object(s)"));
    }

    /// Undo on the DM16 layer: revert the model, then re-rasterize from the
    /// vanilla base snapshot so hidden vanilla tiles come back intact.
    /// Keeps the (clamped) selection so repeated Ctrl+Z keeps targeting DM16.
    pub(super) fn handle_dm16_undo(&mut self) {
        if !self.direct_map16.can_undo() {
            return;
        }
        self.direct_map16.undo();
        self.rerasterize_dm16();
        let n = self.direct_map16.read(|d| d.objects.len());
        self.selected_dm16_indices.retain(|&i| i < n);
        self.dm16_dirty = true;
        self.last_undo_was_dm16 = true;
        self.mark_edited();
    }

    /// Redo on the DM16 layer (mirror of undo).
    pub(super) fn handle_dm16_redo(&mut self) {
        if !self.direct_map16.can_redo() {
            return;
        }
        self.direct_map16.redo();
        self.rerasterize_dm16();
        let n = self.direct_map16.read(|d| d.objects.len());
        self.selected_dm16_indices.retain(|&i| i < n);
        self.dm16_dirty = true;
        self.last_undo_was_dm16 = true;
        self.mark_edited();
    }

    // ── Canvas gestures ──────────────────────────────────────────────

    /// Direct Map16 canvas gestures. Returns true when the click was consumed.
    /// - Ctrl+Shift+RightClick: flood-fill the enclosed blank region.
    /// - Primary click with an armed placement: drop the pattern.
    /// - Right-click with an armed placement: cancel it.
    pub(super) fn handle_dm16_canvas_click(
        &mut self, resp: &egui::Response, origin: Pos2, tile_sz: f32, modifiers: egui::Modifiers,
    ) -> bool {
        let Some(cursor) = resp.hover_pos() else { return false };
        let rel = (cursor - origin) / tile_sz;
        let (tx, ty) = (rel.x.floor() as i32, rel.y.floor() as i32);
        let (level_w, level_h) = self.level_properties.level_dimensions_in_tiles();
        if tx < 0 || ty < 0 || tx as u32 >= level_w || ty as u32 >= level_h {
            return false;
        }
        let (tx, ty) = (tx as u32, ty as u32);
        if resp.clicked_by(egui::PointerButton::Secondary) {
            if modifiers.ctrl && modifiers.shift {
                self.dm16_flood_fill_at(tx, ty);
                return true;
            }
            if self.dm16_placing.is_some() {
                self.dm16_placing = None;
                self.dm16_status = Some("Placement cancelled".to_string());
                return true;
            }
            return false;
        }
        if self.dm16_placing.is_some() && resp.clicked_by(egui::PointerButton::Primary) {
            self.dm16_place_at(tx, ty);
            return true;
        }
        false
    }

    /// Flood-fill the enclosed blank region at (tx, ty) with the current
    /// Map16 pattern (Add Objects selection, else the draw block as 1×1).
    /// One undo step; each merged run becomes one object.
    pub(super) fn dm16_flood_fill_at(&mut self, tx: u32, ty: u32) {
        let (level_w, level_h) = self.level_properties.level_dimensions_in_tiles();
        let rects = flood_fill_runs(&mut |x, y| self.dm16_block_id_at(x, y) == Some(0x25), tx, ty, level_w, level_h);
        if rects.is_empty() {
            self.dm16_status = Some("Flood fill found nothing: click inside an enclosed blank (0x25) area".to_string());
            return;
        }
        let (pw, ph, pattern) = self.dm16_pattern_tiles();
        let objs: Vec<EditableDm16Object> = rects
            .iter()
            .map(|&(x, y, w, h)| EditableDm16Object { x, y, w, h, pw, ph, tiles: pattern.clone(), condition: None })
            .collect();
        let n = objs.len();
        self.dm16_add_objects(objs);
        self.dm16_status = Some(format!("Flood-filled {n} region(s) with the {pw}×{ph} Map16 pattern"));
    }

    /// The current fill/placement pattern: the Add Objects Map16 selection,
    /// else the draw block as a 1×1 pattern.
    fn dm16_pattern_tiles(&self) -> (u32, u32, Vec<u16>) {
        if let Some((sx, sy, w, h)) = self.dm16_selection {
            let mut tiles = Vec::with_capacity((w * h) as usize);
            for by in 0..h {
                for bx in 0..w {
                    tiles.push(((sy + by) * 16 + (sx + bx)) as u16);
                }
            }
            (w, h, tiles)
        } else {
            (1, 1, vec![self.draw_block_id])
        }
    }

    // ── Overlay ──────────────────────────────────────────────────────

    /// Draw DM16 object outlines + the armed-placement ghost on the canvas.
    /// Purple = Direct Map16 (distinct from vanilla objects); cyan = selected.
    pub(super) fn dm16_overlay(&self, painter: &egui::Painter, origin: Pos2, tile_sz: f32) {
        let z = self.zoom;
        let objs = self.direct_map16.read(|d| d.objects.clone());
        for (i, obj) in objs.iter().enumerate() {
            let rect = Rect::from_min_size(
                origin + Vec2::new(obj.x as f32 * tile_sz, obj.y as f32 * tile_sz),
                Vec2::new(obj.w as f32 * tile_sz, obj.h as f32 * tile_sz),
            );
            let selected = self.selected_dm16_indices.contains(&i);
            let (fill, stroke_col) = if selected {
                (Color32::from_rgba_unmultiplied(0, 200, 255, 45), Color32::from_rgb(0, 210, 255))
            } else {
                (Color32::from_rgba_unmultiplied(180, 80, 255, 30), Color32::from_rgb(190, 120, 255))
            };
            painter.rect_filled(rect, CornerRadius::same(2), fill);
            painter.rect_stroke(rect, CornerRadius::same(2), Stroke::new(2.0_f32, stroke_col), StrokeKind::Outside);
            if self.show_object_labels && z >= 0.9 {
                let mut label = String::from("DM16");
                if obj.condition.is_some() {
                    label.push_str(" C");
                }
                painter.text(
                    rect.left_top() + Vec2::new(2.0, 1.0),
                    Align2::LEFT_TOP,
                    label,
                    FontId::monospace(9.0),
                    Color32::WHITE,
                );
            }
        }
        // Armed placement ghost is drawn by the caller (it needs hover pos);
        // see central_panel's placement preview below.
    }

    /// Ghost preview of the armed placement at the hover tile.
    pub(super) fn dm16_placement_preview(&self, painter: &egui::Painter, origin: Pos2, tile_sz: f32, tx: u32, ty: u32) {
        let Some(placement) = &self.dm16_placing else { return };
        let (level_w, level_h) = self.level_properties.level_dimensions_in_tiles();
        let w = placement.pw.min(level_w.saturating_sub(tx)).max(1);
        let h = placement.ph.min(level_h.saturating_sub(ty)).max(1);
        let rect = Rect::from_min_size(
            origin + Vec2::new(tx as f32 * tile_sz, ty as f32 * tile_sz),
            Vec2::new(w as f32 * tile_sz, h as f32 * tile_sz),
        );
        painter.rect_filled(rect, CornerRadius::ZERO, Color32::from_rgba_unmultiplied(0, 210, 255, 60));
        painter.rect_stroke(
            rect,
            CornerRadius::ZERO,
            Stroke::new(2.0_f32, Color32::from_rgb(0, 210, 255)),
            StrokeKind::Outside,
        );
        painter.text(
            rect.left_top() + Vec2::new(2.0, 1.0),
            Align2::LEFT_TOP,
            format!("DM16 {}×{} — click to place", placement.pw, placement.ph),
            FontId::monospace(10.0),
            Color32::WHITE,
        );
    }
}
impl UiLevelEditor {
    /// Update object `i` through `f`, then re-rasterize (one undo step).
    /// Selection-agnostic. Vanilla tiles under the object are restored from
    /// the DM16-free base snapshot, never blanked.
    pub(super) fn dm16_update_object(&mut self, i: usize, f: impl FnOnce(&mut EditableDm16Object)) {
        let valid = self.direct_map16.read(|d| d.objects.get(i).is_some());
        if !valid {
            return;
        }
        self.direct_map16.write(|d| {
            if let Some(obj) = d.objects.get_mut(i) {
                f(obj);
                obj.sanitize();
            }
        });
        self.dm16_mark_dirty();
        self.rerasterize_dm16();
    }

    /// All three DM16 windows: Add Objects, Conditional, Remap.
    pub(super) fn dm16_windows(&mut self, ui: &mut Ui) {
        self.dm16_add_window(ui);
        self.dm16_conditional_window(ui);
        self.dm16_remap_window(ui);
        if let Some(status) = self.dm16_status.take() {
            // Surface transient DM16 feedback through the existing MWL status line.
            self.mwl_status = Some(status);
        }
    }

    /// "Add Objects / Direct Map16": drag-select a rectangular Map16 pattern
    /// from the 512-block picker, then arm canvas placement.
    fn dm16_add_window(&mut self, ui: &mut Ui) {
        let mut open = self.dm16_add_open;
        egui::Window::new("Add Objects / Direct Map16").open(&mut open).default_size(Vec2::new(340.0, 560.0)).show(
            ui.ctx(),
            |ui| {
                ui.label("Drag a rectangular Map16 pattern, then arm placement and click the level.");
                let tex = self.tile_picker.texture(ui.ctx());
                let tex_size = tex.size();
                let max_w = ui.available_width().min(300.0);
                let display_w = max_w.min(tex_size[0] as f32 * 1.5);
                let display_h = display_w * (tex_size[1] as f32 / tex_size[0] as f32);
                let (rect, resp) = ui.allocate_exact_size(vec2(display_w, display_h), Sense::click_and_drag());
                ui.painter().image(tex.id(), rect, Rect::from_min_size(Pos2::ZERO, vec2(1.0, 1.0)), Color32::WHITE);

                let to_tile = |pos: Pos2| -> Option<(u32, u32)> {
                    let rel = pos - rect.min;
                    let px = rel.x / display_w * tex_size[0] as f32;
                    let py = rel.y / display_h * tex_size[1] as f32;
                    self.tile_picker.block_at_pixel(px, py).map(|id| (id as u32 % 16, id as u32 / 16))
                };

                if resp.drag_started_by(egui::PointerButton::Primary) {
                    if let Some(p) = resp.interact_pointer_pos().and_then(to_tile) {
                        self.dm16_sel_drag_start = Some(p);
                        self.dm16_selection = Some((p.0, p.1, 1, 1));
                    }
                }
                if resp.dragged_by(egui::PointerButton::Primary) {
                    if let (Some(start), Some(cur)) = (self.dm16_sel_drag_start, resp.hover_pos().and_then(to_tile)) {
                        let sx = start.0.min(cur.0);
                        let sy = start.1.min(cur.1);
                        let w = start.0.max(cur.0) - sx + 1;
                        let h = start.1.max(cur.1) - sy + 1;
                        self.dm16_selection = Some((sx, sy, w, h));
                    }
                }
                if resp.drag_stopped_by(egui::PointerButton::Primary) {
                    self.dm16_sel_drag_start = None;
                }

                // Selection outline.
                if let Some((sx, sy, w, h)) = self.dm16_selection {
                    let scale_x = display_w / tex_size[0] as f32;
                    let scale_y = display_h / tex_size[1] as f32;
                    let sel = Rect::from_min_size(
                        rect.min + vec2(sx as f32 * 16.0 * scale_x, sy as f32 * 16.0 * scale_y),
                        vec2(w as f32 * 16.0 * scale_x, h as f32 * 16.0 * scale_y),
                    );
                    ui.painter().rect_stroke(
                        sel,
                        CornerRadius::ZERO,
                        Stroke::new(2.0_f32, Color32::YELLOW),
                        StrokeKind::Outside,
                    );
                }

                if let Some((sx, sy, w, h)) = self.dm16_selection {
                    ui.label(format!(
                        "Pattern: {w}×{h} blocks (${:03X}–${:03X})",
                        sy * 16 + sx,
                        (sy + h - 1) * 16 + (sx + w - 1)
                    ));
                } else {
                    ui.label("No pattern selected yet — drag on the picker.");
                }

                ui.horizontal(|ui| {
                    let can_arm = self.dm16_selection.is_some();
                    if ui.add_enabled(can_arm, egui::Button::new("Place on level…")).clicked() {
                        if let Some((sx, sy, w, h)) = self.dm16_selection {
                            let mut tiles: Vec<u16> = Vec::with_capacity((w * h) as usize);
                            for by in 0..h {
                                for bx in 0..w {
                                    tiles.push(((sy + by) * 16 + (sx + bx)) as u16);
                                }
                            }
                            self.dm16_placing = Some(Dm16Placement { pw: w, ph: h, tiles });
                            self.dm16_status = Some(format!(
                                "Direct Map16 pattern {w}×{h} armed — click the level to place, right-click to cancel"
                            ));
                        }
                    }
                    if self.dm16_placing.is_some() && ui.button("Cancel placement").clicked() {
                        self.dm16_placing = None;
                    }
                });
                if self.dm16_placing.is_some() {
                    ui.colored_label(Color32::from_rgb(0, 210, 255), "Placement armed: click the level canvas.");
                }
                ui.separator();
                ui.small(
                    "Tip: Ctrl+Shift+RightClick a blank enclosed area to flood-fill it with the selected pattern.",
                );
            },
        );
        self.dm16_add_open = open;
    }

    /// "Conditional Direct Map16": attach a RAM flag to one object.
    fn dm16_conditional_window(&mut self, ui: &mut Ui) {
        let Some(idx) = self.dm16_cond_open else { return };
        let mut open = true;
        let mut close = false;
        let mut apply: Option<(u16, u8)> = None;
        let mut clear = false;
        let still_valid = self.direct_map16.read(|d| idx < d.objects.len());
        if !still_valid {
            self.dm16_cond_open = None;
            return;
        }
        egui::Window::new("Conditional Direct Map16").open(&mut open).show(ui.ctx(), |ui| {
            ui.label(format!("Object #{idx} renders only when the flag is set."));
            ui.horizontal(|ui| {
                ui.label("RAM address:");
                ui.add(egui::Slider::new(&mut self.dm16_cond_addr, 0..=0xFFFF).hexadecimal(4, false, false));
            });
            ui.horizontal(|ui| {
                ui.label("Bit:");
                ui.add(egui::Slider::new(&mut self.dm16_cond_bit, 0..=8));
            });
            ui.small(if self.dm16_cond_bit <= 7 {
                format!("Requires bit {} of ${:04X} to be set.", self.dm16_cond_bit, self.dm16_cond_addr)
            } else {
                format!("Requires byte ${:04X} to be nonzero.", self.dm16_cond_addr)
            });
            ui.separator();
            ui.colored_label(
                Color32::YELLOW,
                "Stock ROM: this condition is inert metadata. It does nothing until an ASM patch evaluates it at runtime.",
            );
            ui.horizontal(|ui| {
                if ui.button("Set condition").clicked() {
                    apply = Some((self.dm16_cond_addr as u16, self.dm16_cond_bit as u8));
                    close = true;
                }
                if ui.button("Clear condition").clicked() {
                    clear = true;
                    close = true;
                }
            });
        });
        if !open {
            close = true;
        }
        if close {
            self.dm16_cond_open = None;
        }
        if let Some((ram_addr, bit)) = apply {
            self.dm16_update_object(idx, |o| o.condition = Some((ram_addr, bit)));
            self.dm16_status = Some(format!("Conditional flag set: ${ram_addr:04X} bit {bit}"));
        } else if clear {
            self.dm16_update_object(idx, |o| o.condition = None);
            self.dm16_status = Some("Condition cleared".to_string());
        }
    }

    /// "Remap Direct Map16": old→new tile IDs across the level's DM16 objects.
    fn dm16_remap_window(&mut self, ui: &mut Ui) {
        let mut open = self.dm16_remap_open;
        let mut apply = false;
        egui::Window::new("Remap Direct Map16").open(&mut open).show(ui.ctx(), |ui| {
            ui.label("Replace Map16 tile IDs in every Direct Map16 object of this level.");
            let mut remove: Option<usize> = None;
            for (i, (old, new)) in self.dm16_remap_rows.iter_mut().enumerate() {
                ui.horizontal(|ui| {
                    ui.label("Old:");
                    ui.add(egui::Slider::new(old, 0..=0x1FF).hexadecimal(3, false, false));
                    ui.label("New:");
                    ui.add(egui::Slider::new(new, 0..=0x1FF).hexadecimal(3, false, false));
                    if ui.small_button("✕").clicked() {
                        remove = Some(i);
                    }
                });
            }
            if let Some(i) = remove {
                self.dm16_remap_rows.remove(i);
            }
            ui.horizontal(|ui| {
                if ui.button("Add row").clicked() {
                    self.dm16_remap_rows.push((0, 0));
                }
                if ui.button("Apply remap").clicked() {
                    apply = true;
                }
            });
        });
        self.dm16_remap_open = open;
        if apply {
            let rows: Vec<(u16, u16)> = self.dm16_remap_rows.iter().map(|&(o, n)| (o as u16, n as u16)).collect();
            if rows.is_empty() {
                self.dm16_status = Some("Remap: no rows to apply".to_string());
                return;
            }
            let mut replaced = 0usize;
            self.direct_map16.write(|d| {
                for obj in &mut d.objects {
                    for t in &mut obj.tiles {
                        for &(old, new) in &rows {
                            if *t == old {
                                *t = new;
                                replaced += 1;
                                break;
                            }
                        }
                    }
                }
            });
            self.dm16_mark_dirty();
            // Re-rasterize: the objects keep their footprints, only tile IDs
            // change; vanilla tiles underneath stay intact.
            self.rerasterize_dm16();
            self.dm16_remap_open = false;
            self.dm16_status =
                Some(format!("Remapped {replaced} Map16 tile(s) across the level's Direct Map16 objects"));
        }
    }

    /// Left-panel inspector section for the DM16 selection.
    pub(super) fn dm16_inspector(&mut self, ui: &mut Ui) {
        if self.selected_dm16_indices.is_empty() {
            return;
        }
        ui.separator();
        ui.label(RichText::new("Selected Direct Map16:").strong());
        let indices: Vec<usize> = self.selected_dm16_indices.iter().copied().collect();
        let n_objs = self.direct_map16.read(|d| d.objects.len());
        let indices: Vec<usize> = indices.into_iter().filter(|&i| i < n_objs).collect();
        self.selected_dm16_indices = indices.iter().copied().collect();
        if indices.len() == 1 {
            let idx = indices[0];
            let Some(obj) = self.direct_map16.read(|d| d.objects.get(idx).cloned()) else { return };
            let (level_w, level_h) = self.level_properties.level_dimensions_in_tiles();
            ui.label(format!(
                "  Pattern: {}×{} blocks (first ${:03X})",
                obj.pw,
                obj.ph,
                obj.tiles.first().copied().unwrap_or(0)
            ));
            ui.label(format!("  Object: {}×{} tiles", obj.w, obj.h));
            let mut changed = false;
            let mut new_x = obj.x as i32;
            let mut new_y = obj.y as i32;
            let mut new_w = obj.w as i32;
            let mut new_h = obj.h as i32;
            ui.horizontal(|ui| {
                ui.label("X:");
                changed |= ui.add(egui::Slider::new(&mut new_x, 0..=(level_w.saturating_sub(1) as i32))).changed();
            });
            ui.horizontal(|ui| {
                ui.label("Y:");
                changed |= ui.add(egui::Slider::new(&mut new_y, 0..=(level_h.saturating_sub(1) as i32))).changed();
            });
            ui.horizontal(|ui| {
                ui.label("W:");
                changed |= ui.add(egui::Slider::new(&mut new_w, 1..=64)).changed();
            });
            ui.horizontal(|ui| {
                ui.label("H:");
                changed |= ui.add(egui::Slider::new(&mut new_h, 1..=64)).changed();
            });
            if changed {
                let nx = (new_x as u32).min(level_w.saturating_sub(1));
                let ny = (new_y as u32).min(level_h.saturating_sub(1));
                self.dm16_update_object(idx, |o| {
                    o.x = nx;
                    o.y = ny;
                    o.w = new_w.max(1) as u32;
                    o.h = new_h.max(1) as u32;
                });
            }
            ui.small("Resizing repeats the pattern (like Lunar Magic).");
            match &obj.condition {
                Some((ram_addr, bit)) => {
                    ui.label(format!(
                        "  Condition: ${ram_addr:04X} {}",
                        if *bit <= 7 { format!("bit {bit}") } else { "nonzero".to_string() }
                    ));
                }
                None => {
                    ui.label("  Condition: none");
                }
            }
            ui.horizontal(|ui| {
                if ui.button("Set condition…").clicked() {
                    self.dm16_cond_addr = obj.condition.map(|(a, _)| a as u32).unwrap_or(0x13CE);
                    self.dm16_cond_bit = obj.condition.map(|(_, b)| b as i32).unwrap_or(0);
                    self.dm16_cond_open = Some(idx);
                }
                if ui.button("Remap tiles…").clicked() {
                    if self.dm16_remap_rows.is_empty() {
                        self.dm16_remap_rows.push((0, 0));
                    }
                    self.dm16_remap_open = true;
                }
            });
            ui.colored_label(Color32::YELLOW, "Condition is inert on a stock ROM — it needs an ASM patch to evaluate.");
        } else {
            ui.label(format!("  {} Direct Map16 objects selected", indices.len()));
        }
        ui.small("Delete key removes the selected Direct Map16 objects.");
    }
}

#[cfg(test)]
mod dm16_editor_tests {
    use std::collections::HashSet;

    use super::flood_fill_runs;

    /// Build a fillable predicate from a set of (x, y) tiles.
    fn pred_of(set: &HashSet<(u32, u32)>) -> impl FnMut(u32, u32) -> bool + '_ {
        move |x, y| set.contains(&(x, y))
    }

    fn full_rect(w: u32, h: u32) -> HashSet<(u32, u32)> {
        (0..w).flat_map(|x| (0..h).map(move |y| (x, y))).collect()
    }

    #[test]
    fn flood_fill_single_run_merges_rows() {
        let set = full_rect(4, 3);
        let mut p = pred_of(&set);
        let rects = flood_fill_runs(&mut p, 1, 1, 4, 3);
        assert_eq!(rects, vec![(0, 0, 4, 3)]);
    }

    #[test]
    fn flood_fill_respects_enclosure() {
        // A 5x5 area with a wall column at x=2; fill from the left side.
        let mut set = HashSet::new();
        for x in 0..5u32 {
            for y in 0..5u32 {
                if x != 2 {
                    set.insert((x, y));
                }
            }
        }
        let mut p = pred_of(&set);
        let rects = flood_fill_runs(&mut p, 0, 0, 5, 5);
        assert_eq!(rects, vec![(0, 0, 2, 5)]);
    }

    #[test]
    fn flood_fill_l_shape_decomposes_to_runs() {
        // L: row 0 has x=0..3, rows 1..2 have x=0 only.
        let mut set = HashSet::new();
        for x in 0..3u32 {
            set.insert((x, 0));
        }
        set.insert((0, 1));
        set.insert((0, 2));
        let mut p = pred_of(&set);
        let mut rects = flood_fill_runs(&mut p, 0, 0, 3, 3);
        rects.sort();
        assert_eq!(rects, vec![(0, 0, 3, 1), (0, 1, 1, 2)]);
    }

    #[test]
    fn flood_fill_empty_and_out_of_bounds() {
        let mut set = HashSet::new();
        let mut p = pred_of(&set);
        assert!(flood_fill_runs(&mut p, 0, 0, 8, 8).is_empty());
        let full = full_rect(8, 8);
        let mut p2 = pred_of(&full);
        assert!(flood_fill_runs(&mut p2, 9, 9, 8, 8).is_empty());
        // Start on a non-fillable tile inside the bounds.
        let mut p3 = pred_of(&full);
        assert!(flood_fill_runs(&mut p3, 0, 0, 0, 0).is_empty());
    }

    #[test]
    fn flood_fill_4_connected_not_diagonal() {
        // Two tiles touching only diagonally are separate regions.
        let mut set = HashSet::from([(0, 0), (1, 1)]);
        let mut p = pred_of(&set);
        let rects = flood_fill_runs(&mut p, 0, 0, 4, 4);
        assert_eq!(rects, vec![(0, 0, 1, 1)]);
    }
}
