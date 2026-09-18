//! Lunar Magic v3.00 "Secret Exit 2 / 3" direction-to-enable editor.
//!
//! LM v3.00 "made it possible to use Secret Exit 2 and Secret Exit 3 in the
//! game" and gave the overworld editor "direction to enable settings for
//! both". The vanilla ROM has no per-exit direction table (movement
//! directions come from event activation and runtime state — verified against
//! SMWDisX `bank_04.asm`), so smw-editor persists these settings in its own
//! `SMWSEXIT` RATS block; see `smwe_rom::overworld::secret_exits` for the
//! exact on-disk story and the stock-ROM caveats.
//!
//! The matching goal tapes are placed in the *level* editor: sprite `$7B`
//! (Goal Point) with Extra bits = 2 (Secret Exit 2) or 3 (Secret Exit 3) —
//! the sprite picker names those variants.

use egui::{Context, ScrollArea, Slider};
use smwe_rom::overworld::secret_exits::{SecretExitEntry, DIR_DOWN, DIR_LEFT, DIR_MASK, DIR_RIGHT, DIR_UP, MAX_LEVEL};

use super::UiWorldEditor;

/// Direction bits in checkbox order, with the vanilla
/// `OWLevelTileSettings` bit values (`bank_04.asm`).
const DIRS: [(u8, &str); 4] = [(DIR_UP, "Up"), (DIR_DOWN, "Down"), (DIR_LEFT, "Left"), (DIR_RIGHT, "Right")];

/// Checkbox row editing a 4-bit direction mask. Returns true when the user
/// changed something.
fn dir_mask_editor(ui: &mut egui::Ui, label: &str, mask: &mut u8) -> bool {
    let mut changed = false;
    ui.label(label);
    ui.horizontal(|ui| {
        for (bit, name) in DIRS {
            let mut on = *mask & bit != 0;
            if ui.checkbox(&mut on, name).changed() {
                if on {
                    *mask |= bit;
                } else {
                    *mask &= !bit;
                }
                *mask &= DIR_MASK;
                changed = true;
            }
        }
    });
    changed
}

impl UiWorldEditor {
    pub(super) fn secret_exits_window(&mut self, ctx: &Context) {
        if !self.show_secret_exits {
            return;
        }
        let mut open = self.show_secret_exits;
        egui::Window::new("Secret Exits 2/3 — direction to enable")
            .open(&mut open)
            .resizable(true)
            .default_size([500.0, 460.0])
            .show(ctx, |ui| {
                ui.label(
                    "Movement directions granted on the overworld when the player \
                     clears a level through Secret Exit 2 or Secret Exit 3. \
                     Saved with Ctrl+S.",
                );
                ui.separator();

                ui.horizontal(|ui| {
                    ui.label("Level:");
                    let mut lvl = self.secret_exit_level;
                    if ui.add(Slider::new(&mut lvl, 0..=MAX_LEVEL).hexadecimal(3, false, false)).changed() {
                        self.secret_exit_level = lvl;
                    }
                    ui.monospace(format!("(${:03X})", self.secret_exit_level));
                });
                ui.add_space(4.0);

                let level = self.secret_exit_level;
                let current = self.edit_state.read(|s| s.secret_exits.get(level)).unwrap_or(SecretExitEntry {
                    level,
                    exit2: 0,
                    exit3: 0,
                });
                let mut exit2 = current.exit2;
                let mut exit3 = current.exit3;
                let changed2 = dir_mask_editor(ui, "Directions to enable on Secret Exit 2:", &mut exit2);
                let changed3 = dir_mask_editor(ui, "Directions to enable on Secret Exit 3:", &mut exit3);
                if changed2 || changed3 {
                    self.edit_state.write(|s| {
                        s.secret_exits.set(SecretExitEntry { level, exit2, exit3 });
                    });
                    self.has_edits = true;
                }

                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    let has_entry = self.edit_state.read(|s| s.secret_exits.get(level).is_some());
                    if has_entry && ui.button(format!("Remove settings for level ${level:03X}")).clicked() {
                        self.edit_state.write(|s| s.secret_exits.remove(level));
                        self.has_edits = true;
                    }
                    if !has_entry {
                        ui.weak(format!("No settings stored for level ${level:03X}."));
                    }
                });

                ui.separator();
                ui.label("Levels with settings:");
                let levels: Vec<u16> =
                    self.edit_state.read(|s| s.secret_exits.entries.iter().map(|e| e.level).collect());
                if levels.is_empty() {
                    ui.weak("None yet.");
                } else {
                    ScrollArea::vertical().max_height(110.0).show(ui, |ui| {
                        for lvl in levels {
                            let entry = self.edit_state.read(|s| s.secret_exits.get(lvl)).unwrap_or(SecretExitEntry {
                                level: lvl,
                                exit2: 0,
                                exit3: 0,
                            });
                            let selected = self.secret_exit_level == lvl;
                            if ui
                                .selectable_label(
                                    selected,
                                    format!(
                                        "Level ${lvl:03X} — exit 2: {:#04X}, exit 3: {:#04X}",
                                        entry.exit2, entry.exit3
                                    ),
                                )
                                .clicked()
                            {
                                self.secret_exit_level = lvl;
                            }
                        }
                    });
                }

                ui.separator();
                ui.small(
                    "Goal tapes: in the level editor, place sprite $7B (Goal Point) \
                     with Extra bits = 2 for Secret Exit 2 or 3 for Secret Exit 3 — \
                     the sprite picker names those variants.",
                );
                ui.small(
                    "Stock-ROM note: the vanilla game has no per-exit direction \
                     table, and goal tapes with extra bits 2/3 hit vanilla's \
                     quirky exit-mode paths — real Secret Exit 2/3 behavior \
                     in-game needs Lunar Magic v3.00's ASM (not installed by \
                     this editor). The settings above are stored faithfully \
                     regardless.",
                );
            });
        self.show_secret_exits = open;
    }
}
