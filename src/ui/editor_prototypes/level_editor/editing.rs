use egui::Pos2;

use super::{object_layer::EditableObject, UiLevelEditor};
use crate::ui::editing_mode::EditingMode;

impl UiLevelEditor {
    pub(super) fn handle_editing_interaction(&mut self, resp: &egui::Response, origin: Pos2, tile_sz: f32) {
        if self.edit_sprites {
            match self.editing_mode {
                EditingMode::Select | EditingMode::Probe => {
                    if resp.clicked_by(egui::PointerButton::Primary) {
                        if let Some(pos) = resp.hover_pos() {
                            self.select_sprite_at(pos, origin, tile_sz);
                        }
                    }
                }
                EditingMode::Erase => {
                    if resp.clicked_by(egui::PointerButton::Primary) {
                        if let Some(pos) = resp.hover_pos() {
                            self.erase_sprite_at(pos, origin, tile_sz);
                        }
                    }
                }
                EditingMode::Draw => {
                    if resp.clicked_by(egui::PointerButton::Primary) {
                        if let Some(pos) = resp.hover_pos() {
                            self.place_sprite_at(pos, origin, tile_sz);
                        }
                    }
                }
                _ => {}
            }
            return;
        }
        match self.editing_mode {
            EditingMode::Select | EditingMode::Probe => {
                // A drag that just ended with a change consumed the release;
                // don't also treat it as a click-select.
                if !self.suppress_click_select && resp.clicked_by(egui::PointerButton::Primary) {
                    if let Some(pos) = resp.hover_pos() {
                        self.select_object_at(pos, origin, tile_sz);
                    }
                }
            }
            EditingMode::Erase => {
                if resp.clicked_by(egui::PointerButton::Primary) {
                    if let Some(pos) = resp.hover_pos() {
                        self.erase_object_at(pos, origin, tile_sz);
                    }
                }
            }
            EditingMode::Draw => {
                if resp.clicked_by(egui::PointerButton::Primary) {
                    if let Some(pos) = resp.hover_pos() {
                        self.place_object_at(pos, origin, tile_sz);
                    }
                }
            }
            _ => {}
        }
    }

    fn sprite_at(&mut self, pos: Pos2, origin: Pos2, _tile_sz: f32) -> Option<usize> {
        let rel_px = (pos - origin) / self.zoom;
        let sprite_entries = self.sprites.read(|sprites| sprites.sprites.clone());
        for (i, spr) in sprite_entries.iter().enumerate().rev() {
            let (min_dx, min_dy, max_dx, max_dy) = self.sprite_pixel_bounds(spr.sprite_id).unwrap_or((0, 0, 16, 16));
            let left = spr.x as f32 * 16.0 + min_dx as f32;
            let top = spr.y as f32 * 16.0 + min_dy as f32;
            let right = spr.x as f32 * 16.0 + max_dx as f32;
            let bottom = spr.y as f32 * 16.0 + max_dy as f32;
            if rel_px.x >= left && rel_px.x < right && rel_px.y >= top && rel_px.y < bottom {
                return Some(i);
            }
        }
        None
    }

    fn select_sprite_at(&mut self, pos: Pos2, origin: Pos2, tile_sz: f32) {
        let idx = self.sprite_at(pos, origin, tile_sz);
        self.selected_sprite_indices.clear();
        if let Some(i) = idx {
            self.selected_sprite_indices.insert(i);
        }
    }

    fn erase_sprite_at(&mut self, pos: Pos2, origin: Pos2, tile_sz: f32) {
        if let Some(idx) = self.sprite_at(pos, origin, tile_sz) {
            self.sprites.write(|sprites| {
                sprites.sprites.remove(idx);
            });
            self.mark_edited();
            self.selected_sprite_indices.clear();
            self.rebuild_sprite_tiles();
        }
    }

