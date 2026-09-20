//! Lunar Magic-style **custom user toolbar** (LM v2.31+).
//!
//! Lunar Magic loads a `usertoolbar.txt` file from beside its executable at
//! startup and builds a second toolbar from it. Each button is either an
//! *external* command (launches a program — the "scripting buttons" use case:
//! Asar, PIXI, a text editor, …) or an *internal* `LM_…` command, plus spacer
//! separators. Buttons can carry tooltips and keyboard shortcuts, and the file
//! can hide the bar while keeping shortcuts active (`LM_NO_TOOLBAR`).
//!
//! This crate is the UI-free parser + data model for that file format, so the
//! editor UI and the headless screenshot binary share one implementation. The
//! grammar below is reconstructed from the LM 3.63 help topic
//! *Technical Information → Custom User Toolbar* and verified against its two
//! canonical example files (see the unit tests):
//!
//! ```text
//! LM_DISPLAY_ERRORS 10, LM_NO_TOOLBAR, LM_SETIMAGE_SIZE 24,
//! ***START***, LM_SPACER,
//! ***START***, LM_VIEW_OVERWORLD
//!
//! 0,Open overworld from oracle toolbar
//!
//! LM_DEFAULT
//!
//! 'o',VK_CONTROL,VK_SHIFT
//!
//! ***START***
//!
//! "notepad.exe" "oracle argument.txt"
//!
//! 0,External oracle button\nSecond tooltip line
//!
//! LM_USEIMAGE_LIST
//!
//! 'e',VK_CONTROL
//!
//! "%4"
//!
//! ***END***
//! ```
//!
//! Rules implemented here:
//! - The file is UTF-8 (an optional BOM is stripped). Blank lines are ignored.
//! - Comma-separated tokens before the first `***START***` are global options.
//!   Known options: `LM_DISPLAY_ERRORS <n>` (cap on shown errors),
//!   `LM_NO_TOOLBAR` (hide the second bar, keep shortcuts), `LM_SETIMAGE_SIZE
//!   <n>` (Windows-only icon size; parsed but unused here),
//!   `LM_USEIMAGE_FORCE` / `LM_USEIMAGE_FORCE_ALL` (Windows-only icon source;
//!   parsed but unused here), `LM_ALLOW_MULT_INSTANCES` /
//!   `LM_ALLOW_MULT_INSTANCES_FORCE_ALL` (a button click may start another
//!   process instead of being a no-op while one is running),
//!   `LM_NO_CONSOLE_WINDOW` (hide the console of launched console programs;
//!   Windows-only), `LM_OPEN_OTHER` (launch via the OS file association
//!   instead of directly). Unknown `LM_…` tokens are reported, not fatal.
//! - `***START***` opens a button definition; `***END***` closes it. A new
//!   `***START***` implicitly closes the previous definition.
//! - The first field after `***START***` is the command: `LM_SPACER` (a
//!   separator, takes no further fields), an `LM_…` internal command name, or
//!   a quoted command line (`"program" "arg with spaces" …`).
//! - The remaining four fields are one per (non-blank) line:
//!   1. `<icon-index>,<tooltip>` — the icon index selects an icon from the
//!      program's resources on Windows (not portable; smw-editor shows the
//!      tooltip's first line as the button label instead). `\n` inside the
//!      tooltip becomes a real newline.
//!   2. `LM_DEFAULT` or `LM_USEIMAGE_LIST` — Windows-only icon source;
//!      parsed and retained, unused here.
//!   3. Shortcut: `'x',VK_CONTROL,VK_SHIFT` — a quoted character, a Windows
//!      virtual-key name (`VK_A`…`VK_Z`, `VK_0`…`VK_9`, `VK_F1`…`VK_F24`,
//!      `VK_PAUSE`, numpad operators, navigation keys), or a raw hex value
//!      like `0x4F`; modifiers `VK_CONTROL`/`VK_SHIFT`/`VK_MENU` (= Alt).
//!   4. Working directory, quoted; LM documents it as executable-relative.
//!      smw-editor resolves relative paths against the executable's directory.
//!
//! Not modeled (Windows-only LM internals with no portable meaning):
//! executable icon extraction, the bitmap strip, and the `$BECA`
//! interprocess-notification protocol LM uses to hand external tools the ROM
//! path. smw-editor instead substitutes `{rom}` in command arguments and sets
//! the `SMW_ROM` environment variable for launched children — documented in
//! `docs/USER_TOOLBAR.md`.

