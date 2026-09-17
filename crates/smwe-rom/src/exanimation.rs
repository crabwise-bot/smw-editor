//! Editable ExAnimation — per-level custom tile and palette animation.
//!
//! Lunar Magic parity (LM v1.60/v1.70 "Edit ExAnimated Frames", overworld
//! variant v2.40): each level (plus one global list that runs in every level)
//! can own a list of animation frames. A frame is either a *line* frame —
//! copy the graphics of up to N 8x8 tiles from elsewhere in VRAM into a
//! destination VRAM slot, one source set per animation step — or a *palette*
//! frame — write N colors into CGRAM per step, or rotate a ring of colors.
//!
//! # Storage format (smw-editor native, documented)
//!
//! There is no vanilla ROM structure for this (LM implements it as an ASM
//! hack), so smw-editor stores the data in a single RATS-tagged free-space
//! block. The block payload is:
//!
//! ```text
//! "SMWEXAN1"            8 bytes magic
//! version               u8 (=1)
//! level_count           u16 LE
//! per level entry:
//!   level               u16 LE (0x000-0x1FF)
//!   flags               u8 (bit 0 = disable original game animations)
//!   frame_count         u16 LE
//!   per frame:
//!     kind              u8 (0=Line8x8, 1=Line16x16, 2=Palette, 3=PaletteRotate)
//!     dest              u16 LE (VRAM word address for line kinds,
//!                              CGRAM word address for palette kinds)
//!     speed             u8 (editor ticks per animation step; 0 = every tick)
//!     trigger           u8 (0=Always, 1=On/Off, 2=Manual, 3=OneShot)
//!     frames            u16 LE (1..=0x100 animation steps)
//!     units_per_frame   u8
//!     payload           frames * units_per_frame u16 LE entries:
//!                       line kinds: source VRAM word addresses of 8x8 tiles
//!                       palette:    SNES BGR555 colors
//!                       rotate:      one ring of units_per_frame BGR555 colors
//!                       (frames = rotation steps shown)
//! global entry: flags u8, frame_count u16 LE, frames as above
//! ```
//!
//! The RATS tag is the standard `STAR` + size + ~size header LM itself uses,
//! so other tools' free-space scanners won't clobber the block.
//!
//! # Preview semantics
//!
//! [`apply_tick`] advances the animation for one editor tick (the level view
//! ticks every ~133ms, matching the game's animated-tile rate). Triggers
//! other than `Always` are stored for the game but ignored by the preview —
//! the dialog says so next to the trigger picker. Line frames copy 4bpp tile
//! graphics (32 bytes per 8x8 tile) inside VRAM; palette frames write CGRAM.
//!
//! # In-game playback
//!
//! The editor authors and previews the data; making it run in a real game
//! still requires installing Lunar Magic's ExAnimation ASM hack (LM does this
//! itself when you use its ExAnimation dialog). This module documents the
//! data so a future installer — or LM — can consume it.

use std::collections::BTreeMap;

use thiserror::Error;

// -------------------------------------------------------------------------------------------------
// Constants
// -------------------------------------------------------------------------------------------------

/// Magic at the start of the RATS payload.
pub const EXANIM_MAGIC: &[u8; 8] = b"SMWEXAN1";
/// Payload format version.
pub const EXANIM_FORMAT_VERSION: u8 = 1;
/// Maximum animation steps per frame, matching LM's 0x100-frame cap.
pub const EXANIM_MAX_FRAMES: u16 = 0x100;
/// Maximum tiles/colors per animation step (sanity cap for parsing).
pub const EXANIM_MAX_UNITS_PER_FRAME: usize = 64;
/// Maximum frames in one animation list (level or global).
pub const EXANIM_MAX_FRAME_ENTRIES: usize = 0x100;
/// Bytes per 4bpp 8x8 tile in VRAM.
pub const VRAM_TILE_BYTES: usize = 32;

// -------------------------------------------------------------------------------------------------
// Types
// -------------------------------------------------------------------------------------------------

