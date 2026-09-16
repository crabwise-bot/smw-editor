//! Overworld/level "message box" (dialog) text.
//!
//! Ported from SMWDisX `bank_05.asm` (`CODE_05B1BC`/`CODE_05B208`, the message
//! render routine) and cross-checked against `symbols/SMW_U.sym` for exact
//! per-message byte boundaries (see module docs below for how those were derived).
//!
//! Message bytes are NOT ASCII: each byte (0x00-0x7F) is a tile number into the
//! message font tileset (GFX2A, "Message Box Letters", SNES $0BCB7B, 2bpp,
//! 128 tiles) drawn via SMW's "dynamic stripe image" (Layer 3) mechanism
//! (confirmed in `CODE_05B208`: `LDA.W MessageBoxes,Y` is stored directly as a
//! tile number, with `AND #$7F` stripping bit 7 before use). Bit 7 is NOT a
//! hold/repeat flag: it means "fill the remainder of this 18-cell row with
//! `$1F` blanks" (see [`crate::font_map`] for the exact row-fill semantics).
//! The editor renders these tiles with the real GFX2A graphics for a true
//! WYSIWYG preview.
//!
//! Messages are looked up exclusively through a 25-entry pointer table
//! (`MESSAGE_POINTER_TABLE_SNES`, offsets relative to `MESSAGE_BOXES_SNES`) —
//! confirmed in `CODE_05B1BC`: `LDA.W DATA_05A5A7,X` (X = message-type index)
//! gives the starting offset used by the render loop. This means messages can
//! be freely resized/reordered as long as the pointer table is kept in sync,
//! *but* the whole blob is NOT repointable: it's addressed directly by
//! hardcoded ASM (`LDA.W MessageBoxes,Y`), not through a 3-byte ROM pointer,
//! so the combined size of all messages must stay within the original
//! `MESSAGE_BOXES_MAX_SIZE` budget (the routine `ClearMessageStripe` — real
//! code, not data — begins immediately after in ROM).
//!
//! Per-message byte boundaries were derived from consecutive label addresses
//! in `symbols/SMW_U.sym` (`IntroMessage`=`MessageBoxes` through
//! `ClearMessageStripe`), which is exact ground truth for the vanilla U ROM,
//! not a guess: each message's length is exactly the gap to the next label.

use crate::snes_utils::{
    addr::{AddrPc, AddrSnes},
    rom::Rom,
};

pub const MESSAGE_BOXES_SNES: AddrSnes = AddrSnes(0x05A5D9);
/// Exclusive end of the message data (start of `ClearMessageStripe`, real
/// code) — the hard upper bound for the combined size of all messages.
pub const MESSAGE_BOXES_END_SNES: AddrSnes = AddrSnes(0x05B0FF);
pub const MESSAGE_BOXES_MAX_SIZE: usize = 0x05B0FF - 0x05A5D9;

pub const MESSAGE_POINTER_TABLE_SNES: AddrSnes = AddrSnes(0x05A5A7);
pub const MESSAGE_POINTER_COUNT: usize = 25;

pub const MESSAGE_COUNT: usize = 22;

/// Names for the 22 unique vanilla messages, in ROM storage order (matching
/// `MESSAGE_START_OFFSETS`).
pub const MESSAGE_NAMES: [&str; MESSAGE_COUNT] = [
    "Intro",
    "Switch Palace",
    "Yoshi Gone",
    "Rescue Yoshi",
    "Fill Yellow (Yoshi Coin)",
    "Item Box (? Block)",
    "Hold Item",
    "Spin Jump",
    "Midway Point",
    "Dragon Coin",
    "Jump/Climb High",
    "Start+Select Reset",
    "Bonus Stars",
    "Climb Door",
    "Iggy Koopa",
    "Cape Mario",
    "Secret Exit",
    "Ghost House",
    "Screen Scroll",
    "Star World",
    "Vanilla Dome (CI2)",
    "Special World",
];

