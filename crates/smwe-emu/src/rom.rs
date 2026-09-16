#![allow(clippy::identity_op)]

//! Storage for ROM files, mapper support, etc.

use std::collections::HashMap;

#[derive(Debug, Copy, Clone)]
pub enum Mapper {
    NoRom,
    LoRom,
    HiRom,
    /// LoROM for banks $00-$7D plus a second 4 MiB window at $80-$FF:$8000-$FFFF.
    ExLoRom,
    /// HiROM for banks $40-$7D/$C0-$FF plus a second 4 MiB window at $80-$BF:$0000-$FFFF.
    ExHiRom,
}

impl Mapper {
    pub fn map_to_file(&self, addr: usize) -> Option<usize> {
        match self {
            Mapper::NoRom => Some(addr),
            Mapper::LoRom => {
                if (addr&0xFE0000)==0x7E0000        //wram
                || (addr&0x408000)==0x000000        //hardware regs, ram mirrors, other strange junk
                || (addr&0x708000)==0x700000
                {
                    //sram (low parts of banks 70-7D)
                    None
                } else {
                    Some((addr & 0x7F0000) >> 1 | (addr & 0x7FFF))
                }
            }
            Mapper::HiRom => {
                if (addr&0xFE0000)==0x7E0000       //wram
                || (addr&0x408000)==0x000000
                {
                    //hardware regs, ram mirrors, other strange junk
                    None
                } else {
                    Some(addr & 0x3FFFFF)
                }
            }
            Mapper::ExLoRom => {
                let bank = (addr >> 16) & 0xFF;
                if bank >= 0x80 && (addr & 0x8000) != 0 {
                    // Upper 4 MiB window (replaces the LoROM mirror).
                    Some(0x400000 | ((addr & 0x7F0000) >> 1 | (addr & 0x7FFF)))
                } else if (addr & 0xFE0000) == 0x7E0000
                    || (addr & 0x408000) == 0x000000
                    || (addr & 0x708000) == 0x700000
                {
                    None
                } else {
                    Some((addr & 0x7F0000) >> 1 | (addr & 0x7FFF))
                }
            }
            Mapper::ExHiRom => {
                let bank = (addr >> 16) & 0xFF;
                if (0x80..=0xBF).contains(&bank) {
                    // Upper 4 MiB window (replaces the HiROM mirror).
                    Some(0x400000 | (addr & 0x3FFFFF))
                } else if (addr & 0xFE0000) == 0x7E0000 || (addr & 0x408000) == 0x000000 {
                    None
                } else {
                    Some(addr & 0x3FFFFF)
                }
            }
        }
    }

    pub fn map_to_addr(&self, offset: usize) -> usize {
        match self {
            Mapper::NoRom => offset,
            Mapper::LoRom => {
                let in_bank = offset & 0x7FFF;
                let bank = offset >> 15;
                (bank << 16) + in_bank + 0x8000
            }
            Mapper::HiRom => offset | 0xC00000,
            Mapper::ExLoRom => {
                if offset < 0x400000 {
                    let in_bank = offset & 0x7FFF;
                    let bank = offset >> 15;
                    (bank << 16) + in_bank + 0x8000
                } else {
                    let bank = 0x80 + ((offset >> 15) & 0x7F);
                    (bank << 16) | 0x8000 | (offset & 0x7FFF)
                }
            }
            Mapper::ExHiRom => {
                if offset < 0x400000 {
                    offset | 0xC00000
                } else {
                    0x800000 | (offset & 0x3FFFFF)
                }
            }
        }
    }
}

#[derive(Debug, Clone)]
pub struct Rom {
    buf:     Vec<u8>,
    mapper:  Mapper,
    symbols: HashMap<String, u32>,
}

