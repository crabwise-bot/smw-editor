# Custom User Toolbar (Lunar Magic v2.31+ parity)

Lunar Magic lets users define a second toolbar via a `usertoolbar.txt` file
placed beside `Lunar Magic.exe`. smw-editor reads the same file format from
either of these locations (first one found wins):

1. beside the smw-editor executable (LM's own convention), or
2. the per-user config dir: `%APPDATA%\smwe\usertoolbar.txt` on Windows,
   `~/Library/Application Support/smwe/usertoolbar.txt` on macOS,
   `$XDG_CONFIG_HOME/smwe/usertoolbar.txt` (else `~/.config/smwe/`) on Linux.

With no file present, there is no second toolbar — exactly like LM.

## File format

The format is LM's (see LM 3.63 help → *Technical Information → Custom User
Toolbar*). Global options come first as comma-separated tokens; each button is
a `***START***` … `***END***` definition (a new `***START***` also closes the
previous one):

```text
LM_DISPLAY_ERRORS 10,
***START***, LM_SPACER,
***START***, LM_VIEW_OVERWORLD

0,Open the world map editor

LM_DEFAULT

'o',VK_CONTROL,VK_SHIFT

***START***

"asar.exe" "{rom}"

0,Assemble patches with Asar

LM_DEFAULT

VK_F9

***END***
```

Global options: `LM_DISPLAY_ERRORS <n>` (how many parse/launch errors are
shown, default 10), `LM_NO_TOOLBAR` (hide the bar but keep shortcuts working),
`LM_SETIMAGE_SIZE <n>`, `LM_USEIMAGE_FORCE`, `LM_USEIMAGE_FORCE_ALL`,
`LM_ALLOW_MULT_INSTANCES`, `LM_ALLOW_MULT_INSTANCES_FORCE_ALL`,
`LM_NO_CONSOLE_WINDOW`, `LM_OPEN_OTHER`. The Windows-only icon options are
parsed and ignored elsewhere.

Each button after the command line has four one-per-line fields:

1. `<icon-index>,<tooltip>` — LM picks an icon from the program's resources;
   smw-editor labels the button with the tooltip's first line instead. `\n`
   inside the tooltip makes a second line.
2. `LM_DEFAULT` or `LM_USEIMAGE_LIST` — Windows-only icon source.
3. Shortcut — a quoted character (`'o'`), a Windows virtual-key name
   (`VK_F5`, `VK_PAUSE`, numpad operators, …) or a raw hex value (`0x4F`),
   plus any of `VK_CONTROL`, `VK_SHIFT`, `VK_MENU` (= Alt). Shortcuts work
   even with `LM_NO_TOOLBAR`, but not while you're typing in a text field.
4. Working directory (quoted) — relative paths resolve against the
   executable's directory, like LM.

Button kinds:

- `LM_SPACER` — a separator, no further fields.
- `LM_…` — an internal command. smw-editor maps `LM_VIEW_OVERWORLD` (opens
  the World Map Editor), `LM_FILE_DELETE_LEVEL` (opens Delete Levels), and
  `LM_FILE_EXPORT_DIRECTORY_BITMAP` (opens batch level-image export; LM
  writes BMP, smw-editor exports PNG per its LM v2.30/v3.20 image-export
  parity). Any other `LM_…` name is accepted but renders as a disabled
  button — LM knows 317 of them and most have no editor action here.
- `"program" "arg …"` — an external command ("scripting button"). Clicking
  launches the program; while its process is still running, clicks are a
  no-op (LM would focus the existing window instead) unless
  `LM_ALLOW_MULT_INSTANCES` is set.

## ROM path for external tools

LM hands external tools the ROM path through a Windows-only message
protocol. smw-editor instead substitutes `{rom}` in the command's arguments
(and working directory) with the open ROM's path, and sets the `SMW_ROM`
environment variable for the child process. Example: `"asar.exe" "{rom}"`.