/// Byte offsets (relative to `MESSAGE_BOXES_SNES`) where each message starts,
/// in ROM storage order. Derived directly from consecutive label addresses in
/// `symbols/SMW_U.sym`; each message's length is the gap to the next entry
/// (or to `MESSAGE_BOXES_END_SNES` for the last one).
const MESSAGE_START_OFFSETS: [u32; MESSAGE_COUNT] = [
    0x0000, // Intro       (0x05A5D9)
    0x008D, // Switch Palace (0x05A666)
    0x0109, // Yoshi Gone   (0x05A6E2)
    0x0191, // Rescue Yoshi (0x05A76A)
    0x020A, // Fill Yellow  (0x05A7E3)
    0x0291, // Item Box     (0x05A86A)
    0x030B, // Hold Item    (0x05A8E4)
    0x038F, // Spin Jump    (0x05A968)
    0x041D, // Midway Point (0x05A9F6)
    0x04A0, // Dragon Coin  (0x05AA79)
    0x0518, // Jump/Climb High (0x05AAF1)
    0x05A4, // Start+Select (0x05AB7D)
    0x061D, // Bonus Stars  (0x05ABF6)
    0x06A6, // Climb Door   (0x05AC7F)
    0x0730, // Iggy Koopa   (0x05AD09)
    0x07B2, // Cape Mario   (0x05AD8B)
    0x083C, // Secret Exit  (0x05AE15)
    0x08B7, // Ghost House  (0x05AE90)
    0x0911, // Screen Scroll (0x05AEEA)
    0x099D, // Star World   (0x05AF76)
    0x0A2C, // Vanilla Dome (0x05B005)
    0x0A9E, // Special World (0x05B077)
];

/// For each of the 25 pointer-table entries, which `MESSAGE_NAMES`/
/// `MESSAGE_START_OFFSETS` index it refers to. Some messages (Switch Palace)
/// are referenced by more than one entry (once per palace color). Derived
/// from `DATA_05A5A7` in `bank_05.asm` (`dw XMessage-MessageBoxes` list).
pub const POINTER_TO_MESSAGE: [usize; MESSAGE_POINTER_COUNT] =
    [1, 1, 1, 1, 0, 5, 8, 10, 12, 17, 15, 6, 16, 19, 21, 9, 20, 13, 14, 18, 11, 7, 2, 4, 3];

/// First pointer-table slot (0-24) that displays message `i`. Routines like
/// `CODE_05B1BC` take the message-*type* index (into `DATA_05A5A7`), not the
/// message number, so preview/render entry points go through this.
pub fn pointer_slot_for_message(i: usize) -> u8 {
    POINTER_TO_MESSAGE.iter().position(|&m| m == i).unwrap_or(0) as u8
}

#[derive(Debug, Clone)]
pub struct MessageBoxes {
    /// Raw tile-index bytes (0x00-0x7F, bit 7 reserved) for each message, in
    /// `MESSAGE_NAMES` order.
    pub messages: Vec<Vec<u8>>,
}

impl MessageBoxes {
    pub fn parse(rom: &Rom) -> anyhow::Result<Self> {
        let base_pc = AddrPc::try_from_lorom(MESSAGE_BOXES_SNES)
            .map_err(|e| anyhow::anyhow!("MessageBoxes addr conversion: {e}"))?
            .0 as usize;
        let end_pc = AddrPc::try_from_lorom(MESSAGE_BOXES_END_SNES)
            .map_err(|e| anyhow::anyhow!("MessageBoxes end addr conversion: {e}"))?
            .0 as usize;
        if end_pc > rom.0.len() {
            anyhow::bail!("MessageBoxes data extends past end of ROM");
        }

        let mut messages = Vec::with_capacity(MESSAGE_COUNT);
        for i in 0..MESSAGE_COUNT {
            let start = base_pc + MESSAGE_START_OFFSETS[i] as usize;
            let end = if i + 1 < MESSAGE_COUNT { base_pc + MESSAGE_START_OFFSETS[i + 1] as usize } else { end_pc };
            if end > rom.0.len() || start > end {
                anyhow::bail!("Message {i} ({}) out of range", MESSAGE_NAMES[i]);
            }
            messages.push(rom.0[start..end].to_vec());
        }
        Ok(Self { messages })
    }