use std::fmt;

/// Global options parsed from the file header.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct GlobalOptions {
    /// `LM_DISPLAY_ERRORS <n>`: cap on the number of parse/launch errors
    /// shown to the user. Defaults to 10 when absent (LM's example files all
    /// use 10).
    pub display_errors:                 u32,
    /// `LM_NO_TOOLBAR`: hide the second toolbar; shortcuts stay active.
    pub no_toolbar:                     bool,
    /// `LM_SETIMAGE_SIZE <n>`: Windows-only icon size. Parsed, unused here.
    pub setimage_size:                  Option<u32>,
    /// `LM_USEIMAGE_FORCE` / `LM_USEIMAGE_FORCE_ALL`: Windows-only icon
    /// source overrides. Parsed, unused here.
    pub useimage_force:                 bool,
    pub useimage_force_all:             bool,
    /// `LM_ALLOW_MULT_INSTANCES` (+ `_FORCE_ALL`): a click may launch another
    /// process even while one from the same button is still running.
    pub allow_mult_instances:           bool,
    pub allow_mult_instances_force_all: bool,
    /// `LM_NO_CONSOLE_WINDOW`: hide the console for launched console programs
    /// (Windows-only; parsed, applied on Windows only).
    pub no_console_window:              bool,
    /// `LM_OPEN_OTHER`: launch via the OS file association (ShellExecute /
    /// `open` / `xdg-open`) instead of running the program directly.
    pub open_other:                     bool,
}

/// Where a button's image comes from. Windows-only in LM; retained for
/// format fidelity.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ImageMode {
    /// `LM_DEFAULT`: the icon embedded in the launched program.
    #[default]
    Default,
    /// `LM_USEIMAGE_LIST`: the shared bitmap strip.
    UseImageList,
}

/// A keyboard shortcut: main key plus modifiers.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Shortcut {
    pub key:   KeySpec,
    pub ctrl:  bool,
    pub alt:   bool,
    pub shift: bool,
}

/// The main key of a shortcut.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum KeySpec {
    /// A printable character, e.g. `'o'`.
    Char(char),
    /// `VK_F1`…`VK_F24`.
    Function(u8),
    /// Navigation / numpad / misc named keys.
    Named(NamedKey),
}

/// Named non-character keys we map.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NamedKey {
    Pause,
    NumpadMultiply,
    NumpadAdd,
    NumpadSubtract,
    NumpadDecimal,
    NumpadDivide,
    Left,
    Right,
    Up,
    Down,
    Home,
    End,
    Insert,
    Delete,
    PageUp,
    PageDown,
    Space,
    Tab,
    Return,
    Escape,
    Backspace,
}

/// A button that launches an external program.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExternalButton {
    /// Program plus arguments, in order (`"prog" "arg"` → `["prog", "arg"]`).
    pub command:    Vec<String>,
    /// Icon index into the program's icon resources (Windows-only).
    pub icon_index: u32,
    /// Tooltip; `\n` escapes already converted to newlines.
    pub tooltip:    String,
    pub image:      ImageMode,
    pub shortcut:   Option<Shortcut>,
    /// Working directory line, verbatim (quotes stripped).
    pub workdir:    Option<String>,
}

/// A button that invokes an internal editor command by `LM_…` name.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InternalButton {
    /// The `LM_…` keyword, e.g. `LM_VIEW_OVERWORLD`.
    pub name:       String,
    pub icon_index: u32,
    pub tooltip:    String,
    pub image:      ImageMode,
    pub shortcut:   Option<Shortcut>,
}

impl InternalButton {
    /// First tooltip line (or the command name) — the button label the UI shows.
    pub fn label(&self) -> &str {
        first_tooltip_line(&self.tooltip).unwrap_or(self.name.as_str())
    }
}

impl ExternalButton {
    /// First tooltip line (or the program name) — the button label the UI shows.
    pub fn label(&self) -> &str {
        first_tooltip_line(&self.tooltip).unwrap_or_else(|| self.command.first().map(String::as_str).unwrap_or("?"))
    }
}

fn first_tooltip_line(tooltip: &str) -> Option<&str> {
    let line = tooltip.lines().next()?.trim();
    if line.is_empty() {
        None
    } else {
        Some(line)
    }
}

/// One entry of the second toolbar.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ToolbarButton {
    Spacer,
    External(ExternalButton),
    Internal(InternalButton),
}

