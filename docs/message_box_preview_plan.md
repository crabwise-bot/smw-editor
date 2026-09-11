# Message Box WYSIWYG Preview — Implementation Plan (Phase 1: read-only, Phase 2: editable text)

**Status 2026-09-10: Phase 1 DONE and merged (PR #5); Phase 2 DONE — this PR.**
Phase 1's "BLOCKED — no SMW ROM" is resolved: Justin provided his own dump
(`smw.smc`, kept out of the repo; `ROM_PATH` for tests).

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

## Phase 2 — Editable text (DONE 2026-09-10)

The message editor now has a multiline text field per message: decoded text
is shown, typing re-encodes to font-tile bytes live.

**Codec** (`crates/smwe-rom/src/font_map.rs`):
- `decode_editable_text`: 8×18 grid → 8 `\n`-joined lines. `\n` is the
  line-break representation; there are no other control codes (the real
  `CODE_05B208` has none — every source byte is consumed as a tile index).
- `encode_editable_text`: inverts the row-fill — trailing spaces per row are
  dropped, the last content byte gets bit 7 (fill rest of row with `$1F`
  blanks); a fully blank row encodes to one `0x9F` byte, matching the vanilla
  pattern of one bit-7 terminator per row.
- `encode_message_checked`: additionally enforces the per-message byte
  budget (the message's vanilla length — the 22-message blob isn't
  repointable, so no message may grow past its original span).
- Non-text graphic tiles (Yoshi's signature `0x60-0x63`, bonus-star icons
  `0x64`/`0x6B` — the only unmapped bytes in the vanilla set) decode as `�`
  (U+FFFD), deliberately distinct from `?` (real glyph, `0x1E`): a typed `?`
  always encodes to `0x1E`, while `�` reuses the original byte at the same
  cell iff that cell held an unmapped graphic. Graphics can be preserved in
  place or deleted via the text field, not moved/inserted (raw byte grid
  remains for that).

**UI** (`src/ui/editor_prototypes/level_editor/message_editor.rs`):
- Monospace multiline `TextEdit` (8 rows) replaces the read-only text
  preview; `used / budget` byte counter with red over-budget/error state;
  encode failures show the message and leave bytes untouched (no silent
  truncation).
- Text buffer re-syncs on message selection change and on raw-byte-grid
  edits (byte-hash comparison).
- Raster preview is live (cache key already included the byte hash).
- The `CODE_05B1BC` readout now also re-runs per edit: the current bytes are
  patched into a scratch ROM image (message blob + recomputed pointer table
  via `to_blob_and_pointers` — exactly what saving writes) so it shows what
  the game will render for the edited text.
- Raw byte sliders moved into a "Raw bytes (advanced)" collapsible.

**Tests:** 11 new `font_map` unit tests (codec round-trips, 1:1 map check,
rejection paths, placeholder rules, budget enforcement); real-ROM ignored
test `real_rom_editable_round_trip` — all 22 vanilla messages decode →
re-encode byte-exactly.

**Screenshot:** `docs/screenshots/message-edit.png` via the new
`render_message_editor` headless binary (egui can't render headless, so the
window chrome is drawn; text, byte counts, and rasters are real ROM output
from the editor's code paths). Shows Intro before/after a typed edit plus a
real rejection message.

## Addendum 2026-09-10: disassembly findings (SMWDisX now available)

Grepped `bank_05.asm`/`bank_00.asm` per `AGENTS.md` instead of guessing from
the Rust side. This answers the plan's open questions:

- **Does `CODE_05B1BC` upload font tiles to VRAM itself? NO.** It appends 8
  rows × 18 tile words to the WRAM dynamic-stripe-image buffer
  (`DynamicStripeImage` at $7F837D; write offset at `DynStripeImgSize`
  $7F837B, per `rammap.asm`). It never writes VRAM. The font graphics
  (tiles $100-$17F) must already be in VRAM from the game's normal GFX upload.
- **Tile format:** each tile word is `$39TT` (TT = message byte & $7F):
  tiles $100-$17F, palette 6, priority 1, no flip. Bit 7 of the message byte
  is the hold/repeat flag: when set, the previous tile is re-emitted without
  advancing the message pointer (`BIT.W _3` / `BMI` in `CODE_05B208`).
- **Stripe command format** (from `LoadStripeImage`, bank_00.asm):
  `[VRAM-dest word][flags/length word][payload bytes…]`; flags/length bit 15
  = vertical, bit 14 = RLE, low 14 bits = payload length in bytes minus 1.
  A first byte with bit 7 set ($FF) terminates the buffer. Row header words
  (U version, `DATA_05A580`, written Y=$0E first): `$E750 $C750 $0751 $2751
  $4751 $6751 $8751 $A751`.
- **Setup:** X = message-type index (0-24) into `DATA_05A5A7`; `DynStripeImgSize`
  must be 0 (game zeroes it at level init; NMI uploader resets it after every
  upload). `MessageBoxTrigger`/`PlayerRidingYoshi`/`SwitchPalaceColor` may be
  zero (level-start state). Types 0-3 JSR to `CODE_05B2EB` (writes OAM tiles,
  not VRAM — harmless headless). The routine falls through into the
  message-box window/HDMA setup (`CODE_05B250`, WRAM-only) and returns via a
  single RTL (bank_05.asm:3475), so the $2000 JSL trampoline pattern applies.
- **Implemented accordingly:** `smwe_emu::emu::render_message` now zeroes
  `DynStripeImgSize`, runs the trampoline, and returns the appended stripe
  bytes (`MessageStripe { stripe, cycles }`) — not a VRAM snapshot. The
  `render_message` binary dumps those bytes and decodes the 8 commands.
- **Still pending a real ROM:** executing the trampoline at all; confirming
  the 8 VRAM row addresses against the Layer 3 tilemap; rasterizing tiles
  $100-$17F with palette 6 into a PNG (recipe: run `decompress_sublevel`
  first so the font tiles are in VRAM, then `render_message` on the same
  CPU); deriving the true font map; GUI screenshot.