/// Detect the cartridge mapper from an *unheadered* ROM buffer by validating the
/// SNES internal header's checksum/complement pair at the LoROM and HiROM
/// locations, then refining with the header's map-mode byte. This mirrors the
/// heuristic used by real emulators and by `smwe-rom`'s header parser, so an
/// expanded LoROM hack (e.g. TOP2020), a HiROM hack, an ExLoROM/ExHiROM image,
/// and SA-1 packs (Mode 23/25) all map correctly instead of everything being
/// forced to LoROM. SA-1 ROMs expose their cartridge ROM to the S-CPU with
/// plain LoROM/HiROM bus addressing, so they map with the base layout.
pub fn detect_mapper(buf: &[u8]) -> Mapper {
    // Complement at header+0x1C, checksum at header+0x1E (little-endian u16s).
    let valid_at = |base: usize| -> bool {
        match (buf.get(base + 0x1C..base + 0x1E), buf.get(base + 0x1E..base + 0x20)) {
            (Some(cpl), Some(csm)) => {
                let cpl = u16::from_le_bytes([cpl[0], cpl[1]]);
                let csm = u16::from_le_bytes([csm[0], csm[1]]);
                (cpl ^ csm) == 0xFFFF
            }
            _ => false,
        }
    };
    // Map-mode byte sits at header+0x15; its low nibble is Nintendo's
    // "Mode 2x" number: 0=LoROM, 1=HiROM, 2=ExLoROM, 3=SA-1 LoROM,
    // 4=ExHiROM, 5=SA-1 HiROM.
    let lo_ok = valid_at(0x7FC0);
    let hi_ok = valid_at(0xFFC0);
    let lo_mode = buf.get(0x7FD5).copied();
    let hi_mode = buf.get(0xFFD5).copied();
    let mapper = match (lo_ok, hi_ok) {
        (true, false) => lo_mode.map_or(Mapper::LoRom, mapper_for_mode),
        (false, true) => hi_mode.map_or(Mapper::HiRom, mapper_for_mode),
        (true, true) => {
            // Both checksums validate (rare); prefer an explicit Ex map mode,
            // then fall back to the LoROM header's declared mode.
            match (lo_mode, hi_mode) {
                (Some(m), _) if m & 0x0F == 0x02 => Mapper::ExLoRom,
                (_, Some(m)) if m & 0x0F == 0x04 => Mapper::ExHiRom,
                (Some(m), _) => mapper_for_mode(m),
                _ => Mapper::LoRom,
            }
        }
        (false, false) => {
            log::warn!("Could not validate ROM checksum at LoROM or HiROM header; assuming LoROM");
            Mapper::LoRom
        }
    };
    // SA-1 ($33-$36) and SuperFX ($13-$16) chip declarations. SA-1 now maps
    // with its base LoROM/HiROM layout (the S-CPU bus view); SuperFX mapping
    // is still unsupported, so warn to explain garbled output.
    if let Some(&rom_type) = buf.get(0x7FD6).filter(|_| lo_ok).or_else(|| buf.get(0xFFD6).filter(|_| hi_ok)) {
        match rom_type & 0xF0 {
            0x30 => log::info!("ROM declares SA-1 chip ($33-$36); mapping with base {mapper:?} layout"),
            0x10 => log::warn!("ROM declares SuperFX; this mapper is not supported"),
            _ => {}
        }
    }
    log::info!("Detected cartridge mapper: {mapper:?}");
    mapper
}

/// Map a map-mode byte to the emulator `Mapper`. SA-1 packs (Mode 23/25)
/// expose their ROM to the S-CPU with plain LoROM/HiROM bus addressing.
fn mapper_for_mode(mode: u8) -> Mapper {
    match mode & 0x0F {
        0x02 => Mapper::ExLoRom,
        0x04 => Mapper::ExHiRom,
        0x01 | 0x05 => Mapper::HiRom,
        _ => Mapper::LoRom,
    }
}

impl Rom {
    /// Construct a ROM, auto-detecting the mapper from the (unheadered) buffer.
    pub fn new(buf: Vec<u8>) -> Self {
        let mapper = detect_mapper(&buf);
        Self { buf, mapper, symbols: HashMap::new() }
    }

