//! Expanded Map16 storage: Lunar Magic 1.70/2.50 parity.
//!
//! What Lunar Magic does (official `readme.txt` in LM 3.63):
//! - v1.70: "4 times more Map16" (pages 0x02+ for FG).
//! - v2.50: "Map16 space has been expanded to 2x the previous limit. There
//!   are now 0x80 pages for FG Map16, and 0x80 pages for BG Map16."
//! - v1.91: the Map16 editor's remap button can remap 16x16 gameplay
//!   "act as" settings.
//! - v3.01: the remap dialog can "set a rectangle of Map16 tiles to use a
//!   rectangle of 'Act as' settings with the specified base (such as
//!   R200-211,S25)".
//!
//! # What this module covers
//!
//! - **Expanded FG pages (0x02-0x7F).** Read: Lunar Magic installs its own
//!   expanded-Map16 pointer table at SNES `$06F553`+ (one bank byte + one
//!   16-bit base per 16-page range — the same table `Tilesets::parse` uses
//!   via `parse_lm_map16`); when that table resolves, pages are read from
//!   and written back to LM's own locations, in place. When it does not
//!   resolve (a ROM Lunar Magic never expanded), pages live in a
//!   smw-editor-owned RATS block (tag `SMW16XFG`, layout below) allocated in
//!   free space. Either way the editor round-trips the data.
//! - **Expanded BG pages (0x02-0x7F).** Vanilla BG pages 0x00/0x01 stay at
//!   the fixed `$0D9100` table (see `map16_file`). Expanded BG pages live in
//!   a RATS block (tag `SMW16XBG`).
//! - **"Acts like" table.** Vanilla SMW dispatches block behavior by
//!   hardcoded ID range (see `block_behavior`), so there is no per-tile
//!   gameplay byte in a vanilla ROM. LM's remap dialog works on per-tile
//!   "act as" settings; this module stores them as a sparse
//!   tile -> act-as table in a RATS block (tag `SMW16ACT`). A tile absent
//!   from the table acts as itself (identity). In-game use of non-identity
//!   act-as values needs a hack that reads them (Lunar Magic's
//!   expanded-Map16 ASM does); the editor preserves and remaps them.
//!
//! # RATS layouts (all payloads are version 1)
//!
//! One RATS block per expanded page, so every page 0x02-0x7F is
//! independently supportable (a single contiguous block could not hold all
//! 0x7E pages without crossing LoROM bank boundaries, which RATS and
//! `find_free_space` forbid):
//!
//! `SMW16FG<p>` / `SMW16BG<p>` (`<p>` = page 0x02-0x7F):
//! ```text
//! offset  size  field
//! 0       8     magic: b"SMW16FG" or b"SMW16BG" + page byte
//! 8       1     version (1)
//! 9       1     page (0x02-0x7F, must match the tag's page byte)
//! 10      2     reserved (0)
//! 12      0x800 raw page bytes (256 tiles x 8 bytes, LM order)
//! ```
//!
//! `SMW16ACT` (acts-like table):
//! ```text
//! offset  size  field
//! 0       8     magic: b"SMW16ACT\0"
//! 8       1     version (1)
//! 9       2     entry count (u16 LE)
//! 11      1     reserved (0)
//! 12      4*count  entries: tile u16 LE, act_as u16 LE
//! ```
//!
//! Standard RATS header (`STAR`, size, ~size) precedes each payload.

use std::collections::HashMap;

use thiserror::Error;

use crate::{
    freespace::find_free_space,
    snes_utils::{
        addr::{AddrPc, AddrSnes},
        rom::RomError,
    },
};

// -------------------------------------------------------------------------------------------------
// Constants
// -------------------------------------------------------------------------------------------------

/// Raw bytes per Map16 page (256 tiles x 8 bytes).
pub const EXPANDED_PAGE_BYTES: usize = 0x800;
/// First expanded page (pages 0x00/0x01 are the vanilla pages).
pub const EXPANDED_FIRST_PAGE: u8 = 0x02;
/// Total FG pages in LM 2.50+ (tiles 0x0000-0x7FFF).
pub const FG_PAGE_COUNT: u8 = 0x80;
/// Total BG pages in LM 2.50+.
pub const BG_PAGE_COUNT: u8 = 0x80;
/// Highest editable Map16 tile id (0x80 pages x 256 tiles - 1).
pub const MAX_TILE_ID: u16 = 0x7FFF;

const RATS_MAGIC: &[u8; 4] = b"STAR";
/// First 7 bytes of a per-page RATS tag; byte 8 is the page number.
const TAG_FG_PREFIX: &[u8; 7] = b"SMW16FG";
const TAG_BG_PREFIX: &[u8; 7] = b"SMW16BG";
const TAG_ACT: &[u8; 8] = b"SMW16ACT";
const PAYLOAD_VERSION: u8 = 1;
/// Payload header size for the per-page tags (magic + version + page + reserved u16).
const PAGE_PAYLOAD_HEADER: usize = 12;
/// Payload header size for the acts tag (magic + version + count u16 + reserved).
const ACT_PAYLOAD_HEADER: usize = 12;