    /// Total combined byte size of all messages; must stay `<=
    /// MESSAGE_BOXES_MAX_SIZE` since the blob isn't repointable.
    pub fn total_size(&self) -> usize {
        self.messages.iter().map(Vec::len).sum()
    }

    /// Concatenate all messages (in `MESSAGE_NAMES` order) into one blob, and
    /// compute the corresponding 25-entry pointer table (relative offsets),
    /// ready to write back to `MESSAGE_BOXES_SNES`/`MESSAGE_POINTER_TABLE_SNES`.
    /// Errors if the combined size exceeds `MESSAGE_BOXES_MAX_SIZE`.
    pub fn to_blob_and_pointers(&self) -> anyhow::Result<(Vec<u8>, [u16; MESSAGE_POINTER_COUNT])> {
        let total = self.total_size();
        if total > MESSAGE_BOXES_MAX_SIZE {
            anyhow::bail!(
                "Combined message size {total} bytes exceeds the {MESSAGE_BOXES_MAX_SIZE}-byte budget \
                 (this data isn't repointable — it's addressed directly by ASM)"
            );
        }

        let mut offsets = [0u16; MESSAGE_COUNT];
        let mut acc = 0u32;
        for (i, msg) in self.messages.iter().enumerate() {
            offsets[i] = acc as u16;
            acc += msg.len() as u32;
        }

        let mut blob = Vec::with_capacity(total);
        for msg in &self.messages {
            blob.extend_from_slice(msg);
        }

        let mut pointers = [0u16; MESSAGE_POINTER_COUNT];
        for (slot, &msg_idx) in POINTER_TO_MESSAGE.iter().enumerate() {
            pointers[slot] = offsets[msg_idx];
        }

        Ok((blob, pointers))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> MessageBoxes {
        MessageBoxes { messages: (0..MESSAGE_COUNT).map(|i| vec![i as u8; 10]).collect() }
    }

    #[test]
    fn total_size_sums_all_messages() {
        assert_eq!(sample().total_size(), MESSAGE_COUNT * 10);
    }

    #[test]
    fn to_blob_and_pointers_concatenates_in_order() {
        let (blob, _) = sample().to_blob_and_pointers().unwrap();
        assert_eq!(blob.len(), MESSAGE_COUNT * 10);
        assert_eq!(&blob[0..10], &[0u8; 10]);
        assert_eq!(&blob[10..20], &[1u8; 10]);
    }

    #[test]
    fn pointer_table_reflects_recomputed_offsets() {
        let (_, pointers) = sample().to_blob_and_pointers().unwrap();
        // Slot 0-3 all point at message 1 (Switch Palace), at offset 10 (after message 0's 10 bytes).
        for slot in 0..4 {
            assert_eq!(pointers[slot], 10);
        }
        // Slot 4 points at message 0 (Intro), offset 0.
        assert_eq!(pointers[4], 0);
    }

    #[test]
    fn oversized_messages_are_rejected() {
        let boxes = MessageBoxes { messages: vec![vec![0u8; MESSAGE_BOXES_MAX_SIZE + 1]; 1] };
        assert!(boxes.to_blob_and_pointers().is_err());
    }

    #[test]
    fn vanilla_boundaries_are_internally_consistent() {
        // Offsets must be strictly increasing and the last message must fit
        // exactly within the budget when using the real vanilla lengths
        // (computed from consecutive symbol addresses).
        for w in MESSAGE_START_OFFSETS.windows(2) {
            assert!(w[0] < w[1]);
        }
        assert!(*MESSAGE_START_OFFSETS.last().unwrap() < MESSAGE_BOXES_MAX_SIZE as u32);
    }
}

#[cfg(test)]
mod real_rom_tests {
    use super::*;
    use crate::SmwRom;

