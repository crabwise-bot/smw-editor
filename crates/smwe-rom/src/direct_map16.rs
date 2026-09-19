// Direct Map16 access (Lunar Magic v1.70–v1.90 parity).
//
// In Lunar Magic, "Add Objects / Direct Map16" lets the user pick a rectangular
// multi-tile Map16 pattern and drop it into the level as one resizable object;
// the pattern repeats when the object is resized. "Conditional Direct Map16"
// attaches a RAM flag (address + bit) to such an object: the game only renders
// it when the flag is set, via Lunar Magic's own ASM hook. "Remap Direct Map16"
// rewrites tile references across a level's Direct Map16 objects.
//
// A stock SMW ROM has no runtime representation for any of this: the object
// stream carries tile *numbers*, and there is no per-object RAM-flag field.
// So this module stores editor-native data exactly like `exanimation.rs` does:
// a documented payload in a standard `STAR` RATS-tagged free-space block. The
// editor stamps the pattern tiles into the level's block map when rendering.
//
// Honest limits (stored in the payload docs below):
// - Rendering the objects in-game needs the Direct Map16 ASM installed in the
//   ROM (Lunar Magic installs it; this editor does not).
// - The conditional flag is inert metadata on a stock ROM: it is parsed,
//   displayed, exported and round-tripped, but the vanilla game never
//   evaluates it.
//
// Payload layout (format version 1):
//
//   0  "SMWDM161"                      magic (8 bytes)
//   8  version                         u8 (= 1)
//   9  level_count                     u16 LE
//   11 per-level records:
//      level                         u16 LE (level number)
//      obj_count                     u16 LE
//      per object:
//        x, y                        u16 LE each (absolute tile coords, level space)
//        w, h                        u8 each (current object size in tiles, 1..=64)
//        pw, ph                      u8 each (stored pattern size, 1..=64)
//        cond_present                u8 (0 = none, 1 = conditional)
//        cond_ram_addr               u16 LE ($7E:xxxx WRAM address; only if cond_present)
//        cond_bit                    u8 (0..=7 = bit, 8 = nonzero byte; only if cond_present)
//        tiles                       pw*ph u16 LE, row-major Map16 block IDs
//
// Pattern repetition: the rendered tile at local (lx, ly) is
// `tiles[(ly % ph) * pw + (lx % pw)]` — resizing the object repeats the pattern.
//
// The RATS block is found by magic scan like the ExAnimation one, with the
// same standard RATS size semantics (`STAR` + size + ~size, size = len - 1).
use std::collections::BTreeMap;

use crate::freespace::find_free_space;

// -------------------------------------------------------------------------------------------------

/// RATS magic identifying a Direct Map16 payload block.
pub const DM16_RATS_MAGIC: &[u8; 8] = b"SMWDM161";
/// Current payload format version.
pub const DM16_FORMAT_VERSION: u8 = 1;
/// Largest allowed object width/height/pattern dimension, in tiles.
pub const DM16_MAX_DIM: u32 = 64;

/// Conditional Direct Map16 flag: render the object only when the given WRAM
/// byte (bank $7E) satisfies the condition.
///
/// Inert metadata on a stock ROM: nothing in the vanilla game evaluates this.
/// It takes effect only when Lunar Magic's conditional Direct Map16 ASM (or
/// equivalent) is installed.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct DirectMap16Condition {
    /// WRAM address (bank $7E), e.g. `0x14AF` for the on/off switch state byte.
    pub ram_addr: u16,
    /// Bit to test: 0..=7. The value 8 means "render when the byte is nonzero".
    pub bit:      u8,
}

impl DirectMap16Condition {
    /// True when the flag should render given the current WRAM byte value.
    pub fn renders_with(&self, byte: u8) -> bool {
        if self.bit >= 8 {
            byte != 0
        } else {
            (byte >> self.bit) & 1 != 0
        }
    }
}

/// One Direct Map16 object: a rectangular Map16 pattern placed as a single
/// resizable level object.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct DirectMap16Object {
    /// Absolute level tile coords of the object's top-left corner.
    pub x:         u32,
    pub y:         u32,
    /// Current object size in tiles (pattern repeats to fill).
    pub w:         u32,
    pub h:         u32,
    /// Stored pattern dimensions in tiles.
    pub pw:        u32,
    pub ph:        u32,
    /// Row-major Map16 block IDs (`pw * ph` entries, 0x000..=0x1FF on vanilla).
    pub tiles:     Vec<u16>,
    /// Optional conditional render flag.
    pub condition: Option<DirectMap16Condition>,
}

