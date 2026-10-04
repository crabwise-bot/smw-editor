//! "Change Events Passed" dialog — Lunar Magic overworld parity.
//!
//! Lunar Magic's overworld editor has a "Change Events Passed" dialog (Edit
//! menu since v3.61; a toolbar button was added in v3.70) for changing which
//! event number you're on and which events have been passed, so you can
//! preview what the overworld looks like at various points in the game. It is
//! testing-only: nothing in the dialog is written to the ROM.
//!
//! The smw-editor adaptation keeps the dialog's two controls:
//! - a "Current event" spinner (0..OW_EVENT_COUNT), dialog bookkeeping from
//!   LM's dialog — changing it scrolls the event list to that event;
//! - a passed-events checklist bound to the same `active_events` preview
//!   state as the left panel's "Events (preview)" section, so the two stay in
//!   sync. Toggling any checkbox re-renders the preview through the real game
//!   init (`load_submap`, which writes the `$1F02-$1F60` passed-events bits
//!   and runs `load_overworld`) — exactly like the existing panel.

use egui::{Align, Context};
use smwe_rom::overworld::OW_EVENT_COUNT;

use super::UiWorldEditor;

/// Label for one row of the passed-events checklist, e.g.
/// `Event   7 (tile offset 0x01A0)`. Unused slots (offset 0) read `(unused)`,
/// matching the left panel's "Events (preview)" labels.
pub(super) fn event_check_label(event: usize, tile_offset: u16) -> String {
    if tile_offset == 0 {
        format!("Event {event:3} (unused)")
    } else {
        format!("Event {event:3} (tile offset {tile_offset:#06X})")
    }
}

impl UiWorldEditor {
    /// "Change Events Passed…" dialog window (LM Edit-menu dialog / v3.70
    /// toolbar button parity). Preview-only: closes over `open` like the
    /// other world-editor dialogs; checkbox edits re-render via
    /// `load_submap` (same as the "Events (preview)" panel, which also
    /// discards the undo stack — the preview is rebuilt from the ROM).
    pub(super) fn events_passed_window(&mut self, ctx: &Context) {
        if !self.show_change_events_passed {
            return;
        }
        let mut open = self.show_change_events_passed;
        let mut scroll_to = self.events_passed_scroll_to.take();
        let mut preview_changed = false;
        egui::Window::new("Change Events Passed").open(&mut open).resizable(true).default_size([360.0, 520.0]).show(
            ctx,
            |ui| {
                ui.horizontal(|ui| {
                    ui.label("Current event:");
                    let mut cur = self.preview_current_event.min(OW_EVENT_COUNT as u8 - 1);
                    if ui
                        .add(egui::DragValue::new(&mut cur).range(0..=(OW_EVENT_COUNT as u8 - 1)))
                        .on_hover_text(
                            "Which event number you're on, for preview purposes. \
                             Changing it jumps the list below to that event; the \
                             passed-event checkboxes are what drive the preview.",
                        )
                        .changed()
                    {
                        self.preview_current_event = cur;
                        scroll_to = Some(cur as usize);
                    }
                });
                ui.horizontal(|ui| {
                    if ui.button("All on").clicked() {
                        self.active_events.iter_mut().for_each(|e| *e = true);
                        preview_changed = true;
                    }
                    if ui.button("All off").clicked() {
                        self.active_events.iter_mut().for_each(|e| *e = false);
                        preview_changed = true;
                    }
                });
                ui.separator();
                egui::ScrollArea::vertical().show(ui, |ui| {
                    for (i, active) in self.active_events.iter_mut().enumerate() {
                        let offset = self.rom.overworld_events.tile_offsets.get(i).copied().unwrap_or(0);
                        let label = event_check_label(i, offset);
                        if offset == 0 {
                            // Unused event slot: shown dimmed, not toggleable.
                            ui.add_enabled(false, egui::Checkbox::new(active, label));
                        } else {
                            let resp = ui.checkbox(active, label);
                            if Some(i) == scroll_to {
                                let rect = resp.rect;
                                ui.scroll_to_rect(rect, Some(Align::Center));
                                scroll_to = None;
                            }
                            if resp.changed() {
                                preview_changed = true;
                            }
                        }
                    }
                });
                ui.separator();
                ui.weak(
                    "Preview only — these settings are not saved to the ROM. The \
                     passed-event checkboxes drive the preview through the game's \
                     $1F02–$1F60 passed-events bits, like the Events panel.",
                );
            },
        );
        self.events_passed_scroll_to = scroll_to;
        self.show_change_events_passed = open;
        if preview_changed {
            // Same as the "Events (preview)" panel: rebuild the preview from
            // the ROM with the new passed-events bits.
            self.load_submap();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::event_check_label;

    #[test]
    fn label_shows_tile_offset() {
        assert_eq!(event_check_label(7, 0x01A0), "Event   7 (tile offset 0x01A0)");
    }

    #[test]
    fn label_marks_unused_slots() {
        assert_eq!(event_check_label(110, 0), "Event 110 (unused)");
    }
}
