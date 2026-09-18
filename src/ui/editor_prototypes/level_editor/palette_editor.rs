use egui::{vec2, Color32, Context, Rect, Sense, Ui, Vec2};
use egui_phosphor::regular as icon;

use super::UiLevelEditor;
use crate::undo::Undo;

const CELL_SIZE: f32 = 20.0;
const COLS: usize = 12;

/// The three 12-color palettes the level palette editor edits (BG, FG,
/// sprite). Wrapped in [`UndoableData`] for Lunar Magic v1.80-style
/// undo/redo. Serialization is a fixed 72-byte little-endian u16 sequence
/// (bg, fg, sprite), so undo deltas are deterministic.
#[derive(Clone, Debug, Default)]
pub(super) struct EditablePalettes {
    pub bg:     [u16; 12],
    pub fg:     [u16; 12],
    pub sprite: [u16; 12],
}

impl EditablePalettes {
    fn group(&self, group: usize) -> &[u16; 12] {
        match group {
            0 => &self.bg,
            1 => &self.fg,
            _ => &self.sprite,
        }
    }

    fn group_mut(&mut self, group: usize) -> &mut [u16; 12] {
        match group {
            0 => &mut self.bg,
            1 => &mut self.fg,
            _ => &mut self.sprite,
        }
    }
}

impl Undo for EditablePalettes {
    fn from_bytes(bytes: Vec<u8>) -> Self {
        let mut palettes = Self::default();
        for (i, chunk) in bytes.chunks_exact(2).enumerate().take(36) {
            let v = u16::from_le_bytes([chunk[0], chunk[1]]);
            palettes.group_mut(i / 12)[i % 12] = v;
        }
        palettes
    }

    fn to_bytes(&self) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(72);
        for &v in self.bg.iter().chain(self.fg.iter()).chain(self.sprite.iter()) {
            bytes.extend_from_slice(&v.to_le_bytes());
        }
        bytes
    }

    fn size_bytes(&self) -> usize {
        72
    }
}

impl UiLevelEditor {
    pub(super) fn palette_editor_window(&mut self, ctx: &Context) {
        if !self.show_palette_editor {
            return;
        }
        let mut open = self.show_palette_editor;
        let win = egui::Window::new("Palette Editor").open(&mut open).resizable(false).show(ctx, |ui| {
            ui.label("Click a color swatch to edit it. Changes save with Ctrl+S.");

            // ── Undo/redo buttons (Lunar Magic v1.80 has these in the ──────
            // palette editors).
            ui.horizontal(|ui| {
                let can_undo = self.palettes.can_undo();
                if ui
                    .add_enabled(can_undo, egui::Button::new(format!("{} Undo", icon::ARROW_COUNTER_CLOCKWISE)))
                    .on_hover_text("Undo palette change (Ctrl+Z)")
                    .clicked()
                {
                    self.palette_undo();
                }
                let can_redo = self.palettes.can_redo();
                if ui
                    .add_enabled(can_redo, egui::Button::new(format!("{} Redo", icon::ARROW_CLOCKWISE)))
                    .on_hover_text("Redo palette change (Ctrl+Y)")
                    .clicked()
                {
                    self.palette_redo();
                }
            });
            ui.separator();

            self.palette_group(ui, "BG Palette", 0);
            ui.separator();
            self.palette_group(ui, "FG Palette", 1);
            ui.separator();
            self.palette_group(ui, "Sprite Palette", 2);
        });
        self.show_palette_editor = open;

        // ── Ctrl+Z / Ctrl+Y while the pointer is over this window ───────────
        // (or while a color drag is in flight, since the picker popup floats
        // outside the window rect). The window is drawn before the central
        // panel, so consuming here wins over the level-canvas undo.
        let palette_active = win.is_some_and(|r| r.response.hovered()) || self.palette_gesture_before.is_some();
        if palette_active {
            if ctx
                .input_mut(|i| i.consume_shortcut(&egui::KeyboardShortcut::new(egui::Modifiers::COMMAND, egui::Key::Z)))
            {
                self.palette_undo();
            }
            if ctx
                .input_mut(|i| i.consume_shortcut(&egui::KeyboardShortcut::new(egui::Modifiers::COMMAND, egui::Key::Y)))
            {
                self.palette_redo();
            }
        }

        // ── Commit the undo step when a color-picker drag gesture ends ──────
        // (snapshot taken on the first change; the picker fires changed() on
        // every drag frame, so committing per-frame would make undo walk back
        // through every intermediate color).
        if self.palette_gesture_before.is_some() && ctx.input(|i| i.pointer.any_released()) {
            self.commit_palette_gesture();
        }
    }