    fn place_sprite_at(&mut self, pos: Pos2, origin: Pos2, tile_sz: f32) {
        let rel = (pos - origin) / tile_sz;
        let target_px_x = rel.x.floor() * 16.0;
        let target_px_y = rel.y.floor() * 16.0;
        let (min_dx, min_dy, _, _) = self.sprite_pixel_bounds(self.draw_sprite_id).unwrap_or((0, 0, 16, 16));
        let anchor_x = ((target_px_x - min_dx as f32) / 16.0).round().max(0.0) as u32;
        let anchor_y = ((target_px_y - min_dy as f32) / 16.0).round().max(0.0) as u32;
        let new_idx = self.sprites.read(|sprites| sprites.sprites.len());
        self.sprites.write(|sprites| {
            sprites.sprites.push(super::sprite_layer::EditableSprite {
                x:          anchor_x,
                y:          anchor_y,
                sprite_id:  self.draw_sprite_id,
                extra_bits: self.draw_sprite_extra_bits,
            });
        });
        self.mark_edited();
        self.selected_sprite_indices.clear();
        self.selected_sprite_indices.insert(new_idx);
        self.rebuild_sprite_tiles();
    }

    fn object_at(&self, pos: Pos2, origin: Pos2, tile_sz: f32) -> Option<usize> {
        let rel = (pos - origin) / tile_sz;
        let tx = rel.x.floor();
        let ty = rel.y.floor();

        let layer_data = self.editing_objects()?;
        layer_data.read(|layer| {
            // Iterate in reverse so topmost (last-placed) objects are hit first.
            for (i, obj) in layer.objects.iter().enumerate().rev() {
                let w = if obj.is_extended { 1.0 } else { ((obj.settings & 0x0F) as f32) + 1.0 };
                let h = if obj.is_extended { 1.0 } else { ((obj.settings >> 4) as f32) + 1.0 };
                if tx >= obj.x as f32 && tx < obj.x as f32 + w && ty >= obj.y as f32 && ty < obj.y as f32 + h {
                    return Some(i);
                }
            }
            None
        })
    }

    fn select_object_at(&mut self, pos: Pos2, origin: Pos2, tile_sz: f32) {
        let idx = self.object_at(pos, origin, tile_sz);
        self.selected_object_indices.clear();
        if let Some(i) = idx {
            self.selected_object_indices.insert(i);
        }
    }

    fn erase_object_at(&mut self, pos: Pos2, origin: Pos2, tile_sz: f32) {
        if self.edit_layer == 2 && self.layer2_objects.is_none() {
            let rel = (pos - origin) / tile_sz;
            let tx = rel.x.floor() as u32;
            let ty = rel.y.floor() as u32;
            let idx = self.block_map_index(tx, ty) as usize;
            if let Some(bg) = &mut self.layer2_background {
                bg.write(|layer| {
                    if let Some(tile) = layer.tile_ids.get_mut(idx) {
                        *tile = 0;
                    }
                });
                self.mark_edited();
            }
            self.set_block_id_at(tx, ty, 0);
            self.rebuild_tiles();
            return;
        }
        if let Some(idx) = self.object_at(pos, origin, tile_sz) {
            // Read object bounds before deleting.
            let layer_data = self.editing_objects().expect("editable object layer missing");
            let (ox, oy, ow, oh) = layer_data.read(|layer| {
                let obj = &layer.objects[idx];
                let w = if obj.is_extended { 1 } else { (obj.settings & 0x0F) + 1 };
                let h = if obj.is_extended { 1 } else { (obj.settings >> 4) + 1 };
                (obj.x, obj.y, w as u32, h as u32)
            });

            // Delete the object.
            self.editing_objects_mut().expect("editable object layer missing").write(|layer| {
                layer.objects.remove(idx);
            });
            self.mark_edited();
            self.selected_object_indices.clear();

            // Blank out the tiles.
            for dy in 0..oh {
                for dx in 0..ow {
                    self.set_block_id_at(ox + dx, oy + dy, 0x25);
                }
            }
            self.rebuild_tiles();
        }
    }