/// Build the 8-byte RATS tag for one expanded page.
fn page_tag(fg: bool, page: u8) -> [u8; 8] {
    let mut tag = [0u8; 8];
    let prefix = if fg { TAG_FG_PREFIX } else { TAG_BG_PREFIX };
    tag[..7].copy_from_slice(prefix);
    tag[7] = page;
    tag
}

// -------------------------------------------------------------------------------------------------
// Errors
// -------------------------------------------------------------------------------------------------

#[derive(Debug, Error)]
pub enum ExpandedMap16Error {
    #[error("Expanded Map16 page {0:#04X} out of range (want 0x02-0x7F)")]
    BadPage(u8),
    #[error("Map16 tile {0:#06X} out of range (want 0x0000-0x7FFF)")]
    BadTile(u16),
    #[error("Expanded page data must be exactly 0x800 bytes, got {0:#X}")]
    BadPageSize(usize),
    #[error("No free space for {0} bytes of expanded Map16 data (expand the ROM or free space first)")]
    NoFreeSpace(usize),
    #[error("Corrupt RATS block for tag {0}")]
    CorruptRats(String),
    #[error("Buffer too small: needed {needed:#X} bytes, have {have:#X}")]
    Truncated { needed: usize, have: usize },
    #[error("ROM error: {0}")]
    Rom(#[from] RomError),
}

// -------------------------------------------------------------------------------------------------
// Address helpers (raw bytes)
// -------------------------------------------------------------------------------------------------

/// Convert a SNES address to a file offset in raw ROM bytes (LoROM).
fn snes_to_file(addr: u32, header_offset: usize) -> Option<usize> {
    let pc = AddrPc::try_from_lorom(AddrSnes(addr)).ok()?;
    pc.as_index().checked_add(header_offset)
}

fn read_u8_at(rom_bytes: &[u8], addr: u32, header_offset: usize) -> Option<u8> {
    rom_bytes.get(snes_to_file(addr, header_offset)?).copied()
}

fn read_u16_at(rom_bytes: &[u8], addr: u32, header_offset: usize) -> Option<u16> {
    let off = snes_to_file(addr, header_offset)?;
    let b = rom_bytes.get(off..off + 2)?;
    Some(u16::from_le_bytes([b[0], b[1]]))
}

/// Lunar Magic's expanded-Map16 page pointer table (installed by LM's ASM
/// hack at `$06F553`+): one bank byte + one 16-bit base per 16-page range.
/// This mirrors the `Range` table in `Tilesets::parse` (`parse_lm_map16`),
/// including the `add` adjustment and the `alt_add` fallback the parser
/// uses — so reads and writes agree on where each page lives.
fn lm_fg_page_bases(rom_bytes: &[u8], header_offset: usize) -> Vec<(u8, u8, u32)> {
    // (start, end, lo_addr, bank_addr, add, alt_add)
    const RANGES: [(u8, u8, u32, u32, u32, Option<u32>); 8] = [
        (0x02, 0x0F, 0x06F553, 0x06F557, 0, Some(0x1000)),
        (0x10, 0x1F, 0x06F55C, 0x06F560, 0, Some(0x8000)),
        (0x20, 0x2F, 0x06F567, 0x06F56B, 1, None),
        (0x30, 0x3F, 0x06F570, 0x06F574, 1, Some(0x8000 + 1)),
        (0x40, 0x4F, 0x06F594, 0x06F598, 0, None),
        (0x50, 0x5F, 0x06F59D, 0x06F5A1, 0, Some(0x8000)),
        (0x60, 0x6F, 0x06F5A8, 0x06F5AC, 1, None),
        (0x70, 0x7F, 0x06F5B1, 0x06F5B5, 1, Some(0x8000 + 1)),
    ];
    let is_valid_lorom = |addr: u32| (addr & 0xFFFF) >= 0x8000;
    // Whole range must be addressable in the file, like `parse_blocks` would need.
    let range_fits = |base: u32, start: u8, end: u8| -> bool {
        let pages = (end - start + 1) as u32;
        let file_end = match snes_to_file(base, header_offset) {
            Some(off) => off,
            None => return false,
        }
        .checked_add(pages as usize * EXPANDED_PAGE_BYTES);
        matches!(file_end, Some(end) if end <= rom_bytes.len())
    };
    let mut out = Vec::new();
    for (start, end, lo_addr, bank_addr, add, alt_add) in RANGES {
        let (Some(bank), Some(lo)) =
            (read_u8_at(rom_bytes, bank_addr, header_offset), read_u16_at(rom_bytes, lo_addr, header_offset))
        else {
            continue;
        };
        let base = ((bank as u32) << 16) | (lo as u32).wrapping_add(add);
        if is_valid_lorom(base) && range_fits(base, start, end) {
            out.push((start, end, base));
            continue;
        }
        if let Some(alt) = alt_add {
            let base = ((bank as u32) << 16) | (lo as u32).wrapping_add(alt);
            if is_valid_lorom(base) && range_fits(base, start, end) {
                out.push((start, end, base));
            }
        }
    }
    out
}

/// SNES address of an expanded FG page's 0x800 data bytes.
///
/// Prefers Lunar Magic's own pointer table (in-place, LM-interchangeable);
/// falls back to this editor's RATS block for ROMs LM never expanded.
pub fn expanded_fg_page_snes(rom_bytes: &[u8], header_offset: usize, page: u8) -> Option<u32> {
    if !(EXPANDED_FIRST_PAGE..FG_PAGE_COUNT).contains(&page) {
        return None;
    }
    for (start, end, base) in lm_fg_page_bases(rom_bytes, header_offset) {
        if (start..=end).contains(&page) {
            return Some(base + (page - start) as u32 * EXPANDED_PAGE_BYTES as u32);
        }
    }
    // Our per-page RATS block.
    let (payload_off, _) = find_rats_page(rom_bytes, header_offset, true, page)?;
    let snes = snes_of_file(payload_off + PAGE_PAYLOAD_HEADER, header_offset)?;
    Some(snes)
}

/// Inverse of `snes_to_file`: file offset -> SNES LoROM address.
fn snes_of_file(file_off: usize, header_offset: usize) -> Option<u32> {
    let pc = file_off.checked_sub(header_offset)?;
    // LoROM: bank = pc / 0x8000, addr = 0x8000 + pc % 0x8000 (upper half mirror).
    let bank = (pc / 0x8000) as u32;
    let addr = 0x8000 + (pc % 0x8000) as u32;
    Some((bank << 16) | addr)
}

/// SNES address of one expanded FG tile's 8 definition bytes.
pub fn expanded_fg_tile_snes(rom_bytes: &[u8], header_offset: usize, tile: u16) -> Option<u32> {
    if tile < 0x200 || tile > MAX_TILE_ID {
        return None;
    }
    let page = (tile >> 8) as u8;
    let base = expanded_fg_page_snes(rom_bytes, header_offset, page)?;
    Some(base + (tile as u32 & 0xFF) * 8)
}

// -------------------------------------------------------------------------------------------------
// RATS block helpers
// -------------------------------------------------------------------------------------------------

/// Locate our RATS payload by its 8-byte tag. Returns the payload's file
/// offset (just past the 8-byte RATS header) and payload length.
fn find_rats_payload(rom_bytes: &[u8], header_offset: usize, tag: &[u8; 8]) -> Option<(usize, usize)> {
    let mut i = header_offset;
    while i + 8 + tag.len() <= rom_bytes.len() {
        if &rom_bytes[i..i + 4] == RATS_MAGIC {
            let size = u16::from_le_bytes([rom_bytes[i + 4], rom_bytes[i + 5]]) as usize;
            let inv = u16::from_le_bytes([rom_bytes[i + 6], rom_bytes[i + 7]]);
            if (size as u16) ^ inv == 0xFFFF {
                let payload_off = i + 8;
                let payload_len = size + 1;
                if payload_off + payload_len <= rom_bytes.len()
                    && rom_bytes.get(payload_off..payload_off + 8) == Some(tag.as_slice())
                {
                    return Some((payload_off, payload_len));
                }
            }
        }
        i += 1;
    }
    None
}

/// Write (or rewrite) one of our RATS blocks. Reuses the existing block when
/// the new payload fits; otherwise allocates fresh free space and erases the
/// old block back to `$FF` so the space is reusable.
fn write_rats_payload(
    rom_bytes: &mut [u8], header_offset: usize, tag: &[u8; 8], payload: &[u8],
) -> Result<(), ExpandedMap16Error> {
    assert_eq!(&payload[0..8], tag.as_slice(), "payload must start with its tag");
    let needed = 8 + payload.len();
    if let Some((payload_off, old_len)) = find_rats_payload(rom_bytes, header_offset, tag) {
        if payload.len() <= old_len {
            let head = payload_off - 8;
            let size = (payload.len() - 1) as u16;
            rom_bytes[head + 4..head + 6].copy_from_slice(&size.to_le_bytes());
            rom_bytes[head + 6..head + 8].copy_from_slice(&(!size).to_le_bytes());
            rom_bytes[payload_off..payload_off + payload.len()].copy_from_slice(payload);
            // Erase any leftover tail bytes back to free-space fill.
            rom_bytes[payload_off + payload.len()..payload_off + old_len].fill(0xFF);
            return Ok(());
        }
        // Erase the old block so its space is reusable free space again.
        let head = payload_off - 8;
        rom_bytes[head..head + 8 + old_len].fill(0xFF);
    }
    // Allocate fresh: prefer high banks (where LM keeps expanded data), but
    // take whatever free run fits — never spanning a LoROM bank boundary.
    let pc_len = rom_bytes.len() - header_offset;
    let start = [0x400000usize, 0x200000, 0x100000, 0x80000].into_iter().find(|&s| s < pc_len).unwrap_or(0);
    let pc = find_free_space(rom_bytes, needed, start, header_offset).ok_or(ExpandedMap16Error::NoFreeSpace(needed))?;
    let head = pc + header_offset;
    rom_bytes[head..head + 4].copy_from_slice(RATS_MAGIC);
    let size = (payload.len() - 1) as u16;
    rom_bytes[head + 4..head + 6].copy_from_slice(&size.to_le_bytes());
    rom_bytes[head + 6..head + 8].copy_from_slice(&(!size).to_le_bytes());
    rom_bytes[head + 8..head + 8 + payload.len()].copy_from_slice(payload);
    Ok(())
}

// -------------------------------------------------------------------------------------------------
// Expanded FG pages
// -------------------------------------------------------------------------------------------------

/// Read one expanded FG page (0x02-0x7F). `None` when the ROM has no data
/// for that page (vanilla ROM, page never written).
pub fn read_expanded_fg_page(
    rom_bytes: &[u8], header_offset: usize, page: u8,
) -> Result<Option<[u8; EXPANDED_PAGE_BYTES]>, ExpandedMap16Error> {
    if !(EXPANDED_FIRST_PAGE..FG_PAGE_COUNT).contains(&page) {
        return Err(ExpandedMap16Error::BadPage(page));
    }
    let Some(snes) = expanded_fg_page_snes(rom_bytes, header_offset, page) else {
        return Ok(None);
    };
    let Some(off) = snes_to_file(snes, header_offset) else {
        return Ok(None);
    };
    let end = off + EXPANDED_PAGE_BYTES;
    if end > rom_bytes.len() {
        return Err(ExpandedMap16Error::Truncated { needed: end, have: rom_bytes.len() });
    }
    let mut out = [0u8; EXPANDED_PAGE_BYTES];
    out.copy_from_slice(&rom_bytes[off..end]);
    Ok(Some(out))
}

/// Write one expanded FG page (0x02-0x7F): in place at LM's own location
/// when its pointer table resolves, otherwise into this editor's RATS block
/// (allocated/extended as needed).
pub fn write_expanded_fg_page(
    rom_bytes: &mut [u8], header_offset: usize, page: u8, data: &[u8],
) -> Result<(), ExpandedMap16Error> {
    if !(EXPANDED_FIRST_PAGE..FG_PAGE_COUNT).contains(&page) {
        return Err(ExpandedMap16Error::BadPage(page));
    }
    if data.len() != EXPANDED_PAGE_BYTES {
        return Err(ExpandedMap16Error::BadPageSize(data.len()));
    }
    // LM-owned location: write in place.
    let lm_base = lm_fg_page_bases(rom_bytes, header_offset)
        .into_iter()
        .find(|&(start, end, _)| (start..=end).contains(&page))
        .map(|(start, _, base)| base + (page - start) as u32 * EXPANDED_PAGE_BYTES as u32);
    if let Some(snes) = lm_base {
        let off = snes_to_file(snes, header_offset)
            .ok_or(ExpandedMap16Error::Truncated { needed: usize::MAX, have: rom_bytes.len() })?;
        let end = off + EXPANDED_PAGE_BYTES;
        if end > rom_bytes.len() {
            return Err(ExpandedMap16Error::Truncated { needed: end, have: rom_bytes.len() });
        }
        rom_bytes[off..end].copy_from_slice(data);
        return Ok(());
    }
    // Our per-page RATS block: fixed-size payload, rewrite in place or
    // allocate fresh.
    let tag = page_tag(true, page);
    let mut payload = Vec::with_capacity(PAGE_PAYLOAD_HEADER + EXPANDED_PAGE_BYTES);
    payload.extend_from_slice(&tag);
    payload.push(PAYLOAD_VERSION);
    payload.push(page);
    payload.extend_from_slice(&[0u8; 2]);
    payload.extend_from_slice(data);
    write_rats_payload(rom_bytes, header_offset, &tag, &payload)
}

/// Read one expanded BG page (0x02-0x7F) from this editor's per-page RATS
/// block. (Vanilla BG pages 0x00/0x01 stay at the fixed `$0D9100` table —
/// see `map16_file`.) `None` when the page was never written.
pub fn read_expanded_bg_page(
    rom_bytes: &[u8], header_offset: usize, page: u8,
) -> Result<Option<[u8; EXPANDED_PAGE_BYTES]>, ExpandedMap16Error> {
    if !(EXPANDED_FIRST_PAGE..BG_PAGE_COUNT).contains(&page) {
        return Err(ExpandedMap16Error::BadPage(page));
    }
    let Some((payload_off, _)) = find_rats_page(rom_bytes, header_offset, false, page) else {
        return Ok(None);
    };
    let start = payload_off + PAGE_PAYLOAD_HEADER;
    let mut out = [0u8; EXPANDED_PAGE_BYTES];
    out.copy_from_slice(&rom_bytes[start..start + EXPANDED_PAGE_BYTES]);
    Ok(Some(out))
}

/// Write one expanded BG page (0x02-0x7F) into this editor's per-page RATS
/// block (rewritten in place when it already exists).
pub fn write_expanded_bg_page(
    rom_bytes: &mut [u8], header_offset: usize, page: u8, data: &[u8],
) -> Result<(), ExpandedMap16Error> {
    if !(EXPANDED_FIRST_PAGE..BG_PAGE_COUNT).contains(&page) {
        return Err(ExpandedMap16Error::BadPage(page));
    }
    if data.len() != EXPANDED_PAGE_BYTES {
        return Err(ExpandedMap16Error::BadPageSize(data.len()));
    }
    let tag = page_tag(false, page);
    let mut payload = Vec::with_capacity(PAGE_PAYLOAD_HEADER + EXPANDED_PAGE_BYTES);
    payload.extend_from_slice(&tag);
    payload.push(PAYLOAD_VERSION);
    payload.push(page);
    payload.extend_from_slice(&[0u8; 2]);
    payload.extend_from_slice(data);
    write_rats_payload(rom_bytes, header_offset, &tag, &payload)
}

/// Locate one expanded page's RATS block by its per-page tag.
fn find_rats_page(rom_bytes: &[u8], header_offset: usize, fg: bool, page: u8) -> Option<(usize, usize)> {
    find_rats_payload(rom_bytes, header_offset, &page_tag(fg, page))
}

/// Read all pages from this editor's per-page RATS blocks as (page, bytes)
/// pairs, in one scan. Pages never written are absent (sparse).
fn read_rats_pages(rom_bytes: &[u8], header_offset: usize, fg: bool) -> Vec<(u8, Vec<u8>)> {
    let prefix: &[u8] = if fg { TAG_FG_PREFIX } else { TAG_BG_PREFIX };
    let mut out = Vec::new();
    let mut i = header_offset;
    while i + 8 + 8 <= rom_bytes.len() {
        if &rom_bytes[i..i + 4] == RATS_MAGIC {
            let size = u16::from_le_bytes([rom_bytes[i + 4], rom_bytes[i + 5]]) as usize;
            let inv = u16::from_le_bytes([rom_bytes[i + 6], rom_bytes[i + 7]]);
            if (size as u16) ^ inv == 0xFFFF {
                let payload_off = i + 8;
                let payload_len = size + 1;
                if payload_off + payload_len <= rom_bytes.len()
                    && payload_len >= PAGE_PAYLOAD_HEADER + EXPANDED_PAGE_BYTES
                    && &rom_bytes[payload_off..payload_off + 7] == prefix
                {
                    let page = rom_bytes[payload_off + 9];
                    if rom_bytes[payload_off + 8] == PAYLOAD_VERSION
                        && (EXPANDED_FIRST_PAGE..FG_PAGE_COUNT).contains(&page)
                        && rom_bytes[payload_off + 7] == page
                    {
                        let start = payload_off + PAGE_PAYLOAD_HEADER;
                        out.push((page, rom_bytes[start..start + EXPANDED_PAGE_BYTES].to_vec()));
                    }
                }
            }
        }
        i += 1;
    }
    out.sort_unstable_by_key(|(p, _)| *p);
    out
}

/// Read all pages from this editor's RATS expanded-FG block as
/// (page, bytes) pairs. Used by `Tilesets::parse` so a reparse sees pages
/// the editor wrote.
pub fn rats_fg_pages(rom_bytes: &[u8], header_offset: usize) -> Result<Vec<(u8, Vec<u8>)>, ExpandedMap16Error> {
    Ok(read_rats_pages(rom_bytes, header_offset, true))
}

/// Highest expanded FG page with any nonzero byte, or `None` when no
/// expanded FG data exists at all (LM table or our RATS block).
pub fn highest_used_fg_page(rom_bytes: &[u8], header_offset: usize) -> Option<u8> {
    let mut top: Option<u8> = None;
    for page in EXPANDED_FIRST_PAGE..FG_PAGE_COUNT {
        match read_expanded_fg_page(rom_bytes, header_offset, page) {
            Ok(Some(data)) if data.iter().any(|&b| b != 0) => top = Some(page),
            _ => {}
        }
    }
    top
}

/// Highest expanded BG page with any nonzero byte, or `None`.
pub fn highest_used_bg_page(rom_bytes: &[u8], header_offset: usize) -> Option<u8> {
    let mut top: Option<u8> = None;
    for page in EXPANDED_FIRST_PAGE..BG_PAGE_COUNT {
        match read_expanded_bg_page(rom_bytes, header_offset, page) {
            Ok(Some(data)) if data.iter().any(|&b| b != 0) => top = Some(page),
            _ => {}
        }
    }
    top
}

// -------------------------------------------------------------------------------------------------
// "Acts like" table (sparse tile -> act-as, identity default)
// -------------------------------------------------------------------------------------------------

/// Read the sparse acts-like table. Tiles absent from the map act as
/// themselves.
pub fn read_acts_table(rom_bytes: &[u8], header_offset: usize) -> Result<HashMap<u16, u16>, ExpandedMap16Error> {
    let Some((payload_off, payload_len)) = find_rats_payload(rom_bytes, header_offset, TAG_ACT) else {
        return Ok(HashMap::new());
    };
    if payload_len < ACT_PAYLOAD_HEADER {
        return Err(ExpandedMap16Error::CorruptRats("SMW16ACT truncated".to_string()));
    }
    if rom_bytes[payload_off + 8] != PAYLOAD_VERSION {
        return Err(ExpandedMap16Error::CorruptRats(format!(
            "SMW16ACT has unsupported version {}",
            rom_bytes[payload_off + 8]
        )));
    }
    let count = u16::from_le_bytes([rom_bytes[payload_off + 9], rom_bytes[payload_off + 10]]) as usize;
    let want = ACT_PAYLOAD_HEADER + count * 4;
    if payload_len < want {
        return Err(ExpandedMap16Error::CorruptRats(format!(
            "SMW16ACT truncated: want {want:#X} payload bytes, have {payload_len:#X}"
        )));
    }
    let mut out = HashMap::with_capacity(count);
    for i in 0..count {
        let off = payload_off + ACT_PAYLOAD_HEADER + i * 4;
        let tile = u16::from_le_bytes([rom_bytes[off], rom_bytes[off + 1]]);
        let act = u16::from_le_bytes([rom_bytes[off + 2], rom_bytes[off + 3]]);
        if tile > MAX_TILE_ID || act > MAX_TILE_ID {
            return Err(ExpandedMap16Error::CorruptRats(format!(
                "SMW16ACT entry {i} out of range ({tile:#06X} -> {act:#06X})"
            )));
        }
        out.insert(tile, act);
    }
    Ok(out)
}

/// Write the sparse acts-like table. Identity entries (tile acts as itself)
/// are dropped — absence means identity.
pub fn write_acts_table(
    rom_bytes: &mut [u8], header_offset: usize, table: &HashMap<u16, u16>,
) -> Result<(), ExpandedMap16Error> {
    let mut entries: Vec<(u16, u16)> = table.iter().filter(|(&t, &a)| t != a).map(|(&t, &a)| (t, a)).collect();
    entries.sort_unstable();
    if entries.len() > u16::MAX as usize {
        return Err(ExpandedMap16Error::CorruptRats("acts table too large".to_string()));
    }
    let mut payload = Vec::with_capacity(ACT_PAYLOAD_HEADER + entries.len() * 4);
    payload.extend_from_slice(TAG_ACT);
    payload.push(PAYLOAD_VERSION);
    payload.extend_from_slice(&(entries.len() as u16).to_le_bytes());
    payload.push(0);
    for (tile, act) in entries {
        payload.extend_from_slice(&tile.to_le_bytes());
        payload.extend_from_slice(&act.to_le_bytes());
    }
    write_rats_payload(rom_bytes, header_offset, TAG_ACT, &payload)
}

/// The tile whose gameplay behavior `tile` uses: the table entry, or the
/// tile itself when absent (identity default).
pub fn act_as_of(table: &HashMap<u16, u16>, tile: u16) -> u16 {
    table.get(&tile).copied().unwrap_or(tile)
}

/// Effective "acts like" value for a tile straight from ROM bytes: the
/// stored value, or the tile itself when absent. BG tiles (>= 0x8000) have
/// no act-as value; returns the tile id unchanged for them.
pub fn act_as_in_rom(rom_bytes: &[u8], header_offset: usize, tile: u16) -> u16 {
    if tile >= 0x8000 {
        return tile;
    }
    read_acts_table(rom_bytes, header_offset).map(|t| act_as_of(&t, tile)).unwrap_or(tile)
}

/// Set one tile's act-as value (`None` resets it to identity).
pub fn set_act_as(table: &mut HashMap<u16, u16>, tile: u16, act_as: Option<u16>) {
    match act_as {
        Some(a) if a != tile => {
            table.insert(tile, a);
        }
        _ => {
            table.remove(&tile);
        }
    }
}

/// Lunar Magic remap-dialog operation (v1.91): remap 16x16 gameplay "act as"
/// settings — every act-as reference pointing into
/// `[src_start, src_end]` is rewritten to `dst_base + (act - src_start)`.
/// Returns the number of entries rewritten.
pub fn remap_act_refs(table: &mut HashMap<u16, u16>, src_start: u16, src_end: u16, dst_base: u16) -> usize {
    let mut rewritten = 0;
    for act in table.values_mut() {
        if (*act >= src_start) && (*act <= src_end) {
            *act = dst_base.wrapping_add(*act - src_start) & MAX_TILE_ID;
            rewritten += 1;
        }
    }
    // Drop entries that became identity.
    table.retain(|&t, &mut a| t != a);
    rewritten
}

/// Lunar Magic remap-dialog operation (v1.91), the `G<src>,+<delta>` form:
/// every act-as reference pointing into `[src_start, src_end]` is shifted
/// by `delta` (wrapping within 0x0000-0x7FFF). Returns the number of
/// entries rewritten.
pub fn remap_act_refs_delta(table: &mut HashMap<u16, u16>, src_start: u16, src_end: u16, delta: i32) -> usize {
    let mut rewritten = 0;
    for act in table.values_mut() {
        if (*act >= src_start) && (*act <= src_end) {
            *act = ((*act as i32 + delta) & MAX_TILE_ID as i32) as u16;
            rewritten += 1;
        }
    }
    // Drop entries that became identity.
    table.retain(|&t, &mut a| t != a);
    rewritten
}

/// Lunar Magic remap-dialog operation (v3.01): set a rectangle of Map16 tiles
/// to use a rectangle of "act as" settings with the specified base — tile
/// `t` in `[start, end]` gets act-as `base + (t - start)`.
pub fn set_act_range_from_base(table: &mut HashMap<u16, u16>, start: u16, end: u16, base: u16) {
    for t in start..=end {
        let act = base.wrapping_add(t - start) & MAX_TILE_ID;
        set_act_as(table, t, Some(act));
    }
}

// -------------------------------------------------------------------------------------------------
// Tests
// -------------------------------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    /// A blank 512KB ROM image (all `$FF`, like free space after expansion).
    fn blank_rom() -> Vec<u8> {
        vec![0xFFu8; 0x80000]
    }