impl ToolbarButton {
    pub fn shortcut(&self) -> Option<&Shortcut> {
        match self {
            ToolbarButton::Spacer => None,
            ToolbarButton::External(b) => b.shortcut.as_ref(),
            ToolbarButton::Internal(b) => b.shortcut.as_ref(),
        }
    }

    pub fn label(&self) -> &str {
        match self {
            ToolbarButton::Spacer => "",
            ToolbarButton::External(b) => b.label(),
            ToolbarButton::Internal(b) => b.label(),
        }
    }

    pub fn tooltip(&self) -> &str {
        match self {
            ToolbarButton::Spacer => "",
            ToolbarButton::External(b) => b.tooltip.as_str(),
            ToolbarButton::Internal(b) => b.tooltip.as_str(),
        }
    }
}

/// A non-fatal parse problem. Parsing never fails outright: LM shows up to
/// `LM_DISPLAY_ERRORS` of these and keeps the buttons that parsed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParseError {
    pub line:    usize,
    pub message: String,
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "usertoolbar.txt:{}: {}", self.line, self.message)
    }
}

/// Result of parsing a `usertoolbar.txt` file.
#[derive(Clone, Debug, Default)]
pub struct UserToolbarConfig {
    pub options: GlobalOptions,
    pub buttons: Vec<ToolbarButton>,
    /// Non-fatal problems; the UI shows at most `options.display_errors`.
    pub errors:  Vec<ParseError>,
}

/// Split a comma-separated token line, keeping `"quoted, parts"` together.
fn split_tokens(line: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut cur = String::new();
    let mut in_quotes = false;
    for ch in line.chars() {
        match ch {
            '"' => {
                in_quotes = !in_quotes;
                cur.push(ch);
            }
            ',' if !in_quotes => {
                tokens.push(cur.trim().to_string());
                cur.clear();
            }
            _ => cur.push(ch),
        }
    }
    tokens.push(cur.trim().to_string());
    tokens.into_iter().filter(|t| !t.is_empty()).collect()
}

/// Split a quoted command line into program + args:
/// `"notepad.exe" "a b.txt" x` → `["notepad.exe", "a b.txt", "x"]`.
/// Unquoted words split on whitespace; quotes are stripped.
pub fn split_command_line(line: &str) -> Vec<String> {
    let mut parts = Vec::new();
    let mut cur = String::new();
    let mut in_quotes = false;
    let mut has_content = false;
    for ch in line.chars() {
        match ch {
            '"' => {
                in_quotes = !in_quotes;
                has_content = true;
            }
            c if c.is_whitespace() && !in_quotes => {
                if has_content {
                    parts.push(std::mem::take(&mut cur));
                    has_content = false;
                }
            }
            c => {
                cur.push(c);
                has_content = true;
            }
        }
    }
    if has_content {
        parts.push(cur);
    }
    parts
}

/// Strip one pair of surrounding double quotes.
fn unquote(s: &str) -> &str {
    let t = s.trim();
    if t.len() >= 2 && t.starts_with('"') && t.ends_with('"') {
        &t[1..t.len() - 1]
    } else {
        t
    }
}

/// Parse `<icon-index>,<tooltip>`; `\n` escapes become real newlines.
fn parse_tooltip_line(line: &str) -> (u32, String) {
    let (index, tooltip) = match line.split_once(',') {
        Some((num, rest)) => match num.trim().parse::<u32>() {
            Ok(n) => (n, rest),
            Err(_) => (0, line),
        },
        None => (0, line),
    };
    (index, tooltip.replace("\\n", "\n"))
}

fn parse_image_mode(line: &str, errors: &mut Vec<ParseError>, lineno: usize) -> ImageMode {
    match line.trim().to_ascii_uppercase().as_str() {
        "LM_DEFAULT" => ImageMode::Default,
        "LM_USEIMAGE_LIST" => ImageMode::UseImageList,
        other => {
            errors.push(ParseError {
                line:    lineno,
                message: format!("unknown image mode `{other}`, using LM_DEFAULT"),
            });
            ImageMode::Default
        }
    }
}