    fn place_object_at(&mut self, pos: Pos2, origin: Pos2, tile_sz: f32) {
        let rel = (pos - origin) / tile_sz;
        let tx = rel.x.floor() as u32;
        let ty = rel.y.floor() as u32;

        if self.edit_layer == 2 && self.layer2_objects.is_none() {
            let idx = self.block_map_index(tx, ty) as usize;
            let draw_block = self.draw_block_id.min(0xFF) as u8;
            if let Some(bg) = &mut self.layer2_background {
                bg.write(|layer| {
                    if let Some(tile) = layer.tile_ids.get_mut(idx) {
                        *tile = draw_block;
                    }
                });
                self.mark_edited();
                self.set_block_id_at(tx, ty, draw_block as u16);
                self.rebuild_tiles();
            }
            return;
        }

        let w =
            if self.draw_object_settings & 0x0F == 0 { 1_u32 } else { ((self.draw_object_settings & 0x0F) + 1) as u32 };
        let h = if self.draw_object_settings >> 4 == 0 { 1_u32 } else { ((self.draw_object_settings >> 4) + 1) as u32 };

        let new_obj = EditableObject {
            x:           tx,
            y:           ty,
            id:          self.draw_object_id,
            settings:    self.draw_object_settings,
            is_extended: false,
            extended_id: 0,
        };

        let layer_data = self.editing_objects_mut().expect("editable object layer missing");
        let new_idx = layer_data.read(|layer| layer.objects.len());
        layer_data.write(|layer| {
            layer.objects.push(new_obj);
        });
        self.mark_edited();
        self.selected_object_indices.clear();
        self.selected_object_indices.insert(new_idx);

        // Write block IDs into the WRAM block map.
        let block_id = self.draw_block_id;
        for dy in 0..h {
            for dx in 0..w {
                self.set_block_id_at(tx + dx, ty + dy, block_id);
            }
        }
        self.rebuild_tiles();
    }

    pub(super) fn delete_selected_objects(&mut self) {
        if self.edit_sprites {
            if self.selected_sprite_indices.is_empty() {
                return;
            }
            let indices: Vec<usize> = self.selected_sprite_indices.iter().copied().collect();
            self.sprites.write(|sprites| {
                let mut keep = Vec::with_capacity(sprites.sprites.len());
                for (i, spr) in sprites.sprites.drain(..).enumerate() {
                    if !indices.contains(&i) {
                        keep.push(spr);
                    }
                }
                sprites.sprites = keep;
            });
            self.mark_edited();
            self.selected_sprite_indices.clear();
            self.rebuild_sprite_tiles();
            return;
        }
        if self.selected_object_indices.is_empty() {
            return;
        }
        if self.edit_layer == 2 && self.layer2_objects.is_none() {
            return;
        }
        // Read object bounds before deleting.
        let layer_data = self.editing_objects().expect("editable object layer missing");
        let objects_to_blank: Vec<(u32, u32, u32, u32)> = layer_data.read(|layer| {
            self.selected_object_indices
                .iter()
                .filter_map(|&i| layer.objects.get(i))
                .map(|obj| {
                    let w = if obj.is_extended { 1 } else { (obj.settings & 0x0F) as u32 + 1 };
                    let h = if obj.is_extended { 1 } else { (obj.settings >> 4) as u32 + 1 };
                    (obj.x, obj.y, w, h)
                })
                .collect()
        });

        // Collect indices and delete objects.
        let indices: Vec<usize> = self.selected_object_indices.iter().copied().collect();
        self.editing_objects_mut().expect("editable object layer missing").write(|layer| {
            let mut keep = Vec::with_capacity(layer.objects.len());
            for (i, obj) in layer.objects.drain(..).enumerate() {
                if !indices.contains(&i) {
                    keep.push(obj);
                }
            }
            layer.objects = keep;
        });
        self.mark_edited();
        self.selected_object_indices.clear();

        // Blank out the tiles.
        for (ox, oy, w, h) in objects_to_blank {
            for dy in 0..h {
                for dx in 0..w {
                    self.set_block_id_at(ox + dx, oy + dy, 0x25);
                }
            }
        }
        self.rebuild_tiles();
    }

    pub(super) fn handle_undo(&mut self) {
        if self.edit_sprites {
            self.sprites.undo();
            self.selected_sprite_indices.clear();
            self.rebuild_sprite_tiles();
            return;
        }
        if let Some(layer) = self.editing_objects_mut() {
            layer.undo();
        } else if let Some(bg) = &mut self.layer2_background {
            bg.undo();
        }
        self.selected_object_indices.clear();
    }