    #[test]
    fn expanded_fg_page_round_trips_through_rats() {
        let mut rom = blank_rom();
        assert_eq!(read_expanded_fg_page(&rom, 0, 0x02).unwrap(), None);
        let mut data = [0u8; EXPANDED_PAGE_BYTES];
        for (i, b) in data.iter_mut().enumerate() {
            *b = (i * 7) as u8;
        }
        write_expanded_fg_page(&mut rom, 0, 0x02, &data).unwrap();
        // The block must be RATS-tagged so other tools skip it.
        assert!(find_rats_page(&rom, 0, true, 0x02).is_some());
        let back = read_expanded_fg_page(&rom, 0, 0x02).unwrap().expect("page must read back");
        assert_eq!(back, data);
        // A second page extends the block; the first page survives.
        let mut data3 = [0xABu8; EXPANDED_PAGE_BYTES];
        data3[0] = 0x42;
        write_expanded_fg_page(&mut rom, 0, 0x05, &data3).unwrap();
        assert_eq!(read_expanded_fg_page(&rom, 0, 0x02).unwrap().unwrap(), data);
        assert_eq!(read_expanded_fg_page(&rom, 0, 0x05).unwrap().unwrap(), data3);
        // Pages beyond the block read as absent, not as zeros.
        assert_eq!(read_expanded_fg_page(&rom, 0, 0x06).unwrap(), None);
        // Tile address resolution works off the RATS block.
        let snes = expanded_fg_tile_snes(&rom, 0, 0x205).unwrap();
        let off = snes_to_file(snes, 0).unwrap();
        assert_eq!(rom[off], data[5 * 8]);
    }