/// What a single ExAnimation frame animates.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum ExAnimFrameKind {
    /// Copy `units_per_frame` 8x8 tiles' graphics to `dest` in VRAM.
    Line8x8       = 0,
    /// Same copy semantics as [`ExAnimFrameKind::Line8x8`]; the dialog groups
    /// the units 2x2 as 16x16 blocks, like LM's 16x16 line types.
    Line16x16     = 1,
    /// Write `units_per_frame` BGR555 colors to CGRAM at `dest` per step.
    Palette       = 2,
    /// Rotate a ring of `units_per_frame` BGR555 colors at CGRAM `dest`.
    PaletteRotate = 3,
}

impl ExAnimFrameKind {
    pub fn from_u8(v: u8) -> Option<Self> {
        Some(match v {
            0 => Self::Line8x8,
            1 => Self::Line16x16,
            2 => Self::Palette,
            3 => Self::PaletteRotate,
            _ => return None,
        })
    }

    pub fn is_line(self) -> bool {
        matches!(self, Self::Line8x8 | Self::Line16x16)
    }

    pub fn is_palette(self) -> bool {
        matches!(self, Self::Palette | Self::PaletteRotate)
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Line8x8 => "Line 8x8",
            Self::Line16x16 => "Line 16x16",
            Self::Palette => "Palette",
            Self::PaletteRotate => "Palette rotate",
        }
    }
}

/// When a frame runs. Stored for the game; the editor preview treats every
/// trigger as `Always` (the dialog says so).
#[derive(Copy, Clone, Debug, PartialEq, Eq, Default)]
#[repr(u8)]
pub enum ExAnimTrigger {
    #[default]
    Always  = 0,
    OnOff   = 1,
    Manual  = 2,
    OneShot = 3,
}

impl ExAnimTrigger {
    pub fn from_u8(v: u8) -> Option<Self> {
        Some(match v {
            0 => Self::Always,
            1 => Self::OnOff,
            2 => Self::Manual,
            3 => Self::OneShot,
            _ => return None,
        })
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Always => "Always",
            Self::OnOff => "On/Off switch",
            Self::Manual => "Manual",
            Self::OneShot => "One-shot",
        }
    }
}

/// One animation frame: `frames` steps of `units_per_frame` units each.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExAnimFrame {
    pub kind:            ExAnimFrameKind,
    pub dest:            u16,
    /// Editor ticks per animation step; 0 means every tick.
    pub speed:           u8,
    pub trigger:         ExAnimTrigger,
    /// Animation steps, 1..=[`EXANIM_MAX_FRAMES`].
    pub frames:          u16,
    pub units_per_frame: u8,
    /// Line kinds: source VRAM word addresses. Palette: BGR555 colors.
    /// Rotate: one ring of `units_per_frame` colors.
    pub payload:         Vec<u16>,
}

impl ExAnimFrame {
    /// Number of `u16` payload entries this frame needs.
    pub fn payload_len(&self) -> usize {
        if self.kind == ExAnimFrameKind::PaletteRotate {
            self.units_per_frame as usize
        } else {
            self.frames as usize * self.units_per_frame as usize
        }
    }

    fn validate(&self) -> Result<(), ExAnimError> {
        if self.frames == 0 || self.frames > EXANIM_MAX_FRAMES {
            return Err(ExAnimError::BadFrameCount(self.frames));
        }
        if self.units_per_frame == 0 || self.units_per_frame as usize > EXANIM_MAX_UNITS_PER_FRAME {
            return Err(ExAnimError::BadUnitsPerFrame(self.units_per_frame));
        }
        if self.payload.len() != self.payload_len() {
            return Err(ExAnimError::PayloadLengthMismatch {
                expected: self.payload_len(),
                actual:   self.payload.len(),
            });
        }
        Ok(())
    }
}

impl Default for ExAnimFrame {
    fn default() -> Self {
        Self {
            kind:            ExAnimFrameKind::Line8x8,
            dest:            0,
            speed:           0,
            trigger:         ExAnimTrigger::Always,
            frames:          2,
            units_per_frame: 1,
            payload:         vec![0, 0],
        }
    }
}

/// One animation list: the frames plus whether the original game animations
/// are suppressed while it plays.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ExAnimation {
    pub frames:           Vec<ExAnimFrame>,
    pub disable_original: bool,
}

/// All ExAnimation data in the ROM: per-level lists plus the global list
/// that runs in every level (LM's "global ExAnimation list").
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ExAnimationData {
    pub levels: BTreeMap<u16, ExAnimation>,
    pub global: ExAnimation,
}