    /// Undo one palette edit (Lunar Magic v1.80 palette-editor undo).
    /// An in-flight color drag is committed first, so Ctrl+Z mid-drag undoes
    /// the drag rather than the edit before it.
    fn palette_undo(&mut self) {
        self.commit_palette_gesture();
        if self.palettes.can_undo() {
            self.palettes.undo();
            self.palette_dirty = true;
            self.mark_edited();
        }
    }

    /// Redo one palette edit (Lunar Magic v1.80 palette-editor redo).
    fn palette_redo(&mut self) {
        self.commit_palette_gesture();
        if self.palettes.can_redo() {
            self.palettes.redo();
            self.palette_dirty = true;
            self.mark_edited();
        }
    }

    /// Commit an in-flight color-drag gesture as a single undo step, if any.
    fn commit_palette_gesture(&mut self) {
        if let Some(before) = self.palette_gesture_before.take() {
            self.palettes.commit_change(&before);
        }
    }

    fn palette_group(&mut self, ui: &mut Ui, label: &str, group: usize) {
        let p = &self.level_properties;
        let index = match group {
            0 => p.palette_bg,
            1 => p.palette_fg,
            _ => p.palette_sprite,
        };
        ui.label(format!("{label} (index {index:X})"));

        let colors: [u16; 12] = self.palettes.read(|pal| *pal.group(group));

        // Draw grid of colored cells
        let total_w = CELL_SIZE * COLS as f32;
        let (grid_rect, _) = ui.allocate_exact_size(vec2(total_w, CELL_SIZE), Sense::hover());

        let mut changed = false;
        for col in 0..COLS {
            let raw = colors[col];
            let cell_min = grid_rect.min + vec2(col as f32 * CELL_SIZE, 0.0);
            let cell_rect = Rect::from_min_size(cell_min, Vec2::splat(CELL_SIZE));

            let r = ((raw & 0x1F) as f32 / 31.0 * 255.0) as u8;
            let g = (((raw >> 5) & 0x1F) as f32 / 31.0 * 255.0) as u8;
            let b = (((raw >> 10) & 0x1F) as f32 / 31.0 * 255.0) as u8;
            let c32 = Color32::from_rgb(r, g, b);

            // Fill the cell
            ui.painter().rect_filled(cell_rect, egui::CornerRadius::ZERO, c32);
            // Thin border
            ui.painter().rect_stroke(
                cell_rect,
                egui::CornerRadius::ZERO,
                egui::Stroke::new(1.0_f32, Color32::from_gray(80)),
                egui::StrokeKind::Outside,
            );

            // Highlight selected cell
            let selected = self.selected_palette_group == group as u8
                && self.selected_palette_idx == col
                && self.selected_palette_group < 3;
            if selected {
                ui.painter().rect_stroke(
                    cell_rect,
                    egui::CornerRadius::ZERO,
                    egui::Stroke::new(2.0_f32, Color32::WHITE),
                    egui::StrokeKind::Outside,
                );
            }

            // Detect click
            let resp = ui.interact(cell_rect, egui::Id::new(("pal_cell", group, col, index)), Sense::click());
            if resp.clicked() {
                self.selected_palette_group = group as u8;
                self.selected_palette_idx = col;
            }
        }

        // If this group has a selected cell, show a color picker below
        if self.selected_palette_group == group as u8 && self.selected_palette_idx < COLS {
            let col = self.selected_palette_idx;
            let raw = colors[col];
            let mut c32 = Color32::from_rgb(
                ((raw & 0x1F) as f32 / 31.0 * 255.0) as u8,
                (((raw >> 5) & 0x1F) as f32 / 31.0 * 255.0) as u8,
                (((raw >> 10) & 0x1F) as f32 / 31.0 * 255.0) as u8,
            );
            ui.horizontal(|ui| {
                ui.label(format!("Color {}:", col));
                if ui.color_edit_button_srgba(&mut c32).changed() {
                    // Convert sRGBA back to ABGR1555
                    let r5 = (c32.r() as u16 * 31 / 255) & 0x1F;
                    let g5 = (c32.g() as u16 * 31 / 255) & 0x1F;
                    let b5 = (c32.b() as u16 * 31 / 255) & 0x1F;
                    let new_raw = r5 | (g5 << 5) | (b5 << 10);
                    // Gesture-style edit: snapshot once on the first change,
                    // mutate directly; a single undo step is committed when
                    // the drag ends (see palette_editor_window), so undo
                    // restores the pre-drag color instead of walking back
                    // through every intermediate drag frame.
                    if self.palette_gesture_before.is_none() {
                        self.palette_gesture_before = Some(self.palettes.read(|pal| pal.clone()));
                    }
                    self.palettes.data_mut().group_mut(group)[col] = new_raw;
                    changed = true;
                }
                let raw2 = self.palettes.read(|pal| pal.group(group)[col]);
                ui.monospace(format!("{:04X}", raw2));
            });
        }

        if changed {
            self.palette_dirty = true;
            self.mark_edited();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::undo::UndoableData;

    #[test]
    fn palette_undo_redo_round_trip() {
        let mut palettes = UndoableData::new(EditablePalettes::default());
        // One write() = one undo step, like a single committed edit.
        palettes.write(|p| p.bg[3] = 0x7FFF);
        palettes.write(|p| p.sprite[0] = 0x001F);
        assert!(palettes.can_undo());
        assert!(!palettes.can_redo());

        palettes.undo();
        assert_eq!(palettes.read(|p| p.sprite[0]), 0);
        assert_eq!(palettes.read(|p| p.bg[3]), 0x7FFF);
        assert!(palettes.can_redo());

        palettes.undo();
        assert_eq!(palettes.read(|p| p.bg[3]), 0);
        assert!(!palettes.can_undo());

        palettes.redo();
        palettes.redo();
        assert_eq!(palettes.read(|p| p.bg[3]), 0x7FFF);
        assert_eq!(palettes.read(|p| p.sprite[0]), 0x001F);
        assert!(!palettes.can_redo());
    }

    #[test]
    fn palette_gesture_commit_is_single_step() {
        let mut palettes = UndoableData::new(EditablePalettes::default());
        // Gesture-style edit: snapshot, mutate directly across several
        // "frames", commit once — the color-picker drag path.
        let before = palettes.read(|p| p.clone());
        for v in [0x1111u16, 0x2222, 0x3333] {
            palettes.data_mut().fg[7] = v;
        }
        palettes.commit_change(&before);
        assert!(palettes.can_undo());
        palettes.undo();
        assert_eq!(palettes.read(|p| p.fg[7]), 0);
        // A second undo must not exist: the whole drag was one step.
        assert!(!palettes.can_undo());
    }

    #[test]
    fn palette_serialization_is_stable() {
        let mut p = EditablePalettes::default();
        p.bg[0] = 0x1234;
        p.sprite[11] = 0x7FFF;
        let bytes = p.to_bytes();
        assert_eq!(bytes.len(), 72);
        let back = EditablePalettes::from_bytes(bytes);
        assert_eq!(back.to_bytes(), p.to_bytes());
    }
}