    #[test]
    fn expanded_bg_page_round_trips_through_rats() {
        let mut rom = blank_rom();
        let mut data = [0x11u8; EXPANDED_PAGE_BYTES];
        data[EXPANDED_PAGE_BYTES - 1] = 0x22;
        write_expanded_bg_page(&mut rom, 0, 0x10, &data).unwrap();
        assert!(find_rats_page(&rom, 0, false, 0x10).is_some());
        assert_eq!(find_rats_page(&rom, 0, true, 0x10), None, "BG must not touch FG pages");
        let back = read_expanded_bg_page(&rom, 0, 0x10).unwrap().expect("page must read back");
        assert_eq!(back, data);
        assert_eq!(read_expanded_bg_page(&rom, 0, 0x02).unwrap(), None);
    }

    #[test]
    fn per_page_blocks_support_the_full_page_range() {
        let mut rom = blank_rom();
        // The highest pages must work exactly like the lowest ones: one
        // RATS block per page, no contiguous mega-block.
        let mut data = [0x77u8; EXPANDED_PAGE_BYTES];
        data[0] = 0x7F;
        write_expanded_fg_page(&mut rom, 0, 0x7F, &data).unwrap();
        write_expanded_bg_page(&mut rom, 0, 0x7F, &data).unwrap();
        assert_eq!(read_expanded_fg_page(&rom, 0, 0x7F).unwrap().unwrap(), data);
        assert_eq!(read_expanded_bg_page(&rom, 0, 0x7F).unwrap().unwrap(), data);
        // Sparse: unwritten pages stay absent even with high pages present.
        assert_eq!(read_expanded_fg_page(&rom, 0, 0x02).unwrap(), None);
        assert_eq!(read_expanded_bg_page(&rom, 0, 0x40).unwrap(), None);
        // The single-scan reader finds every written page.
        let pages = rats_fg_pages(&rom, 0).unwrap();
        assert_eq!(pages.len(), 1);
        assert_eq!(pages[0].0, 0x7F);
    }