/// Map a Windows virtual-key name or hex value to a [`KeySpec`].
fn parse_key_spec(token: &str) -> Option<KeySpec> {
    let t = token.trim();
    // Quoted character: 'o'
    if t.len() == 3 && t.starts_with('\'') && t.ends_with('\'') {
        return Some(KeySpec::Char(t.chars().nth(1)?));
    }
    let up = t.to_ascii_uppercase();
    // Raw hex value, e.g. 0x4F — the ASCII letters/digits land on their VK codes.
    if let Some(hex) = up.strip_prefix("0X") {
        if let Ok(code) = u32::from_str_radix(hex, 16) {
            return vk_code_to_key(code);
        }
        return None;
    }
    if let Some(name) = up.strip_prefix("VK_") {
        // Letters and digits.
        if name.len() == 1 {
            let ch = name.chars().next()?;
            if ch.is_ascii_alphanumeric() {
                return Some(KeySpec::Char(ch));
            }
        }
        // Function keys.
        if let Some(n) = name.strip_prefix('F') {
            if let Ok(f) = n.parse::<u8>() {
                if (1..=24).contains(&f) {
                    return Some(KeySpec::Function(f));
                }
            }
        }
        let named = match name {
            "PAUSE" => NamedKey::Pause,
            "MULTIPLY" => NamedKey::NumpadMultiply,
            "ADD" => NamedKey::NumpadAdd,
            "SUBTRACT" => NamedKey::NumpadSubtract,
            "DECIMAL" => NamedKey::NumpadDecimal,
            "DIVIDE" => NamedKey::NumpadDivide,
            "LEFT" => NamedKey::Left,
            "RIGHT" => NamedKey::Right,
            "UP" => NamedKey::Up,
            "DOWN" => NamedKey::Down,
            "HOME" => NamedKey::Home,
            "END" => NamedKey::End,
            "INSERT" => NamedKey::Insert,
            "DELETE" => NamedKey::Delete,
            "PRIOR" => NamedKey::PageUp,
            "NEXT" => NamedKey::PageDown,
            "SPACE" => NamedKey::Space,
            "TAB" => NamedKey::Tab,
            "RETURN" => NamedKey::Return,
            "ESCAPE" => NamedKey::Escape,
            "BACK" => NamedKey::Backspace,
            _ => return None,
        };
        return Some(KeySpec::Named(named));
    }
    None
}

/// Map a raw VK code to a key where it has a stable meaning
/// (letters, digits, F-keys).
fn vk_code_to_key(code: u32) -> Option<KeySpec> {
    match code {
        0x30..=0x39 => Some(KeySpec::Char((b'0' + (code - 0x30) as u8) as char)),
        0x41..=0x5A => Some(KeySpec::Char((b'A' + (code - 0x41) as u8) as char)),
        0x70..=0x87 => Some(KeySpec::Function((code - 0x70 + 1) as u8)),
        0x13 => Some(KeySpec::Named(NamedKey::Pause)),
        0x6A => Some(KeySpec::Named(NamedKey::NumpadMultiply)),
        0x6B => Some(KeySpec::Named(NamedKey::NumpadAdd)),
        0x6D => Some(KeySpec::Named(NamedKey::NumpadSubtract)),
        0x6E => Some(KeySpec::Named(NamedKey::NumpadDecimal)),
        0x6F => Some(KeySpec::Named(NamedKey::NumpadDivide)),
        0x25 => Some(KeySpec::Named(NamedKey::Left)),
        0x26 => Some(KeySpec::Named(NamedKey::Up)),
        0x27 => Some(KeySpec::Named(NamedKey::Right)),
        0x28 => Some(KeySpec::Named(NamedKey::Down)),
        0x24 => Some(KeySpec::Named(NamedKey::Home)),
        0x23 => Some(KeySpec::Named(NamedKey::End)),
        0x2D => Some(KeySpec::Named(NamedKey::Insert)),
        0x2E => Some(KeySpec::Named(NamedKey::Delete)),
        0x21 => Some(KeySpec::Named(NamedKey::PageUp)),
        0x22 => Some(KeySpec::Named(NamedKey::PageDown)),
        0x20 => Some(KeySpec::Named(NamedKey::Space)),
        0x09 => Some(KeySpec::Named(NamedKey::Tab)),
        0x0D => Some(KeySpec::Named(NamedKey::Return)),
        0x1B => Some(KeySpec::Named(NamedKey::Escape)),
        0x08 => Some(KeySpec::Named(NamedKey::Backspace)),
        _ => None,
    }
}

