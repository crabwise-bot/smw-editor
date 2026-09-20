//! The second, user-defined toolbar (Lunar Magic v2.31+ parity).
//!
//! LM loads `usertoolbar.txt` from beside its executable at startup and
//! builds a second toolbar from it: external "scripting buttons" that launch
//! programs, internal `LM_…` command buttons, and spacers. This module owns
//! the runtime state (parsed config + tracked child processes) and renders
//! the strip; the file format itself lives in `smwe_usertoolbar`.
//!
//! LM-faithful behaviors kept here:
//! - The bar is hidden entirely when the file sets `LM_NO_TOOLBAR`, but
//!   shortcuts stay active.
//! - External buttons launch one process at a time per button by default; a
//!   click while its process is still running is a no-op (LM would focus the
//!   existing window — not portable, so we do nothing). `LM_ALLOW_MULT_
//!   INSTANCES` / `_FORCE_ALL` lets each click start a new process.
//! - `LM_OPEN_OTHER` launches via the OS file association (`xdg-open` /
//!   `open` / `cmd /C start`) instead of running the program directly.
//! - Parse + launch errors surface in an error window, capped at
//!   `LM_DISPLAY_ERRORS` (default 10).
//!
//! Honest adaptations (documented, no portable LM equivalent):
//! - LM extracts button icons from the program's resources on Windows. Here
//!   each button is labeled with its tooltip's first line.
//! - LM hands external tools the ROM path through its `$BECA` Windows-message
//!   protocol. Here `{rom}` in an argument (or the working directory) is
//!   replaced with the open ROM's path, and `SMW_ROM` is set in the child's
//!   environment.
//! - Relative working directories resolve against the executable's
//!   directory, like LM.
//! - Shortcuts are skipped while a text field has keyboard focus so typing in
//!   a dialog can't trigger a toolbar action; built-in shortcuts (e.g.
//!   Ctrl+S) are consumed before toolbar shortcuts, so a user binding that
//!   collides with one is shadowed by it.

use std::{
    collections::HashMap,
    path::PathBuf,
    process::{Child, Command},
};

use egui::{Context, Key, KeyboardShortcut, Modifiers, TopBottomPanel, Window};
use smwe_usertoolbar::{
    default_search_paths,
    parse,
    ExternalButton,
    KeySpec,
    NamedKey,
    Shortcut,
    ToolbarButton,
    UserToolbarConfig,
};

/// Internal `LM_…` commands smw-editor can actually perform. Any other parsed
/// internal command renders as a disabled button — LM itself knows 317 of
/// them; only the ones with a real editor action are wired.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ToolbarAction {
    /// `LM_VIEW_OVERWORLD` — open the World Map Editor tab.
    OpenWorldEditor,
    /// `LM_FILE_DELETE_LEVEL` — open File > Levels > Delete Levels from ROM...
    OpenDeleteLevels,
    /// `LM_FILE_EXPORT_DIRECTORY_BITMAP` — open File > Levels > Export
    /// Multiple Levels to Image Files... (LM writes BMP; smw-editor's parity
    /// for LM v2.30/v3.20 level-image export is PNG).
    OpenBatchPngExport,
}

/// Map an internal command name to an editor action, if one exists.
pub fn map_internal_command(name: &str) -> Option<ToolbarAction> {
    match name {
        "LM_VIEW_OVERWORLD" => Some(ToolbarAction::OpenWorldEditor),
        "LM_FILE_DELETE_LEVEL" => Some(ToolbarAction::OpenDeleteLevels),
        "LM_FILE_EXPORT_DIRECTORY_BITMAP" => Some(ToolbarAction::OpenBatchPngExport),
        _ => None,
    }
}

