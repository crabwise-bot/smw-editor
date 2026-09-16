use egui::{ComboBox, Context, ScrollArea};
use smwe_rom::xref::{XrefIndex, XrefQuery};

use super::UiLevelEditor;

// -------------------------------------------------------------------------------------------------

/// What the cross-reference search looks for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(super) enum XrefQueryKind {
    #[default]
    Sprite,
    StandardObject,
    ExtendedObject,
    BackgroundTile,
    Music,
    ExitDestination,
}

impl XrefQueryKind {
    fn all() -> [XrefQueryKind; 6] {
        [
            XrefQueryKind::Sprite,
            XrefQueryKind::StandardObject,
            XrefQueryKind::ExtendedObject,
            XrefQueryKind::BackgroundTile,
            XrefQueryKind::Music,
            XrefQueryKind::ExitDestination,
        ]
    }

    fn label(self) -> &'static str {
        match self {
            XrefQueryKind::Sprite => "Sprite ID",
            XrefQueryKind::StandardObject => "Standard object ID",
            XrefQueryKind::ExtendedObject => "Extended object ID",
            XrefQueryKind::BackgroundTile => "Layer-2 BG tile ID",
            XrefQueryKind::Music => "Music track",
            XrefQueryKind::ExitDestination => "Exit destination level",
        }
    }

    /// Inclusive upper bound accepted for the query value.
    fn max_value(self) -> u32 {
        match self {
            XrefQueryKind::Sprite => 0xFF,
            XrefQueryKind::StandardObject => 0xFF,
            XrefQueryKind::ExtendedObject => 0xFF,
            XrefQueryKind::BackgroundTile => 0xFF,
            XrefQueryKind::Music => 7,
            XrefQueryKind::ExitDestination => 0x1FF,
        }
    }
}

// -------------------------------------------------------------------------------------------------

/// State for the cross-reference search window. The index is built lazily
/// from the already-parsed levels the first time the window opens.
#[derive(Default)]
pub(super) struct XrefSearchState {
    index:      Option<XrefIndex>,
    query_kind: XrefQueryKind,
    query_text: String,
    results:    Option<Vec<u16>>,
    error:      Option<String>,
}

// -------------------------------------------------------------------------------------------------

/// Parse a query value: `0x3F`-style hex or plain decimal, range-checked.
fn parse_query_value(text: &str, max: u32) -> Result<u32, String> {
    let trimmed = text.trim();
    let (digits, radix) = match trimmed.strip_prefix("0x").or_else(|| trimmed.strip_prefix("0X")) {
        Some(hex) => (hex, 16),
        None => (trimmed, 10),
    };
    let value = u32::from_str_radix(digits, radix)
        .map_err(|_| format!("\"{trimmed}\" is not a valid number (use 0x3F hex or decimal)"))?;
    if value > max {
        return Err(format!("Value {value:#X} out of range (max {max:#X})"));
    }
    Ok(value)
}

fn run_search(index: &XrefIndex, kind: XrefQueryKind, text: &str) -> Result<Vec<u16>, String> {
    let value = parse_query_value(text, kind.max_value())?;
    let query = match kind {
        XrefQueryKind::Sprite => XrefQuery::Sprite(value as u8),
        XrefQueryKind::StandardObject => XrefQuery::StandardObject(value as u8),
        XrefQueryKind::ExtendedObject => XrefQuery::ExtendedObject(value as u8),
        XrefQueryKind::BackgroundTile => XrefQuery::BackgroundTile(value as u8),
        XrefQueryKind::Music => XrefQuery::Music(value as u8),
        XrefQueryKind::ExitDestination => XrefQuery::ExitDestination(value as u16),
    };
    Ok(index.search(query))
}

// -------------------------------------------------------------------------------------------------

/// Read-only "find all references" over the ROM: which levels use a given
/// sprite, object, background tile, music track, or exit destination.
/// Clicking a result jumps the level editor to that level.
impl UiLevelEditor {
    pub(super) fn xref_search_window(&mut self, ctx: &Context) {
        if !self.show_xref_search {
            return;
        }
        // The levels are already parsed at ROM load; summarizing them is cheap.
        if self.xref_search.index.is_none() {
            self.xref_search.index = Some(XrefIndex::build(&self.rom.levels));
        }

        let mut open = self.show_xref_search;
        egui::Window::new("Cross-Reference Search").open(&mut open).resizable(true).default_size([420.0, 480.0]).show(
            ctx,
            |ui| {
                let state = &mut self.xref_search;
                ui.label("Find every level that uses a sprite, object, background tile, music track, or exit.");
                if let Some(index) = &state.index {
                    ui.small(format!("Indexed {} levels (read-only).", index.level_count()));
                }
                ui.separator();

                ui.horizontal(|ui| {
                    ui.label("Look for:");
                    ComboBox::from_id_salt("xref_query_kind").selected_text(state.query_kind.label()).show_ui(
                        ui,
                        |ui| {
                            for kind in XrefQueryKind::all() {
                                ui.selectable_value(&mut state.query_kind, kind, kind.label());
                            }
                        },
                    );
                });

                let mut search_now = false;
                ui.horizontal(|ui| {
                    ui.label("ID (0x3F hex or decimal):");
                    let response = ui.text_edit_singleline(&mut state.query_text);
                    if response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                        search_now = true;
                    }
                    if ui.button("Search").clicked() {
                        search_now = true;
                    }
                });
                if state.query_kind == XrefQueryKind::Music {
                    match parse_query_value(&state.query_text, 7) {
                        Ok(v) => {
                            ui.small(format!("Track: {}", smwe_rom::music::format_music_track(v as u8)));
                        }
                        Err(_) => {
                            ui.small("Enter a music track 0-7.");
                        }
                    }
                }

                if search_now {
                    match &state.index {
                        Some(index) => match run_search(index, state.query_kind, &state.query_text) {
                            Ok(results) => {
                                state.results = Some(results);
                                state.error = None;
                            }
                            Err(e) => {
                                state.results = None;
                                state.error = Some(e);
                            }
                        },
                        None => state.error = Some("Index not built yet.".to_owned()),
                    }
                }

                if let Some(error) = &state.error {
                    ui.colored_label(egui::Color32::from_rgb(220, 60, 60), error);
                }

                ui.separator();
                match &state.results {
                    None => {
                        ui.small("No search run yet.");
                    }
                    Some(results) => {
                        ui.label(format!(
                            "{} level{} found:",
                            results.len(),
                            if results.len() == 1 { "" } else { "s" }
                        ));
                        ScrollArea::vertical().max_height(300.0).id_salt("xref_results").show(ui, |ui| {
                            let mut jump_to = None;
                            for level in results {
                                if ui.button(format!("Level 0x{level:03X}")).clicked() {
                                    jump_to = Some(*level);
                                }
                            }
                            if let Some(level) = jump_to {
                                self.pending_level_num = Some(level);
                            }
                        });
                    }
                }
            },
        );
        self.show_xref_search = open;
    }
}