/// Parse a shortcut line like `'o',VK_CONTROL,VK_SHIFT`.
fn parse_shortcut(line: &str, errors: &mut Vec<ParseError>, lineno: usize) -> Option<Shortcut> {
    let tokens = split_tokens(line);
    if tokens.is_empty() {
        return None;
    }
    let mut ctrl = false;
    let mut alt = false;
    let mut shift = false;
    let mut key: Option<KeySpec> = None;
    for token in &tokens {
        match token.to_ascii_uppercase().as_str() {
            "VK_CONTROL" | "VK_LCONTROL" | "VK_RCONTROL" => ctrl = true,
            "VK_MENU" | "VK_LMENU" | "VK_RMENU" => alt = true,
            "VK_SHIFT" | "VK_LSHIFT" | "VK_RSHIFT" => shift = true,
            _ => {
                if key.is_none() {
                    key = parse_key_spec(token);
                }
            }
        }
    }
    match key {
        Some(key) => Some(Shortcut { key, ctrl, alt, shift }),
        None => {
            errors.push(ParseError { line: lineno, message: format!("unrecognized shortcut `{line}`, ignored") });
            None
        }
    }
}

/// Apply one global option token; returns true if it was a known option.
fn apply_global_option(token: &str, options: &mut GlobalOptions, errors: &mut Vec<ParseError>, lineno: usize) {
    let mut words = token.split_whitespace();
    let name = words.next().unwrap_or("").to_ascii_uppercase();
    let arg = words.next();
    let known = match name.as_str() {
        "LM_DISPLAY_ERRORS" => {
            match arg.and_then(|a| a.parse::<u32>().ok()) {
                Some(n) => options.display_errors = n,
                None => errors.push(ParseError {
                    line:    lineno,
                    message: format!("LM_DISPLAY_ERRORS needs a number, got `{token}`"),
                }),
            }
            true
        }
        "LM_NO_TOOLBAR" => {
            options.no_toolbar = true;
            true
        }
        "LM_SETIMAGE_SIZE" => {
            match arg.and_then(|a| a.parse::<u32>().ok()) {
                Some(n) => options.setimage_size = Some(n),
                None => errors.push(ParseError {
                    line:    lineno,
                    message: format!("LM_SETIMAGE_SIZE needs a number, got `{token}`"),
                }),
            }
            true
        }
        "LM_USEIMAGE_FORCE" => {
            options.useimage_force = true;
            true
        }
        "LM_USEIMAGE_FORCE_ALL" => {
            options.useimage_force_all = true;
            true
        }
        "LM_ALLOW_MULT_INSTANCES" => {
            options.allow_mult_instances = true;
            true
        }
        "LM_ALLOW_MULT_INSTANCES_FORCE_ALL" => {
            options.allow_mult_instances_force_all = true;
            true
        }
        "LM_NO_CONSOLE_WINDOW" => {
            options.no_console_window = true;
            true
        }
        "LM_OPEN_OTHER" => {
            options.open_other = true;
            true
        }
        // LM_USEIMAGE_LIST is a per-button image mode, not a global option.
        _ => false,
    };
    if !known {
        errors.push(ParseError { line: lineno, message: format!("unknown global option `{token}`") });
    }
}

