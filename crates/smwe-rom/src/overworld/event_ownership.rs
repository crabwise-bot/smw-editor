//! Event ownership table: which overworld event each level triggers on completion.
//!
//! SNES `$05D608` (PC `0x2D608`), 93 bytes — one per translevel (`0x00`–`0x5C`,
//! the same index space as [`crate::overworld::level_names::LEVEL_NAMES_COUNT`]).
//! Each byte is the event number (`0..OW_EVENT_COUNT`) that the game writes to
//! WRAM `OverworldEvent` when that level is beaten (`bank_05.asm`:
//! `LDY.W TranslevelNo` / `LDA.W DATA_05D608,Y` / `STA.W OverworldEvent`).
//! `$FF` means the level triggers no event.
//!
//! This is the "event ownership" the parity doc tracks: Lunar Magic shows the
//! reveal-tile preview per event, but offers no UI for choosing *which* level
//! (or action) *triggers* which event. Editing here is an in-place,
//! one-byte-per-level write — no relocation patch needed.

use crate::overworld::OW_EVENT_COUNT;
use crate::snes_utils::addr::{AddrPc, AddrSnes};

/// SNES address of the events-by-translevel table (`DATA_05D608`).
pub const EVENT_OWNERSHIP_SNES: AddrSnes = AddrSnes(0x05D608);

/// One byte per translevel (`0x00`–`0x5C`), matching `LEVEL_NAMES_COUNT`.
pub const EVENT_OWNERSHIP_COUNT: usize = 93;

/// Table value meaning "this level triggers no event".
pub const NO_EVENT: u8 = 0xFF;

/// The 93-byte events-by-translevel table decoded from the ROM.
#[derive(Debug, Clone)]
pub struct EventOwnership {
    /// Raw table bytes, index = translevel (`0x00`–`0x5C`).
    pub table: Vec<u8>,
}

impl EventOwnership {
    /// Parse the table from ROM bytes (`header_offset` = `0x200` if the ROM
    /// has an SMC header, else `0`).
    pub fn parse(rom: &[u8], header_offset: usize) -> anyhow::Result<Self> {
        let pc = AddrPc::try_from_lorom(EVENT_OWNERSHIP_SNES)
            .map_err(|e| anyhow::anyhow!("EventOwnership addr conversion: {e}"))?;
        let start = pc.0 as usize + header_offset;
        let end = start + EVENT_OWNERSHIP_COUNT;
        let table = rom
            .get(start..end)
            .ok_or_else(|| anyhow::anyhow!("Event ownership table extends past end of ROM"))?
            .to_vec();
        Ok(Self { table })
    }

    /// The event triggered when `translevel` is beaten, or `None` for `$FF`
    /// (no event). Returns `Err` for out-of-range translevels.
    pub fn event_for(&self, translevel: usize) -> anyhow::Result<Option<u8>> {
        let byte = self
            .table
            .get(translevel)
            .copied()
            .ok_or_else(|| anyhow::anyhow!("translevel {translevel:#04X} out of range (0..{EVENT_OWNERSHIP_COUNT})"))?;
        Ok(if byte == NO_EVENT { None } else { Some(byte) })
    }

    /// Assign (or clear, with `None` → `$FF`) the event for `translevel`.
    ///
    /// Rejects event numbers `>= OW_EVENT_COUNT` (only `$FF` is the sentinel;
    /// no other value above the event count is meaningful to the game).
    pub fn set_event(&mut self, translevel: usize, event: Option<u8>) -> anyhow::Result<()> {
        let slot = self
            .table
            .get_mut(translevel)
            .ok_or_else(|| anyhow::anyhow!("translevel {translevel:#04X} out of range (0..{EVENT_OWNERSHIP_COUNT})"))?;
        match event {
            None => *slot = NO_EVENT,
            Some(e) if (e as usize) < OW_EVENT_COUNT => *slot = e,
            Some(e) => {
                anyhow::bail!("event {e:#04X} out of range (0..{OW_EVENT_COUNT:#04X}, or clear for no event)")
            }
        }
        Ok(())
    }