    #[test]
    fn rats_rewrite_reuses_space() {
        let mut rom = blank_rom();
        let data = [0x55u8; EXPANDED_PAGE_BYTES];
        write_expanded_fg_page(&mut rom, 0, 0x02, &data).unwrap();
        let (off1, _) = find_rats_page(&rom, 0, true, 0x02).unwrap();
        // Rewriting the same page reuses the block in place.
        let data2 = [0x66u8; EXPANDED_PAGE_BYTES];
        write_expanded_fg_page(&mut rom, 0, 0x02, &data2).unwrap();
        let (off2, _) = find_rats_page(&rom, 0, true, 0x02).unwrap();
        assert_eq!(off1, off2, "rewrite should reuse the block in place");
        assert_eq!(read_expanded_fg_page(&rom, 0, 0x02).unwrap().unwrap(), data2);
    }

    #[test]
    fn acts_table_sparse_identity_default() {
        let mut rom = blank_rom();
        let table = read_acts_table(&rom, 0).unwrap();
        assert!(table.is_empty());
        assert_eq!(act_as_of(&table, 0x205), 0x205, "absent tile acts as itself");

        let mut table = HashMap::new();
        set_act_as(&mut table, 0x205, Some(0x130));
        set_act_as(&mut table, 0x300, Some(0x300)); // identity -> dropped
        write_acts_table(&mut rom, 0, &table).unwrap();
        assert!(find_rats_payload(&rom, 0, TAG_ACT).is_some());

        let back = read_acts_table(&rom, 0).unwrap();
        assert_eq!(back.len(), 1);
        assert_eq!(act_as_of(&back, 0x205), 0x130);
        assert_eq!(act_as_of(&back, 0x300), 0x300);
    }