fn key_spec_to_egui(key: &KeySpec) -> Option<Key> {
    match key {
        KeySpec::Char(c) => {
            let u = c.to_ascii_uppercase();
            Some(match u {
                'A' => Key::A,
                'B' => Key::B,
                'C' => Key::C,
                'D' => Key::D,
                'E' => Key::E,
                'F' => Key::F,
                'G' => Key::G,
                'H' => Key::H,
                'I' => Key::I,
                'J' => Key::J,
                'K' => Key::K,
                'L' => Key::L,
                'M' => Key::M,
                'N' => Key::N,
                'O' => Key::O,
                'P' => Key::P,
                'Q' => Key::Q,
                'R' => Key::R,
                'S' => Key::S,
                'T' => Key::T,
                'U' => Key::U,
                'V' => Key::V,
                'W' => Key::W,
                'X' => Key::X,
                'Y' => Key::Y,
                'Z' => Key::Z,
                '0' => Key::Num0,
                '1' => Key::Num1,
                '2' => Key::Num2,
                '3' => Key::Num3,
                '4' => Key::Num4,
                '5' => Key::Num5,
                '6' => Key::Num6,
                '7' => Key::Num7,
                '8' => Key::Num8,
                '9' => Key::Num9,
                _ => return None,
            })
        }
        KeySpec::Function(n) => Some(match n {
            1 => Key::F1,
            2 => Key::F2,
            3 => Key::F3,
            4 => Key::F4,
            5 => Key::F5,
            6 => Key::F6,
            7 => Key::F7,
            8 => Key::F8,
            9 => Key::F9,
            10 => Key::F10,
            11 => Key::F11,
            12 => Key::F12,
            13 => Key::F13,
            14 => Key::F14,
            15 => Key::F15,
            16 => Key::F16,
            17 => Key::F17,
            18 => Key::F18,
            19 => Key::F19,
            20 => Key::F20,
            21 => Key::F21,
            22 => Key::F22,
            23 => Key::F23,
            24 => Key::F24,
            _ => return None,
        }),
        // egui has no Pause or numpad-operator keys; those shortcuts parse
        // (LM accepts them) but can never fire here.
        KeySpec::Named(NamedKey::Pause)
        | KeySpec::Named(NamedKey::NumpadMultiply)
        | KeySpec::Named(NamedKey::NumpadAdd)
        | KeySpec::Named(NamedKey::NumpadSubtract)
        | KeySpec::Named(NamedKey::NumpadDecimal)
        | KeySpec::Named(NamedKey::NumpadDivide) => None,
        KeySpec::Named(named) => Some(match named {
            NamedKey::Left => Key::ArrowLeft,
            NamedKey::Right => Key::ArrowRight,
            NamedKey::Up => Key::ArrowUp,
            NamedKey::Down => Key::ArrowDown,
            NamedKey::Home => Key::Home,
            NamedKey::End => Key::End,
            NamedKey::Insert => Key::Insert,
            NamedKey::Delete => Key::Delete,
            NamedKey::PageUp => Key::PageUp,
            NamedKey::PageDown => Key::PageDown,
            NamedKey::Space => Key::Space,
            NamedKey::Tab => Key::Tab,
            NamedKey::Return => Key::Enter,
            NamedKey::Escape => Key::Escape,
            NamedKey::Backspace => Key::Backspace,
            _ => return None,
        }),
    }
}

fn shortcut_to_egui(s: &Shortcut) -> Option<KeyboardShortcut> {
    let key = key_spec_to_egui(&s.key)?;
    Some(KeyboardShortcut::new(
        Modifiers { ctrl: s.ctrl, alt: s.alt, shift: s.shift, mac_cmd: false, command: s.ctrl },
        key,
    ))
}

/// Runtime state for the user toolbar: parsed config plus tracked children.
pub struct UserToolbarState {
    pub config:        UserToolbarConfig,
    /// Where the config was loaded from (`None` = no file found).
    pub source:        Option<PathBuf>,
    /// Button index → running child process.
    running:           HashMap<usize, Child>,
    /// Launch errors (parse errors live in `config.errors`).
    runtime_errors:    Vec<String>,
    error_window_open: bool,
}

