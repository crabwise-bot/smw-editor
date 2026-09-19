//! Lunar Magic v1.30 "Change Overworld Music" dialog (world editor).
//!
//! One music track per overworld submap (7 rows). The game reads the track
//! from `OverworldMusic` (`$048D8A`) at overworld init and from the mirror
//! table `OverworldMusic2` (`$04DBC8`) on submap swaps, so the dialog writes
//! both tables in sync on save — see
//! `smwe_rom::overworld::submap_music` for the on-disk story and the
//! SMWDisX references.

use egui::{ComboBox, Context};
use smwe_rom::overworld::{
    submap_music::{format_submap_music_track, SUBMAP_MUSIC_LEN, SUBMAP_MUSIC_TRACKS},
    SUBMAP_NAMES,
};

use super::UiWorldEditor;

impl UiWorldEditor {
    pub(super) fn submap_music_window(&mut self, ctx: &Context) {
        if !self.show_submap_music {
            return;
        }
        let mut open = self.show_submap_music;
        egui::Window::new("Overworld Submap Music").open(&mut open).resizable(true).default_size([430.0, 360.0]).show(
            ctx,
            |ui| {
                ui.label("The music the game plays on each overworld submap. Saved with Ctrl+S.");
                ui.separator();

                let current = self.edit_state.read(|s| s.submap_music);
                let mut tracks = current.tracks;
                let mut changed = false;
                egui::Grid::new("submap_music_grid").num_columns(2).spacing([12.0, 6.0]).striped(true).show(ui, |ui| {
                    ui.strong("Submap");
                    ui.strong("Music");
                    ui.end_row();
                    for submap in 0..SUBMAP_MUSIC_LEN {
                        ui.label(SUBMAP_NAMES.get(submap).copied().unwrap_or("Submap"));
                        let mut track = tracks[submap];
                        ComboBox::from_id_salt(format!("world_editor.submap_music_{submap}"))
                            .selected_text(format_submap_music_track(track))
                            .show_ui(ui, |ui| {
                                for (id, name) in SUBMAP_MUSIC_TRACKS {
                                    ui.selectable_value(&mut track, id, format!("{id}: {name}"));
                                }
                            });
                        if track != tracks[submap] {
                            tracks[submap] = track;
                            changed = true;
                        }
                        ui.end_row();
                    }
                });

                if changed {
                    self.edit_state.write(|s| {
                        s.submap_music.tracks = tracks;
                    });
                    self.submap_music_dirty = true;
                    self.has_edits = true;
                }

                ui.separator();
                ui.weak(
                    "Stored in place at $048D8A (overworld init) and $04DBC8 \
                     (submap swap). Both tables are written together so the \
                     music can't desync between warps.",
                );
            },
        );
        self.show_submap_music = open;
    }
}