/// Parse the full text of a `usertoolbar.txt` file.
pub fn parse(text: &str) -> UserToolbarConfig {
    let mut cfg = UserToolbarConfig::default();
    cfg.options.display_errors = 10;

    // Non-blank lines with 1-based physical line numbers.
    let text = text.strip_prefix('\u{FEFF}').unwrap_or(text);
    let lines: Vec<(usize, &str)> =
        text.lines().enumerate().map(|(i, l)| (i + 1, l.trim())).filter(|(_, l)| !l.is_empty()).collect();

    // Partially built button: (command token, is_internal, icon, tooltip, image, shortcut, workdir).
    struct Partial {
        command:    String,
        internal:   bool,
        icon_index: u32,
        tooltip:    String,
        image:      ImageMode,
        shortcut:   Option<Shortcut>,
        workdir:    Option<String>,
    }

    let mut buttons: Vec<ToolbarButton> = Vec::new();
    let mut errors: Vec<ParseError> = Vec::new();

    // The button currently being built, if any.
    let mut partial: Option<Partial> = None;
    // Finish the open definition, if any (explicit ***END***, a new
    // ***START***, or EOF — all three close it, per LM).
    let close_partial = |partial: &mut Option<Partial>, buttons: &mut Vec<ToolbarButton>| {
        if let Some(p) = partial.take() {
            let button = if p.internal {
                ToolbarButton::Internal(InternalButton {
                    name:       p.command,
                    icon_index: p.icon_index,
                    tooltip:    p.tooltip,
                    image:      p.image,
                    shortcut:   p.shortcut,
                })
            } else {
                ToolbarButton::External(ExternalButton {
                    command:    split_command_line(&p.command),
                    icon_index: p.icon_index,
                    tooltip:    p.tooltip,
                    image:      p.image,
                    shortcut:   p.shortcut,
                    workdir:    p.workdir,
                })
            };
            buttons.push(button);
        }
    };

    #[derive(Clone, Copy, PartialEq, Eq)]
    enum Ctx {
        Global,
        /// Waiting for the command token.
        WantCommand,
        /// Reading positional fields; counts remaining
        /// (tooltip, image, shortcut, workdir).
        WantFields(u8),
    }
    let mut ctx = Ctx::Global;

    // Start a button definition from a command token.
    let start_command = |cmd: &str,
                         lineno: usize,
                         partial: &mut Option<Partial>,
                         buttons: &mut Vec<ToolbarButton>,
                         errors: &mut Vec<ParseError>|
     -> Ctx {
        if cmd == "LM_SPACER" {
            buttons.push(ToolbarButton::Spacer);
            Ctx::Global
        } else if cmd.starts_with("LM_") {
            *partial = Some(Partial {
                command:    cmd.to_string(),
                internal:   true,
                icon_index: 0,
                tooltip:    String::new(),
                image:      ImageMode::Default,
                shortcut:   None,
                workdir:    None,
            });
            Ctx::WantFields(4)
        } else if cmd.starts_with('"') {
            *partial = Some(Partial {
                command:    cmd.to_string(),
                internal:   false,
                icon_index: 0,
                tooltip:    String::new(),
                image:      ImageMode::Default,
                shortcut:   None,
                workdir:    None,
            });
            Ctx::WantFields(4)
        } else {
            errors.push(ParseError {
                line:    lineno,
                message: format!(
                    "expected LM_SPACER, an LM_ command, or a quoted command after ***START***, got `{cmd}`"
                ),
            });
            Ctx::Global
        }
    };

    for (lineno, line) in &lines {
        let lineno = *lineno;
        // Bare structural lines terminate any open definition first.
        if *line == "***START***" || *line == "***END***" {
            close_partial(&mut partial, &mut buttons);
            ctx = if *line == "***START***" { Ctx::WantCommand } else { Ctx::Global };
            continue;
        }
        match ctx {
            Ctx::Global => {
                // Comma-separated: global options, or inline button starts.
                for token in split_tokens(line) {
                    match token.as_str() {
                        "***START***" => {
                            close_partial(&mut partial, &mut buttons);
                            ctx = Ctx::WantCommand;
                        }
                        "***END***" => {
                            close_partial(&mut partial, &mut buttons);
                            ctx = Ctx::Global;
                        }
                        _ => {
                            if ctx == Ctx::Global {
                                apply_global_option(&token, &mut cfg.options, &mut errors, lineno);
                            } else {
                                // Inline command token after ***START*** on the same line.
                                ctx = start_command(&token, lineno, &mut partial, &mut buttons, &mut errors);
                            }
                        }
                    }
                }
                // If the line ended while waiting for a command, the next
                // non-blank line is the command field.
            }
            Ctx::WantCommand => {
                // The whole line is the command field (LM's fields are
                // line-oriented; only the ***START*** line itself may carry
                // the command inline, handled above).
                ctx = start_command(line, lineno, &mut partial, &mut buttons, &mut errors);
            }
            Ctx::WantFields(left) => {
                if let Some(p) = partial.as_mut() {
                    match 4 - left {
                        0 => {
                            let (icon, tooltip) = parse_tooltip_line(line);
                            p.icon_index = icon;
                            p.tooltip = tooltip;
                        }
                        1 => p.image = parse_image_mode(line, &mut errors, lineno),
                        2 => p.shortcut = parse_shortcut(line, &mut errors, lineno),
                        _ => {
                            let dir = unquote(line);
                            p.workdir = if dir.is_empty() { None } else { Some(dir.to_string()) };
                        }
                    }
                    if left == 1 {
                        close_partial(&mut partial, &mut buttons);
                        ctx = Ctx::Global;
                    } else {
                        ctx = Ctx::WantFields(left - 1);
                    }
                } else {
                    ctx = Ctx::Global;
                }
            }
        }
    }
    // EOF implicitly closes an open definition.
    close_partial(&mut partial, &mut buttons);

    cfg.buttons = buttons;
    cfg.errors = errors;
    cfg
}