    pub(super) fn handle_redo(&mut self) {
        if self.edit_sprites {
            self.sprites.redo();
            self.selected_sprite_indices.clear();
            self.rebuild_sprite_tiles();
            return;
        }
        if let Some(layer) = self.editing_objects_mut() {
            layer.redo();
        } else if let Some(bg) = &mut self.layer2_background {
            bg.redo();
        }
        self.selected_object_indices.clear();
    }
}

// ── Lunar Magic-style drag handles ──────────────────────────────────────
// Selecting a layer-1 object shows 8 drag handles (4 corners + 4 edge
// midpoints), like Lunar Magic. Dragging the object body moves it;
// dragging a handle resizes it (width/height live in the settings byte:
// low nibble = width-1, high nibble = height-1, so 1..=16 tiles each).
//
// The drag is transient: `object_drag` holds the in-progress geometry and
// the overlay draws live feedback from it. Only when the pointer is
// released is a single undoable write committed to the real object layer,
// so one drag == one undo step.

/// The 8 LM-style resize handles around a selected object's rectangle.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub(super) enum DragHandle {
    Nw,
    N,
    Ne,
    E,
    Se,
    S,
    Sw,
    W,
}

/// In-progress object drag. `handle == None` means a body move.
#[derive(Clone, Debug)]
pub(super) struct ObjectDrag {
    pub index:       usize,
    /// Object id at grab time; the commit is skipped if the object at
    /// `index` no longer matches (deleted/undone mid-drag).
    pub check_id:    u8,
    pub handle:      Option<DragHandle>,
    pub is_extended: bool,
    pub orig_x:      u32,
    pub orig_y:      u32,
    pub orig_w:      u32,
    pub orig_h:      u32,
    /// Press-tile minus origin, so the object doesn't jump on grab (move only).
    pub grab_dx:     i32,
    pub grab_dy:     i32,
    pub cur_x:       u32,
    pub cur_y:       u32,
    pub cur_w:       u32,
    pub cur_h:       u32,
}

/// Object footprint in tiles from the settings byte (1x1 for extended).
pub(super) fn object_dims(settings: u8, is_extended: bool) -> (u32, u32) {
    if is_extended {
        (1, 1)
    } else {
        ((settings & 0x0F) as u32 + 1, (settings >> 4) as u32 + 1)
    }
}

/// Settings byte encoding a w×h footprint (each 1..=16).
pub(super) fn settings_for_dims(w: u32, h: u32) -> u8 {
    (((h.clamp(1, 16) - 1) << 4) | (w.clamp(1, 16) - 1)) as u8
}

/// Resize an (ox, oy, ow, oh) tile rect by dragging `handle` to tile
/// (`tile_x`, `tile_y`). The dragged edge/corner follows the pointer; the
/// opposite edge stays fixed. Returns the new (x, y, w, h), clamped to
/// 1..=16 tiles per axis.
pub(super) fn apply_resize(
    ox: u32, oy: u32, ow: u32, oh: u32, handle: DragHandle, tile_x: i32, tile_y: i32,
) -> (u32, u32, u32, u32) {
    use DragHandle::*;
    let (mut nx, mut ny, mut nw, mut nh) = (ox, oy, ow, oh);
    match handle {
        E | Ne | Se => {
            nw = (tile_x - ox as i32 + 1).clamp(1, 16) as u32;
        }
        W | Nw | Sw => {
            // Right edge stays fixed; clamp the left edge so the width
            // stays within 1..=16 tiles even far from the origin.
            let right = ox as i32 + ow as i32;
            let nl = tile_x.max(0).min(right - 1).max(right - 16);
            nw = (right - nl) as u32;
            nx = nl as u32;
        }
        _ => {}
    }
    match handle {
        S | Se | Sw => {
            nh = (tile_y - oy as i32 + 1).clamp(1, 16) as u32;
        }
        N | Ne | Nw => {
            // Bottom edge stays fixed; same 1..=16 clamp as the W branch.
            let bottom = oy as i32 + oh as i32;
            let nt = tile_y.max(0).min(bottom - 1).max(bottom - 16);
            nh = (bottom - nt) as u32;
            ny = nt as u32;
        }
        _ => {}
    }
    (nx, ny, nw, nh)
}

