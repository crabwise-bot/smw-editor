//! "Change Music & Time Limit Settings" bypass dialog — Lunar Magic v1.70
//! parity (typed track numbers v3.32, 16-bit track IDs v3.70).
//!
//! Per-level override of the level header's music and time-limit settings.
//! The data lives in the editor-native `SMWMUSBP` RATS block (same pattern as
//! the Super GFX Bypass); Apply writes the bypass table (dirty → merged into
//! the ROM's block by `save_to_rom`) and applies the time override to the
//! emulated WRAM timer (`InGameTimerHundreds/Tens/Ones`), mirroring what LM's
//! bypass ASM does at level load.

use egui::Context;
use smwe_rom::music_bypass::{
    bypass_named_tracks,
    format_track_id,
    timer_bcd_digits,
    MusicBypass,
    MUSIC_TRACK_MAX_ID,
    TIME_LIMIT_MAX_SECONDS,
    WRAM_TIMER_HUNDREDS,
    WRAM_TIMER_ONES,
    WRAM_TIMER_TENS,
};

use super::UiLevelEditor;

/// Working state for the dialog's editable rows (the stored table is only
/// touched on Apply).
#[derive(Clone, Debug)]
pub(super) struct MusicTimeBypassEdit {
    pub music_enabled: bool,
    /// Track ID as typed/shown in hex (LM v3.32 parity: typable track numbers).
    pub track_hex:     String,
    pub time_enabled:  bool,
    /// Seconds as typed (0-999; 0 = no time limit, like header timer 0).
    pub seconds_text:  String,
}

impl MusicTimeBypassEdit {
    pub(super) fn from_stored(bypass: Option<MusicBypass>) -> Self {
        Self {
            music_enabled: bypass.map(|b| b.music.is_some()).unwrap_or(false),
            track_hex:     bypass.and_then(|b| b.music).map(|t| format!("{t:X}")).unwrap_or_default(),
            time_enabled:  bypass.map(|b| b.time_limit.is_some()).unwrap_or(false),
            seconds_text:  bypass.and_then(|b| b.time_limit).map(|s| s.to_string()).unwrap_or_default(),
        }
    }

    /// Parse the typed fields into a [`MusicBypass`]. Returns `None` on any
    /// invalid input (the dialog shows the error instead of applying).
    fn to_bypass(&self) -> Option<MusicBypass> {
        let music = if self.music_enabled {
            let track = u16::from_str_radix(self.track_hex.trim().trim_start_matches("0x"), 16).ok()?;
            if track > MUSIC_TRACK_MAX_ID {
                return None;
            }
            Some(track)
        } else {
            None
        };
        let time_limit = if self.time_enabled {
            let seconds: u16 = self.seconds_text.trim().parse().ok()?;
            if seconds > TIME_LIMIT_MAX_SECONDS {
                return None;
            }
            Some(seconds)
        } else {
            None
        };
        Some(MusicBypass { music, time_limit })
    }
}

impl UiLevelEditor {
    /// Mirror LM's time-bypass ASM at level load for the current level: when a
    /// time override is stored, write its BCD digits to the emulated
    /// `InGameTimerHundreds/Tens/Ones` — the same addresses `CODE_0584E3`
    /// writes from the header `TimerTable`. Music has no WRAM mirror: the
    /// vanilla `MusicBackup` byte can't hold 16-bit track IDs and the editor
    /// plays no audio, so the bypassed track is recorded as the level's
    /// effective music (shown in this dialog and the Level Header override
    /// indicator).
    pub(super) fn apply_music_time_bypass_to_wram(&mut self) {
        let level = self.level_num;
        let Some(seconds) = self.music_bypass_data.get(level).and_then(|b| b.time_limit) else { return };
        let (h, t, o) = timer_bcd_digits(seconds);
        let wram = &mut self.cpu.mem.wram;
        if wram.len() > WRAM_TIMER_ONES {
            wram[WRAM_TIMER_HUNDREDS] = h;
            wram[WRAM_TIMER_TENS] = t;
            wram[WRAM_TIMER_ONES] = o;
        }
    }

