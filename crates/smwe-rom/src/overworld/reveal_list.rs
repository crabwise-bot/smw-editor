//! Editable overworld "reveal tile list" (Lunar Magic v2.30 "Edit Reveal Tile
//! List" menu).
//!
//! When a "destruction" event fires, the game swaps layer-1 tiles: for each
//! active event, if the tile at its offset (the `$04D85D` per-event offset
//! table) matches a "before" ID, it is replaced with the parallel "after" ID
//! (SMWDisX `bank_04.asm`, `CODE_04DA49`). The before/after ID lists live at
//! SNES `$04DA1D`/`$04DA33`, 22 bytes each, and are global — every event
//! consults the same list.
//!
//! The last entry is special in the vanilla game: the switch-palace reveal
//! also writes the tile *after* the event's offset (see
//! [`OverworldEvents::apply`][crate::overworld::OverworldEvents::apply]). The
//! editor surfaces that quirk in the UI so it isn't edited blindly.
//!
//! Editing is in place — 22 + 22 bytes, fixed count — so no relocation patch
//! is needed and untouched ROMs stay byte-identical.

use crate::snes_utils::addr::{AddrPc, AddrSnes};

/// SNES address of the "before" tile IDs for the reveal-tile swap
/// (1 byte/entry, parallel to [`REVEAL_AFTER_SNES`]).
pub const REVEAL_BEFORE_SNES: AddrSnes = AddrSnes(0x04DA1D);
/// SNES address of the "after" tile IDs for the reveal-tile swap
/// (1 byte/entry, parallel to [`REVEAL_BEFORE_SNES`]).
pub const REVEAL_AFTER_SNES: AddrSnes = AddrSnes(0x04DA33);
/// Fixed number of before/after reveal pairs.
pub const REVEAL_COUNT: usize = 22;
/// Index of the switch-palace entry whose reveal also writes the tile after
/// the event's offset (matches [`OverworldEvents::apply`][crate::overworld::OverworldEvents::apply]).
pub const REVEAL_SWITCH_PALACE_INDEX: usize = REVEAL_COUNT - 1;

/// The editable reveal-tile list: which layer-1 tiles are revealed into which
/// other tiles when an event passes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RevealTileList {
    /// "Before" tile IDs, len [`REVEAL_COUNT`], parallel to [`Self::after`].
    pub before: Vec<u8>,
    /// "After" tile IDs, len [`REVEAL_COUNT`], parallel to [`Self::before`].
    pub after:  Vec<u8>,
}

impl RevealTileList {
    /// Parse the two 22-byte lists from ROM bytes (`header_offset` = `0x200`
    /// if the ROM has an SMC header, else `0`).
    pub fn parse(rom: &[u8], header_offset: usize) -> anyhow::Result<Self> {
        let read = |addr: AddrSnes, what: &str| -> anyhow::Result<Vec<u8>> {
            let pc = AddrPc::try_from_lorom(addr).map_err(|e| anyhow::anyhow!("{what} addr conversion: {e}"))?.0
                as usize
                + header_offset;
            let end = pc + REVEAL_COUNT;
            rom.get(pc..end).map(|s| s.to_vec()).ok_or_else(|| anyhow::anyhow!("{what} extends past end of ROM"))
        };
        Ok(Self {
            before: read(REVEAL_BEFORE_SNES, "reveal-before list")?,
            after:  read(REVEAL_AFTER_SNES, "reveal-after list")?,
        })
    }

    /// Write the two lists back into `rom_bytes` in place. Fails if either
    /// list has the wrong length (the game reads exactly [`REVEAL_COUNT`]
    /// entries; a short/long list would corrupt neighboring data).
    pub fn apply_to_rom(&self, rom_bytes: &mut [u8], header_offset: usize) -> anyhow::Result<()> {
        if self.before.len() != REVEAL_COUNT || self.after.len() != REVEAL_COUNT {
            anyhow::bail!(
                "reveal list must hold exactly {REVEAL_COUNT} before/after pairs (got {}/{})",
                self.before.len(),
                self.after.len()
            );
        }
        let write = |rom_bytes: &mut [u8], addr: AddrSnes, data: &[u8], what: &str| -> anyhow::Result<()> {
            let pc = AddrPc::try_from_lorom(addr).map_err(|e| anyhow::anyhow!("{what} addr conversion: {e}"))?.0
                as usize
                + header_offset;
            let end = pc + REVEAL_COUNT;
            let dst = rom_bytes.get_mut(pc..end).ok_or_else(|| anyhow::anyhow!("{what} write range out of bounds"))?;
            dst.copy_from_slice(data);
            Ok(())
        };
        write(rom_bytes, REVEAL_BEFORE_SNES, &self.before, "reveal-before list")?;
        write(rom_bytes, REVEAL_AFTER_SNES, &self.after, "reveal-after list")?;
        Ok(())
    }