fn take<'a>(data: &'a [u8], pos: &mut usize, n: usize) -> Result<&'a [u8], Dm16Error> {
    if *pos + n > data.len() {
        return Err(Dm16Error::Truncated);
    }
    let s = &data[*pos..*pos + n];
    *pos += n;
    Ok(s)
}

impl DirectMap16Object {
    /// The tile rendered at local coords (lx, ly): the pattern repeats when
    /// the object is larger than the pattern (LM "pattern repeats on resize").
    pub fn tile_at(&self, lx: u32, ly: u32) -> u16 {
        debug_assert!(!self.tiles.is_empty());
        debug_assert!(self.pw >= 1 && self.ph >= 1);
        self.tiles[((ly % self.ph) * self.pw + (lx % self.pw)) as usize]
    }

    /// Whether absolute tile (tx, ty) is covered by this object.
    pub fn covers(&self, tx: u32, ty: u32) -> bool {
        tx >= self.x && ty >= self.y && tx < self.x + self.w && ty < self.y + self.h
    }

    fn validate(&self) -> Result<(), Dm16Error> {
        for (name, dim) in [("w", self.w), ("h", self.h), ("pw", self.pw), ("ph", self.ph)] {
            if dim == 0 || dim > DM16_MAX_DIM {
                return Err(Dm16Error::BadDimension(name));
            }
        }
        if self.tiles.len() as u32 != self.pw * self.ph {
            return Err(Dm16Error::BadTileCount);
        }
        if let Some(c) = &self.condition {
            if c.bit > 8 {
                return Err(Dm16Error::BadConditionBit);
            }
        }
        Ok(())
    }

    fn encoded_len(&self) -> usize {
        let cond_len = if self.condition.is_some() { 4 } else { 0 };
        4 + 4 + 1 + self.tiles.len() * 2 + cond_len
    }

    fn encode(&self, out: &mut Vec<u8>) {
        out.extend_from_slice(&(self.x as u16).to_le_bytes());
        out.extend_from_slice(&(self.y as u16).to_le_bytes());
        out.push(self.w as u8);
        out.push(self.h as u8);
        out.push(self.pw as u8);
        out.push(self.ph as u8);
        match &self.condition {
            None => out.push(0),
            Some(c) => {
                out.push(1);
                out.extend_from_slice(&c.ram_addr.to_le_bytes());
                out.push(c.bit);
            }
        }
        for t in &self.tiles {
            out.extend_from_slice(&t.to_le_bytes());
        }
    }

    fn decode(data: &[u8], pos: &mut usize) -> Result<Self, Dm16Error> {
        let x = u16::from_le_bytes(take(data, pos, 2)?.try_into().unwrap()) as u32;
        let y = u16::from_le_bytes(take(data, pos, 2)?.try_into().unwrap()) as u32;
        let w = take(data, pos, 1)?[0] as u32;
        let h = take(data, pos, 1)?[0] as u32;
        let pw = take(data, pos, 1)?[0] as u32;
        let ph = take(data, pos, 1)?[0] as u32;
        let cond_present = take(data, pos, 1)?[0];
        let condition = if cond_present == 0 {
            None
        } else {
            let ram_addr = u16::from_le_bytes(take(data, pos, 2)?.try_into().unwrap());
            let bit = take(data, pos, 1)?[0];
            Some(DirectMap16Condition { ram_addr, bit })
        };
        let tile_count = (pw as usize) * (ph as usize);
        let tile_bytes = take(data, pos, tile_count * 2)?;
        let mut tiles = Vec::with_capacity(tile_count);
        for chunk in tile_bytes.chunks_exact(2) {
            tiles.push(u16::from_le_bytes(chunk.try_into().unwrap()));
        }
        let obj = DirectMap16Object { x, y, w, h, pw, ph, tiles, condition };
        obj.validate()?;
        Ok(obj)
    }
}

// -------------------------------------------------------------------------------------------------

