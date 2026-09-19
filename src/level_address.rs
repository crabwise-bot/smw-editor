//! Lunar Magic v1.11 "Open Level from Address" support.
//!
//! LM's File-menu command takes a *PC address in hex* — an exact headerless
//! ROM offset of a Layer-1 object stream that is absent from the main level
//! pointer table (v1.11 changelog: "useful for viewing some things that aren't
//! in the main level pointer table. Try address 0x30338 for the boss monster
//! test room. ^^").
//!
//! Semantics, per LM's help (`file_open_address.htm`):
//! - the value is an exact ROM offset for Layer 1 data; sprites, entrances
//!   and background are *not* loaded from the address,
//! - the displayed level number remains the preceding ordinary slot,
//! - saving inserts the imported Layer 1 into that ordinary `$000..$1FF` slot,
//! - the raw source pointer/address is neither discovered nor repaired.
//!
//! [`parse_layer1_from_address`] decodes the object stream at a headerless PC
//! address; [`splice_layer1_into_slot`] writes `[primary header][objects]` into
//! a scratch ROM copy and repoints the slot's Layer-1 pointer, so the
//! emulator can decompress the imported data for display.

use anyhow::{anyhow, Context};
use smwe_rom::{
    level::{object_layer::ObjectLayer, PRIMARY_HEADER_SIZE},
    snes_utils::addr::{AddrPc, AddrSnes},
};

use crate::rom_freespace::find_free_space;

/// A Layer-1 object stream decoded from a raw ROM address.
pub struct ImportedLayer1 {
    /// The parsed object layer (raw bytes include the `$FF` terminator).
    pub layer:          ObjectLayer,
    /// Number of bytes consumed from the ROM, including the `$FF` terminator.
    pub bytes_consumed: usize,
}

/// Decode the Layer-1 object stream at headerless PC address `pc`.
///
/// Returns an error when `pc` is outside the ROM or when no `$FF`-terminated
/// object stream starts there. An address pointing at a lone `$FF` yields an
/// empty layer (0 objects), matching LM (it simply shows nothing).
pub fn parse_layer1_from_address(rom: &[u8], pc: u32) -> anyhow::Result<ImportedLayer1> {
    let data = rom
        .get(pc as usize..)
        .with_context(|| format!("PC address 0x{pc:X} is beyond the end of the ROM (0x{:X} bytes)", rom.len()))?;
    let (_rest, (layer, consumed)) = ObjectLayer::parse(data)
        .map_err(|_| anyhow!("No Layer-1 object stream at PC 0x{pc:X}: no $FF terminator found"))?;
    Ok(ImportedLayer1 { layer, bytes_consumed: consumed })
}

