# ROM expansion

File > **Expand ROM...** — Lunar Magic's "Expand ROM", for when a hack runs
out of free space entirely.

## What it does

Grows a LoROM image to 1 MB, 2 MB, or 4 MB (32 Mbit, the LoROM cap):

1. Appends `$FF`-filled banks to reach the target size.
2. Rewrites the internal header:
   - ROM size byte (`0x7FD7`) → the new size's exponent (`2^N` KB),
   - checksum/complement pair (`0x7FDC`/`0x7FDE`) recomputed,
   - map mode, ROM type, and every other field untouched.
3. Preserves a 512-byte SMC header if the file had one.

The appended space is `$FF` fill, so the existing free-space scanner
(`smwe_rom::freespace::find_free_space`, used by level layer/sprite data, GFX
files, message boxes, and overworld layer 2) picks it up automatically —
no other code changes were needed. A unit test pins this wiring
(`rom_expansion::tests::new_space_is_visible_to_find_free_space`).

## UI behavior

- The dialog lists only targets larger than the current size and defaults to
  the largest (like LM). A ROM already at 4 MB shows "already at the maximum
  LoROM size".
- Unsaved editor-tab edits are merged first (same as Save), then the file is
  rewritten with the same `.bak`-backup + temp-file + rename discipline as
  Save, and the project is reloaded so the new size is visible everywhere.

## Checksum convention

The recomputed checksum is the 16-bit wrapping sum of every byte of the
headerless image *excluding* the 4 complement/checksum bytes themselves,
with the complement stored as `checksum ^ 0xFFFF`. This is self-consistent:
recomputing over an expanded image reproduces the stored value. (The vanilla
SMW header's stored checksum doesn't match a plain byte sum either —
Nintendo's build tools computed it differently — so retail ROMs can't be used
to calibrate this; self-consistency is what matters, and the header-detection
heuristic only needs the pair to be complementary.)

## Limits

- LoROM only, capped at 4 MB. Larger needs ExLoROM, which is a separate
  mapper feature (see the SA-1/ExLoROM backlog item).
- Only 512 KB / 1 MB / 2 MB sources are accepted; anything else is refused
  with an error rather than guessed at.

## Verification

- 7 unit tests in `crates/smwe-rom/src/rom_expansion.rs` on synthetic images
  (header fixups, checksum self-consistency, target listing, input rejection,
  free-space visibility, SMC handling).
- Real-ROM check: a copy of the vanilla ROM was expanded to 4 MB and re-opened
  through `SmwRom::from_file` — header parses, size byte reads `0x0C`, all
  levels/GFX/message data parse unchanged.
- Screenshot: `docs/screenshots/rom-expand.png` (headless mock of the dialog;
  every number on it — sizes, header bytes, free-space bars — is real output
  from `expand_rom` on the vanilla ROM).
