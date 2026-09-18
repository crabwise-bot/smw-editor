mod dev_utils;
mod editing_mode;
mod editor_prototypes;
mod exanimation_dialog;
mod style;
mod tab_viewer;
mod tool;
mod welcome;
mod world_editor;

pub mod clipboard;

use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

use anyhow::Context as _;
use eframe::{CreationContext, Frame};
use egui::*;
use egui_dock::{DockArea, DockState, Style as DockStyle};
use egui_file_dialog::FileDialog;
use egui_phosphor::Variant;
use smwe_rom::{
    rom_expansion::{expand_rom, expansion_targets, format_size, split_smc_header},
    snes_utils::rom::Rom,
    SmwRom,
};

use crate::{
    level_png_export::{level_export_filename, level_png_bytes, LevelPngOptions, LEVEL_COUNT},
    project::Project,
    ui::{
        dev_utils::address_converter::UiAddressConverter,
        editor_prototypes::{level_editor::UiLevelEditor, sprite_map_editor::UiSpriteMapEditor},
        tab_viewer::EditorToolTabViewer,
        tool::DockableEditorTool,
        world_editor::UiWorldEditor,
    },
};

pub struct UiMainWindow {
    gl:                       Arc<glow::Context>,
    dock_style:               DockStyle,
    dock_state:               DockState<Box<dyn DockableEditorTool>>,
    /// Path of the currently-open ROM (for Save).
    rom_path:                 Option<PathBuf>,
    /// Set when a Save error needs to be shown.
    save_error:               Option<String>,
    /// In-egui file dialog for Open ROM.
    open_dialog:              FileDialog,
    /// In-egui file dialog for Save As.
    save_as_dialog:           FileDialog,
    /// In-egui file dialog for BPS patch export.
    bps_export_dialog:        FileDialog,
    /// In-egui file dialog for IPS patch export.
    ips_export_dialog:        FileDialog,
    /// Expand-ROM dialog (File > Expand ROM...).
    show_expand_dialog:       bool,
    /// Selected expansion target size in bytes.
    expand_target:            usize,
    /// Status line shown in the Expand-ROM dialog.
    expand_status:            Option<String>,
    /// In-egui file dialog for single-level PNG export (File > Export Level to PNG...).
    png_export_dialog:        FileDialog,
    /// Translevel chosen for the pending single-level PNG export.
    png_export_level:         Option<u16>,
    /// Status line for the last single-level PNG export.
    png_export_status:        Option<String>,
    /// Batch level-export dialog (File > Levels > Export Multiple Levels to Image Files...).
    show_batch_export_dialog: bool,
    /// In-egui directory picker for the batch export output folder.
    batch_export_dir_dialog:  FileDialog,
    /// Hex strings for the batch export range (inclusive), e.g. "000"–"1FF".
    batch_from:               String,
    batch_to:                 String,
    /// Batch export output folder.
    batch_out_dir:            Option<PathBuf>,
    /// Batch export layer toggles (mirror the single-level options).
    batch_include_l1:         bool,
    batch_include_l2:         bool,
    batch_include_sprites:    bool,
    /// Status line shown in the batch-export dialog.
    batch_status:             Option<String>,
    /// Set when user tries to close the app with unsaved changes
    show_exit_dialog:         bool,
}

impl UiMainWindow {
    pub fn new(cc: &CreationContext) -> Self {
        let mut fonts = FontDefinitions::default();
        egui_phosphor::add_to_fonts(&mut fonts, Variant::Regular);
        cc.egui_ctx.set_fonts(fonts);
        cc.egui_ctx.set_visuals(Visuals::dark());

        let mut dock_style = DockStyle::from_egui(&cc.egui_ctx.style());
        dock_style.tab.tab_body.inner_margin = Margin::ZERO;

        Self {
            gl: Arc::clone(cc.gl.as_ref().expect("must use the glow renderer")),
            dock_style,
            dock_state: DockState::new(vec![]),
            rom_path: None,
            save_error: None,
            open_dialog: FileDialog::new(),
            save_as_dialog: FileDialog::new(),
            bps_export_dialog: FileDialog::new(),
            ips_export_dialog: FileDialog::new(),
            show_expand_dialog: false,
            expand_target: 0,
            expand_status: None,
            png_export_dialog: FileDialog::new(),
            png_export_level: None,
            png_export_status: None,
            show_batch_export_dialog: false,
            batch_export_dir_dialog: FileDialog::new(),
            batch_from: "000".to_string(),
            batch_to: "1FF".to_string(),
            batch_out_dir: None,
            batch_include_l1: true,
            batch_include_l2: true,
            batch_include_sprites: true,
            batch_status: None,
            show_exit_dialog: false,
        }
    }
}

