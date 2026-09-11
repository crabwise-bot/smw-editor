# Custom level names (overworld name table)

**Status:** implemented — PR `feature/custom-level-names` (2026-09-11).
Beyond Lunar Magic: LM has no level-name editing (row 33 of
`docs/LUNAR_MAGIC_PARITY.md`).

## What the game does

When Mario stands on a level tile on the overworld, `CODE_049D07`
(`SMWDisX/bank_04.asm`, U version) builds the name shown in the corner from
three shared string fragments:

- `LevelNames` (SNES `$04A0FC`, **93 entries × 2 bytes**) is indexed by the
  level tile's *translevel* number. Each entry packs three fragment selectors:
  - bits 15–8 (`_1 & $7F`): index into the T1 fragment table
  - bits 7–4 (`_0 >> 4`): index into the T2 fragment table
  - bits 3–0 (`_0 & $F`): index into the T3 fragment table
- `T1`/`T2`/`T3` (SNES `$049C91`/`$049CCF`/`$049CED`, **31/15/13 entries ×
  2 bytes**) hold 16-bit byte-offsets into `LevelNameStrings`. (T3 has only
  13 valid slots even though the entry format allows 16 — indices 13–15 would
  read into `CODE_049D07` itself; vanilla never uses them.)
- `LevelNameStrings` (SNES `$049AC5`, **460 bytes**) holds the fragment text.
  Each fragment is a byte string; the displayed tile is `byte & $7F`, and bit 7
  on a byte marks the last character of the fragment.

Piece selection rules in `CODE_049D07`:

- T1 fragment is skipped if its first byte has bit 7 set (the vanilla T1
  table's index-0 entry `$01CB` points at the pool's last byte `$9F`, which
  has bit 7 set; the empty T1 fragment the editor emits is a single `$80`
  byte).
- T2 fragment is skipped if its first byte is `$9F`.
- T3 fragment is always emitted.
- The composed name is written into a `$26`-byte (38-byte = **19-character**)
  stripe-image buffer, then padded with blanks — longer names are silently
  truncated by the game.

## Text encoding

Bytes are tile indices into the overworld name font, not ASCII
(`crates/smwe-rom/src/overworld/level_names.rs::tile_to_char`):

- `$00–$19` → `A–Z`, `$1F` → space, `$5A` → `#`, `$5D` → `'`
- `$64–$6A` → `1–7` (as used by `#1 IGGY'S CASTLE` … `#7 LARRY'S CASTLE`),
  `$6D` → `0`
- Vanilla alternate encodings the ROM actually uses: `$32–$37` → `" ILLUS"`
  (in `"FOREST OF ILLUSION"`), `$38–$3C` → `"YELLO"` (in `"YELLOW SWITCH
  PALACE"`), `$1C` → `L` (in `"CHOCOLATE GHOST HOUSE"`).

All 93 vanilla names decode to the known in-game names (verified by the
`real_rom_vanilla_names` ignored test, e.g. `YOSHI'S HOUSE`,
`DONUT PLAINS 3`, `GREEN SWITCH PALACE`, `#2 MORTON'S CASTLE`).

## The relocation patch (why custom names need one)

The vanilla 460-byte string pool is **100% full** — there is no room to add or
lengthen a fragment. When the user customizes any name, saving applies a
minimal, reversible patch (`level_names::apply_to_rom`):

1. The three fragment tables move to `$FF`-filled space at SNES `$04A1B6`
   (immediately after `LevelNames`, which ends at `$04A1B6`), growing to
   **93/16/16 slots** (T2/T3 are hard-capped at 16 by the 4-bit packed entry
   format; T1 gets 7 bits).
2. The string pool grows **in place** from 460 to **578 bytes** (SNES
   `$049AC5–$049D06`), absorbing the old table space.
3. Three `LDA.W …,Y` address constants in `CODE_049D07` are rewritten to the
   new table bases (same instruction size, no code motion):
   - PC `0x021D2F`: `$049C91` → `$04A1B6`
   - PC `0x021D48`: `$049CCF` → `$04A270`
   - PC `0x021D61`: `$049CED` → `$04A290`

`is_patch_applied` / `is_vanilla` detect the patch state from those bytes.

## Encoder

`level_names::encode_names` takes the 93 names (vanilla + overrides):

1. Normalizes: uppercase, collapse whitespace.
2. Splits each name into shared `(prefix, middle, suffix)` fragments at word
   boundaries (`split_name`).
3. Merges the rarest T2/T3 fragments into T1 until the slot caps fit.
4. Lays out the pool (T1, T2, T3 fragments, then `$80`/`$9F` skip fragments)
   and emits the 93 packed `LevelNames` entries.

Vanilla names re-encode into ~460 of the 578 pool bytes. The
`real_rom_patch_round_trip` ignored test applies the patch to a ROM copy and
re-decodes all 93 names byte-exactly.

## UI

World editor tile-inspect panel, per selected level tile (translevel):

- Single-line text field; the vanilla name is the hint text.
- Clearing the field (or retyping the vanilla name) removes the override.
- **Byte-budget enforcement** (`level_names::check_name`, mirroring the
  message-box editor): the game draws at most 19 tiles, so names longer than
  19 characters or containing characters with no name-font tile (`A–Z 0–9
  space # '` only) are refused with a red error instead of truncated; the
  field shows `Name encodes to N / 19 tiles`.
- A customized name shows an orange "needs the name-table relocation patch on
  save" note. Saving with no custom names leaves the ROM byte-identical in
  this area.

## Verification

- `cargo check --lib` clean; `cargo test -p smwe-rom` (incl. new
  `check_name_enforces_budget_and_charset` unit test).
- Real-ROM ignored tests (`ROM_PATH=~/workspace/smw-editor/smw.smc`):
  `real_rom_vanilla_names` (all 93 decode; spot-checks) and
  `real_rom_patch_round_trip` (encode → patch → decode round-trip).
- Screenshot: `docs/screenshots/custom-level-names.png` (headless mock
  binary `src/bin/render_level_name_editor.rs`; names, budgets, and the
  rejection message are real ROM output).