    /// Write the table back into `rom_bytes` in place
    /// (`header_offset` = `0x200` if the ROM has an SMC header, else `0`).
    pub fn apply_to_rom(&self, rom_bytes: &mut [u8], header_offset: usize) -> anyhow::Result<()> {
        anyhow::ensure!(
            self.table.len() == EVENT_OWNERSHIP_COUNT,
            "event ownership table has {} entries, need {EVENT_OWNERSHIP_COUNT}",
            self.table.len()
        );
        let pc = AddrPc::try_from_lorom(EVENT_OWNERSHIP_SNES)
            .map_err(|e| anyhow::anyhow!("EventOwnership addr conversion: {e}"))?;
        let start = pc.0 as usize + header_offset;
        let dst = rom_bytes
            .get_mut(start..start + EVENT_OWNERSHIP_COUNT)
            .ok_or_else(|| anyhow::anyhow!("Event ownership table write range out of bounds"))?;
        dst.copy_from_slice(&self.table);
        Ok(())
    }
}

/// Vanilla table from SMWDisX `bank_05.asm` `DATA_05D608` (93 bytes).
#[cfg(test)]
pub(crate) fn vanilla_table() -> Vec<u8> {
    vec![
        0xFF, 0x1F, 0x20, 0xFF, 0x0B, 0x0D, 0x0E, 0x0F, 0x28, 0x09, 0x10, 0x21, 0x22, 0x23, 0x24, 0x25,
        0x27, 0x60, 0xFF, 0x12, 0x02, 0x07, 0xFF, 0xFF, 0x4E, 0xFF, 0x4D, 0x4A, 0x4C, 0x4B, 0x36, 0x35,
        0x61, 0x63, 0x62, 0x48, 0x46, 0x06, 0x05, 0x04, 0x00, 0x01, 0x03, 0x19, 0xFF, 0x1D, 0x1A, 0x14,
        0x44, 0x45, 0x42, 0x3E, 0x40, 0x41, 0x43, 0x3D, 0x3B, 0x39, 0x38, 0x4F, 0x17, 0x1B, 0x15, 0x29,
        0x1C, 0x30, 0x2A, 0x32, 0x2C, 0x37, 0x34, 0x2E, 0x6D, 0x6C, 0x6B, 0x6A, 0x69, 0x64, 0x65, 0x66,
        0x67, 0x68, 0x56, 0x53, 0x54, 0x5F, 0x57, 0x59, 0x51, 0x5A, 0x5D, 0x50, 0x5C,
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn synthetic_rom() -> Vec<u8> {
        // 0x40000 bytes so the table fits at PC 0x2D608 with no header.
        let pc = AddrPc::try_from_lorom(EVENT_OWNERSHIP_SNES).unwrap().0 as usize;
        let mut rom = vec![0u8; pc + EVENT_OWNERSHIP_COUNT + 16];
        rom[pc..pc + EVENT_OWNERSHIP_COUNT].copy_from_slice(&vanilla_table());
        rom
    }

    #[test]
    fn parse_reads_table_at_05d608() {
        let rom = synthetic_rom();
        let eo = EventOwnership::parse(&rom, 0).unwrap();
        assert_eq!(eo.table, vanilla_table());
        assert_eq!(eo.table.len(), EVENT_OWNERSHIP_COUNT);
    }

    #[test]
    fn parse_accounts_for_smc_header() {
        let mut rom = vec![0u8; 0x200];
        rom.extend(synthetic_rom());
        let eo = EventOwnership::parse(&rom, 0x200).unwrap();
        assert_eq!(eo.table, vanilla_table());
    }

    #[test]
    fn known_vanilla_entries() {
        let eo = EventOwnership { table: vanilla_table() };
        // Translevel 0x00 triggers no event; 0x29 (YOSHI'S ISLAND 1) triggers event 1.
        assert_eq!(eo.event_for(0x00).unwrap(), None);
        assert_eq!(eo.event_for(0x29).unwrap(), Some(0x01));
        assert_eq!(eo.event_for(0x5C).unwrap(), Some(0x5C));
    }

    #[test]
    fn every_vanilla_value_is_no_event_or_valid() {
        let eo = EventOwnership { table: vanilla_table() };
        for (tl, &b) in eo.table.iter().enumerate() {
            assert!(
                b == NO_EVENT || (b as usize) < OW_EVENT_COUNT,
                "translevel {tl:#04X} has out-of-range event {b:#04X}"
            );
        }
    }

    #[test]
    fn set_event_assign_and_clear() {
        let mut eo = EventOwnership { table: vanilla_table() };
        eo.set_event(0x29, Some(0x42)).unwrap();
        assert_eq!(eo.event_for(0x29).unwrap(), Some(0x42));
        eo.set_event(0x29, None).unwrap();
        assert_eq!(eo.event_for(0x29).unwrap(), None);
        assert_eq!(eo.table[0x29], NO_EVENT);
    }

    #[test]
    fn set_event_rejects_out_of_range() {
        let mut eo = EventOwnership { table: vanilla_table() };
        let err = eo.set_event(0x29, Some(OW_EVENT_COUNT as u8)).unwrap_err();
        assert!(err.to_string().contains("out of range"), "unexpected: {err}");
        let err = eo.set_event(EVENT_OWNERSHIP_COUNT, Some(0x01)).unwrap_err();
        assert!(err.to_string().contains("out of range"), "unexpected: {err}");
        // $FF cannot be assigned as an event number; use None to clear.
        let err = eo.set_event(0x29, Some(0xFF)).unwrap_err();
        assert!(err.to_string().contains("out of range"), "unexpected: {err}");
    }

    #[test]
    fn apply_to_rom_round_trip() {
        let mut eo = EventOwnership { table: vanilla_table() };
        eo.set_event(0x29, Some(0x42)).unwrap();
        eo.set_event(0x00, Some(0x07)).unwrap();
        let mut rom = synthetic_rom();
        eo.apply_to_rom(&mut rom, 0).unwrap();
        let back = EventOwnership::parse(&rom, 0).unwrap();
        assert_eq!(back.event_for(0x29).unwrap(), Some(0x42));
        assert_eq!(back.event_for(0x00).unwrap(), Some(0x07));
        assert_eq!(back.event_for(0x01).unwrap(), Some(0x1F));
    }

    /// Real-ROM check: table parses at `$05D608`, values are all sane, and the
    /// Yoshi's Island 1 assignment matches the disassembly.
    ///
    /// Run with: `ROM_PATH=/path/to/smw.smc cargo test -p smwe-rom --lib -- --ignored`
    #[test]
    #[ignore]
    fn real_rom_event_ownership_table() {
        let path = std::env::var("ROM_PATH").expect("ROM_PATH not set");
        let raw = std::fs::read(path).expect("read ROM");
        let (rom, header_offset) =
            if raw.len() % 0x400 == 0x200 { (&raw[0x200..], 0x200usize) } else { (&raw[..], 0usize) };
        let eo = EventOwnership::parse(rom, header_offset).expect("parse event ownership");
        assert_eq!(eo.table.len(), EVENT_OWNERSHIP_COUNT);
        for (tl, &b) in eo.table.iter().enumerate() {
            assert!(
                b == NO_EVENT || (b as usize) < OW_EVENT_COUNT,
                "translevel {tl:#04X} has out-of-range event {b:#04X}"
            );
        }
        assert_eq!(eo.event_for(0x29).unwrap(), Some(0x01), "Yoshi's Island 1 should trigger event 1");
        assert_eq!(eo.event_for(0x00).unwrap(), None);
        // Spot-check the first 8 bytes against SMWDisX DATA_05D608.
        assert_eq!(&eo.table[..8], &[0xFF, 0x1F, 0x20, 0xFF, 0x0B, 0x0D, 0x0E, 0x0F]);
    }
}