#[derive(Debug, Error)]
pub enum ExAnimError {
    #[error("no ExAnimation block found in ROM")]
    NotFound,
    #[error("corrupt ExAnimation block: {0}")]
    Corrupt(String),
    #[error("bad frame count {0} (must be 1..=0x100)")]
    BadFrameCount(u16),
    #[error("bad units-per-frame {0} (must be 1..=64)")]
    BadUnitsPerFrame(u8),
    #[error("payload length mismatch: expected {expected} u16s, got {actual}")]
    PayloadLengthMismatch { expected: usize, actual: usize },
    #[error("too many frames in one animation list (max 0x100)")]
    TooManyFrames,
    #[error("no free space for {0} bytes of ExAnimation data")]
    NoFreeSpace(usize),
    #[error("ExAnimation payload too large for a RATS block ({0} bytes, max 65536)")]
    TooLarge(usize),
}

// -------------------------------------------------------------------------------------------------
// Serialization
// -------------------------------------------------------------------------------------------------

fn encode_frame(frame: &ExAnimFrame, out: &mut Vec<u8>) -> Result<(), ExAnimError> {
    frame.validate()?;
    out.push(frame.kind as u8);
    out.extend_from_slice(&frame.dest.to_le_bytes());
    out.push(frame.speed);
    out.push(frame.trigger as u8);
    out.extend_from_slice(&frame.frames.to_le_bytes());
    out.push(frame.units_per_frame);
    for &w in &frame.payload {
        out.extend_from_slice(&w.to_le_bytes());
    }
    Ok(())
}

fn decode_frame(input: &[u8]) -> Result<(ExAnimFrame, usize), ExAnimError> {
    if input.len() < 8 {
        return Err(ExAnimError::Corrupt("truncated frame header".into()));
    }
    let kind = ExAnimFrameKind::from_u8(input[0])
        .ok_or_else(|| ExAnimError::Corrupt(format!("unknown frame kind {}", input[0])))?;
    let dest = u16::from_le_bytes([input[1], input[2]]);
    let speed = input[3];
    let trigger = ExAnimTrigger::from_u8(input[4])
        .ok_or_else(|| ExAnimError::Corrupt(format!("unknown trigger {}", input[4])))?;
    let frames = u16::from_le_bytes([input[5], input[6]]);
    let units_per_frame = input[7];
    let frame = ExAnimFrame { kind, dest, speed, trigger, frames, units_per_frame, payload: Vec::new() };
    // Validate counts before trusting the payload length.
    if frame.frames == 0 || frame.frames > EXANIM_MAX_FRAMES {
        return Err(ExAnimError::BadFrameCount(frame.frames));
    }
    if frame.units_per_frame == 0 || frame.units_per_frame as usize > EXANIM_MAX_UNITS_PER_FRAME {
        return Err(ExAnimError::BadUnitsPerFrame(frame.units_per_frame));
    }
    let need = frame.payload_len() * 2;
    if input.len() < 8 + need {
        return Err(ExAnimError::Corrupt("truncated frame payload".into()));
    }
    let mut payload = Vec::with_capacity(frame.payload_len());
    for i in 0..frame.payload_len() {
        payload.push(u16::from_le_bytes([input[8 + 2 * i], input[8 + 2 * i + 1]]));
    }
    Ok((ExAnimFrame { payload, ..frame }, 8 + need))
}

/// Serialize one animation list (flags + frames), shared by ROM storage and
/// the MWL section-6 payload.
pub fn encode_animation(anim: &ExAnimation) -> Result<Vec<u8>, ExAnimError> {
    if anim.frames.len() > EXANIM_MAX_FRAME_ENTRIES {
        return Err(ExAnimError::TooManyFrames);
    }
    let mut out = Vec::new();
    out.push(anim.disable_original as u8);
    out.extend_from_slice(&(anim.frames.len() as u16).to_le_bytes());
    for frame in &anim.frames {
        encode_frame(frame, &mut out)?;
    }
    Ok(out)
}