/// Editor-native Direct Map16 storage, keyed by level number.
///
/// This is the in-ROM model; the level editor keeps its own undoable
/// per-level copy and writes it back through here on save.
#[derive(Clone, Debug, Default)]
pub struct DirectMap16Data {
    /// Level number -> Direct Map16 objects for that level.
    pub levels: BTreeMap<u16, Vec<DirectMap16Object>>,
}

// -------------------------------------------------------------------------------------------------

#[derive(Debug, thiserror::Error)]
pub enum Dm16Error {
    #[error("no Direct Map16 RATS block found in ROM")]
    NotFound,
    #[error("truncated Direct Map16 payload")]
    Truncated,
    #[error("unsupported Direct Map16 format version {0}")]
    BadVersion(u8),
    #[error("bad object dimension {0}")]
    BadDimension(&'static str),
    #[error("tile count does not match pattern dimensions")]
    BadTileCount,
    #[error("bad condition bit (must be 0..=8)")]
    BadConditionBit,
    #[error("Direct Map16 payload too large for a RATS block ({0} bytes, max 65536)")]
    TooLarge(usize),
    #[error("no free space large enough for the Direct Map16 block")]
    NoFreeSpace,
}

impl DirectMap16Data {
    /// Objects for a level (empty slice when the level has none).
    pub fn objects_for(&self, level: u16) -> &[DirectMap16Object] {
        self.levels.get(&level).map(Vec::as_slice).unwrap_or(&[])
    }

    /// Encode the payload (without RATS header).
    pub fn encode(&self) -> Vec<u8> {
        let mut len = DM16_RATS_MAGIC.len() + 1 + 2;
        for objects in self.levels.values() {
            len += 2 + 2 + objects.iter().map(|o| o.encoded_len()).sum::<usize>();
        }
        let mut out = Vec::with_capacity(len);
        out.extend_from_slice(DM16_RATS_MAGIC);
        out.push(DM16_FORMAT_VERSION);
        out.extend_from_slice(&(self.levels.len() as u16).to_le_bytes());
        for (level, objects) in &self.levels {
            out.extend_from_slice(&level.to_le_bytes());
            out.extend_from_slice(&(objects.len() as u16).to_le_bytes());
            for obj in objects {
                obj.encode(&mut out);
            }
        }
        out
    }

    /// Decode a payload previously produced by [`Self::encode`].
    pub fn decode(data: &[u8]) -> Result<Self, Dm16Error> {
        let mut pos = 0usize;
        if take(data, &mut pos, 8)? != DM16_RATS_MAGIC {
            return Err(Dm16Error::NotFound);
        }
        let version = take(data, &mut pos, 1)?[0];
        if version != DM16_FORMAT_VERSION {
            return Err(Dm16Error::BadVersion(version));
        }
        let level_count = u16::from_le_bytes(take(data, &mut pos, 2)?.try_into().unwrap()) as usize;
        let mut levels = BTreeMap::new();
        for _ in 0..level_count {
            let level = u16::from_le_bytes(take(data, &mut pos, 2)?.try_into().unwrap());
            let obj_count = u16::from_le_bytes(take(data, &mut pos, 2)?.try_into().unwrap()) as usize;
            let mut objects = Vec::with_capacity(obj_count);
            for _ in 0..obj_count {
                objects.push(DirectMap16Object::decode(data, &mut pos)?);
            }
            levels.insert(level, objects);
        }
        Ok(DirectMap16Data { levels })
    }

    /// Parse the Direct Map16 RATS block from raw ROM bytes.
    pub fn parse(rom: &[u8]) -> Result<Self, Dm16Error> {
        let payload = find_rats_payload(rom)?;
        Self::decode(payload)
    }