    #[test]
    fn remap_act_refs_rewrites_range() {
        let mut table = HashMap::new();
        // LM v3.01 op: tiles R200-211 use act-as settings from base S25.
        set_act_range_from_base(&mut table, 0x200, 0x211, 0x25);
        assert_eq!(act_as_of(&table, 0x200), 0x25);
        assert_eq!(act_as_of(&table, 0x211), 0x36);
        // LM v1.91 op: remap act-as settings 0x25-0x36 -> 0x100+.
        let n = remap_act_refs(&mut table, 0x25, 0x36, 0x100);
        assert_eq!(n, 0x12);
        assert_eq!(act_as_of(&table, 0x200), 0x100);
        assert_eq!(act_as_of(&table, 0x211), 0x111);
        // A tile outside the source range is untouched.
        set_act_as(&mut table, 0x400, Some(0x12D));
        let n = remap_act_refs(&mut table, 0x25, 0x36, 0x100);
        assert_eq!(n, 0);
        assert_eq!(act_as_of(&table, 0x400), 0x12D);
    }

    #[test]
    fn remap_act_refs_delta_shifts_range() {
        let mut table = HashMap::new();
        set_act_range_from_base(&mut table, 0x200, 0x203, 0x100);
        // G100-103,+25: shift references by +0x25.
        let n = remap_act_refs_delta(&mut table, 0x100, 0x103, 0x25);
        assert_eq!(n, 4);
        assert_eq!(act_as_of(&table, 0x200), 0x125);
        assert_eq!(act_as_of(&table, 0x203), 0x128);
        // Negative delta wraps within 0x0000-0x7FFF.
        let n = remap_act_refs_delta(&mut table, 0x125, 0x128, -0x125);
        assert_eq!(n, 4);
        assert_eq!(act_as_of(&table, 0x200), 0x0);
    }