/// Parse one animation list produced by [`encode_animation`].
pub fn decode_animation(input: &[u8]) -> Result<(ExAnimation, usize), ExAnimError> {
    if input.len() < 3 {
        return Err(ExAnimError::Corrupt("truncated animation header".into()));
    }
    let disable_original = input[0] != 0;
    let frame_count = u16::from_le_bytes([input[1], input[2]]) as usize;
    if frame_count > EXANIM_MAX_FRAME_ENTRIES {
        return Err(ExAnimError::TooManyFrames);
    }
    let mut frames = Vec::with_capacity(frame_count);
    let mut pos = 3;
    for _ in 0..frame_count {
        let (frame, used) = decode_frame(&input[pos..])?;
        frames.push(frame);
        pos += used;
    }
    Ok((ExAnimation { frames, disable_original }, pos))
}

fn encode_payload(data: &ExAnimationData) -> Result<Vec<u8>, ExAnimError> {
    let mut out = Vec::with_capacity(64);
    out.extend_from_slice(EXANIM_MAGIC);
    out.push(EXANIM_FORMAT_VERSION);
    out.extend_from_slice(&(data.levels.len() as u16).to_le_bytes());
    for (&level, anim) in &data.levels {
        out.extend_from_slice(&level.to_le_bytes());
        out.extend_from_slice(&encode_animation(anim)?);
    }
    out.extend_from_slice(&encode_animation(&data.global)?);
    Ok(out)
}

fn decode_payload(payload: &[u8]) -> Result<ExAnimationData, ExAnimError> {
    if payload.len() < 11 || &payload[..8] != EXANIM_MAGIC {
        return Err(ExAnimError::Corrupt("bad magic".into()));
    }
    if payload[8] != EXANIM_FORMAT_VERSION {
        return Err(ExAnimError::Corrupt(format!("unsupported version {}", payload[8])));
    }
    let level_count = u16::from_le_bytes([payload[9], payload[10]]) as usize;
    let mut levels = BTreeMap::new();
    let mut pos = 11;
    for _ in 0..level_count {
        if pos + 2 > payload.len() {
            return Err(ExAnimError::Corrupt("truncated level entry".into()));
        }
        let level = u16::from_le_bytes([payload[pos], payload[pos + 1]]);
        pos += 2;
        let (anim, used) = decode_animation(&payload[pos..]).map_err(|e| match e {
            ExAnimError::Corrupt(s) => ExAnimError::Corrupt(format!("level {level:03X}: {s}")),
            other => other,
        })?;
        pos += used;
        levels.insert(level, anim);
    }
    let (global, _) = decode_animation(&payload[pos..]).map_err(|e| match e {
        ExAnimError::Corrupt(s) => ExAnimError::Corrupt(format!("global list: {s}")),
        other => other,
    })?;
    Ok(ExAnimationData { levels, global })
}

// -------------------------------------------------------------------------------------------------
// ROM storage (single RATS-tagged free-space block)
// -------------------------------------------------------------------------------------------------

/// Scan `rom_bytes` (raw file bytes, SMC header included if present) for the
/// ExAnimation RATS block. Returns the file offset of the `STAR` tag.
fn find_block(rom_bytes: &[u8]) -> Option<usize> {
    let mut i = 0usize;
    while i + 16 < rom_bytes.len() {
        if &rom_bytes[i..i + 4] == b"STAR" {
            let size = u16::from_le_bytes([rom_bytes[i + 4], rom_bytes[i + 5]]) as usize;
            let inv = u16::from_le_bytes([rom_bytes[i + 6], rom_bytes[i + 7]]);
            if size as u16 ^ inv == 0xFFFF {
                let data_start = i + 8;
                // Minimum viable payload: magic + version + 0 levels + empty global.
                // The claimed size must fit inside the ROM before slicing.
                let payload_end = data_start.saturating_add(size).saturating_add(1);
                if payload_end <= rom_bytes.len()
                    && data_start + 14 <= rom_bytes.len()
                    && &rom_bytes[data_start..data_start + 8] == EXANIM_MAGIC
                    && decode_payload(&rom_bytes[data_start..payload_end]).is_ok()
                {
                    return Some(i);
                }
            }
        }
        i += 1;
    }
    None
}