    /// Write this data to raw ROM bytes: allocate a fresh RATS block in free
    /// space, then erase the old block and write the new one. Levels with no
    /// objects are dropped; when nothing remains, the old block is simply
    /// erased. Allocation happens before the erase so a failed allocation
    /// cannot destroy the existing block.
    pub fn write_to_rom(&self, rom: &mut [u8], header_offset: usize) -> Result<(), Dm16Error> {
        let mut pruned = self.levels.clone();
        pruned.retain(|_, objs| !objs.is_empty());
        if pruned.is_empty() {
            erase_rats_block(rom);
            return Ok(());
        }
        let payload = DirectMap16Data { levels: pruned }.encode();
        if payload.is_empty() || payload.len() > 0x10000 {
            return Err(Dm16Error::TooLarge(payload.len()));
        }
        let total = 8 + payload.len(); // RATS tag + payload
        let pc = find_free_space(rom, total, 0x008000, header_offset).ok_or(Dm16Error::NoFreeSpace)?;
        erase_rats_block(rom);
        // Standard RATS size semantics: the size field is payload_len - 1.
        let size_field = (payload.len() - 1) as u16;
        let base = pc + header_offset;
        let end = base + total;
        if end > rom.len() {
            return Err(Dm16Error::NoFreeSpace);
        }
        rom[base..base + 4].copy_from_slice(b"STAR");
        rom[base + 4..base + 6].copy_from_slice(&size_field.to_le_bytes());
        rom[base + 6..base + 8].copy_from_slice(&(!size_field).to_le_bytes());
        rom[base + 8..end].copy_from_slice(&payload);
        Ok(())
    }