    #[test]
    fn rejects_bad_pages_and_tiles() {
        let mut rom = blank_rom();
        assert!(matches!(read_expanded_fg_page(&rom, 0, 0x01), Err(ExpandedMap16Error::BadPage(0x01))));
        assert!(matches!(
            write_expanded_fg_page(&mut rom, 0, 0x80, &[0u8; EXPANDED_PAGE_BYTES]),
            Err(ExpandedMap16Error::BadPage(0x80))
        ));
        assert!(matches!(
            write_expanded_fg_page(&mut rom, 0, 0x02, &[0u8; 100]),
            Err(ExpandedMap16Error::BadPageSize(100))
        ));
        assert_eq!(expanded_fg_tile_snes(&rom, 0, 0x1FF), None);
        assert_eq!(expanded_fg_tile_snes(&rom, 0, 0x8000), None);
    }

    // Real-ROM tests: need `ROM_PATH` pointing at a headerless SMW ROM.
    fn test_rom_bytes() -> Option<Vec<u8>> {
        std::env::var("ROM_PATH").ok().and_then(|p| std::fs::read(p).ok())
    }

    #[test]
    #[ignore]
    fn vanilla_rom_has_no_expanded_pages() {
        let rom = test_rom_bytes().expect("ROM_PATH must point at a headerless SMW ROM");
        // A vanilla ROM has no LM pointer table and no RATS block.
        assert_eq!(read_expanded_fg_page(&rom, 0, 0x02).unwrap(), None);
        assert_eq!(read_acts_table(&rom, 0).unwrap().len(), 0);
        assert_eq!(highest_used_fg_page(&rom, 0), None);
    }

