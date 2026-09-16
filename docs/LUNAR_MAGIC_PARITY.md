# Lunar Magic Feature Parity Tracker

Tracks what Lunar Magic (LM) can do vs. what smw-editor currently supports, so
work can be prioritized toward feature completeness. Update this file as
features land — flip status and add the PR/commit that did it.

Status legend: ✅ Done · 🟡 Partial · ⛔ Missing

**Confidence note:** this was built from one codebase-survey pass plus
targeted greps, not an exhaustive audit. Two follow-up checks already
corrected the first draft (sprite extra-bits were wrongly marked missing;
freespace-finding was undersold as overworld-only). Treat ⛔ rows as "not
found by grep," not proven absent — re-verify with a grep/read before relying
on a row for planning if it's been a while since the file was touched. Missing
areas not yet cross-checked in depth: layer 3 "tide"/water settings across
levels, direct Map16 import/export file format, and player (Mario/Yoshi)
graphics customization. (ROM search/analysis is now covered: PR #4 added ROM-wide cross-reference
search — "find all references" for sprites, objects, tiles, music, and exits.)

## Level Editing

| Feature | Status | Notes |
|---|---|---|
| Object placement/move/resize (layer 1) | ✅ | `level_editor/object_layer.rs`, `editing.rs` |
| Sprite placement | ✅ | `sprite_layer.rs`, `sprite_catalog.rs`; global per-ID behavior editable via `sprite_tweaker_editor.rs` (see Sprites section) |
| Primary/secondary header editing | ✅ | `properties.rs`, `left_panel.rs` — raw byte fields |
| Screen exits / secondary entrances | ✅ | `secondary_entrance_editor.rs` |
| Map16 tile picker & editor | ✅ | `tile_picker.rs`, `map16_editor.rs` |
| Palette editor (BG/FG/sprite) | ✅ | `palette_editor.rs` |
| Layer 2/3 background editing | 🟡 | `background_layer.rs` minimal (~510 bytes); the 5-byte Layer 2 object-data header is now user-editable in the Level Header window (hex byte sliders) and written back on save instead of copied verbatim. The game skips these bytes (SMWDisX `bank_05.asm`: +5 "to ignore Layer 2's header"), so edits only change the stored bytes; in the vanilla ROM they mirror the level's primary header. Model: `Layer2Data::Objects { header, objects }` + `LAYER2_HEADER_SIZE` (`crates/smwe-rom/src/level/mod.rs`); mwl export now carries the model's header. Verified by real-ROM ignored tests (parsed header == pointer bytes for every L2-objects level; edit → write → re-parse round-trip with the object stream byte-identical). See `docs/screenshots/l2-header-editing.png` |
| Music selection | ✅ | Named vanilla track picker (`crates/smwe-rom/src/music.rs`, derived from SMWDisX `LevelMusicTable` in `bank_05.asm`): all 8 tracks with raw-value fallback for remapped ROMs; xref search also shows track names |
| Custom level names (overworld name table) | 🟡 | Beyond Lunar Magic (LM has no level-name editing): implemented — see "Custom level names" in Overworld Editing below. All 93 vanilla names decode from the real ROM; encode→patch→decode round-trip verified |
| Message box / dialog text editor | ✅ | `crates/smwe-rom/src/message_boxes.rs` + `level_editor/message_editor.rs` — all 22 vanilla messages, verified against real ROM (exact byte boundaries derived from `symbols/SMW_U.sym` label addresses; total size matches the ROM's already-100%-utilized 2854-byte budget exactly). PR #5 added a read-only WYSIWYG preview: true GFX2A message font rasterization (SNES $0BCB7B, 2bpp, 128 tiles) plus a `CODE_05B1BC` stripe capture, shown as an 8×18 raster per message, validated against the real ROM. Phase 2 (this PR) made the text itself editable: a multiline field shows decoded text (`\n` = line break; the real routine has no other control codes) and typing re-encodes to tile bytes live, with per-message byte-budget enforcement (`used / budget`, over-budget input refused, not truncated). Non-text graphic tiles (Yoshi's signature, bonus-star icons) show as `�` and are preserved in place. Decode→encode round-trips all 22 vanilla messages byte-exactly |
| Import/export level as `.mwl` | 🟡 | `crates/smwe-rom/src/mwl.rs` + `level_editor/mwl.rs`: toolbar Export/Import buttons. Binary MWL v3.63 container (header, 8-entry directory, 8 sections); level-info (level number + secondary header), Layer 1 (5-byte primary header + objects), Layer 2 objects or legacy background (0x800 LE words; vanilla mapping verified: entries 0x000–0x1AF→words 0x000–0x1AF, 0x1B0–0x35F→0x200–0x3AF; descriptor $08/$18 from LM 3.63's own binary; high byte from the game's $E8FE boundary, confirmed in SMWDisX `bank_05.asm`), sprites (1-byte header + stream). Export applies unsaved edits via scratch ROM; import repoints into free space, writes the ROM atomically (backup + rename), and reloads. Validated on the real ROM: export shape of level 0x105, export→import→re-export payload-identical round-trips for a background level (0x0) and an L2-objects level (0x9). v1 limits: vanilla layouts only (no RATS/LC_LZ2/3); palette, secondary entrances, ExAnimation, ExGFX/bypass sections export empty and are rejected on import when non-empty. See `docs/screenshots/mwl-import-export.png` |
| Move/resize via drag handles (LM-style) | ✅ | `level_editor/editing.rs` (`DragHandle`, `ObjectDrag`, `update_object_drag`) + overlay in `central_panel.rs`: selecting a single layer-1 object shows 8 LM-style handles (4 corners + 4 edge midpoints, white squares with black borders) plus a yellow selection outline. Dragging the body moves the object; dragging a handle resizes it (settings byte: low nibble = width−1, high nibble = height−1, clamped 1–16 tiles; drag edge follows the pointer, opposite edge fixed). Live transient feedback during the drag; release commits exactly one undoable write (`obj.x/y` + settings) and moves the tile-map footprint (old area blanked with 0x25, blocks re-stamped with edge-stretch for grown cells). Extended (1×1) objects move but don't resize (their settings byte is the object ID). Hover cursors per handle direction, grab hand on body; object drags suppress canvas panning and release-click re-selection. Unit-tested geometry incl. settings round-trip over all 1–16×1–16 sizes and the W/N far-drag 16-tile clamp regression test. See `docs/screenshots/drag-handles.png` and the animated `docs/screenshots/drag-handles.gif` |