impl UserToolbarState {
    /// Load `usertoolbar.txt` from the first search path that exists.
    /// Absent file → empty config (LM shows no second toolbar then).
    pub fn load() -> Self {
        let mut source = None;
        let mut config = UserToolbarConfig::default();
        config.options.display_errors = 10;
        for path in default_search_paths() {
            if let Ok(text) = std::fs::read_to_string(&path) {
                config = parse(&text);
                source = Some(path);
                break;
            }
        }
        let error_window_open = !config.errors.is_empty();
        Self { config, source, running: HashMap::new(), runtime_errors: Vec::new(), error_window_open }
    }

    /// True when the strip should render: a file was found, it defines
    /// buttons, and `LM_NO_TOOLBAR` is not set.
    pub fn strip_visible(&self) -> bool {
        self.source.is_some() && !self.config.buttons.is_empty() && !self.config.options.no_toolbar
    }

    fn child_running(&mut self, index: usize) -> bool {
        if let Some(child) = self.running.get_mut(&index) {
            match child.try_wait() {
                Ok(None) => true,
                _ => {
                    self.running.remove(&index);
                    false
                }
            }
        } else {
            false
        }
    }

    /// Reap finished children so they don't become zombies.
    pub fn reap_children(&mut self) {
        let dead: Vec<usize> = self
            .running
            .iter_mut()
            .filter_map(|(i, c)| match c.try_wait() {
                Ok(Some(_)) | Err(_) => Some(*i),
                Ok(None) => None,
            })
            .collect();
        for i in dead {
            self.running.remove(&i);
        }
    }

    fn note_error(&mut self, message: String) {
        self.runtime_errors.push(message);
        self.error_window_open = true;
    }

    /// Substitute `{rom}` with the open ROM's path (left literal when no ROM
    /// is open).
    fn substitute(arg: &str, rom_path: Option<&str>) -> String {
        match rom_path {
            Some(p) => arg.replace("{rom}", p),
            None => arg.to_string(),
        }
    }

    fn resolve_workdir(workdir: Option<&str>, rom_path: Option<&str>) -> Option<PathBuf> {
        let raw = workdir?;
        let sub = Self::substitute(raw, rom_path);
        let path = PathBuf::from(&sub);
        if path.is_absolute() {
            Some(path)
        } else {
            // LM documents the working directory as executable-relative.
            let exe_dir = std::env::current_exe().ok()?.parent()?.to_path_buf();
            Some(exe_dir.join(path))
        }
    }

    /// Launch an external button's program. Returns an internal action only
    /// for internal buttons (handled by the caller); external launches happen
    /// here.
    fn activate_external(&mut self, index: usize, button: &ExternalButton, rom_path: Option<&str>) {
        let allow_another =
            self.config.options.allow_mult_instances || self.config.options.allow_mult_instances_force_all;
        let open_other = self.config.options.open_other;
        #[cfg(target_os = "windows")]
        let no_console_window = self.config.options.no_console_window;
        if !allow_another && self.child_running(index) {
            // LM focuses the already-running program; not portable, so no-op.
            return;
        }
        let Some((program, args)) = button.command.split_first() else {
            self.note_error(format!("button #{}: empty command", index + 1));
            return;
        };
        let args: Vec<String> = args.iter().map(|a| Self::substitute(a, rom_path)).collect();
        let workdir = Self::resolve_workdir(button.workdir.as_deref(), rom_path);

        let spawn_result = if open_other {
            // LM_OPEN_OTHER: hand the target to the OS file association.
            let target = Self::substitute(program, rom_path);
            #[cfg(target_os = "windows")]
            let r = Command::new("cmd").args(["/C", "start", "", &target]).spawn();
            #[cfg(target_os = "macos")]
            let r = Command::new("open").arg(&target).spawn();
            #[cfg(not(any(target_os = "windows", target_os = "macos")))]
            let r = Command::new("xdg-open").arg(&target).spawn();
            r
        } else {
            let mut cmd = Command::new(Self::substitute(program, rom_path));
            cmd.args(&args);
            if let Some(dir) = workdir {
                cmd.current_dir(dir);
            }
            if let Some(p) = rom_path {
                cmd.env("SMW_ROM", p);
            }
            #[cfg(target_os = "windows")]
            if no_console_window {
                use std::os::windows::process::CommandExt;
                // CREATE_NO_WINDOW
                cmd.creation_flags(0x08000000);
            }
            cmd.spawn()
        };
        match spawn_result {
            Ok(child) => {
                self.running.insert(index, child);
            }
            Err(e) => self.note_error(format!("button #{} ({}): failed to launch: {e}", index + 1, button.label())),
        }
    }