    /// Verifies message parsing against the real ROM: correct message count,
    /// nonzero lengths matching the derived boundaries, and that re-encoding
    /// reproduces a blob of the same total size. Run with `ROM_PATH=/path/to/
    /// smw.smc cargo test -p smwe-rom --lib -- --ignored real_rom_message_boxes`.
    #[test]
    #[ignore]
    fn real_rom_message_boxes() {
        let rom_path = std::env::var("ROM_PATH").expect("set ROM_PATH");
        let rom = SmwRom::from_file(rom_path).expect("parse ROM");
        let boxes = &rom.message_boxes;

        assert_eq!(boxes.messages.len(), MESSAGE_COUNT);
        for (i, msg) in boxes.messages.iter().enumerate() {
            println!("{:24} ({:3} bytes): {:02X?}", MESSAGE_NAMES[i], msg.len(), &msg[..msg.len().min(16)]);
            assert!(!msg.is_empty(), "message {} ({}) is empty", i, MESSAGE_NAMES[i]);
        }

        let total = boxes.total_size();
        assert!(total <= MESSAGE_BOXES_MAX_SIZE);
        println!("total size: {total} / {MESSAGE_BOXES_MAX_SIZE} budget");

        let (blob, pointers) = boxes.to_blob_and_pointers().unwrap();
        assert_eq!(blob.len(), total);
        println!("pointers: {pointers:?}");
    }

    /// Verifies the Phase 2 editable-text codec against the real ROM: every
    /// vanilla message must decode to editable text and re-encode
    /// byte-exactly (each vanilla message has one bit-7 row terminator per
    /// row, so the canonical encoder regenerates the original bytes).
    /// Also reports any unmapped bytes per message (non-text graphic tiles,
    /// shown as '�' in the editor).
    ///
    /// Run with `ROM_PATH=/path/to/smw.smc cargo test -p smwe-rom --lib
    /// -- --ignored real_rom_editable_round_trip -- --nocapture`.
    #[test]
    #[ignore]
    fn real_rom_editable_round_trip() {
        use crate::font_map::{decode_editable_text, encode_editable_text, FontMap};
        let rom_path = std::env::var("ROM_PATH").expect("set ROM_PATH");
        let rom = SmwRom::from_file(rom_path).expect("parse ROM");
        let map = FontMap::real();

        for (i, msg) in rom.message_boxes.messages.iter().enumerate() {
            let text = decode_editable_text(&map, msg);
            let back = encode_editable_text(&map, msg, &text)
                .unwrap_or_else(|e| panic!("{}: re-encode failed: {e}", MESSAGE_NAMES[i]));
            assert_eq!(&back, msg, "{}: editable round-trip is not byte-exact", MESSAGE_NAMES[i]);

            // Report unmapped (graphic) bytes for the human-readable log.
            let mut unmapped: Vec<u8> = msg.iter().map(|b| b & 0x7F).filter(|b| map.char_for(*b).is_none()).collect();
            unmapped.sort_unstable();
            unmapped.dedup();
            println!("{:24} {:3} bytes  unmapped: {:02X?}", MESSAGE_NAMES[i], msg.len(), unmapped);
            println!("---");
            println!("{text}");
            println!("---");
        }
    }

    /// Dumps all 22 messages' raw bytes in exactly the tuple format consumed
    /// by `smwe_rom::font_map::derive_font_map`, so the TRUE font map can be
    /// completed the moment a real ROM is available: pair each dumped byte
    /// sequence with its known vanilla English text (fill in the `""`
    /// placeholders), identify the control-code bytes empirically (line break,
    /// end-of-message — compare byte positions against known line breaks in
    /// the English text), and pass the pairs to `derive_font_map`.
    ///
    /// Run with `ROM_PATH=/path/to/smw.smc cargo test -p smwe-rom --lib
    /// -- --ignored real_rom_dump_font_map_input -- --nocapture`.
    #[test]
    #[ignore]
    fn real_rom_dump_font_map_input() {
        let rom_path = std::env::var("ROM_PATH").expect("set ROM_PATH");
        let rom = SmwRom::from_file(rom_path).expect("parse ROM");

        println!("// === derive_font_map input (fill in each message's known vanilla English text) ===");
        println!("// let pairs: &[(&[u8], &str)] = &[");
        for (i, msg) in rom.message_boxes.messages.iter().enumerate() {
            let hex: Vec<String> = msg.iter().map(|b| format!("0x{b:02X}")).collect();
            println!("//   // {} ({} bytes)", MESSAGE_NAMES[i], msg.len());
            println!("//   (&[{}], \"\"),", hex.join(", "));
        }
        println!("// ];");
        println!("// let control_codes: &[u8] = &[/* line break, end-of-message, ... */];");
        println!("// let font_map = smwe_rom::font_map::derive_font_map(pairs, control_codes).unwrap();");
    }