## Overworld Editing

| Feature | Status | Notes |
|---|---|---|
| Submap viewing/navigation | ✅ | `world_editor/mod.rs::load_submap` |
| Layer 1 tile paint/erase | ✅ | `editing.rs` |
| Layer 2 tile paint/erase + repoint on save | ✅ | `write_overworld_l2_stream`, `patch_snes_pointer` |
| Save/repoint to ROM | ✅ | `find_free_space`, `patch_snes_pointer` |
| Path tile drawing (the visual road/dots) | ✅ | Confirmed path tiles are ordinary L1 tiles (SMWDisX has no separate path-data table for the general case) — already fully paintable via the existing L1 draw/erase tools. Only a curated "path piece" picker/auto-tile UX is missing, not the underlying capability |
| Path movement data (LineGuide-style step tables) | N/A | Investigated and ruled out as a separate system for the general case — SMW's ~10 hard-coded special-case connections (`HardCodedOWPaths`/`OWHardCodedTiles`/`OWHardCodedDirs`, `bank_04.asm:~1646`) are the only exception, not worth building UI for |
| Event tile preview toggles (which "destruction" events are shown) | ✅ | `crates/smwe-rom/src/overworld/mod.rs::OverworldEvents` parses the real reveal-tile-swap tables (`$04D85D` tile offsets, `$04DA1D`/`$04DA33` before/after tile IDs, ported from SMWDisX `CODE_04DA49`) + tests; `world_editor` now has a per-event checkbox panel (`events_panel`) replacing the old blanket "activate all events" hack — toggling writes real `OWEventsActivated` WRAM bits so the actual emulated game code applies the swap. Verified against the real ROM (`render_ow_submap --dump-events`: 49 tiles change when all events applied, matching expected castle/fortress/switch-palace reveal graphics) |
| Event *ownership* editing (which level/action triggers which event) | ✅ (beyond LM) | `crates/smwe-rom/src/overworld/event_ownership.rs`: parses the 93-byte events-by-translevel table `DATA_05D608` (SNES `$05D608`, PC `0x2D608`) — the game reads it into WRAM `OverworldEvent` on level completion (`bank_05.asm`: `LDY.W TranslevelNo` / `LDA.W DATA_05D608,Y`). Each byte is the triggered event (`0..OW_EVENT_COUNT`), `$FF` = no event. All vanilla values validated (none out of range). UI: new "Event ownership (by level)" panel in the world editor with one combo per translevel — `None (no event)` or `Event N` annotated with the tile it reveals (from the real reveal-tile table) — plus level names from the name-table work. Out-of-range events are refused, not clamped. Save writes the table back in place (no relocation patch needed); untouched ROMs stay byte-identical. Verified against the real ROM incl. an ignored round-trip test (`real_rom_event_ownership_table`). See `docs/screenshots/event-ownership.png` |
| Layer 2 event tiles (separate from Layer 1 reveal-tile swaps) | ✅ | `crates/smwe-rom/src/overworld/mod.rs::OverworldL2Events` parses the real tables (SMWDisX `bank_04.asm`): the 371-entry 4-byte table at `$04DD8D` (`data_word`/`dest_word`), the 121-word cumulative per-event boundary table at `$04E359` (event `e` owns entries `boundaries[e]..boundaries[e+1]`; driven by `CODE_04E453`/`CODE_04E6D3`/`CODE_04E6F9`), and the 44-row "silent event" tables (`$04E8E4` event numbers, `$04E910` flags bit 0 = L2 vs Map16 edit, `$04E93C`/`$04E994` dest/data words, `CODE_04E9EC`). Entry kind decoded from `data_word` (`< $0900` = VRAM tile stream via dynamic stripe, `>= $0900` = tilemap copy from WRAM `$7F8000+data_word`); target tile decoded exactly like the game (`x_px = ((dest & $3E) << 2)`, `y_px = (((dest >> 3) as u8) & $F8)`). UI: new "Layer 2 events" section in `world_editor`'s events panel lists per-event entry ranges with kind + target tile (and silent rows), plus on-map target markers (cyan = tile stream, orange = tilemap copy) that follow the event checkboxes. Verified: real-ROM ignored test matches the disassembly byte-for-byte (first entries, boundaries, silent rows); all 13 of event 1's entries hand-checked against the ASM bytes. See `docs/l2_event_tiles.md` and `docs/screenshots/l2-events-map.png`, `docs/screenshots/l2-events-panel.png` |
| Level-number display per tile (read-only, vanilla-accurate) | ✅ | `crates/smwe-rom/src/overworld/mod.rs::level_number_at`/`translevel_at` (+ tests), surfaced in `world_editor/mod.rs` tile-inspect panel. Confirmed against real ROM (`render_ow_submap --dump-levels`): matches vanilla's translevel scan-order numbering including the documented 0x25→0x01 wraparound |
| Level-number free reassignment (LM-style, arbitrary) | ✅ | Turned out not to need code injection: a single existing instruction (`LDA.L $7ED000,X` at SNES `$05D89B`, confirmed byte-for-byte against a real ROM: `BF 00 D0 7E`) is repointed to a custom ROM table instead of the vanilla WRAM-computed one, using the same `layer1_tiles` index space. `encode_custom_level_number` inverts the vanilla remap so the existing (unmodified) remap code still produces the right final number; verified with an exhaustive round-trip test over all 220 representable values (0x00-0xDB). UI in `world_editor/mod.rs` tile-inspect panel; only touches the ROM if the user actually overrides a level number, so untouched hacks stay byte-identical to vanilla in this area. Known limitation: repeated saves with active overrides allocate a fresh table each time rather than reusing one in place (documented in code, harmless but wasteful) |
| Custom level names (overworld name table) | ✅ | `crates/smwe-rom/src/overworld/level_names.rs`: decodes the 93-entry `LevelNames` table (SNES `$04A0FC`) via the three fragment tables (`$049C91`/`$049CCF`/`$049CED`, `CODE_049D07` in `bank_04.asm`). The vanilla 460-byte string pool is 100% full, so customized names trigger a relocation patch: tables move to `$FF`-filled space at `$04A1B6` (93/16/16 slots; T2/T3 capped at 16 by the 4-bit packed entry format), pool expands in place to 578 bytes, and three `LDA.W` address constants are rewritten. Encoder splits names into shared (prefix, middle, suffix) fragments, merging rare fragments into T1 to fit the slot caps. UI: text field in `world_editor` tile-inspect panel per translevel, vanilla name as hint, and byte-budget enforcement (`check_name`: at most 19 tiles — the game's `$26`-byte stripe buffer — `A–Z 0–9 space # '` only; over-long/invalid input refused with a red error, mirroring the message-box editor). Verified: all 93 vanilla names decode correctly; encode→patch→decode round-trip passes on the real ROM. See `docs/custom_level_names.md` and `docs/screenshots/custom-level-names.png` |
| Layer 2 scroll properties (not raw tiles) | ⛔ | Not found |
| Animated overworld tiles (preview) | 🟡 | `smwe-emu::emu::{init_ow_anim_water, advance_ow_anim_frame}`: Rust transcription of `CODE_048086`/`OW_Tile_Animation` (`bank_04.asm`). Initializes the 96-byte water/waterfall buffer at `$7E0AF6` from decompressed GFX14 (`GFX14_OWAnimation`) via `DATA_048000` pointers (`$7EB480/$7EB498/$7EB4B0`), then rotates bits per visible frame (8 game-frames = 133ms). VBlank DMA replica uploads 352 bytes to VRAM word `$0750` (4bpp tiles 117–127). World editor ticks every 133ms and re-uploads. Editing the animation data (not just previewing) is not implemented — no LM-parity editable table was found. Indicator sprites were investigated: `OWScrollArrowStripe` is a fixed border graphic, and Mario/OW sprites are gameplay rendering, not an LM-style editable indicator list. See `docs/screenshots/ow-animated-tiles.gif` |
| Overworld undo/redo | ✅ | Added 2026 (commit `77dfc73`) |

## Graphics / Map16 / Palette Tools

| Feature | Status | Notes |
|---|---|---|
| VRAM/GFX viewer widget | ✅ | `crates/smwe-widgets/src/vram_view.rs` |
| Palette viewer widget | ✅ | `crates/smwe-widgets/palette_view.rs` |
| Map16 editor | ✅ | `map16_editor.rs` |
| Map16 page import/export | ✅ | `crates/smwe-rom/src/map16_file.rs` + `level_editor/map16_file.rs` + `src/bin/render_map16.rs` — export/import Map16 pages (FG pages 0-1 per tileset 0-4, BG pages 0-1) as raw 0x800-byte files that are Lunar Magic `Map16Page.bin` compatible (verified interchangeable shape: no header, same 8-bytes-per-tile layout). Import writes the pages' fixed vanilla ROM addresses in place (no repointing). Round-trip verified against the real ROM: export → import → re-export is byte-identical for FG pages (tilesets 0 and 2) and BG pages. UI lives in the Map16 Block Editor window (page/tileset selectors, Export page / Import, status line). v1 limits: vanilla layouts only; no LM-expanded pages 2+, no ExGFX remapping, no "acts like" bytes |
| Vanilla GFX file reading (0x00-0x33) | ✅ | `crates/smwe-rom/src/graphics/gfx_file/` — decompresses into `rom.gfx.files` for VRAM composition |
| GFX write plumbing (compress + tile encode + repoint) | ✅ | `compression::lc_lz2::compress` (direct-copy + byte-fill, verified round-trip against real ROM GFX data) + `GfxFile::to_raw_bytes`/`decode_tiles` (tile encoders, exact inverse of the existing decoders, verified round-trip against real ROM data) + pointer-table read/repoint logic in `level_editor/mod.rs::save_to_rom` |
| ExGFX import/export UI | ✅ | `level_editor/gfx_editor.rs` — export any GFX file slot (0x00-0x33) as a lossless grayscale PNG (pixel intensity = color index, not a colored preview) via `rfd` file dialogs; import a PNG back, staged in `gfx_edits` and written on save (LC_LZ2-compressed, repointed via `find_free_space` if the new data doesn't fit in place). Verified end-to-end against real ROM GFX file 0 (export→import→re-encode reproduces the original bytes exactly, and compresses/decompresses correctly) |
| Colored (palette-aware) GFX preview/import, per-level GFX slot *browser* tied to the editor | 🟡 | Per-file import is still grayscale-index-only (deliberate, for exact round-tripping), but the new 8x8 tile editor (`level_editor/tile_editor.rs`) adds the palette-colored WYSIWYG view: every tile of a GFX file rendered through a selectable 16-color CGRAM row, with a per-level FG/BG/sprite slot browser cross-linked to the editor — double-clicking a Map16 sub-tile preview jumps straight into editing that tile's pixels in its source GFX file (mapped via the level's ObjectTileset → OBJECTGFXLIST row → one of the four 0x80-tile upload slots, matching `CODE_00AA35`) |
| 8x8 tile bitmap import/export | ✅ | Replaced by a real pixel editor, not a bitmap pipe: `level_editor/tile_editor.rs` shows a 16-column palette-colored grid of every tile in any GFX file (2/3/4/8bpp-aware), a selectable CGRAM palette row, a zoomed 8x8 pixel canvas with left-drag painting + right-click eyedropper, and Apply stages the edited tile back through `GfxFile::to_raw_bytes` into the existing `gfx_edits` save path (LC_LZ2 + repoint on save). Double-clicking a Map16 sub-tile preview opens the editor on that tile's source GFX file and selects the tile. Round-trip verified: painted pixels re-encode with byte diffs confined to the edited tile; real-ROM ignored test confirms tileset 0 maps to OBJECTGFXLIST row `Normal 1 = [0x14, 0x17, 0x19, 0x15]` |

## Sprites

| Feature | Status | Notes |
|---|---|---|
| Sprite placement in levels | ✅ | See above |
| Sprite extra bits (2-bit position field per sprite) | ✅ | `sprite_layer.rs`, `left_panel.rs:103` — confirmed editable, corrected from an earlier pass that missed it |
| Sprite Map / OAM tile editor | ✅ | `src/ui/editor_prototypes/sprite_map_editor/` |
| Sprite tweaker/behavior byte editing (6 global tables, "Sprite Header Editor") | ✅ | `crates/smwe-rom/src/sprite_tweakers.rs` parses the 6 ROM tables ($07F26C/$07F335/$07F3FE/$07F4C7/$07F590/$07F659, 0xC9 entries each) with named bit accessors (+ tests); `sprite_tweaker_editor.rs` in the level editor exposes all of them with save-to-ROM support. Verified against real ROM: Goomba (0x0F) shows `can_be_jumped_on=true`, `dies_when_jumped_on=false`, matching known vanilla behavior. Edits are global (affect every placement of that sprite ID), matching how LM's own editor works |
| Sprite category distinction (cluster/extended/generator vs. normal) | 🟡 | `SpriteTweakers::has_tweakers()` now encodes the boundary (IDs `>= 0xC9` don't have tweaker bytes) and the tweaker editor warns when selecting one; no dedicated `SpriteCategory` type or separate editing UI for those categories yet |
| Custom sprite insertion (SA-1/UberASM-style dropins) | ⛔ | Not found |

## Music / Sound

| Feature | Status | Notes |
|---|---|---|
| Music track selection (header nibble) | ✅ | Named picker with all 8 vanilla tracks (SMWDisX `LevelMusicTable`); raw byte shown alongside for custom/remapped ROMs |
| Music/SPC data import or editing | ⛔ | Not found |

## Data / ASM / Patches

| Feature | Status | Notes |
|---|---|---|
| BPS/IPS patch export | ✅ | `smwe-bps`, `smwe-ips` plus File menu export dialogs in `src/ui/mod.rs` |
| ASM insertion tool / hijack manager | ⛔ | No user-facing ASM editor |
| 65816 disassembler | ✅ (library only) | `crates/smwe-rom/src/disassembler`, `crates/wdc65816` — not exposed as a user-facing ASM editor |
| ROM expansion (expand to 1/2/4MB, freespace tracking) | ✅ | `crates/smwe-rom/src/rom_expansion.rs`: `expand_rom` grows a LoROM image (512KB/1MB/2MB) to 1/2/4MB by appending `$FF`-filled banks, then fixes the internal header (ROM size byte + recomputed checksum/complement pair, map mode untouched); SMC headers preserved on write. UI: File > Expand ROM... dialog with target picker, `.bak` backup, unsaved-edits-saved-first. The appended `$FF` space is picked up automatically by the unified `find_free_space`/`find_free_space_in` scanner used by level layer1/layer2/sprite data, GFX files, message boxes, and overworld L2 — verified with a unit test. See `docs/rom_expansion.md` and `docs/screenshots/rom-expand.png` |
| Title screen editor | ✅ | `Title/Credits…` window in the level editor exposes fixed-slot title data: opening overworld submap immediate operand, title demo controller playback (`TitleScreenInputSeq`), and the Layer-3 title stripe image (`TitleScreenStripe`). The stripe now has a WYSIWYG editor (`crates/smwe-rom/src/title_stripe.rs` + `level_editor/title_credits_editor.rs`): parses the 905-byte stripe into an editable 64×64 tile grid (exact `LoadStripeImage` DMA semantics, 3-byte headers, 32-word vertical stride), previews it with real title GFX/palette from the emulator (level 0xEB + `CODE_00ADA6`/`CODE_00922F`), click/drag painting, right-click tile picking, tile-word editor (tile/palette/priority/flips), and byte-budget enforcement (`used / 1108`, over-budget refused). Raw bytes kept under a collapsed advanced section |
| Credits editor | 🟡 | `Title/Credits…` window exposes raw ending enemy-name stripe images (`EnemyNameStripe00..0C`) with decoded text summaries and fixed-slot bounds checks. Staff roll text/scripts, credits scene scripts, HDMA, sprite choreography, and ending special enemy-name overlays are not modeled yet |

## Save / Export

| Feature | Status | Notes |
|---|---|---|
| Save to ROM | ✅ | `src/ui/mod.rs::save_rom`/`save_rom_as`/`write_rom_to_path` |
| SMC header detection on save | ✅ | `write_rom_to_path` |
| Repointing / freespace allocation | 🟡 | Free-space *scanning* is now unified in `src/rom_freespace.rs` (with tests), used by level L1/L2/sprite data, GFX files, message boxes, and overworld L2 — previously duplicated verbatim in two places. Writing the new pointer bytes themselves is still feature-specific (different pointer table layouts per feature: 3-byte SNES pointers, GFX's 3-lookup-table split, message boxes' 25-entry u16 table, etc.), which is inherent to the ROM format rather than something to further unify |

## Misc Tools

| Feature | Status | Notes |
|---|---|---|
| Lunar Magic-style level-editor toolbar | ✅ | `src/ui/editor_prototypes/level_editor/toolbar.rs` — horizontal icon toolbar (phosphor icons) mirroring LM's main window: Save-to-ROM / Reload / level-number box / zoom (row 1); Layer 1 / Layer 2 / sprite edit-target toggles, Select/Insert/Erase/Probe tools, grid+overlay+label view toggles, and sub-editor launchers Header/Map16/GFX/Palette/Sprite-behavior/Messages/Secondary-entrances/Title-credits (row 2); plus a bottom status bar (mode · edit target · level · size · selection). Controls were lifted out of `left_panel.rs`, which now holds only the tile/sprite palette + selection inspector. Icon buttons drive existing `UiLevelEditor` state, so all wiring was already functional. Keyboard shortcuts match Lunar Magic (source: SMW Central "Lunar Magic Shortcuts", Thomas, v3.60): PageUp/PageDown step level number, backtick toggles Layer 1/2 editing, F8 toggles grid, Ctrl -/= zoom. **Known divergences from real LM:** (1) the Select/Insert/Erase/Probe tool row is this editor's own interaction model — LM has no tool palette (it uses left-click=select/move, right-click=paste-place, Delete=erase gestures instead); (2) the grid/overlay/label view toggles don't map to LM's per-layer 1/2/3/4/6 view keys; (3) exact button order and pixel-art icons aren't matched (LM skins its toolbar via a `Lunar Magic.ff4` 16×16-button bitmap). **Follow-up:** Undo/Redo buttons not added — `UndoableData` supports undo but object edits paint straight into the WRAM block map with no "re-rasterize object list → WRAM" path, so undo would revert the model while leaving the canvas stale; needs that raster path first |
| Address converter (PC/SNES) | ✅ | `src/ui/dev_utils/address_converter.rs` |
| Welcome screen / ROM open flow | ✅ | `src/ui/welcome.rs` and the File menu use the egui-native open dialog; `project_creator.rs` is currently unused legacy source |
| Mapper auto-detection (LoROM/HiROM/SA-1/ExLoROM/ExHiROM) | ✅ | Detected from header checksum + map-mode byte (`smwe_emu::rom::detect_mapper`): SA-1 packs (Mode $23/$25) map with their base LoROM/HiROM S-CPU bus layout, ExLoROM/ExHiROM get their upper 4 MiB windows; `smwe_rom::MapMode` parses all Mode 2x values and the dev-utils address converter offers ExLoROM/ExHiROM modes |
| Block editor: "acts like" reference | ✅ | `crates/smwe-rom/src/block_behavior.rs` — vanilla dispatches block collision/interaction by hardcoded ID range, not a per-block data byte (source: SMW Central Data Repository, "Detailed explanation of interaction of each tile," MarioFanGamer, 17 Oct 2024). A custom block already gets any of these behaviors for free by using a Map16 ID from the matching range with custom graphics (already fully supported by the existing Map16 editor). Surfaced as an "Acts like: ..." label in `map16_editor.rs` when selecting/editing a block. Ranges + specific tile behaviors covered by tests |
| Block editor: novel (non-vanilla) custom behavior via ASM insertion | ⛔ | Giving a block a behavior that doesn't correspond to any existing vanilla ID range would need a real JSL hook into the interaction dispatcher — genuine new code, not a data patch (unlike the overworld level-number case above). Not started; this is a categorically higher-risk piece of work than anything else in this tracker |
| Graphics editor | ⛔ | README lists as "Planned" |
| ASM code editor | ⛔ | README lists as "Planned" |
| Music editor | ⛔ | README lists as "Planned" |

## Known correctness gaps affecting parity work

- ~~Custom Layer-2 backgrounds in hacked levels can render scrambled~~ FIXED:
  root cause was `UploadSpriteGFX`'s decompression overrunning the `$7EAD00`
  buffer into the `$7EB900` BG tilemap on ROMs with Lunar-Magic-sized GFX files
  (harmless on hardware where the BG is converted to VRAM first; fatal for the
  editor which renders from that WRAM). `decompress_sublevel`/`decompress_extram`
  now snapshot the BG tilemap after `CODE_05801E` and restore it at the end.
- ~~SA-1 and ExLoROM/ExHiROM ROMs are not supported by the mapper or ROM header
  parser~~ FIXED: SA-1 map modes ($23/$25/$33/$35) now parse, SA-1 packs map
  with their base LoROM/HiROM S-CPU bus layout, and ExLoROM/ExHiROM get full
  upper-window address math in both the header parser and the emulator mapper.
- Title/credits editing is currently U-ROM fixed-address only for the modeled
  slots; non-U variants have different `TitleScreenInputSeq`/stripe addresses
  in the symbols and need region-aware address selection before they are safe.

## Biggest gaps to close for parity (suggested priority)

1. **Block editor: novel custom behavior via ASM insertion** — the "acts like" reference (free, ID-range-based) now works; this remaining piece needs a real JSL hook into the interaction dispatcher, the highest-risk item in this tracker (actual new code, not a data patch).
2. ~~**Message box font/WYSIWYG preview** — raw tile-index byte editing works and is verified against real ROM data; still needed: identify the message font's GFX source (Layer 3 "dynamic stripe image") so users can see/type readable text instead of raw tile numbers.~~ **DONE** via PR #5 (read-only WYSIWYG preview) and PR #6 (editable text with live re-encoding), both merged.
3. **Custom sprite insertion / cluster-extended-generator sprite category editing** — tweaker byte editing now covers the ~0xC9 normal sprite IDs; the other categories still have no dedicated support.
4. **ExGFX colored preview + per-level slot browser cross-linking** — the core import/export loop works now; this is the remaining UX polish.
5. ~~**ROM expansion** — free-space *scanning* is now unified (`src/rom_freespace.rs`), but there's still no way to grow the ROM itself (LM's "expand to 3/4MB") for when a hack runs out of space entirely.~~ **DONE** — `crates/smwe-rom/src/rom_expansion.rs` + File > Expand ROM... dialog: expands LoROM images to 1/2/4MB with `$FF` fill and a fixed internal header (size byte + checksum); the new space flows into the unified free-space scanner automatically.