    /// Handle a click/shortcut on button `index`; returns an internal action
    /// for the main window to perform, if any.
    pub fn activate(&mut self, index: usize, rom_path: Option<&str>, has_rom: bool) -> Option<ToolbarAction> {
        let button = self.config.buttons.get(index)?.clone();
        match button {
            ToolbarButton::Spacer => None,
            ToolbarButton::External(b) => {
                self.activate_external(index, &b, rom_path);
                None
            }
            ToolbarButton::Internal(b) => {
                let action = map_internal_command(&b.name)?;
                // Internal actions all need a ROM today.
                if !has_rom {
                    return None;
                }
                Some(action)
            }
        }
    }

    /// Render the second toolbar strip (below the main menu bar). Returns
    /// internal actions requested by clicks.
    pub fn show_strip(&mut self, ctx: &Context, rom_path: Option<&str>, has_rom: bool) -> Vec<ToolbarAction> {
        let mut actions = Vec::new();
        if !self.strip_visible() {
            return actions;
        }
        // Clone the button list so `activate` can take &mut self inside the closure.
        let buttons = self.config.buttons.clone();
        TopBottomPanel::top("user_toolbar_bar").show(ctx, |ui| {
            ui.horizontal(|ui| {
                for (index, button) in buttons.iter().enumerate() {
                    match button {
                        ToolbarButton::Spacer => {
                            ui.separator();
                        }
                        ToolbarButton::External(b) => {
                            let resp = ui.button(b.label()).on_hover_text(b.tooltip.as_str());
                            if resp.clicked() {
                                if let Some(a) = self.activate(index, rom_path, has_rom) {
                                    actions.push(a);
                                }
                            }
                        }
                        ToolbarButton::Internal(b) => {
                            if map_internal_command(&b.name).is_some() {
                                let resp = ui
                                    .add_enabled(has_rom, egui::Button::new(b.label()))
                                    .on_hover_text(b.tooltip.as_str());
                                if resp.clicked() {
                                    if let Some(a) = self.activate(index, rom_path, has_rom) {
                                        actions.push(a);
                                    }
                                }
                            } else {
                                ui.add_enabled(false, egui::Button::new(b.label())).on_disabled_hover_text(format!(
                                    "{}\n({} is not mapped to an action in smw-editor)",
                                    b.tooltip.as_str(),
                                    b.name
                                ));
                            }
                        }
                    }
                }
            });
        });
        actions
    }

    /// Fire toolbar shortcuts. Call before the menu bar so user shortcuts win
    /// over built-ins; skipped while a text field has focus.
    pub fn poll_shortcuts(&mut self, ctx: &Context, rom_path: Option<&str>, has_rom: bool) -> Vec<ToolbarAction> {
        let mut actions = Vec::new();
        if self.source.is_none() || ctx.wants_keyboard_input() {
            return actions;
        }
        let shortcuts: Vec<(usize, KeyboardShortcut)> = self
            .config
            .buttons
            .iter()
            .enumerate()
            .filter_map(|(i, b)| Some((i, shortcut_to_egui(b.shortcut()?)?)))
            .collect();
        for (index, egui_shortcut) in shortcuts {
            if ctx.input_mut(|i| i.consume_shortcut(&egui_shortcut)) {
                if let Some(a) = self.activate(index, rom_path, has_rom) {
                    actions.push(a);
                }
            }
        }
        actions
    }