    pub(super) fn music_time_bypass_window(&mut self, ctx: &Context) {
        if !self.show_music_time_bypass {
            return;
        }
        // Resync the working copy when the level changed under the dialog.
        if self.music_bypass_edit_level != self.level_num {
            self.music_bypass_edit = MusicTimeBypassEdit::from_stored(self.music_bypass_data.get(self.level_num));
            self.music_bypass_edit_level = self.level_num;
            self.music_bypass_error = None;
        }

        let level_num = self.level_num;
        let header_music = self.level_properties.music;
        let header_timer = self.level_properties.timer;
        let header_seconds = [0u16, 200, 300, 400][usize::from(header_timer & 3)];
        let effective_music = self.music_bypass_data.effective_music_label(level_num, header_music);
        let effective_seconds = self.music_bypass_data.effective_time_seconds(level_num, header_timer);

        let mut open = self.show_music_time_bypass;
        let mut apply_clicked = false;
        let mut clear_clicked = false;
        let mut edit = self.music_bypass_edit.clone();
        let mut error: Option<String> = self.music_bypass_error.clone();

        egui::Window::new(format!("Change Music & Time Limit Settings — level {level_num:03X}"))
            .open(&mut open)
            .resizable(true)
            .default_size([480.0, 380.0])
            .show(ctx, |ui| {
                ui.label(
                    "Override this level's header music and time-limit settings, \
                     like Lunar Magic's \"Change Music & Time Limit Settings\" dialog. \
                     Stored in the editor-native SMWMUSBP block; the real game \
                     needs LM's bypass ASM to honor it.",
                );
                ui.separator();

                // ── Music row ──
                ui.strong("Music");
                ui.checkbox(&mut edit.music_enabled, "Override music");
                ui.horizontal(|ui| {
                    ui.label("Track:");
                    // Named-track dropdown (LM v1.70) …
                    let selected_label = edit
                        .track_hex
                        .trim()
                        .trim_start_matches("0x")
                        .parse::<u16>()
                        .ok()
                        .filter(|t| *t <= MUSIC_TRACK_MAX_ID)
                        .map(format_track_id)
                        .unwrap_or_else(|| "— pick or type a track —".to_string());
                    egui::ComboBox::from_id_salt("music_bypass_track_picker").selected_text(selected_label).show_ui(
                        ui,
                        |ui| {
                            for (id, name) in bypass_named_tracks() {
                                if ui.selectable_label(false, format!("{id:X}: {name}")).clicked() {
                                    edit.track_hex = format!("{id:X}");
                                }
                            }
                        },
                    );
                    // … plus the typable track-number field (LM v3.32).
                    ui.label("ID (hex):");
                    let before = edit.track_hex.clone();
                    ui.add(egui::TextEdit::singleline(&mut edit.track_hex).desired_width(70.0));
                    if edit.track_hex != before {
                        error = None;
                    }
                });
                ui.small(format!("Effective music: {effective_music}."));
                ui.small(format!(
                    "16-bit track IDs up to 0x{MUSIC_TRACK_MAX_ID:X} (LM v3.70). \
                     Header plays: {} (SPC {}).",
                    smwe_rom::music::format_music_track(header_music),
                    smwe_rom::music::music_track_spc_id(header_music)
                        .map(|id| id.to_string())
                        .unwrap_or_else(|| "custom".into()),
                ));
                ui.separator();

                // ── Time row ──
                ui.strong("Time limit");
                ui.checkbox(&mut edit.time_enabled, "Override time limit");
                ui.horizontal(|ui| {
                    ui.label("Seconds (0–999):");
                    let before = edit.seconds_text.clone();
                    ui.add(egui::TextEdit::singleline(&mut edit.seconds_text).desired_width(70.0));
                    if edit.seconds_text != before {
                        error = None;
                    }
                    ui.small("0 = no time limit (like header timer 0)");
                });
                ui.small(format!(
                    "Header timer setting {header_timer} = {header_seconds} s → effective: {effective_seconds} s. \
                     0 s = no time limit (like header timer 0)."
                ));
                ui.separator();

                if let Some(err) = &error {
                    ui.colored_label(egui::Color32::YELLOW, err);
                }
                ui.horizontal(|ui| {
                    if ui.button("Apply").clicked() {
                        apply_clicked = true;
                    }
                    if ui.button("Clear bypass").clicked() {
                        clear_clicked = true;
                    }
                    ui.small("Apply updates the emulated timer immediately.");
                });
            });
        self.show_music_time_bypass = open;
        self.music_bypass_edit = edit;
        self.music_bypass_error = error;
        if clear_clicked {
            self.music_bypass_edit = MusicTimeBypassEdit::from_stored(None);
            self.music_bypass_error = None;
            apply_clicked = true;
        }
        if apply_clicked {
            match self.music_bypass_edit.to_bypass() {
                Some(bypass) => {
                    if self.music_bypass_data.set(self.level_num, bypass).is_ok() {
                        self.music_bypass_dirty = true;
                        self.mark_edited();
                        self.apply_music_time_bypass_to_wram();
                        self.music_bypass_error = None;
                    }
                }
                None => {
                    self.music_bypass_error = Some(format!(
                        "Invalid input: track ID must be hex 0–{MUSIC_TRACK_MAX_ID:X}, \
                         seconds must be 0–{TIME_LIMIT_MAX_SECONDS}."
                    ));
                }
            }
        }
    }
}