impl ExAnimationData {
    /// Parse the ExAnimation block from raw ROM bytes. Returns
    /// [`ExAnimError::NotFound`] when no block exists yet (a fresh ROM).
    pub fn parse(rom_bytes: &[u8]) -> Result<Self, ExAnimError> {
        let tag = find_block(rom_bytes).ok_or(ExAnimError::NotFound)?;
        let size = u16::from_le_bytes([rom_bytes[tag + 4], rom_bytes[tag + 5]]) as usize;
        let end = tag.saturating_add(8).saturating_add(size).saturating_add(1);
        let payload =
            rom_bytes.get(tag + 8..end).ok_or_else(|| ExAnimError::Corrupt("ExAnimation block overruns ROM".into()))?;
        decode_payload(payload)
    }

    /// Write the data to ROM: erase any existing block (fill with `0xFF` so
    /// it reads as free space again), allocate fresh free space, and write a
    /// new RATS-tagged block. Empty data (no levels, no global frames) erases
    /// the block without writing a new one.
    pub fn write_to_rom(&self, rom_bytes: &mut [u8], header_offset: usize) -> Result<(), ExAnimError> {
        // Erase any existing block first.
        if let Some(tag) = find_block(rom_bytes) {
            let size = u16::from_le_bytes([rom_bytes[tag + 4], rom_bytes[tag + 5]]) as usize;
            let end = (tag + 8 + size + 1).min(rom_bytes.len());
            rom_bytes[tag..end].fill(0xFF);
        }

        let payload = encode_payload(self)?;
        // Nothing to store: leave the ROM without a block.
        if self.levels.is_empty() && self.global.frames.is_empty() {
            return Ok(());
        }

        let total = 8 + payload.len(); // RATS tag + payload
        let pc = crate::freespace::find_free_space(rom_bytes, total, 0x008000, header_offset)
            .ok_or(ExAnimError::NoFreeSpace(total))?;
        let file_off = pc + header_offset;
        if payload.is_empty() || payload.len() > 0x10000 {
            return Err(ExAnimError::TooLarge(payload.len()));
        }
        let size_field = (payload.len() - 1) as u16;
        rom_bytes[file_off..file_off + 4].copy_from_slice(b"STAR");
        rom_bytes[file_off + 4..file_off + 6].copy_from_slice(&size_field.to_le_bytes());
        rom_bytes[file_off + 6..file_off + 8].copy_from_slice(&(!size_field).to_le_bytes());
        rom_bytes[file_off + 8..file_off + 8 + payload.len()].copy_from_slice(&payload);
        Ok(())
    }

    /// The animation list that plays for `level`: the global frames first,
    /// then the level's own frames. `disable_original` is set if either list
    /// asks for it.
    pub fn for_level(&self, level: u16) -> ExAnimation {
        let mut frames = self.global.frames.clone();
        let mut disable_original = self.global.disable_original;
        if let Some(local) = self.levels.get(&level) {
            frames.extend(local.frames.iter().cloned());
            disable_original |= local.disable_original;
        }
        ExAnimation { frames, disable_original }
    }

    /// Mutable access to a level's list, creating it on demand.
    pub fn level_mut(&mut self, level: u16) -> &mut ExAnimation {
        self.levels.entry(level).or_default()
    }

    /// Read-only access to a level's list, if the level has one.
    pub fn level(&self, level: u16) -> Option<&ExAnimation> {
        self.levels.get(&level)
    }
}

// -------------------------------------------------------------------------------------------------
// Preview: apply one tick to VRAM/CGRAM
// -------------------------------------------------------------------------------------------------