/// Default `usertoolbar.txt` search paths, in order:
/// 1. beside the executable (LM's own convention),
/// 2. the per-user config dir (`%APPDATA%\smwe`, `~/Library/Application
///    Support/smwe`, `$XDG_CONFIG_HOME/smwe` or `~/.config/smwe`).
pub fn default_search_paths() -> Vec<std::path::PathBuf> {
    let mut paths = Vec::new();
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            paths.push(dir.join("usertoolbar.txt"));
        }
    }
    #[cfg(target_os = "windows")]
    {
        let base =
            std::env::var_os("APPDATA").map(std::path::PathBuf::from).unwrap_or_else(|| std::path::PathBuf::from("."));
        paths.push(base.join("smwe").join("usertoolbar.txt"));
    }
    #[cfg(target_os = "macos")]
    {
        if let Some(home) = std::env::var_os("HOME") {
            paths.push(
                std::path::PathBuf::from(home)
                    .join("Library")
                    .join("Application Support")
                    .join("smwe")
                    .join("usertoolbar.txt"),
            );
        }
    }
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    {
        let base = std::env::var_os("XDG_CONFIG_HOME")
            .map(std::path::PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|h| std::path::PathBuf::from(h).join(".config")))
            .unwrap_or_else(|| std::path::PathBuf::from("."));
        paths.push(base.join("smwe").join("usertoolbar.txt"));
    }
    paths
}

#[cfg(test)]
mod tests {
    use super::*;

    /// LM 3.63's own example: global options, an inline spacer, and an
    /// internal command whose definition is implicitly terminated.
    const VISIBLE: &str = "LM_DISPLAY_ERRORS 10, ***START***, LM_SPACER, ***START***, LM_VIEW_OVERWORLD\n\
         \n\
         0,Open overworld from oracle toolbar\n\
         \n\
         LM_DEFAULT\n\
         \n\
         'o',VK_CONTROL,VK_SHIFT\n\
         \n\
         ***END***\n";

    /// LM 3.63's own example: LM_NO_TOOLBAR, image size, a spacer, an
    /// internal command, and an external command with quoted argument,
    /// multi-line tooltip, image list, modifiers, and a working directory.
    const FULL: &str = "LM_DISPLAY_ERRORS 10, LM_NO_TOOLBAR, LM_SETIMAGE_SIZE 24, ***START***, LM_SPACER, ***START***, LM_VIEW_OVERWORLD\n\
         \n\
         0,Open overworld from oracle toolbar\n\
         \n\
         LM_DEFAULT\n\
         \n\
         'o',VK_CONTROL,VK_SHIFT\n\
         \n\
         ***START***\n\
         \n\
         \"notepad.exe\" \"oracle argument.txt\"\n\
         \n\
         0,External oracle button\\nSecond tooltip line\n\
         \n\
         LM_USEIMAGE_LIST\n\
         \n\
         'e',VK_CONTROL\n\
         \n\
         \"%4\"\n\
         \n\
         ***END***\n";

    #[test]
    fn parses_lm_visible_fixture() {
        let cfg = parse(VISIBLE);
        assert_eq!(cfg.options.display_errors, 10);
        assert!(!cfg.options.no_toolbar);
        assert!(cfg.errors.is_empty(), "unexpected errors: {:?}", cfg.errors);
        assert_eq!(cfg.buttons.len(), 2);
        assert_eq!(cfg.buttons[0], ToolbarButton::Spacer);
        match &cfg.buttons[1] {
            ToolbarButton::Internal(b) => {
                assert_eq!(b.name, "LM_VIEW_OVERWORLD");
                assert_eq!(b.icon_index, 0);
                assert_eq!(b.tooltip, "Open overworld from oracle toolbar");
                assert_eq!(b.image, ImageMode::Default);
                let s = b.shortcut.as_ref().expect("shortcut");
                assert_eq!(s.key, KeySpec::Char('o'));
                assert!(s.ctrl && s.shift && !s.alt);
                assert_eq!(b.label(), "Open overworld from oracle toolbar");
            }
            other => panic!("expected internal button, got {other:?}"),
        }
    }