    /// Construct a ROM with an explicit mapper, bypassing auto-detection.
    pub fn new_with_mapper(buf: Vec<u8>, mapper: Mapper) -> Self {
        Self { buf, mapper, symbols: HashMap::new() }
    }

    pub fn set_mapper(&mut self, mapper: Mapper) {
        self.mapper = mapper;
    }

    pub fn load_symbols(&mut self, data: &str) {
        for i in data.lines() {
            let i = if let Some(comment) = i.find(';') { &i[..comment] } else { i }.trim();
            if i.is_empty() {
                continue;
            }
            if let Some(v) = i.find(' ') {
                match u32::from_str_radix(&i[..v], 16) {
                    Ok(addr) => {
                        self.symbols.insert(i[v + 1..].to_string(), addr);
                    }
                    Err(_e) => {}
                }
            }
        }
    }

    pub fn resolve(&self, symbol: &str) -> Option<u32> {
        self.symbols.get(symbol).copied()
    }

    pub fn read(&self, addr: u32) -> Option<u8> {
        self.mapper.map_to_file(addr as _).and_then(|c| self.buf.get(c).copied())
    }

    pub fn read_u16(&self, addr: u32) -> Option<u16> {
        Some(u16::from_le_bytes([self.read(addr + 0)?, self.read(addr + 1)?]))
    }

    pub fn read_u32(&self, addr: u32) -> Option<u32> {
        Some(u32::from_le_bytes([self.read(addr + 0)?, self.read(addr + 1)?, self.read(addr + 2)?, 0]))
    }

    pub fn resize(&mut self, new_size: usize) {
        self.buf.resize(new_size, 0);
    }

    pub fn mapper(&self) -> Mapper {
        self.mapper
    }

    pub fn as_slice(&self) -> &[u8] {
        &self.buf
    }

