# Layer 2 event tiles (`$04DD8D`)

**Status:** implemented — PR `feature/l2-event-tiles` (2026-09-11).
Beyond Lunar Magic: LM's event editing covers the Layer 1 reveal-tile swaps;
the Layer 2 side of overworld events was identified but unparsed (row 50 of
`docs/LUNAR_MAGIC_PARITY.md`).

## What the game does

When a "destruction" event fires (castle/fortress/switch palace beaten), the
game runs an animated event sequence (`OverworldEventProcess` states in
SMWDisX `bank_04.asm`). Besides the Layer 1 reveal-tile swap (`CODE_04DA49`,
already covered by `OverworldEvents`), the sequence also writes **Layer 2**
tiles — this is how e.g. a path appears or terrain changes on the background
layer. Two ROM data sources feed it:

### 1. The Layer 2 event entry table — SNES `$04DD8D`

**371 entries × 4 bytes**: `data_word` (u16 LE) then `dest_word` (u16 LE).
Table extent `$04DD8D..$04E359` (exclusive) = 1484 bytes; the last per-event
boundary is exactly 371, confirming the size.

Which entries an event touches comes from the **cumulative boundary table**
at SNES `$04E359`: **121 u16 words**. Event `e` (0..119) owns entries
`boundaries[e]..boundaries[e+1]`. (The disassembly labels the first word
`DATA_04E359` and the rest `DATA_04E35B`, but the code reads them as one
cumulative array: `LDA.L DATA_04E359,X` → start, `LDA.L DATA_04E35B,X` → end,
with X = event×2 — so word `e` is simultaneously event `e-1`'s end and event
`e`'s start. The values are monotonically non-decreasing, ending at 371.)
Driven by `CODE_04E453` (event-number path, gated on `OWEventsActivated`
bits), `CODE_04E6D3` (animation setup), and `CODE_04E6F9` (per-entry
animation frame).

Per entry (`CODE_04E4A9` / `CODE_04EE30`):

- `data_word < $0900` → **tile stream**: that many 8×8 tiles are streamed to
  VRAM through the dynamic stripe image (`CODE_04E824`).
- `data_word >= $0900` → **tilemap copy**: bytes are copied from the WRAM
  buffer at `$7F8000 + data_word` onto `OWLayer2Tilemap` (`CODE_04E4D0` /
  `CODE_04E76C`), starting at the tilemap offset in `dest_word`.

The on-screen target position is decoded from `dest_word` exactly like
`CODE_04E6F9` does:

```
x_px = (dest_word & $3E) << 2
y_px = (((dest_word >> 3) as u8) & $F8)
```

i.e. an 8×8-tile coordinate on the 64×32 overworld map.

### 2. The "silent event" tables — SNES `$04E8E4` / `$04E910` / `$04E93C` / `$04E994`

44 explicit rows, driven by `CODE_04E9EC`:

- `$04E8E4`: 44 u8 event numbers to match against the triggered event.
- `$04E910`: 44 u8 flags; **bit 0 set = Layer 2 event** (runs the entry
  through `CODE_04E4A9`), clear = direct Map16 tile edit (not L2).
- `$04E93C`: 44 u16 `dest_word`s.
- `$04E994`: 44 u16 `data_word`s (same meaning as the entry table's).

25 of the 44 rows are Layer 2 events.

## Implementation

`crates/smwe-rom/src/overworld/mod.rs`:

- `L2EventEntry { data_word, dest_word }` with `kind()` → `L2EventKind::{TileStream(u16), TilemapCopy(u16)}` and `target_tile() -> (u8, u8)` (the `CODE_04E6F9` decode above).
- `SilentEvent { event_no, is_l2, data_word, dest_word }` with `as_entry()`.
- `OverworldL2Events { entries, boundaries, silent_events }`: `parse()` reads
  all four table groups from the ROM and validates the boundaries (cumulative,
  last ≤ entry count); `entries_for_event(e) -> Option<Range<usize>>`;
  `silent_l2_events_for(event_no) -> Vec<&SilentEvent>`.
- Wired into `SmwRom` as `overworld_l2_events` (parse failure → empty, logged).

`src/ui/world_editor/mod.rs`:

- New "Layer 2 events" section in the events panel: summary counts, per-event
  entry ranges with kind + target tile per entry, and silent rows.
- On-map target markers (toggleable): cyan ring = tile stream, orange ring =
  tilemap copy, drawn at each active event's entry targets. The markers follow
  the existing event checkboxes; the animated L2 sequence itself only runs
  in-game, so the panel/markers are the preview.

## Verification

- Unit tests: kind classification on the `$0900` threshold, `target_tile()`
  brute-forced against the SNES decode formula, cumulative-boundary ranges,
  silent-event filtering.
- Real-ROM ignored test `l2_event_tables_match_disassembly`
  (`ROM_PATH=~/workspace/smw-editor/smw.smc cargo test -p smwe-rom --lib -- --ignored`):
  371 entries, first entries `(0x0900, 0x23CC)` / `(0x0904, 0x238C)`,
  boundaries `[0, 0, 0x0D, 0x0D, 0x10, 0x15, …]`, last boundary = 371,
  44 silent rows — all matching the disassembly byte-for-byte.
- All 13 of event 1's entries hand-checked against the ASM `db` bytes.
- Screenshots: `docs/screenshots/l2-events-map.png` (real emulated main-map
  render + markers for event 1, via `render_ow_submap --l2-markers=1`) and
  `docs/screenshots/l2-events-panel.png` (headless mock of the panel section
  with real parsed values, via the new `render_l2_event_panel` bin).

## Known limitations

- Read-only: the tables are parsed and displayed, not edited. Repointing or
  extending them is a separate piece of work (entry edits would need the
  boundary table rebuilt).
- The editor does not run the animated `OverworldEventProcess` sequence, so
  the L2 changes can't be previewed applied to the map — only their targets
  are shown.