/// Advance the animation by one editor tick, writing into raw SNES VRAM
/// (0x10000 bytes, word-addressed) and CGRAM (0x200 bytes).
///
/// `tick` is a monotonically increasing counter; each frame shows step
/// `(tick / max(speed, 1)) % frames`. Out-of-range destinations and sources
/// are skipped, never panicking — the dialog validates, this is the safety
/// net for hand-built data.
pub fn apply_tick(anim: &ExAnimation, tick: u64, vram: &mut [u8], cgram: &mut [u8]) {
    for frame in &anim.frames {
        if frame.frames == 0 {
            continue;
        }
        let step = (tick / frame.speed.max(1) as u64) as usize % frame.frames as usize;
        match frame.kind {
            ExAnimFrameKind::Line8x8 | ExAnimFrameKind::Line16x16 => {
                let units = frame.units_per_frame as usize;
                let base = step * units;
                for u in 0..units {
                    let src_word = match frame.payload.get(base + u) {
                        Some(&w) => w as usize,
                        None => continue,
                    };
                    let src_off = src_word * 2;
                    let dst_off = frame.dest as usize * 2 + u * VRAM_TILE_BYTES;
                    if src_off + VRAM_TILE_BYTES <= vram.len() && dst_off + VRAM_TILE_BYTES <= vram.len() {
                        let (src, dst) = if src_off < dst_off {
                            let (a, b) = vram.split_at_mut(dst_off);
                            (&a[src_off..src_off + VRAM_TILE_BYTES], &mut b[..VRAM_TILE_BYTES])
                        } else if src_off > dst_off {
                            let (a, b) = vram.split_at_mut(src_off);
                            (&b[..VRAM_TILE_BYTES], &mut a[dst_off..dst_off + VRAM_TILE_BYTES])
                        } else {
                            continue;
                        };
                        dst.copy_from_slice(src);
                    }
                }
            }
            ExAnimFrameKind::Palette => {
                let units = frame.units_per_frame as usize;
                let base = step * units;
                for u in 0..units {
                    let color = match frame.payload.get(base + u) {
                        Some(&c) => c,
                        None => continue,
                    };
                    let off = frame.dest as usize * 2 + u * 2;
                    if off + 2 <= cgram.len() {
                        cgram[off..off + 2].copy_from_slice(&color.to_le_bytes());
                    }
                }
            }
            ExAnimFrameKind::PaletteRotate => {
                let ring = &frame.payload;
                if ring.is_empty() {
                    continue;
                }
                let rot = (tick / frame.speed.max(1) as u64) as usize % ring.len();
                for (u, _) in ring.iter().enumerate() {
                    let color = ring[(u + rot) % ring.len()];
                    let off = frame.dest as usize * 2 + u * 2;
                    if off + 2 <= cgram.len() {
                        cgram[off..off + 2].copy_from_slice(&color.to_le_bytes());
                    }
                }
            }
        }
    }
}