impl eframe::App for UiMainWindow {
    fn update(&mut self, ctx: &Context, _frame: &mut Frame) {
        let rom: Option<Arc<SmwRom>> = ctx.data(|data| data.get_temp(Id::new("rom")));

        // Check if user is trying to close the app
        let is_finishing = ctx.input(|i| i.viewport().close_requested());
        if is_finishing && !self.show_exit_dialog && self.has_any_unsaved_changes() {
            self.show_exit_dialog = true;
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
        }

        // Menu bar always on top.
        self.main_menu_bar(ctx, rom.as_ref());

        // Open dialog.
        self.show_open_dialog(ctx);

        // Save As dialog (egui-native, no native file picker needed).
        self.show_save_as_dialog(ctx);

        // BPS export dialog.
        self.show_bps_export_dialog(ctx, rom.as_ref());

        // IPS export dialog.
        self.show_ips_export_dialog(ctx, rom.as_ref());

        // Single-level PNG export dialog (File > Export Level to PNG...).
        self.show_png_export_dialog(ctx);

        // Batch level-export dialog (File > Levels > Export Multiple Levels to Image Files...).
        if self.show_batch_export_dialog {
            self.batch_export_window(ctx);
        }
        self.batch_export_dir_dialog.update(ctx);
        if let Some(dir) = self.batch_export_dir_dialog.take_picked() {
            self.batch_out_dir = Some(dir);
            self.batch_status = None;
        }

        // Save error toast.
        if let Some(err) = &self.save_error.clone() {
            let mut open = true;
            Window::new("Save Error").open(&mut open).show(ctx, |ui| {
                ui.label(err);
                if ui.button("OK").clicked() {
                    self.save_error = None;
                }
            });
            if !open {
                self.save_error = None;
            }
        }

        // PNG export status toast.
        if let Some(status) = &self.png_export_status.clone() {
            let mut open = true;
            Window::new("Level PNG Export").open(&mut open).show(ctx, |ui| {
                ui.label(status);
                if ui.button("OK").clicked() {
                    self.png_export_status = None;
                }
            });
            if !open {
                self.png_export_status = None;
            }
        }

        // Expand ROM dialog.
        if self.show_expand_dialog {
            let mut open = true;
            let mut close_requested = false;
            let rom_len = rom.as_ref().map(|r| r.rom.bytes().len()).unwrap_or(0);
            let targets = expansion_targets(rom_len);
            Window::new("Expand ROM").open(&mut open).resizable(false).show(ctx, |ui| {
                if let Some(r) = rom.as_ref() {
                    ui.label(format!("Current size: {} ({})", format_size(rom_len), r.internal_header.map_mode));
                }
                ui.separator();
                if targets.is_empty() {
                    ui.label("This ROM is already at the maximum LoROM size (4 MB).");
                } else {
                    ui.label("Expand to:");
                    for t in &targets {
                        ui.radio_value(
                            &mut self.expand_target,
                            *t,
                            format!("{} ({} Mbit)", format_size(*t), t / 0x2_0000),
                        );
                    }
                    ui.separator();
                    ui.label(
                        "Appends $FF-filled banks and updates the internal header\n\
                         (ROM size byte + checksum). A .bak backup of the original\n\
                         file is kept next to the ROM. Unsaved edits are saved first.",
                    )
                    .on_hover_text("Same layout Lunar Magic produces for LoROM expansion");
                }
                ui.separator();
                ui.horizontal(|ui| {
                    let can_expand = !targets.is_empty();
                    if ui.add_enabled(can_expand, Button::new("Expand")).clicked() {
                        let ctx2 = ctx.clone();
                        self.perform_rom_expansion(&ctx2);
                    }
                    if ui.button("Cancel").clicked() {
                        close_requested = true;
                    }
                });
                if let Some(status) = &self.expand_status.clone() {
                    ui.separator();
                    ui.label(status);
                }
            });
            if !open || close_requested {
                self.show_expand_dialog = false;
                self.expand_status = None;
            }
        }

        // Welcome / splash when no ROM is open and no tabs.
        if rom.is_none() && self.dock_state.iter_all_tabs().count() == 0 {
            CentralPanel::default().show(ctx, |ui| {
                let mut open_requested = false;
                let chosen = welcome::draw_welcome(ui, &mut open_requested);
                if open_requested {
                    self.open_dialog = FileDialog::new();
                    self.open_dialog.pick_file();
                }
                if let Some(path) = chosen {
                    self.load_rom_from_path(ctx, path);
                }
            });
        } else {
            CentralPanel::default().show(ctx, |_ui| {});
        }

        DockArea::new(&mut self.dock_state).style(self.dock_style.clone()).show(ctx, &mut EditorToolTabViewer);

        // Check if any level editor is requesting a save
        self.check_for_save_requests(ctx);

        // Exit confirmation dialog
        if self.show_exit_dialog {
            egui::Window::new("⚠️  Unsaved Changes")
                .anchor(egui::Align2::CENTER_CENTER, Vec2::ZERO)
                .collapsible(false)
                .resizable(false)
                .show(ctx, |ui| {
                    ui.label("You have unsaved changes in open editors.");
                    ui.label("Do you want to save before exiting?");
                    ui.separator();
                    ui.horizontal(|ui| {
                        if ui.button("💾 Save & Exit").clicked() {
                            // Save all editors before closing
                            if self.rom_path.is_some() {
                                let path = self.rom_path.clone().unwrap();
                                if self.write_rom_to_path(&path, &path).is_ok() {
                                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                                } else {
                                    self.show_exit_dialog = false;
                                }
                            } else {
                                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                            }
                        }
                        if ui.button("❌ Exit Without Saving").clicked() {
                            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                        }
                        if ui.button("⏸️ Cancel").clicked() {
                            self.show_exit_dialog = false;
                        }
                    });
                });
        }
    }
}

