# Message Box WYSIWYG Preview — Implementation Plan (Phase 1: read-only)

**Status: BLOCKED — no SMW ROM on this machine.** The app exits at startup
without one (`src/main.rs:41-45`: "No ROM path defined (ROM_PATH not set,
--rom missing, ./smw.smc not found)"), the emulator needs a ROM to execute
`CODE_05B1BC`, and the empirical font-map derivation needs the vanilla
message bytes. Do not implement blind: repo `AGENTS.md` requires reproduction
and image proof for visual work.

Unblock by placing a vanilla SMW ROM at `./smw.smc` or setting `ROM_PATH`,
then work this plan top to bottom. Every step below is verifiable once a ROM
exists.

## Background (already established)

- `crates/smwe-rom/src/message_boxes.rs` parses the 22 vanilla messages
  (byte boundaries derived from `symbols/SMW_U.sym` label addresses;
  `real_rom_message_boxes` ignored-test verifies against a real ROM).
- Message bytes are 0x00–0x7F tile indices; bit 7 is a hold/repeat flag
  (`AND #$7F` strips it in `CODE_05B208`).
- `CODE_05B1BC` (render entry): `LDA.W DATA_05A5A7,X` with X = message-type
  index (0–24, into the 25-entry pointer table) gives the start offset used by
  the `CODE_05B208` render loop. Symbols confirmed in `symbols/SMW_U.sym`.
- Editor UI: `src/ui/editor_prototypes/level_editor/message_editor.rs`
  (`message_editor_window`), state in `UiLevelEditor.message_boxes`
  (`src/ui/editor_prototypes/level_editor/mod.rs:152`).
- Emulator trampoline pattern: `smwe_emu::emu::decompress_sublevel`
  (`crates/smwe-emu/src/emu.rs:528`) — JSL chain at `$2000`, run to end PC.
- VRAM→pixel rasterization exists: `render_tile` in `src/bin/render_level.rs:274`.

## Step 1 — Empirically derive the byte→character font map (needs ROM)

In `crates/smwe-rom/src/message_boxes.rs` (new module section):

1. With `ROM_PATH` set, run the existing `real_rom_message_boxes` ignored
   test to dump the 22 raw byte sequences.
2. Align each byte sequence against its known vanilla English text
   (well-documented; e.g. Intro = "WELCOME TO DINOSAUR LAND!..."). Strip bit 7
   first; identify control codes empirically (line break, end-of-message —
   compare byte positions against known line breaks in the English text).
3. Produce `pub const FONT_MAP: [(u8, char); N]` (byte → character).
4. Add `#[test] font_map_round_trips_all_vanilla_messages`: for each of the
   22 messages, map bytes→chars→bytes and assert byte-exact equality with the
   ROM dump. This is the required `cargo test -p smwe-rom` gate.

Do NOT hardcode the map from memory. The whole point is empirical derivation.

## Step 2 — `render_message` in smwe-emu (needs ROM)

New function in `crates/smwe-emu/src/emu.rs`, following the
`decompress_sublevel` trampoline pattern:

```rust
pub fn render_message(cpu: &mut Cpu<CheckedMem>, msg_type: u8) -> ImageBuffer<Rgb<u8>>
```

- Set X = `msg_type` (0–24), JSL to resolved `CODE_05B1BC`, run to return.
- Capture the Layer 3 stripe tilemap from VRAM (find where the routine leaves
  it — check against `../SMWDisX/bank_05.asm` if available; the repo expects
  it at `../SMWDisX/`, currently absent on this machine).
- Rasterize with the existing `render_tile` logic (move/share it out of
  `src/bin/render_level.rs` if needed — do not duplicate it).
- Open question to resolve with the disassembly: does `CODE_05B1BC` upload
  the font tiles to VRAM itself, or is separate font-upload setup needed?
  Answer from `bank_05.asm`, not from guessing.

## Step 3 — `--message=N` headless render (needs ROM)

Extend `src/bin/render_level.rs` (or add `src/bin/render_message.rs`):

```
cargo run --bin render_message -- --message=0 --out=/tmp/msg0.png
```

Render all 22 messages (`--message=0..21`); sanity-check several PNGs against
known in-game text (screenshots/videos of vanilla SMW). This is the required
image proof before touching the GUI.

## Step 4 — Preview pane in the message editor (needs ROM)

In `message_editor.rs::message_editor_window`, next to the byte grid:

- Read-only preview pane rendering the selected message via `render_message`
  (cache the `ImageBuffer` per message index; re-render on selection change
  or when `message_boxes_dirty`).
- Display via the existing egui texture path (check how the level editor
  shows rendered tiles — follow that pattern, don't invent a new one).
- Keep the "no readable-text preview yet" label until the font map lands;
  replace it with the pixel preview once Step 1–3 are proven.

## Step 5 — Verification gate (all required before PR)

- `cargo test -p smwe-rom` — font-map round-trip test passes (Step 1).
- 22 message PNGs rendered; spot-checked against real in-game text (Step 3).
- `cargo check --lib` (repo minimum) + `cargo test` on touched crates.
- GUI screenshot via `xvfb-run` showing the preview pane in the message
  editor (needs ROM: app won't start without one).

## Explicitly out of scope (Phase 2)

Text→bytes editing (typing readable text). Phase 1 is read-only preview only.