/// Splice `l1_bytes` (including the `$FF` terminator) into `rom` as the
/// Layer-1 block of `level_num`, reusing that level's `primary_header`.
///
/// `rom` is the *headerless* ROM image and is modified in place, mirroring the
/// level editor's save path: the block is written in place when it fits the
/// old block, otherwise the old block is erased (`$FF`-filled) and the level
/// pointer-table entry (`$05E000`, 3-byte LoROM each) is repointed at free
/// space. The source address of the imported data is never touched.
pub fn splice_layer1_into_slot(
    rom: &mut [u8], level_num: u32, primary_header: &[u8; PRIMARY_HEADER_SIZE], l1_bytes: &[u8],
) -> anyhow::Result<()> {
    let tbl_pc =
        AddrPc::try_from_lorom(AddrSnes(0x05E000)).context("cannot map Layer-1 pointer table to PC")?.as_index();
    let ptr_entry_off =
        tbl_pc.checked_add(level_num as usize * 3).context("Layer-1 pointer table entry overflows ROM")?;
    let ptr_off = rom
        .get(ptr_entry_off..ptr_entry_off + 3)
        .with_context(|| format!("Layer-1 pointer table entry for level {level_num:03X} out of range"))?;
    let old_snes = u32::from_le_bytes([ptr_off[0], ptr_off[1], ptr_off[2], 0]);
    let old_pc = AddrPc::try_from_lorom(AddrSnes(old_snes)).context("cannot map old Layer-1 address to PC")?.as_index();

    // Current block size: 5-byte header + existing object bytes.
    let old_block = {
        let data = rom
            .get(old_pc + PRIMARY_HEADER_SIZE..)
            .with_context(|| format!("Layer-1 data for level {level_num:03X} out of range (PC 0x{old_pc:X})"))?;
        let (_rest, (existing, _)) = ObjectLayer::parse(data)
            .map_err(|_| anyhow!("Existing Layer-1 data for level {level_num:03X} has no $FF terminator"))?;
        PRIMARY_HEADER_SIZE + existing.as_bytes().len()
    };
    let new_block = PRIMARY_HEADER_SIZE + l1_bytes.len();

    let dest = if new_block <= old_block {
        old_pc
    } else {
        let pc = find_free_space(rom, new_block, 0x008000, 0)
            .with_context(|| format!("No free space for level {level_num:03X} layer 1 ({new_block} bytes)"))?;
        rom.get_mut(old_pc..old_pc + old_block)
            .with_context(|| format!("Old Layer-1 block for level {level_num:03X} out of range"))?
            .fill(0xFF);
        let snes = AddrSnes::try_from_lorom(AddrPc(pc as u32)).context("cannot map new Layer-1 address to SNES")?.0;
        let ptr = rom.get_mut(ptr_entry_off..ptr_entry_off + 3).context("pointer table entry out of range")?;
        ptr.copy_from_slice(&snes.to_le_bytes()[..3]);
        pc
    };

    let dest_end = dest.checked_add(new_block).context("Layer-1 block overflows ROM")?;
    let block = rom.get_mut(dest..dest_end).context("Layer-1 destination out of range")?;
    block[..PRIMARY_HEADER_SIZE].copy_from_slice(primary_header);
    block[PRIMARY_HEADER_SIZE..].copy_from_slice(l1_bytes);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a minimal synthetic headerless ROM: pointer table at LoROM
    /// `$05E000` (PC `0x2E000`) with level 0's Layer-1 at PC `0x40000`.
    fn synthetic_rom() -> Vec<u8> {
        let mut rom = vec![0xFFu8; 0x80000];
        // Level 0 Layer-1 pointer -> SNES $888000 -> PC 0x40000.
        let tbl = 0x2E000;
        rom[tbl] = 0x00;
        rom[tbl + 1] = 0x80;
        rom[tbl + 2] = 0x88;
        // [5-byte header][two objects][$FF].
        rom[0x40000..0x40005].copy_from_slice(&[0x37, 0x3A, 0x35, 0x00, 0x00]);
        rom[0x40005..0x4000C].copy_from_slice(&[0x10, 0x20, 0x30, 0x40, 0x50, 0x60, 0xFF]);
        rom
    }

    #[test]
    fn parses_stream_at_address() {
        let rom = synthetic_rom();
        let imported = parse_layer1_from_address(&rom, 0x40005).unwrap();
        assert_eq!(imported.layer.objects().len(), 2);
        assert_eq!(imported.bytes_consumed, 7);
        assert_eq!(imported.layer.as_bytes(), &[0x10, 0x20, 0x30, 0x40, 0x50, 0x60, 0xFF]);
    }

    #[test]
    fn lone_ff_yields_empty_layer() {
        let rom = synthetic_rom();
        let imported = parse_layer1_from_address(&rom, 0x4000B).unwrap();
        assert_eq!(imported.layer.objects().len(), 0);
        assert_eq!(imported.bytes_consumed, 1);
    }

    #[test]
    fn rejects_address_beyond_rom() {
        let rom = synthetic_rom();
        assert!(parse_layer1_from_address(&rom, 0x80000).is_err());
    }

    #[test]
    fn rejects_unterminated_stream() {
        let mut rom = vec![0x00u8; 0x1000];
        rom[0x100..0x106].copy_from_slice(&[0x10, 0x20, 0x30, 0x40, 0x50, 0x60]);
        assert!(parse_layer1_from_address(&rom, 0x100).is_err());
    }

    #[test]
    fn splice_repoints_when_bigger() {
        let mut rom = synthetic_rom();
        // New data (11 bytes) > old block (5 + 7 = 12)? No: make it bigger.
        let new_l1 = vec![0x01u8; 20];
        let mut new_l1 = new_l1;
        new_l1.push(0xFF);
        let header = [0x37, 0x3A, 0x35, 0x00, 0x00];
        splice_layer1_into_slot(&mut rom, 0, &header, &new_l1).unwrap();
        // Old block erased, pointer moved to free space (>= 0x8000).
        assert!(rom[0x40000..0x4000C].iter().all(|&b| b == 0xFF));
        let tbl = 0x2E000;
        let new_snes = u32::from_le_bytes([rom[tbl], rom[tbl + 1], rom[tbl + 2], 0]);
        let new_pc = AddrPc::try_from_lorom(AddrSnes(new_snes)).unwrap().as_index();
        assert_ne!(new_pc, 0x40000);
        assert_eq!(&rom[new_pc..new_pc + 5], &header);
        assert_eq!(&rom[new_pc + 5..new_pc + 5 + new_l1.len()], &new_l1);
    }

    #[test]
    fn splice_writes_in_place_when_smaller() {
        let mut rom = synthetic_rom();
        let new_l1 = vec![0xAA, 0xBB, 0xCC, 0xFF];
        let header = [0x37, 0x3A, 0x35, 0x00, 0x00];
        splice_layer1_into_slot(&mut rom, 0, &header, &new_l1).unwrap();
        // Pointer untouched.
        assert_eq!(&rom[0x2E000..0x2E003], &[0x00, 0x80, 0x88]);
        assert_eq!(&rom[0x40000..0x40005], &header);
        assert_eq!(&rom[0x40005..0x40009], &new_l1);
    }

    /// Real-ROM fixture: FuSoYa's own v1.11 example, "Try address 0x30338 for
    /// the boss monster test room."
    ///
    /// Run with `ROM_PATH=/path/to/smw.smc cargo test -p smw-editor --lib -- --ignored`
    #[test]
    #[ignore]
    fn real_rom_boss_test_room_decodes() {
        let rom_path = std::env::var("ROM_PATH").expect("set ROM_PATH");
        let rom = std::fs::read(rom_path).expect("cannot read ROM");
        let imported = parse_layer1_from_address(&rom, 0x30338).unwrap();
        assert_eq!(imported.bytes_consumed, 100); // 33 objects + $FF
        assert_eq!(imported.layer.objects().len(), 33);
        assert_eq!(*imported.layer.as_bytes().last().unwrap(), 0xFF);
    }

    /// Real-ROM edge case: 0x30263 starts with a lone `$FF` (end of the
    /// preceding leftover stream) — must decode to an empty layer, not an
    /// error.
    ///
    /// Run with `ROM_PATH=/path/to/smw.smc cargo test -p smw-editor --lib -- --ignored`
    #[test]
    #[ignore]
    fn real_rom_empty_stream_is_not_an_error() {
        let rom_path = std::env::var("ROM_PATH").expect("set ROM_PATH");
        let rom = std::fs::read(rom_path).expect("cannot read ROM");
        assert_eq!(rom[0x30263], 0xFF);
        let imported = parse_layer1_from_address(&rom, 0x30263).unwrap();
        assert_eq!(imported.layer.objects().len(), 0);
        assert_eq!(imported.bytes_consumed, 1);
    }
}