/// The 8 handle rects (white LM-style squares) around `rect`, in
/// clockwise order starting at the top-left corner.
pub(super) fn drag_handle_rects(rect: egui::Rect, handle_px: f32) -> [(DragHandle, egui::Rect); 8] {
    use DragHandle::*;
    let xs = [rect.left(), rect.center().x, rect.right()];
    let ys = [rect.top(), rect.center().y, rect.bottom()];
    let at = |hx: usize, hy: usize| {
        egui::Rect::from_center_size(egui::Pos2::new(xs[hx], ys[hy]), egui::Vec2::splat(handle_px))
    };
    [
        (Nw, at(0, 0)),
        (N, at(1, 0)),
        (Ne, at(2, 0)),
        (E, at(2, 1)),
        (Se, at(2, 2)),
        (S, at(1, 2)),
        (Sw, at(0, 2)),
        (W, at(0, 1)),
    ]
}

/// Hit-test the handles with a slightly generous grab area.
pub(super) fn handle_at(rect: egui::Rect, handle_px: f32, pos: egui::Pos2) -> Option<DragHandle> {
    drag_handle_rects(rect, handle_px + 4.0).into_iter().find(|(_, r)| r.contains(pos)).map(|(h, _)| h)
}

impl UiLevelEditor {
    /// Advance the object-drag state machine. Call once per frame, before
    /// panning is handled, so an object drag suppresses canvas panning.
    /// Sets `suppress_click_select` for one frame when a drag ends with a
    /// change, so the release click doesn't re-trigger selection.
    pub(super) fn update_object_drag(
        &mut self, resp: &egui::Response, origin: Pos2, tile_sz: f32, level_w: u32, level_h: u32,
    ) {
        self.suppress_click_select = false;
        let primary = egui::PointerButton::Primary;

        // ── Finish an in-progress drag ──
        if self.object_drag.is_some() && resp.drag_stopped_by(primary) {
            let d = self.object_drag.take().expect("checked above");
            let changed = d.cur_x != d.orig_x || d.cur_y != d.orig_y || d.cur_w != d.orig_w || d.cur_h != d.orig_h;
            if changed {
                self.commit_object_drag(&d);
                self.suppress_click_select = true;
            }
            return;
        }

        // ── Continue an in-progress drag (live feedback) ──
        if let Some(d) = self.object_drag.as_mut() {
            if resp.dragged_by(primary) {
                if let Some(pos) = resp.hover_pos() {
                    let tx = ((pos.x - origin.x) / tile_sz).floor() as i32;
                    let ty = ((pos.y - origin.y) / tile_sz).floor() as i32;
                    match d.handle {
                        None => {
                            // Body move: tile-snapped, clamped to the level.
                            d.cur_x = (tx - d.grab_dx).max(0).min(level_w.saturating_sub(d.cur_w).max(0) as i32) as u32;
                            d.cur_y = (ty - d.grab_dy).max(0).min(level_h.saturating_sub(d.cur_h).max(0) as i32) as u32;
                        }
                        Some(handle) => {
                            let (nx, ny, nw, nh) = apply_resize(d.orig_x, d.orig_y, d.orig_w, d.orig_h, handle, tx, ty);
                            // Keep a resize inside the level.
                            d.cur_x = nx.min(level_w.saturating_sub(nw));
                            d.cur_y = ny.min(level_h.saturating_sub(nh));
                            d.cur_w = nw.min(level_w);
                            d.cur_h = nh.min(level_h);
                        }
                    }
                }
            }
            return;
        }

        // ── Start a drag on press ──
        if self.edit_sprites
            || (self.editing_mode != EditingMode::Select && self.editing_mode != EditingMode::Probe)
            || self.selected_object_indices.len() != 1
            || !resp.drag_started_by(primary)
        {
            return;
        }
        let idx = *self.selected_object_indices.iter().next().expect("len == 1");
        let Some((oid, ox, oy, ow, oh, is_extended)) = self.editing_objects().and_then(|layer_data| {
            layer_data.read(|layer| {
                layer.objects.get(idx).map(|obj| {
                    let (w, h) = object_dims(obj.settings, obj.is_extended);
                    (obj.id, obj.x, obj.y, w, h, obj.is_extended)
                })
            })
        }) else {
            // The selected index vanished (e.g. undone mid-frame).
            return;
        };
        let Some(pos) = resp.hover_pos() else { return };
        let rect = egui::Rect::from_min_size(
            origin + egui::vec2(ox as f32 * tile_sz, oy as f32 * tile_sz),
            egui::vec2(ow as f32 * tile_sz, oh as f32 * tile_sz),
        );
        let handle_px = (7.0 * self.zoom).clamp(6.0, 14.0);
        if !is_extended {
            if let Some(handle) = handle_at(rect, handle_px, pos) {
                self.object_drag = Some(ObjectDrag {
                    index: idx,
                    check_id: oid,
                    handle: Some(handle),
                    is_extended,
                    orig_x: ox,
                    orig_y: oy,
                    orig_w: ow,
                    orig_h: oh,
                    grab_dx: 0,
                    grab_dy: 0,
                    cur_x: ox,
                    cur_y: oy,
                    cur_w: ow,
                    cur_h: oh,
                });
                return;
            }
        }
        if rect.contains(pos) {
            let tx = ((pos.x - origin.x) / tile_sz).floor() as i32;
            let ty = ((pos.y - origin.y) / tile_sz).floor() as i32;
            self.object_drag = Some(ObjectDrag {
                index: idx,
                check_id: oid,
                handle: None,
                is_extended,
                orig_x: ox,
                orig_y: oy,
                orig_w: ow,
                orig_h: oh,
                grab_dx: tx - ox as i32,
                grab_dy: ty - oy as i32,
                cur_x: ox,
                cur_y: oy,
                cur_w: ow,
                cur_h: oh,
            });
        }
    }