// -------------------------------------------------------------------------------------------------
// Tests
// -------------------------------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_frame() -> ExAnimFrame {
        ExAnimFrame {
            kind:            ExAnimFrameKind::Line8x8,
            dest:            0x1000,
            speed:           2,
            trigger:         ExAnimTrigger::Always,
            frames:          3,
            units_per_frame: 2,
            payload:         vec![0x2000, 0x2010, 0x2020, 0x2030, 0x2040, 0x2050],
        }
    }

    #[test]
    fn frame_round_trip() {
        let frame = sample_frame();
        let mut out = Vec::new();
        encode_frame(&frame, &mut out).unwrap();
        let (back, used) = decode_frame(&out).unwrap();
        assert_eq!(used, out.len());
        assert_eq!(back, frame);
    }

    #[test]
    fn animation_round_trip() {
        let anim = ExAnimation { frames: vec![sample_frame()], disable_original: true };
        let enc = encode_animation(&anim).unwrap();
        let (back, _) = decode_animation(&enc).unwrap();
        assert_eq!(back, anim);
    }

    #[test]
    fn data_round_trip() {
        let mut data = ExAnimationData::default();
        data.levels.insert(0x105, ExAnimation { frames: vec![sample_frame()], disable_original: false });
        data.global.frames.push(ExAnimFrame {
            kind:            ExAnimFrameKind::PaletteRotate,
            dest:            0x20,
            speed:           1,
            trigger:         ExAnimTrigger::Always,
            frames:          4,
            units_per_frame: 4,
            payload:         vec![0x001F, 0x03E0, 0x7C00, 0x7FFF],
        });
        let payload = encode_payload(&data).unwrap();
        let back = decode_payload(&payload).unwrap();
        assert_eq!(back, data);
    }

    #[test]
    fn rejects_bad_counts() {
        let mut frame = sample_frame();
        frame.frames = 0;
        assert!(matches!(encode_frame(&frame, &mut Vec::new()), Err(ExAnimError::BadFrameCount(0))));
        frame.frames = 0x101;
        assert!(matches!(encode_frame(&frame, &mut Vec::new()), Err(ExAnimError::BadFrameCount(0x101))));
        let mut frame = sample_frame();
        frame.units_per_frame = 0;
        assert!(matches!(encode_frame(&frame, &mut Vec::new()), Err(ExAnimError::BadUnitsPerFrame(0))));
    }

    #[test]
    fn rejects_payload_mismatch() {
        let mut frame = sample_frame();
        frame.payload.pop();
        assert!(matches!(encode_frame(&frame, &mut Vec::new()), Err(ExAnimError::PayloadLengthMismatch { .. })));
    }

    #[test]
    fn write_and_parse_in_scratch_rom() {
        // A scratch "ROM": free space everywhere, like erased flash.
        let mut rom = vec![0xFFu8; 0x100000];
        let mut data = ExAnimationData::default();
        data.levels.insert(0x105, ExAnimation { frames: vec![sample_frame()], disable_original: true });
        data.write_to_rom(&mut rom, 0).unwrap();

        let back = ExAnimationData::parse(&rom).unwrap();
        assert_eq!(back, data);

        // Writing empty data erases the block again.
        ExAnimationData::default().write_to_rom(&mut rom, 0).unwrap();
        assert!(matches!(ExAnimationData::parse(&rom), Err(ExAnimError::NotFound)));
        // And the erased region reads as free space again.
        assert!(rom.iter().all(|&b| b == 0xFF));
    }

    #[test]
    fn parse_ignores_unrelated_rats_blocks() {
        let mut rom = vec![0xFFu8; 0x10000];
        // Some other tool's RATS block.
        rom[0x100..0x104].copy_from_slice(b"STAR");
        rom[0x104..0x106].copy_from_slice(&7u16.to_le_bytes());
        rom[0x106..0x108].copy_from_slice(&(!7u16).to_le_bytes());
        assert!(matches!(ExAnimationData::parse(&rom), Err(ExAnimError::NotFound)));

        let mut data = ExAnimationData::default();
        data.levels.insert(0x105, ExAnimation { frames: vec![sample_frame()], disable_original: false });
        data.write_to_rom(&mut rom, 0).unwrap();
        assert_eq!(ExAnimationData::parse(&rom).unwrap(), data);
    }

    #[test]
    fn for_level_merges_global_first() {
        let mut data = ExAnimationData::default();
        data.global.frames.push(sample_frame());
        data.levels.insert(0x105, ExAnimation { frames: vec![sample_frame()], disable_original: true });
        let merged = data.for_level(0x105);
        assert_eq!(merged.frames.len(), 2);
        assert!(merged.disable_original);
        let other = data.for_level(0x106);
        assert_eq!(other.frames.len(), 1);
        assert!(!other.disable_original);
    }

    #[test]
    fn apply_tick_line_frame_copies_tiles() {
        let frame = sample_frame(); // dest 0x1000, 3 steps of 2 tiles
        let anim = ExAnimation { frames: vec![frame], disable_original: false };
        let mut vram = vec![0u8; 0x10000];
        // Paint source tiles with distinct markers.
        for (i, &src) in [0x2000u16, 0x2010, 0x2020, 0x2030, 0x2040, 0x2050].iter().enumerate() {
            vram[src as usize * 2] = 0xA0 + i as u8;
        }
        let mut cgram = vec![0u8; 0x200];

        // speed=2: ticks 0,1 -> step 0; ticks 2,3 -> step 1.
        apply_tick(&anim, 0, &mut vram, &mut cgram);
        assert_eq!(vram[0x1000 * 2], 0xA0);
        assert_eq!(vram[0x1000 * 2 + 32], 0xA1);
        apply_tick(&anim, 3, &mut vram, &mut cgram);
        assert_eq!(vram[0x1000 * 2], 0xA2);
        assert_eq!(vram[0x1000 * 2 + 32], 0xA3);
        apply_tick(&anim, 4, &mut vram, &mut cgram);
        assert_eq!(vram[0x1000 * 2], 0xA4);
        // Wraps around.
        apply_tick(&anim, 6, &mut vram, &mut cgram);
        assert_eq!(vram[0x1000 * 2], 0xA0);
    }

    #[test]
    fn apply_tick_palette_and_rotate() {
        let pal = ExAnimFrame {
            kind:            ExAnimFrameKind::Palette,
            dest:            0x10,
            speed:           1,
            trigger:         ExAnimTrigger::Always,
            frames:          2,
            units_per_frame: 2,
            payload:         vec![0x001F, 0x03E0, 0x7C00, 0x7FFF],
        };
        let anim = ExAnimation { frames: vec![pal], disable_original: false };
        let mut vram = vec![0u8; 0x10000];
        let mut cgram = vec![0u8; 0x200];
        apply_tick(&anim, 0, &mut vram, &mut cgram);
        assert_eq!(u16::from_le_bytes([cgram[0x20], cgram[0x21]]), 0x001F);
        assert_eq!(u16::from_le_bytes([cgram[0x22], cgram[0x23]]), 0x03E0);
        apply_tick(&anim, 1, &mut vram, &mut cgram);
        assert_eq!(u16::from_le_bytes([cgram[0x20], cgram[0x21]]), 0x7C00);

        let rot = ExAnimFrame {
            kind:            ExAnimFrameKind::PaletteRotate,
            dest:            0x30,
            speed:           1,
            trigger:         ExAnimTrigger::Always,
            frames:          3,
            units_per_frame: 3,
            payload:         vec![0x0001, 0x0002, 0x0003],
        };
        let anim = ExAnimation { frames: vec![rot], disable_original: false };
        apply_tick(&anim, 0, &mut vram, &mut cgram);
        assert_eq!(u16::from_le_bytes([cgram[0x60], cgram[0x61]]), 0x0001);
        apply_tick(&anim, 1, &mut vram, &mut cgram);
        assert_eq!(u16::from_le_bytes([cgram[0x60], cgram[0x61]]), 0x0002);
        assert_eq!(u16::from_le_bytes([cgram[0x64], cgram[0x65]]), 0x0001);
    }

    #[test]
    fn apply_tick_never_panics_on_bad_data() {
        let frame = ExAnimFrame {
            kind:            ExAnimFrameKind::Line8x8,
            dest:            0xFFF0, // near the end of VRAM
            speed:           0,
            trigger:         ExAnimTrigger::Always,
            frames:          1,
            units_per_frame: 4,
            payload:         vec![0xFFFF, 0xFFFF, 0xFFFF, 0xFFFF], // out of range sources
        };
        let anim = ExAnimation { frames: vec![frame], disable_original: false };
        let mut vram = vec![0u8; 0x10000];
        let mut cgram = vec![0u8; 0x200];
        apply_tick(&anim, 0, &mut vram, &mut cgram);
        apply_tick(&anim, 999, &mut vram, &mut cgram);
    }

    /// Real-ROM test: write the block into a scratch *copy* of the real ROM
    /// (the file itself is never touched), parse it back, and confirm the
    /// rest of the ROM still parses.
    #[test]
    #[ignore]
    fn real_rom_write_parse_round_trip() {
        let path = std::env::var("ROM_PATH").expect("ROM_PATH must point at a real SMW ROM for ignored tests");
        let raw = std::fs::read(path).expect("cannot read ROM");
        let header_offset = if raw.len() % 0x400 == 0x200 { 0x200 } else { 0 };
        let mut rom = raw.clone();

        let mut data = ExAnimationData::default();
        data.levels.insert(0x105, ExAnimation { frames: vec![sample_frame()], disable_original: true });
        data.global.frames.push(ExAnimFrame {
            kind:            ExAnimFrameKind::Palette,
            dest:            0x08,
            speed:           4,
            trigger:         ExAnimTrigger::Always,
            frames:          2,
            units_per_frame: 3,
            payload:         vec![0x7FFF, 0x7FFF, 0x7FFF, 0x0000, 0x0000, 0x0000],
        });
        data.write_to_rom(&mut rom, header_offset).unwrap();

        let back = ExAnimationData::parse(&rom).unwrap();
        assert_eq!(back, data);

        // The ROM still parses as a whole with the block present.
        let parsed = crate::SmwRom::from_rom(crate::snes_utils::rom::Rom::new(rom).unwrap()).unwrap();
        assert_eq!(parsed.exanimation, data);
    }
}