impl UiMainWindow {
    fn open_tool<ToolType>(&mut self, tool: ToolType)
    where
        ToolType: 'static + DockableEditorTool,
    {
        log::info!("Opened {}", tool.title().text());
        self.dock_state.push_to_focused_leaf(Box::new(tool));
    }

    fn show_open_dialog(&mut self, ctx: &Context) {
        self.open_dialog.update(ctx);
        if let Some(path) = self.open_dialog.take_picked() {
            self.load_rom_from_path(ctx, path);
        }
    }

    fn load_rom_from_path(&mut self, ctx: &Context, path: PathBuf) {
        match Project::new(&path) {
            Ok(project) => {
                Project::add_to_recent(&path);
                ctx.data_mut(|data| {
                    data.insert_temp(Project::project_title_id(), project.title.clone());
                    data.insert_temp(Project::rom_id(), Arc::clone(&project.rom));
                });
                self.rom_path = Some(path.clone());
                let rom: Arc<SmwRom> = Arc::clone(&project.rom);
                match UiLevelEditor::new(Arc::clone(&self.gl), rom, path) {
                    Ok(editor) => self.open_tool(editor),
                    Err(e) => self.save_error = Some(format!("Failed to open level editor: {e}")),
                }
            }
            Err(e) => self.save_error = Some(format!("Failed to open ROM: {e}")),
        }
    }

    fn save_rom(&mut self, ctx: &Context) {
        let Some(path) = &self.rom_path else {
            self.save_error = Some("No ROM path — open a ROM first.".into());
            return;
        };
        let rom: Option<Arc<SmwRom>> = ctx.data(|d| d.get_temp(Id::new("rom")));
        let Some(_) = rom else {
            self.save_error = Some("No ROM loaded.".into());
            return;
        };
        match self.write_rom_to_path(path, path) {
            Ok(()) => {
                if let Err(e) = self.reload_rom_into_context(ctx, path) {
                    self.save_error = Some(format!("Saved ROM, but reload failed: {e}"));
                } else {
                    log::info!("Saved ROM to {}", path.display());
                }
            }
            Err(e) => self.save_error = Some(format!("Save failed: {e}")),
        }
    }