    /// Show parse/launch errors, capped at `LM_DISPLAY_ERRORS`.
    pub fn show_error_window(&mut self, ctx: &Context) {
        if !self.error_window_open {
            return;
        }
        let cap = self.config.options.display_errors.max(1) as usize;
        let mut parse: Vec<String> = self.config.errors.iter().map(|e| e.to_string()).collect();
        parse.extend(self.runtime_errors.iter().cloned());
        let total = parse.len();
        let shown: Vec<String> = parse.into_iter().take(cap).collect();
        let source = self
            .source
            .as_ref()
            .map(|p| p.display().to_string())
            .unwrap_or_else(|| "<no usertoolbar.txt found>".to_string());
        let mut open = true;
        let mut close_pressed = false;
        Window::new("User Toolbar Errors").open(&mut open).show(ctx, |ui| {
            ui.label(format!("{source}:"));
            for e in &shown {
                ui.label(e);
            }
            if total > shown.len() {
                ui.label(format!("…and {} more (LM_DISPLAY_ERRORS {})", total - shown.len(), cap));
            }
            if ui.button("Close").clicked() {
                close_pressed = true;
            }
        });
        if !open || close_pressed {
            self.error_window_open = false;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn internal_command_routing() {
        assert_eq!(map_internal_command("LM_VIEW_OVERWORLD"), Some(ToolbarAction::OpenWorldEditor));
        assert_eq!(map_internal_command("LM_FILE_DELETE_LEVEL"), Some(ToolbarAction::OpenDeleteLevels));
        assert_eq!(map_internal_command("LM_FILE_EXPORT_DIRECTORY_BITMAP"), Some(ToolbarAction::OpenBatchPngExport));
        // LM knows hundreds of internal commands; only mapped ones route.
        assert_eq!(map_internal_command("LM_LEVEL_EXITS"), None);
        assert_eq!(map_internal_command("LM_SPACER"), None);
    }

    #[test]
    fn rom_placeholder_substitution() {
        assert_eq!(UserToolbarState::substitute("{rom}", Some("/x/game.smc")), "/x/game.smc");
        assert_eq!(
            UserToolbarState::substitute("--rom={rom} --verbose", Some("/x/game.smc")),
            "--rom=/x/game.smc --verbose"
        );
        // No ROM open: the literal survives (the tool may not need one).
        assert_eq!(UserToolbarState::substitute("{rom}", None), "{rom}");
        assert_eq!(UserToolbarState::substitute("plain", Some("/x")), "plain");
    }

    #[test]
    fn workdir_resolution() {
        // Absolute paths pass through untouched.
        let abs = UserToolbarState::resolve_workdir(Some("/tmp/work"), Some("/x/game.smc"));
        assert_eq!(abs, Some(PathBuf::from("/tmp/work")));
        // None stays None.
        assert_eq!(UserToolbarState::resolve_workdir(None, Some("/x")), None);
        // Relative paths resolve against the executable's directory (LM behavior).
        let rel = UserToolbarState::resolve_workdir(Some("tools"), Some("/x/game.smc")).unwrap();
        let exe_dir = std::env::current_exe().unwrap().parent().unwrap().to_path_buf();
        assert_eq!(rel, exe_dir.join("tools"));
        // {rom} is substituted in the working directory too.
        let sub = UserToolbarState::resolve_workdir(Some("{rom}.d"), None).unwrap();
        assert_eq!(sub, exe_dir.join("{rom}.d"));
    }

    #[test]
    fn key_mapping_covers_fixture_shortcuts() {
        use smwe_usertoolbar::{KeySpec, Shortcut};
        let s = Shortcut { key: KeySpec::Char('o'), ctrl: true, alt: false, shift: true };
        let egui_sc = shortcut_to_egui(&s).expect("'o' maps");
        assert_eq!(egui_sc.logical_key, Key::O);
        assert!(egui_sc.modifiers.ctrl && egui_sc.modifiers.shift && !egui_sc.modifiers.alt);
        // Pause and numpad operators parse (LM accepts them) but have no
        // egui key, so the shortcut stays inert instead of misfiring.
        let s = Shortcut { key: KeySpec::Named(NamedKey::Pause), ctrl: false, alt: false, shift: false };
        assert!(shortcut_to_egui(&s).is_none());
    }
}