    #[test]
    fn parses_lm_full_fixture() {
        let cfg = parse(FULL);
        assert_eq!(cfg.options.display_errors, 10);
        assert!(cfg.options.no_toolbar, "LM_NO_TOOLBAR must hide the bar");
        assert_eq!(cfg.options.setimage_size, Some(24));
        assert!(cfg.errors.is_empty(), "unexpected errors: {:?}", cfg.errors);
        assert_eq!(cfg.buttons.len(), 3);
        assert_eq!(cfg.buttons[0], ToolbarButton::Spacer);
        // The internal command was implicitly terminated by the next ***START***.
        match &cfg.buttons[1] {
            ToolbarButton::Internal(b) => assert_eq!(b.name, "LM_VIEW_OVERWORLD"),
            other => panic!("expected internal button, got {other:?}"),
        }
        match &cfg.buttons[2] {
            ToolbarButton::External(b) => {
                assert_eq!(b.command, vec!["notepad.exe".to_string(), "oracle argument.txt".to_string()]);
                assert_eq!(b.tooltip, "External oracle button\nSecond tooltip line");
                assert_eq!(b.label(), "External oracle button");
                assert_eq!(b.image, ImageMode::UseImageList);
                let s = b.shortcut.as_ref().expect("shortcut");
                assert_eq!(s.key, KeySpec::Char('e'));
                assert!(s.ctrl && !s.shift && !s.alt);
                assert_eq!(b.workdir.as_deref(), Some("%4"));
            }
            other => panic!("expected external button, got {other:?}"),
        }
    }

    #[test]
    fn command_line_splitting() {
        assert_eq!(split_command_line("\"a.exe\" \"b c\" d"), vec!["a.exe", "b c", "d"]);
        assert_eq!(split_command_line("plain.exe --flag"), vec!["plain.exe", "--flag"]);
        assert!(split_command_line("").is_empty());
    }

    #[test]
    fn shortcut_forms() {
        let s = parse_shortcut("'o',VK_CONTROL,VK_SHIFT", &mut vec![], 1).unwrap();
        assert_eq!((s.key, s.ctrl, s.alt, s.shift), (KeySpec::Char('o'), true, false, true));
        let s = parse_shortcut("VK_F5,VK_MENU", &mut vec![], 1).unwrap();
        assert_eq!((s.key, s.alt), (KeySpec::Function(5), true));
        // Raw hex value, like LM accepts.
        let s = parse_shortcut("0x4F,VK_CONTROL", &mut vec![], 1).unwrap();
        assert_eq!(s.key, KeySpec::Char('O'));
        // Pause + numpad operators survive (regression: they must not be
        // rejected or collapsed into punctuation).
        let s = parse_shortcut("VK_PAUSE", &mut vec![], 1).unwrap();
        assert_eq!(s.key, KeySpec::Named(NamedKey::Pause));
        let s = parse_shortcut("VK_MULTIPLY,VK_CONTROL", &mut vec![], 1).unwrap();
        assert_eq!(s.key, KeySpec::Named(NamedKey::NumpadMultiply));
        assert!(parse_shortcut("VK_BOGUS", &mut vec![], 1).is_none());
    }

    #[test]
    fn unknown_options_are_reported_not_fatal() {
        let cfg = parse("LM_DISPLAY_ERRORS 5, LM_FROBNICATE,\n***START***\nLM_SPACER\n***END***\n");
        assert_eq!(cfg.options.display_errors, 5);
        assert_eq!(cfg.errors.len(), 1);
        assert!(cfg.errors[0].message.contains("LM_FROBNICATE"));
        assert_eq!(cfg.buttons, vec![ToolbarButton::Spacer]);
    }

    #[test]
    fn eof_closes_open_definition() {
        let cfg = parse("***START***\nLM_VIEW_OVERWORLD\n0,Tip\n");
        assert_eq!(cfg.buttons.len(), 1);
        match &cfg.buttons[0] {
            ToolbarButton::Internal(b) => {
                assert_eq!(b.tooltip, "Tip");
                assert!(b.shortcut.is_none());
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn tooltip_line_parsing() {
        assert_eq!(parse_tooltip_line("3,Hello\\nWorld"), (3, "Hello\nWorld".to_string()));
        assert_eq!(parse_tooltip_line("NoCommaHere"), (0, "NoCommaHere".to_string()));
    }

    #[test]
    fn bom_is_stripped() {
        let cfg = parse("\u{FEFF}LM_NO_TOOLBAR\n");
        assert!(cfg.options.no_toolbar);
        assert!(cfg.errors.is_empty());
    }
}