    fn save_rom_as(&mut self) {
        let initial_dir = self
            .rom_path
            .as_deref()
            .and_then(|p| p.parent())
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|| PathBuf::from("."));
        let initial_name = self
            .rom_path
            .as_deref()
            .and_then(|p| p.file_name())
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "output.smc".to_string());

        self.save_as_dialog = FileDialog::new().initial_directory(initial_dir).default_file_name(&initial_name);
        self.save_as_dialog.save_file();
    }

    fn export_bps_patch(&mut self) {
        let initial_dir = self
            .rom_path
            .as_deref()
            .and_then(|p| p.parent())
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|| PathBuf::from("."));
        let initial_name = self
            .rom_path
            .as_deref()
            .and_then(|p| p.file_stem())
            .map(|n| format!("{}.bps", n.to_string_lossy()))
            .unwrap_or_else(|| "output.bps".to_string());

        self.bps_export_dialog = FileDialog::new().initial_directory(initial_dir).default_file_name(&initial_name);
        self.bps_export_dialog.save_file();
    }

    fn export_ips_patch(&mut self) {
        let initial_dir = self
            .rom_path
            .as_deref()
            .and_then(|p| p.parent())
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|| PathBuf::from("."));
        let initial_name = self
            .rom_path
            .as_deref()
            .and_then(|p| p.file_stem())
            .map(|n| format!("{}.ips", n.to_string_lossy()))
            .unwrap_or_else(|| "output.ips".to_string());

        self.ips_export_dialog = FileDialog::new().initial_directory(initial_dir).default_file_name(&initial_name);
        self.ips_export_dialog.save_file();
    }

    fn show_save_as_dialog(&mut self, ctx: &Context) {
        self.save_as_dialog.update(ctx);
        if let Some(dest) = self.save_as_dialog.take_picked() {
            let Some(src) = self.rom_path.clone() else {
                return;
            };
            match self.write_rom_to_path(&src, &dest) {
                Ok(_) => {
                    log::info!("Saved ROM as {}", dest.display());
                    if let Err(e) = self.reload_rom_into_context(ctx, &dest) {
                        self.save_error = Some(format!("Saved ROM As, but reload failed: {e}"));
                        return;
                    }
                    self.rom_path = Some(dest.to_path_buf());
                }
                Err(e) => self.save_error = Some(format!("Save As failed: {e}")),
            }
        }
    }

    fn show_bps_export_dialog(&mut self, ctx: &Context, rom: Option<&Arc<SmwRom>>) {
        self.bps_export_dialog.update(ctx);
        if let Some(patch_dest) = self.bps_export_dialog.take_picked() {
            let Some(rom) = rom else {
                self.save_error = Some("No ROM loaded.".into());
                return;
            };
            let Some(src) = self.rom_path.clone() else {
                self.save_error = Some("No ROM path — open a ROM first.".into());
                return;
            };

            match self.create_bps_patch(rom, &src, &patch_dest) {
                Ok(_) => {
                    log::info!("Exported BPS patch to {}", patch_dest.display());
                }
                Err(e) => self.save_error = Some(format!("BPS export failed: {e}")),
            }
        }
    }

    fn create_bps_patch(
        &self, _rom: &Arc<SmwRom>, original_rom_path: &std::path::Path, patch_dest: &std::path::Path,
    ) -> anyhow::Result<()> {
        // Read the original ROM to generate patch against it
        let original_bytes = std::fs::read(original_rom_path)
            .with_context(|| format!("Failed to read original ROM from {}", original_rom_path.display()))?;

        // Create the modified ROM (with all current edits applied)
        let mut modified_bytes = original_bytes.clone();
        let has_smc_header = modified_bytes.len() % 0x400 == 0x200;
        for (_, tab) in self.dock_state.iter_all_tabs() {
            tab.save_to_rom(&mut modified_bytes, has_smc_header)?;
        }

        // Create BPS patch
        let patch = smwe_bps::create_patch(&original_bytes, &modified_bytes)?;

        // Write patch to file
        std::fs::write(patch_dest, patch)
            .with_context(|| format!("Failed to write BPS patch to {}", patch_dest.display()))?;

        Ok(())
    }

    fn show_ips_export_dialog(&mut self, ctx: &Context, rom: Option<&Arc<SmwRom>>) {
        self.ips_export_dialog.update(ctx);
        if let Some(patch_dest) = self.ips_export_dialog.take_picked() {
            let Some(rom) = rom else {
                self.save_error = Some("No ROM loaded.".into());
                return;
            };
            let Some(src) = self.rom_path.clone() else {
                self.save_error = Some("No ROM path — open a ROM first.".into());
                return;
            };

            match self.create_ips_patch(rom, &src, &patch_dest) {
                Ok(_) => {
                    log::info!("Exported IPS patch to {}", patch_dest.display());
                }
                Err(e) => self.save_error = Some(format!("IPS export failed: {e}")),
            }
        }
    }

    fn create_ips_patch(
        &self, _rom: &Arc<SmwRom>, original_rom_path: &std::path::Path, patch_dest: &std::path::Path,
    ) -> anyhow::Result<()> {
        // Read the original ROM to generate patch against it
        let original_bytes = std::fs::read(original_rom_path)
            .with_context(|| format!("Failed to read original ROM from {}", original_rom_path.display()))?;

        // Create the modified ROM (with all current edits applied)
        let mut modified_bytes = original_bytes.clone();
        let has_smc_header = modified_bytes.len() % 0x400 == 0x200;
        for (_, tab) in self.dock_state.iter_all_tabs() {
            tab.save_to_rom(&mut modified_bytes, has_smc_header)?;
        }

        // Create IPS patch
        let patch = smwe_ips::create_patch(&original_bytes, &modified_bytes)?;

        // Write patch to file
        std::fs::write(patch_dest, patch)
            .with_context(|| format!("Failed to write IPS patch to {}", patch_dest.display()))?;

        Ok(())
    }

    /// Read the ROM from disk with every open tab's unsaved edits merged in —
    /// shared by the PNG export actions so exports reflect on-screen edits
    /// (same merge the BPS/IPS exports do).
    fn rom_bytes_with_tab_edits(&self) -> anyhow::Result<Vec<u8>> {
        let Some(src) = self.rom_path.clone() else { anyhow::bail!("No ROM path — open a ROM first.") };
        let mut rom_bytes =
            std::fs::read(&src).with_context(|| format!("Failed to read ROM from {}", src.display()))?;
        let has_smc_header = rom_bytes.len() % 0x400 == 0x200;
        for (_, tab) in self.dock_state.iter_all_tabs() {
            tab.save_to_rom(&mut rom_bytes, has_smc_header)?;
        }
        Ok(rom_bytes)
    }

    /// File > Export Level to PNG... — exports the focused level-editor tab
    /// (falling back to the first open level editor), like LM exports the
    /// active level window.
    fn export_level_png(&mut self) {
        let level = self
            .dock_state
            .find_active_focused()
            .and_then(|(_, tab)| tab.level_number())
            .or_else(|| self.dock_state.iter_all_tabs().find_map(|(_, tab)| tab.level_number()));
        let Some(level) = level else {
            self.save_error = Some("No level editor tab is open — open a level first.".to_string());
            return;
        };
        let initial_dir = self.rom_path.as_ref().and_then(|p| p.parent()).map(|d| d.to_path_buf()).unwrap_or_default();
        self.png_export_level = Some(level);
        self.png_export_status = None;
        self.png_export_dialog =
            FileDialog::new().initial_directory(initial_dir).default_file_name(&level_export_filename(level));
        self.png_export_dialog.save_file();
    }

    fn show_png_export_dialog(&mut self, ctx: &Context) {
        self.png_export_dialog.update(ctx);
        let Some(png_dest) = self.png_export_dialog.take_picked() else {
            return;
        };
        let Some(level) = self.png_export_level else {
            return;
        };
        self.png_export_level = None;
        let result = (|| -> anyhow::Result<()> {
            let rom_bytes = self.rom_bytes_with_tab_edits()?;
            let png = level_png_bytes(&rom_bytes, level, &LevelPngOptions::default())?;
            std::fs::write(&png_dest, &png)
                .with_context(|| format!("Failed to write PNG to {}", png_dest.display()))?;
            Ok(())
        })();
        match result {
            Ok(()) => {
                self.png_export_status = Some(format!("Exported level {level:03X} to {}", png_dest.display()));
                log::info!("Exported level {level:03X} PNG to {}", png_dest.display());
            }
            Err(e) => {
                self.png_export_status = Some(format!("Level export failed: {e}"));
            }
        }
    }

    /// File > Levels > Export Multiple Levels to Image Files... dialog.
    /// Mirrors LM v3.20: a hex level range plus output folder, one PNG per
    /// level named `level_XXX.png`.
    fn batch_export_window(&mut self, ctx: &Context) {
        let mut open = true;
        let mut close_requested = false;
        Window::new("Export Multiple Levels to Image Files")
            .open(&mut open)
            .resizable(false)
            .show(ctx, |ui| {
                ui.label(format!(
                    "Renders each level in the range as a PNG image,\none file per level (level_000.png … level_{:03X}.png).",
                    LEVEL_COUNT - 1
                ));
                ui.separator();
                ui.horizontal(|ui| {
                    ui.label("From level (hex):");
                    ui.text_edit_singleline(&mut self.batch_from);
                    ui.label("To level (hex):");
                    ui.text_edit_singleline(&mut self.batch_to);
                });
                ui.horizontal(|ui| {
                    ui.checkbox(&mut self.batch_include_l1, "Layer 1");
                    ui.checkbox(&mut self.batch_include_l2, "Layer 2");
                    ui.checkbox(&mut self.batch_include_sprites, "Sprites");
                });
                ui.separator();
                ui.horizontal(|ui| {
                    ui.label("Output folder:");
                    ui.label(
                        self.batch_out_dir
                            .as_ref()
                            .map(|d| d.display().to_string())
                            .unwrap_or_else(|| "(not chosen)".to_string()),
                    );
                    if ui.button("Choose...").clicked() {
                        let initial = self
                            .batch_out_dir
                            .clone()
                            .or_else(|| {
                                self.rom_path.as_ref().and_then(|p| p.parent()).map(|d| d.to_path_buf())
                            })
                            .unwrap_or_default();
                        self.batch_export_dir_dialog = FileDialog::new().initial_directory(initial);
                        self.batch_export_dir_dialog.pick_directory();
                    }
                });
                ui.separator();
                ui.horizontal(|ui| {
                    let can_export = self.batch_out_dir.is_some();
                    if ui.add_enabled(can_export, Button::new("Export")).clicked() {
                        self.perform_batch_export();
                    }
                    if ui.button("Close").clicked() {
                        close_requested = true;
                    }
                });
                if let Some(status) = &self.batch_status.clone() {
                    ui.separator();
                    ui.label(status);
                }
            });
        if !open || close_requested {
            self.show_batch_export_dialog = false;
        }
    }

    fn perform_batch_export(&mut self) {
        fn parse_hex(s: &str) -> Option<u16> {
            u16::from_str_radix(s.trim().trim_start_matches("0x").trim_start_matches('$').trim_start_matches('#'), 16)
                .ok()
        }
        let (from, to) = (parse_hex(&self.batch_from), parse_hex(&self.batch_to));
        let (Some(from), Some(to)) = (from, to) else {
            self.batch_status = Some("Invalid level range — enter hex numbers like 000 and 1FF.".to_string());
            return;
        };
        if from > to || to >= LEVEL_COUNT {
            self.batch_status = Some(format!("Range must satisfy 000 ≤ from ≤ to ≤ {:03X}.", LEVEL_COUNT - 1));
            return;
        }
        let Some(out_dir) = self.batch_out_dir.clone() else {
            self.batch_status = Some("Choose an output folder first.".to_string());
            return;
        };
        let opts = LevelPngOptions {
            include_layer1:  self.batch_include_l1,
            include_layer2:  self.batch_include_l2,
            include_sprites: self.batch_include_sprites,
        };
        let rom_bytes = match self.rom_bytes_with_tab_edits() {
            Ok(b) => b,
            Err(e) => {
                self.batch_status = Some(format!("Batch export failed: {e}"));
                return;
            }
        };
        let mut exported = 0u32;
        for level in from..=to {
            match level_png_bytes(&rom_bytes, level, &opts) {
                Ok(png) => {
                    let dest = out_dir.join(level_export_filename(level));
                    if let Err(e) = std::fs::write(&dest, &png) {
                        self.batch_status = Some(format!("Failed writing {}: {e}", dest.display()));
                        return;
                    }
                    exported += 1;
                }
                Err(e) => {
                    self.batch_status = Some(format!("Failed rendering level {level:03X}: {e}"));
                    return;
                }
            }
        }
        self.batch_status = Some(format!("Exported {exported} level PNGs to {}", out_dir.display()));
        log::info!("Batch-exported {exported} level PNGs to {}", out_dir.display());
    }

    fn main_menu_bar(&mut self, ctx: &Context, rom: Option<&Arc<SmwRom>>) {
        let has_rom = rom.is_some();
        // Ctrl+S shortcut.
        if ctx.input_mut(|i| i.consume_shortcut(&KeyboardShortcut::new(Modifiers::CTRL, Key::S))) {
            let ctx2 = ctx.clone();
            self.save_rom(&ctx2);
        }

        TopBottomPanel::top("main_top_bar").show(ctx, |ui| {
            menu::bar(ui, |ui| {
                // ── File ──
                ui.menu_button("File", |ui| {
                    if ui.button("Open ROM...").clicked() {
                        self.open_dialog = FileDialog::new();
                        self.open_dialog.pick_file();
                        ui.close_menu();
                    }
                    ui.add_enabled_ui(has_rom, |ui| {
                        if ui.button("Save ROM        Ctrl+S").clicked() {
                            let ctx2 = ctx.clone();
                            self.save_rom(&ctx2);
                            ui.close_menu();
                        }
                        if ui.button("Save ROM As...").clicked() {
                            self.save_rom_as();
                            ui.close_menu();
                        }
                        ui.separator();
                        if ui.button("Expand ROM...").clicked() {
                            // Default to the largest available target, like Lunar Magic.
                            if let Some(r) = rom {
                                let current = r.rom.bytes().len();
                                self.expand_target = expansion_targets(current).into_iter().last().unwrap_or(0);
                            }
                            self.expand_status = None;
                            self.show_expand_dialog = true;
                            ui.close_menu();
                        }
                        ui.separator();
                        if ui.button("Export BPS Patch...").clicked() {
                            self.export_bps_patch();
                            ui.close_menu();
                        }
                        if ui.button("Export IPS Patch...").clicked() {
                            self.export_ips_patch();
                            ui.close_menu();
                        }
                        ui.separator();
                        if ui.button("Export Level to PNG...").clicked() {
                            self.export_level_png();
                            ui.close_menu();
                        }
                        ui.menu_button("Levels", |ui| {
                            if ui.button("Export Multiple Levels to Image Files...").clicked() {
                                self.batch_from = "000".to_string();
                                self.batch_to = format!("{:03X}", LEVEL_COUNT - 1);
                                self.batch_include_l1 = true;
                                self.batch_include_l2 = true;
                                self.batch_include_sprites = true;
                                self.batch_status = None;
                                self.show_batch_export_dialog = true;
                                ui.close_menu();
                            }
                        });
                    });
                    ui.separator();
                    if ui.button("Exit").clicked() {
                        ctx.send_viewport_cmd(ViewportCommand::Close);
                    }
                });

                // ── Editors ──
                ui.menu_button("Editors", |ui| {
                    ui.add_enabled_ui(has_rom, |ui| {
                        if ui.button("Level Editor").clicked() {
                            let path = self.rom_path.clone().unwrap_or_default();
                            match UiLevelEditor::new(Arc::clone(&self.gl), Arc::clone(rom.unwrap()), path) {
                                Ok(editor) => self.open_tool(editor),
                                Err(e) => self.save_error = Some(format!("Failed to open level editor: {e}")),
                            }
                            ui.close_menu();
                        }
                        if ui.button("World Map Editor").clicked() {
                            let Some(path) = self.rom_path.clone() else {
                                self.save_error =
                                    Some("No ROM path available for emulator-backed overworld view.".into());
                                ui.close_menu();
                                return;
                            };
                            self.open_tool(UiWorldEditor::new(Arc::clone(&self.gl), Arc::clone(rom.unwrap()), path));
                            ui.close_menu();
                        }
                        if ui.button("Sprite Tile Editor").clicked() {
                            self.open_tool(UiSpriteMapEditor::new(Arc::clone(&self.gl), Arc::clone(rom.unwrap())));
                            ui.close_menu();
                        }
                    });
                });

                // ── Tools ──
                ui.menu_button("Tools", |ui| {
                    if ui.button("Address Converter").clicked() {
                        self.open_tool(UiAddressConverter::default());
                        ui.close_menu();
                    }
                });

                // Right-aligned ROM name.
                if has_rom {
                    let title: String =
                        ctx.data(|d| d.get_temp(Project::project_title_id()).unwrap_or_else(|| "ROM".to_string()));
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        ui.label(RichText::new(format!("📁 {title}")).small());
                    });
                }
            });
        });
    }

    /// Write `bytes` to `dest_path` atomically: keep a `.bak` backup of the
    /// previous contents (if any), write via a temp file + rename so a
    /// crash/full-disk mid-write can't corrupt the user's only copy.
    fn atomic_write_with_backup(dest_path: &Path, bytes: &[u8]) -> anyhow::Result<()> {
        if dest_path.exists() {
            let bak_path = dest_path
                .with_extension(format!("{}.bak", dest_path.extension().and_then(|e| e.to_str()).unwrap_or("smc")));
            std::fs::copy(dest_path, &bak_path)
                .with_context(|| format!("Failed to back up {} to {}", dest_path.display(), bak_path.display()))?;
        }

        let dest_dir = dest_path.parent().unwrap_or_else(|| std::path::Path::new("."));
        let tmp_path =
            dest_dir.join(format!(".{}.tmp", dest_path.file_name().and_then(|n| n.to_str()).unwrap_or("rom_save")));
        {
            let mut tmp_file = std::fs::File::create(&tmp_path)
                .with_context(|| format!("Failed to create temp file {}", tmp_path.display()))?;
            use std::io::Write;
            tmp_file.write_all(bytes).with_context(|| format!("Failed to write temp file {}", tmp_path.display()))?;
            tmp_file.sync_all().with_context(|| format!("Failed to flush temp file {}", tmp_path.display()))?;
        }
        std::fs::rename(&tmp_path, dest_path).with_context(|| {
            format!("Failed to move temp file {} into place at {}", tmp_path.display(), dest_path.display())
        })?;
        Ok(())
    }

    fn write_rom_to_path(&self, source_path: &std::path::Path, dest_path: &std::path::Path) -> anyhow::Result<()> {
        let mut rom_bytes =
            std::fs::read(source_path).with_context(|| format!("Failed to read ROM from {}", source_path.display()))?;
        let has_smc_header = rom_bytes.len() % 0x400 == 0x200;
        for (_, tab) in self.dock_state.iter_all_tabs() {
            tab.save_to_rom(&mut rom_bytes, has_smc_header)?;
        }

        Self::atomic_write_with_backup(dest_path, &rom_bytes)
    }

    /// File > Expand ROM... action: merge unsaved tab edits (like Save does),
    /// grow the image to `self.expand_target`, preserve any SMC header, and
    /// reload the project so the new space is visible everywhere.
    fn perform_rom_expansion(&mut self, ctx: &Context) {
        let Some(path) = self.rom_path.clone() else {
            self.expand_status = Some("No ROM is open.".to_string());
            return;
        };
        let target = self.expand_target;
        let result = (|| -> anyhow::Result<usize> {
            let mut rom_bytes =
                std::fs::read(&path).with_context(|| format!("Failed to read ROM from {}", path.display()))?;
            let has_smc_header = rom_bytes.len() % 0x400 == 0x200;
            for (_, tab) in self.dock_state.iter_all_tabs() {
                tab.save_to_rom(&mut rom_bytes, has_smc_header)?;
            }
            let (smc_header, body) = split_smc_header(&rom_bytes);
            let expanded = expand_rom(&Rom::new(body.to_vec())?, target)?;
            let mut out = Vec::with_capacity(target + smc_header.map(|h| h.len()).unwrap_or(0));
            if let Some(h) = smc_header {
                out.extend_from_slice(h);
            }
            out.extend_from_slice(expanded.bytes());
            Self::atomic_write_with_backup(&path, &out)?;
            self.reload_rom_into_context(ctx, &path)?;
            Ok(target)
        })();
        match result {
            Ok(new_size) => {
                self.expand_status = Some(format!(
                    "Expanded to {}. The new $FF space is now available to the free-space scanner.",
                    format_size(new_size)
                ));
            }
            Err(e) => {
                self.expand_status = Some(format!("Expansion failed: {e:#}"));
            }
        }
    }

    fn reload_rom_into_context(&self, ctx: &Context, path: &std::path::Path) -> anyhow::Result<()> {
        let project = Project::new(path)?;
        ctx.data_mut(|data| {
            data.insert_temp(Project::project_title_id(), project.title.clone());
            data.insert_temp(Project::rom_id(), Arc::clone(&project.rom));
        });
        Ok(())
    }

    fn has_any_unsaved_changes(&self) -> bool {
        for (_, tab) in self.dock_state.iter_all_tabs() {
            if tab.has_unsaved_changes() {
                return true;
            }
        }
        false
    }

    fn check_for_save_requests(&mut self, ctx: &Context) {
        let mut should_save = false;
        for (_, tab) in self.dock_state.iter_all_tabs_mut() {
            if tab.take_save_request() {
                should_save = true;
            }
        }
        if should_save {
            if let Some(path) = &self.rom_path.clone() {
                if let Err(e) = self.write_rom_to_path(path, path) {
                    self.save_error = Some(format!("Save failed: {e}"));
                } else {
                    log::info!("Saved ROM to {}", path.display());
                    if let Err(e) = self.reload_rom_into_context(ctx, path) {
                        self.save_error = Some(format!("Saved ROM, but reload failed: {e}"));
                    }
                    for (_, tab) in self.dock_state.iter_all_tabs_mut() {
                        tab.on_save_succeeded();
                    }
                }
            }
        }
    }
}