    #[test]
    #[ignore]
    fn expanded_fg_rats_round_trip_on_rom_scratch() {
        let mut scratch = test_rom_bytes().expect("ROM_PATH must point at a headerless SMW ROM");
        let mut data = [0u8; EXPANDED_PAGE_BYTES];
        for (i, b) in data.iter_mut().enumerate() {
            *b = (i ^ 0x5A) as u8;
        }
        write_expanded_fg_page(&mut scratch, 0, 0x03, &data).unwrap();
        let back = read_expanded_fg_page(&scratch, 0, 0x03).unwrap().expect("must read back");
        assert_eq!(back, data);
        assert_eq!(highest_used_fg_page(&scratch, 0), Some(0x03));
        // Tile-level addressing agrees with the page read.
        let snes = expanded_fg_tile_snes(&scratch, 0, 0x3AB).unwrap();
        let off = snes_to_file(snes, 0).unwrap();
        assert_eq!(&scratch[off..off + 8], &data[0xAB * 8..0xAB * 8 + 8]);
    }

    #[test]
    #[ignore]
    fn acts_table_round_trip_on_rom_scratch() {
        let mut scratch = test_rom_bytes().expect("ROM_PATH must point at a headerless SMW ROM");
        let mut table = HashMap::new();
        set_act_range_from_base(&mut table, 0x200, 0x20F, 0x130);
        write_acts_table(&mut scratch, 0, &table).unwrap();
        let back = read_acts_table(&scratch, 0).unwrap();
        assert_eq!(back.len(), 0x10);
        assert_eq!(act_as_of(&back, 0x205), 0x135);
        assert_eq!(act_as_of(&back, 0x210), 0x210);
    }
}