    /// Event indices whose reveal this row can trigger: events whose current
    /// layer-1 tile at their per-event offset equals `before[index]`
    /// (mirrors the match in `CODE_04DA49`).
    pub fn events_using_row(&self, index: usize, tile_offsets: &[u16], layer1_tiles: &[u8]) -> Vec<usize> {
        let Some(&before) = self.before.get(index) else { return Vec::new() };
        tile_offsets
            .iter()
            .enumerate()
            .filter(|(_, &off)| layer1_tiles.get(off as usize).copied().unwrap_or(0xFF) == before)
            .map(|(e, _)| e)
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vanilla_lists() -> (Vec<u8>, Vec<u8>) {
        // Real bytes from the vanilla ROM ($04DA1D/$04DA33, PC 0x25A1D/0x25A33).
        let before = vec![
            0x6E, 0x6F, 0x70, 0x71, 0x72, 0x73, 0x74, 0x75, 0x59, 0x53, 0x52, 0x83, 0x4D, 0x57, 0x5A, 0x76, 0x78, 0x7A,
            0x7B, 0x7D, 0x7F, 0x54,
        ];
        let after = vec![
            0x66, 0x67, 0x68, 0x69, 0x6A, 0x6B, 0x6C, 0x6D, 0x58, 0x43, 0x44, 0x45, 0x25, 0x5E, 0x5F, 0x77, 0x79, 0x63,
            0x7C, 0x7E, 0x80, 0x23,
        ];
        (before, after)
    }

    #[test]
    fn round_trip_in_place() {
        let (before, after) = vanilla_lists();
        // Scratch "ROM" big enough to hold the tables at their PC offsets.
        let before_pc = AddrPc::try_from_lorom(REVEAL_BEFORE_SNES).unwrap().0 as usize;
        let after_pc = AddrPc::try_from_lorom(REVEAL_AFTER_SNES).unwrap().0 as usize;
        let mut rom = vec![0u8; after_pc + REVEAL_COUNT];
        rom[before_pc..before_pc + REVEAL_COUNT].copy_from_slice(&before);
        rom[after_pc..after_pc + REVEAL_COUNT].copy_from_slice(&after);

        let list = RevealTileList::parse(&rom, 0).unwrap();
        assert_eq!(list.before, before);
        assert_eq!(list.after, after);

        let mut edited = list.clone();
        edited.before[0] = 0x2E;
        edited.after[0] = 0x77;
        edited.apply_to_rom(&mut rom, 0).unwrap();
        assert_eq!(rom[after_pc], 0x77);
        // The edited bytes were written in place; untouched neighbors stay put.
        assert_eq!(rom[before_pc], 0x2E);
        assert_eq!(rom[before_pc + 1], 0x6F);
        assert_eq!(rom[after_pc + 1], 0x67);

        let again = RevealTileList::parse(&rom, 0).unwrap();
        assert_eq!(again, edited);
    }

    #[test]
    fn wrong_length_refused() {
        let mut list = RevealTileList { before: vec![0u8; REVEAL_COUNT], after: vec![0u8; REVEAL_COUNT] };
        list.before.pop();
        let mut rom = vec![0u8; 0x30000];
        assert!(list.apply_to_rom(&mut rom, 0).is_err());
    }

    #[test]
    fn events_using_row_matches_offsets() {
        let list = RevealTileList { before: vec![0x2E, 0xAA], after: vec![0x0E, 0xBB] };
        // offsets: event 0 -> tile 5 (holds 0x2E), event 1 -> tile 9 (holds 0x00)
        let mut tiles = vec![0u8; 16];
        tiles[5] = 0x2E;
        let using = list.events_using_row(0, &[5, 9], &tiles);
        assert_eq!(using, vec![0]);
        assert!(list.events_using_row(1, &[5, 9], &tiles).is_empty());
        assert!(list.events_using_row(99, &[5, 9], &tiles).is_empty());
    }

    /// Parse the real ROM and check the lists against the byte-verified
    /// transcription. Run with `ROM_PATH=/path/to/smw.smc cargo test -p
    /// smwe-rom --lib -- --ignored real_rom_reveal_lists`.
    #[test]
    #[ignore]
    fn real_rom_reveal_lists() {
        let rom_path = std::env::var("ROM_PATH").expect("set ROM_PATH");
        let raw = std::fs::read(rom_path).expect("read ROM");
        let rom_bytes = if raw.len() % 0x400 == 0x200 { raw[0x200..].to_vec() } else { raw };

        let list = RevealTileList::parse(&rom_bytes, 0).expect("reveal list parse");
        let (before, after) = vanilla_lists();
        assert_eq!(list.before, before, "real ROM before-list differs from transcription");
        assert_eq!(list.after, after, "real ROM after-list differs from transcription");
        // Byte-identical write-back on an untouched table.
        let mut copy = rom_bytes.clone();
        list.apply_to_rom(&mut copy, 0).unwrap();
        assert_eq!(copy, rom_bytes);
    }
}
