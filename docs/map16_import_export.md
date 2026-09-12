# Map16 page import/export — design notes

Implemented 2026-09-11 (`crates/smwe-rom/src/map16_file.rs`,
`src/ui/editor_prototypes/level_editor/map16_file.rs`,
`src/bin/render_map16.rs`).

## Why raw pages instead of a new format

Lunar Magic's Map16 page export (`Map16Page.bin`) is a headerless 0x800-byte
dump: 256 tiles × 8 bytes, each tile four little-endian 8×8 words in
upper-left, lower-left, upper-right, lower-right order — exactly the ROM
layout. Matching it byte-for-byte costs nothing and makes files
interchangeable with LM in both directions, so single-page files here use
that layout deliberately (verified: our export of FG page 0 starts with the
same bytes the ROM holds at PC `0x68000`).

## Page numbering

| Page | Contents |
|---|---|
| `0x00` | FG tiles `0x000`-`0x0FF` (tileset-specific) |
| `0x01` | FG tiles `0x100`-`0x1FF` (tileset-specific) |
| `0x10` | BG table tiles `0x00`-`0xFF` (SNES `$0D9100`) |
| `0x11` | BG table tiles `0x100`-`0x1FF` (SNES `$0D9900`) |

The vanilla background Map16 table is 0x1000 bytes at SNES `$0D9100`
(`Map16BGTiles` in the disassembly, `symbols/SMW_U.sym` → `000D9100`);
it is exported as two LM-sized pages.

## Fixed addresses, no repointing

Vanilla FG Map16 lives at fixed ROM addresses (see
`crates/smwe-rom/src/objects/tilesets/data.rs`). `fg_tile_snes()` is the
exact inverse of that parse layout, so export and import always agree on
where a tile lives; import writes in place and never needs the free-space
scanner. This is safe because the data is the same size by construction.

## v1 limits (documented, not silently missing)

- Vanilla layouts only: FG pages 0-1, BG table pages. No LM-expanded pages
  0x02+.
- No ExGFX tile remapping on import.
- No "acts like" bytes — vanilla SMW dispatches block behavior by hardcoded
  ID range (see `block_behavior::category_of`), not per-block data.
- FG pages are tileset-specific; the editor defaults the selector to the
  current level's Map16 tileset.

## Verification

- `cargo test -p smwe-rom --lib map16_file` (unit: container round trips,
  address map spot checks against `tilesets/data.rs`).
- Real-ROM ignored tests (`ROM_PATH=... -- --ignored`): export matches raw
  ROM bytes; export → import → re-export byte-identical for FG page 0
  (tileset 0), FG page 1 (tileset 2), BG page 0, and a 3-page set; plus a
  mutation test proving import actually changes the ROM.
- `docs/screenshots/map16-import-export.png`: real atlas of exported FG
  page 0 rendered through emulator VRAM/CGRAM, with a before/after strip
  proving the import round trip.