    /// Runs all 22 vanilla messages through the REAL `CODE_05B1BC` via the
    /// emulator and validates the full pipeline:
    /// - 8 stripe commands per message (320 bytes total)
    /// - 18 tile words per command, every attribute byte `$39`
    /// - the stripe's 8×18 tile grid matches `font_map::message_cells`
    ///   (the row-aware bit-7 fill decoder) exactly
    /// - each message has exactly 8 bit-7 bytes (one row terminator per row)
    ///   and the decoder consumes exactly the message's full source length
    ///
    /// Run with `ROM_PATH=/path/to/smw.smc cargo test -p smwe-rom --lib
    /// -- --ignored real_rom_message_render_all`.
    #[test]
    #[ignore]
    fn real_rom_message_render_all() {
        use std::sync::Arc;
        let rom_path = std::env::var("ROM_PATH").expect("set ROM_PATH");
        let rom = SmwRom::from_file(&rom_path).expect("parse ROM");

        for (i, msg) in rom.message_boxes.messages.iter().enumerate() {
            let name = MESSAGE_NAMES[i];
            let slot = pointer_slot_for_message(i);

            // Run the genuine CODE_05B1BC.
            let raw = std::fs::read(&rom_path).expect("read ROM");
            let rom_bytes = if raw.len() % 0x400 == 0x200 { raw[0x200..].to_vec() } else { raw };
            let mut emu_rom = smwe_emu::rom::Rom::new(rom_bytes);
            emu_rom.load_symbols(include_str!("../../../symbols/SMW_U.sym"));
            let mut cpu = smwe_emu::Cpu::new(smwe_emu::emu::CheckedMem::new(Arc::new(emu_rom)));
            let stripe = smwe_emu::emu::render_message(&mut cpu, slot);

            // 8 commands, 320 bytes.
            assert_eq!(stripe.stripe.len(), 320, "{name}: stripe length");
            let cmds = smwe_emu::emu::parse_stripe_commands(&stripe.stripe)
                .unwrap_or_else(|e| panic!("{name}: stripe parse failed: {e}"));
            assert_eq!(cmds.len(), 8, "{name}: command count");

            // 18 tile words per command, all attributes $39.
            let mut cells = [[0u8; 18]; 8];
            for (r, cmd) in cmds.iter().enumerate() {
                assert_eq!(cmd.tiles.len(), 18, "{name} row {r}: tile count");
                for (c, &t) in cmd.tiles.iter().enumerate() {
                    assert_eq!(t & 0xFF00, 0x3900, "{name} row {r} col {c}: tile word {t:#06X}");
                    cells[r][c] = (t & 0xFF) as u8;
                }
            }

            // Stripe grid must match the row-aware decoder exactly.
            let expected = crate::font_map::message_cells(msg);
            assert_eq!(cells, expected, "{name}: stripe disagrees with message_cells");

            // Exactly 8 bit-7 bytes (one row terminator per row), and the
            // decoder consumes the full source length.
            let bit7_count = msg.iter().filter(|&&b| b & 0x80 != 0).count();
            assert_eq!(bit7_count, 8, "{name}: expected 8 bit-7 row terminators, found {bit7_count}");
            println!("{name:24} {:3} bytes, 8 terminators, stripe OK", msg.len());
        }
    }
}