    pub fn checksum(&self) -> u16 {
        let size = self.buf.len();
        if size == 0 {
            return 0;
        }
        let base: u16 = self.buf.iter().map(|&b| b as u16).sum();
        if size.is_power_of_two() {
            base
        } else {
            // Mirror the trailing non-power-of-2 portion to fill the gap, matching
            // what a real SNES cartridge exposes on the bus.
            let po2 = size.next_power_of_two() / 2;
            let remainder = size - po2;
            let mirror_sum: u16 = self.buf[po2..po2 + remainder].iter().map(|&b| b as u16).sum();
            base.wrapping_add(mirror_sum)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a minimal ROM buffer with a valid checksum/complement pair written
    /// at the given header location, plus an optional map-mode/rom-type byte.
    fn rom_with_header(header_base: usize, map_mode: u8, rom_type: u8) -> Vec<u8> {
        let mut buf = vec![0u8; 0x10000];
        // Arbitrary checksum; complement is its bitwise inverse so cpl ^ csm == 0xFFFF.
        let checksum: u16 = 0x1234;
        let complement = !checksum;
        buf[header_base + 0x15] = map_mode;
        buf[header_base + 0x16] = rom_type;
        buf[header_base + 0x1C..header_base + 0x1E].copy_from_slice(&complement.to_le_bytes());
        buf[header_base + 0x1E..header_base + 0x20].copy_from_slice(&checksum.to_le_bytes());
        buf
    }

    #[test]
    fn detects_lorom() {
        let buf = rom_with_header(0x7FC0, 0x20, 0x00);
        assert!(matches!(detect_mapper(&buf), Mapper::LoRom));
    }

    #[test]
    fn detects_hirom() {
        let buf = rom_with_header(0xFFC0, 0x21, 0x00);
        assert!(matches!(detect_mapper(&buf), Mapper::HiRom));
    }

    #[test]
    fn defaults_to_lorom_without_valid_checksum() {
        let buf = vec![0u8; 0x10000];
        assert!(matches!(detect_mapper(&buf), Mapper::LoRom));
    }

    #[test]
    fn detects_exlorom_and_exhirom() {
        let buf = rom_with_header(0x7FC0, 0x22, 0x02);
        assert!(matches!(detect_mapper(&buf), Mapper::ExLoRom));
        let buf = rom_with_header(0x7FC0, 0x32, 0x02);
        assert!(matches!(detect_mapper(&buf), Mapper::ExLoRom));
        let buf = rom_with_header(0xFFC0, 0x24, 0x02);
        assert!(matches!(detect_mapper(&buf), Mapper::ExHiRom));
        let buf = rom_with_header(0xFFC0, 0x34, 0x02);
        assert!(matches!(detect_mapper(&buf), Mapper::ExHiRom));
    }

    #[test]
    fn detects_sa1_with_base_layout() {
        // SA-1 LoROM (Mode 23): the S-CPU sees plain LoROM bus addressing.
        let buf = rom_with_header(0x7FC0, 0x23, 0x34);
        assert!(matches!(detect_mapper(&buf), Mapper::LoRom));
        let buf = rom_with_header(0x7FC0, 0x33, 0x33);
        assert!(matches!(detect_mapper(&buf), Mapper::LoRom));
        // SA-1 HiROM (Mode 25): plain HiROM bus addressing.
        let buf = rom_with_header(0xFFC0, 0x25, 0x35);
        assert!(matches!(detect_mapper(&buf), Mapper::HiRom));
        let buf = rom_with_header(0xFFC0, 0x35, 0x36);
        assert!(matches!(detect_mapper(&buf), Mapper::HiRom));
    }

    #[test]
    fn exlorom_maps_both_windows() {
        let m = Mapper::ExLoRom;
        // Lower 4 MiB like LoROM.
        assert_eq!(m.map_to_file(0x008000), Some(0x000000));
        assert_eq!(m.map_to_file(0x7DFFFF), Some(0x3EFFFF));
        // Upper 4 MiB window replaces the LoROM mirror at $80-$FF.
        assert_eq!(m.map_to_file(0x808000), Some(0x400000));
        assert_eq!(m.map_to_file(0xFFFFFF), Some(0x7FFFFF));
        // Junk/WRAM/SRAM still excluded.
        assert_eq!(m.map_to_file(0x7E8000), None);
        assert_eq!(m.map_to_file(0x001234), None);
        // Inverse mapping round-trips.
        for off in [0x000000usize, 0x123456, 0x3EFFFF, 0x400000, 0x5ABCDE, 0x7FFFFF] {
            assert_eq!(m.map_to_file(m.map_to_addr(off)), Some(off), "off={off:#x}");
        }
        assert_eq!(m.map_to_addr(0x400000), 0x808000);
        assert_eq!(m.map_to_addr(0x7FFFFF), 0xFFFFFF);
    }

    #[test]
    fn exhirom_maps_both_windows() {
        let m = Mapper::ExHiRom;
        // Lower 4 MiB like HiROM.
        assert_eq!(m.map_to_file(0xC00000), Some(0x000000));
        assert_eq!(m.map_to_file(0xFFFFFF), Some(0x3FFFFF));
        // Upper 4 MiB window replaces the HiROM mirror at $80-$BF.
        assert_eq!(m.map_to_file(0x800000), Some(0x400000));
        assert_eq!(m.map_to_file(0xBFFFFF), Some(0x7FFFFF));
        assert_eq!(m.map_to_file(0x9ABCDE), Some(0x5ABCDE));
        // Junk/WRAM still excluded.
        assert_eq!(m.map_to_file(0x7E0000), None);
        assert_eq!(m.map_to_file(0x001234), None);
        for off in [0x000000usize, 0x123456, 0x3FFFFF, 0x400000, 0x5ABCDE, 0x7FFFFF] {
            assert_eq!(m.map_to_file(m.map_to_addr(off)), Some(off), "off={off:#x}");
        }
        assert_eq!(m.map_to_addr(0x400000), 0x800000);
        assert_eq!(m.map_to_addr(0x7FFFFF), 0xBFFFFF);
    }
}