    /// Apply old->new Map16 tile-ID mappings to every object of one level.
    /// Returns the number of tile references rewritten.
    pub fn remap_level(&mut self, level: u16, mapping: &[(u16, u16)]) -> usize {
        let mut changed = 0;
        if let Some(objects) = self.levels.get_mut(&level) {
            for obj in objects.iter_mut() {
                for tile in obj.tiles.iter_mut() {
                    if let Some(&(_, new)) = mapping.iter().find(|(old, _)| *old == *tile) {
                        *tile = new;
                        changed += 1;
                    }
                }
            }
        }
        changed
    }
}

// -------------------------------------------------------------------------------------------------

/// Locate the Direct Map16 RATS block in ROM bytes and return its payload
/// (without the 8-byte RATS header). Mirrors the ExAnimation RATS scan:
/// standard `STAR` + size + ~size header, standard size semantics
/// (payload_len = size + 1), payload identified by our magic and validated
/// by a structural decode.
fn find_rats_payload(rom: &[u8]) -> Result<&[u8], Dm16Error> {
    let mut i = 0;
    while i + 16 < rom.len() {
        if &rom[i..i + 4] == b"STAR" {
            let size = u16::from_le_bytes([rom[i + 4], rom[i + 5]]) as usize;
            let comp = u16::from_le_bytes([rom[i + 6], rom[i + 7]]);
            if size as u16 ^ comp == 0xFFFF {
                let payload_start = i + 8;
                let payload_end = payload_start.saturating_add(size).saturating_add(1);
                if payload_end <= rom.len()
                    && payload_start + 8 <= rom.len()
                    && &rom[payload_start..payload_start + 8] == DM16_RATS_MAGIC
                    && DirectMap16Data::decode(&rom[payload_start..payload_end]).is_ok()
                {
                    return Ok(&rom[payload_start..payload_end]);
                }
            }
        }
        i += 1;
    }
    Err(Dm16Error::NotFound)
}

/// Erase the existing Direct Map16 RATS block in place (fill with `0xFF` so
/// it reads as free space again, like the ExAnimation block erase).
fn erase_rats_block(rom: &mut [u8]) {
    let mut i = 0;
    while i + 16 < rom.len() {
        if &rom[i..i + 4] == b"STAR" {
            let size = u16::from_le_bytes([rom[i + 4], rom[i + 5]]) as usize;
            let comp = u16::from_le_bytes([rom[i + 6], rom[i + 7]]);
            if size as u16 ^ comp == 0xFFFF && &rom[i + 8..i + 16] == DM16_RATS_MAGIC {
                let end = (i + 8 + size + 1).min(rom.len());
                rom[i..end].fill(0xFF);
                return;
            }
        }
        i += 1;
    }
}

// -------------------------------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_object() -> DirectMap16Object {
        DirectMap16Object {
            x:         10,
            y:         20,
            w:         4,
            h:         3,
            pw:        2,
            ph:        2,
            tiles:     vec![0x25, 0x26, 0x2B, 0x2C],
            condition: Some(DirectMap16Condition { ram_addr: 0x14AF, bit: 0 }),
        }
    }

    #[test]
    fn encode_decode_round_trip() {
        let mut data = DirectMap16Data::default();
        data.levels.insert(0x105, vec![sample_object()]);
        data.levels.insert(0x0, vec![DirectMap16Object {
            x:         0,
            y:         0,
            w:         1,
            h:         1,
            pw:        1,
            ph:        1,
            tiles:     vec![0x130],
            condition: None,
        }]);
        let decoded = DirectMap16Data::decode(&data.encode()).unwrap();
        assert_eq!(decoded.levels.len(), 2);
        assert_eq!(decoded.objects_for(0x105), &[sample_object()]);
        assert!(decoded.objects_for(0x42).is_empty());
    }

    #[test]
    fn pattern_repeats_on_resize() {
        // 2x2 pattern, object resized to 5x3: tiles repeat.
        let obj = sample_object(); // tiles: [0x25,0x26 / 0x2B,0x2C]
        assert_eq!(obj.tile_at(0, 0), 0x25);
        assert_eq!(obj.tile_at(1, 0), 0x26);
        assert_eq!(obj.tile_at(2, 0), 0x25); // wraps
        assert_eq!(obj.tile_at(0, 1), 0x2B);
        assert_eq!(obj.tile_at(4, 2), 0x25); // (4%2, 2%2) = (0,0)
        assert_eq!(obj.tile_at(3, 1), 0x2C);
    }

    #[test]
    fn condition_evaluates() {
        let bit0 = DirectMap16Condition { ram_addr: 0x14AF, bit: 0 };
        assert!(bit0.renders_with(0x01));
        assert!(!bit0.renders_with(0xFE));
        let any = DirectMap16Condition { ram_addr: 0x14AF, bit: 8 };
        assert!(any.renders_with(0x10));
        assert!(!any.renders_with(0x00));
    }

    #[test]
    fn remap_rewrites_matching_tiles() {
        let mut data = DirectMap16Data::default();
        data.levels.insert(0x105, vec![sample_object()]);
        let changed = data.remap_level(0x105, &[(0x25, 0x130), (0x2C, 0x131)]);
        assert_eq!(changed, 2);
        let obj = &data.objects_for(0x105)[0];
        assert_eq!(obj.tiles, vec![0x130, 0x26, 0x2B, 0x131]);
        // Other levels untouched.
        assert_eq!(data.remap_level(0x0, &[(0x25, 0x130)]), 0);
    }

    #[test]
    fn rom_write_parse_round_trip() {
        // Synthetic 512KB ROM image with enough 0xFF free space.
        let mut rom = vec![0xFFu8; 0x80000];
        let mut data = DirectMap16Data::default();
        data.levels.insert(0x105, vec![sample_object()]);
        data.write_to_rom(&mut rom, 0).unwrap();
        let parsed = DirectMap16Data::parse(&rom).unwrap();
        assert_eq!(parsed.objects_for(0x105), &[sample_object()]);
        // Rewrite replaces the old block (no duplicates).
        let mut data2 = DirectMap16Data::default();
        data2.levels.insert(0x7, vec![sample_object()]);
        data2.write_to_rom(&mut rom, 0).unwrap();
        let parsed2 = DirectMap16Data::parse(&rom).unwrap();
        assert!(parsed2.objects_for(0x105).is_empty());
        assert_eq!(parsed2.objects_for(0x7), &[sample_object()]);
        // Empty data erases the block.
        DirectMap16Data::default().write_to_rom(&mut rom, 0).unwrap();
        assert!(matches!(DirectMap16Data::parse(&rom), Err(Dm16Error::NotFound)));
    }

    #[test]
    fn decode_rejects_bad_magic_and_truncation() {
        assert!(matches!(DirectMap16Data::decode(b"SMWDM160"), Err(Dm16Error::NotFound)));
        let mut data = DirectMap16Data::default();
        data.levels.insert(0x105, vec![sample_object()]);
        let mut enc = data.encode();
        enc.truncate(enc.len() - 3);
        assert!(matches!(DirectMap16Data::decode(&enc), Err(Dm16Error::Truncated)));
    }

    #[test]
    fn validate_rejects_bad_dims() {
        let mut obj = sample_object();
        obj.w = 0;
        assert!(obj.validate().is_err());
        obj.w = 4;
        obj.tiles.pop();
        assert!(obj.validate().is_err());
        obj.tiles.push(0x2C);
        obj.condition = Some(DirectMap16Condition { ram_addr: 0, bit: 9 });
        assert!(obj.validate().is_err());
    }
}