    /// Commit a finished drag: one undoable write to the real object layer,
    /// then move the rendered tiles with the object (old footprint blanked,
    /// new footprint filled from the old one with edge-stretch for grown
    /// cells, exactly like a move for the overlapping region).
    fn commit_object_drag(&mut self, d: &ObjectDrag) {
        let (ow, oh) = (d.orig_w, d.orig_h);
        // Snapshot the blocks under the old footprint before blanking.
        let mut old_blocks = Vec::with_capacity((ow * oh) as usize);
        for dy in 0..oh {
            for dx in 0..ow {
                old_blocks.push(self.block_id_at(d.orig_x + dx, d.orig_y + dy).unwrap_or(0x25));
            }
        }
        // Single undoable write: the object data itself. Skip it if the
        // object at this index changed identity mid-drag (deleted/undone).
        let committed = self.editing_objects_mut().expect("editable object layer missing").write(|layer| {
            let ok = layer.objects.get(d.index).is_some_and(|obj| obj.id == d.check_id);
            if ok {
                if let Some(obj) = layer.objects.get_mut(d.index) {
                    obj.x = d.cur_x;
                    obj.y = d.cur_y;
                    if !d.is_extended {
                        obj.settings = settings_for_dims(d.cur_w, d.cur_h);
                    }
                }
            }
            ok
        });
        self.mark_edited();
        if !committed {
            // Nothing to move; leave the rendered tiles alone.
            self.rebuild_tiles();
            return;
        }
        // Blank the old footprint.
        for dy in 0..oh {
            for dx in 0..ow {
                self.set_block_id_at(d.orig_x + dx, d.orig_y + dy, 0x25);
            }
        }
        // Stamp the new footprint from the old blocks (edge-stretched).
        for dy in 0..d.cur_h {
            for dx in 0..d.cur_w {
                let sx = (dx as i32 + d.cur_x as i32 - d.orig_x as i32).clamp(0, ow as i32 - 1) as u32;
                let sy = (dy as i32 + d.cur_y as i32 - d.orig_y as i32).clamp(0, oh as i32 - 1) as u32;
                self.set_block_id_at(d.cur_x + dx, d.cur_y + dy, old_blocks[(sy * ow + sx) as usize]);
            }
        }
        self.rebuild_tiles();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dims_from_settings() {
        assert_eq!(object_dims(0x00, false), (1, 1));
        assert_eq!(object_dims(0x32, false), (3, 4));
        assert_eq!(object_dims(0xFF, false), (16, 16));
        assert_eq!(object_dims(0xFF, true), (1, 1));
    }

    #[test]
    fn settings_roundtrip() {
        for w in 1..=16 {
            for h in 1..=16 {
                let s = settings_for_dims(w, h);
                assert_eq!(object_dims(s, false), (w, h));
            }
        }
        assert_eq!(settings_for_dims(0, 99), settings_for_dims(1, 16));
    }

    #[test]
    fn resize_east_grows_width() {
        // Object at (5,5) 3x2; drag E handle to tile x=9 -> width 5.
        assert_eq!(apply_resize(5, 5, 3, 2, DragHandle::E, 9, 5), (5, 5, 5, 2));
    }

    #[test]
    fn resize_west_moves_left_edge() {
        // Drag W handle to tile x=3: left edge moves, right edge fixed at 8.
        assert_eq!(apply_resize(5, 5, 3, 2, DragHandle::W, 3, 5), (3, 5, 5, 2));
        // Can't drag past the fixed edge: clamps to width 1.
        assert_eq!(apply_resize(5, 5, 3, 2, DragHandle::W, 99, 5), (7, 5, 1, 2));
    }

    #[test]
    fn resize_north_moves_top_edge() {
        assert_eq!(apply_resize(5, 5, 3, 4, DragHandle::N, 5, 2), (5, 2, 3, 7));
    }

    #[test]
    fn resize_south_grows_height() {
        assert_eq!(apply_resize(5, 5, 3, 2, DragHandle::S, 5, 12), (5, 5, 3, 8));
    }

    #[test]
    fn resize_corner_does_both_axes() {
        assert_eq!(apply_resize(5, 5, 3, 2, DragHandle::Se, 9, 8), (5, 5, 5, 4));
        assert_eq!(apply_resize(5, 5, 3, 4, DragHandle::Nw, 3, 3), (3, 3, 5, 6));
    }

    #[test]
    fn resize_clamps_to_16_and_nonnegative() {
        assert_eq!(apply_resize(0, 0, 3, 2, DragHandle::E, 100, 0), (0, 0, 16, 2));
        assert_eq!(apply_resize(5, 5, 3, 2, DragHandle::W, -50, 5), (0, 5, 8, 2));
    }

    #[test]
    fn resize_west_clamps_width_to_16_far_from_origin() {
        // Regression: dragging the W handle far left of an object at x=100
        // used to produce a >16-wide rect (tile-map corruption on commit).
        // Right edge (103) stays fixed, width clamps to 16.
        assert_eq!(apply_resize(100, 5, 3, 2, DragHandle::W, 0, 5), (87, 5, 16, 2));
        assert_eq!(apply_resize(5, 100, 3, 2, DragHandle::N, 5, 0), (5, 86, 3, 16));
    }

    #[test]
    fn handles_are_eight_around_rect() {
        let rect = egui::Rect::from_min_size(egui::Pos2::new(10.0, 20.0), egui::Vec2::new(30.0, 40.0));
        let handles = drag_handle_rects(rect, 8.0);
        assert_eq!(handles.len(), 8);
        let centers: Vec<egui::Pos2> = handles.iter().map(|(_, r)| r.center()).collect();
        // Corners sit exactly on the rect corners.
        assert!(centers.contains(&egui::Pos2::new(10.0, 20.0)));
        assert!(centers.contains(&egui::Pos2::new(40.0, 60.0)));
        // Edge midpoints sit on the edge centers.
        assert!(centers.contains(&egui::Pos2::new(25.0, 20.0)));
        assert!(centers.contains(&egui::Pos2::new(25.0, 60.0)));
        // Hit-testing finds the right handle.
        assert_eq!(handle_at(rect, 8.0, egui::Pos2::new(40.0, 60.0)), Some(DragHandle::Se));
        assert_eq!(handle_at(rect, 8.0, egui::Pos2::new(25.0, 20.0)), Some(DragHandle::N));
        assert_eq!(handle_at(rect, 8.0, egui::Pos2::new(25.0, 40.0)), None);
    }
}
