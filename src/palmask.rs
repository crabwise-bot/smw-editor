//! `.palmask` files for selective palette import (Lunar Magic v2.40,
//! 2016-09-24: "added support for .palmask files, which let palette files
//! specify which colors in the palette should be imported into the level,
//! plus new palette-editor buttons to work with them").
//!
//! Recovered semantics (observed against real Lunar Magic 3.63 through
//! Wine-driven tests, documented in a clean-room reimplementation's public
//! architecture notes and run log — the strongest public evidence of the
//! format, since the official help file only describes it in prose):
//!
//! - A `.palmask` file is exactly 257 bytes. Byte `i` (`0..=256`)
//!   corresponds to palette word `i` of the 257-word working buffer the
//!   palette files carry (the `.mw3` layout). A zero byte keeps the
//!   destination word; **any nonzero byte** takes the source word.
//! - The palette editor's Import button discovers an *optional* same-name
//!   `.palmask` next to the chosen palette file (`foo.mw3` → `foo.palmask`)
//!   and applies it; when no mask file exists the import is unmasked.
//! - Masked application validates all three shapes (source palette words,
//!   mask bytes, destination words) *before* touching anything, so a
//!   malformed mask or palette file can never half-apply; then it copies
//!   only the selected words, and finally handles the selected row-zero
//!   indices `0, 16, …, 240` exactly like the ordinary loader — they are
//!   cleared to the backdrop word (word 256), because row-zero colors are
//!   unsupported in the file frame of reference.
//! - The masked import auto-enables the level's custom palette, and the
//!   palette editor republishes its current selector beside every palette
//!   export as `<export name>.palmask`.
//!
//! This editor's level palette is the 36 colors of the level's BG/FG/sprite
//! rows (`.mw3` words `0..36`; words `36..257` are zero on export and
//! ignored on import), so the masked import composes a 257-word window
//! from the current 36 colors, applies the recovered loader semantics
//! over the whole window, and writes words `0..36` back. Word 256 (the
//! backdrop) reads as 0 in that window — the editor never models it — so a
//! selected row-zero index (`0`, `16`, `32` within the 36-color range)
//! imports as the backdrop (0), exactly per the recovered loader. The UI
//! edits mask bits for words `0..36` only; bits `36..257` stay at the
//! default (selected), matching Lunar Magic's default "everything
//! enabled" transient selector.
//!
//! Honest limits: LM's exact v2.40 palette-editor button labels are not in
//! the public docs, so the mask buttons here ("Edit mask", "Select
//! all/none", "Invert", "Save/Load mask") implement the documented
//! *capability* (buttons to work with the masks), not the exact labels. The
//! byte value written for a selected word is `0x01`; any nonzero byte is
//! accepted on read, which is the only property the recovered loader
//! depends on.

use std::fmt;

/// Words in Lunar Magic's 257-word palette working buffer (16×16 colors +
/// the backdrop word), and bytes in a `.palmask` file.
pub const PALMASK_WORDS: usize = 257;
/// Row-zero word indices (`0, 16, …, 240`): unsupported in the file frame
/// of reference, cleared to the backdrop word on import.
pub const ROW_ZERO_INDICES: [usize; 16] = [0, 16, 32, 48, 64, 80, 96, 112, 128, 144, 160, 176, 192, 208, 224, 240];
/// Byte value written for a selected word. The recovered rule only needs
/// "nonzero selects the source"; `0x01` round-trips through this module.
pub const PALMASK_SELECTED_BYTE: u8 = 0x01;

/// Errors from the `.palmask` parser. The parser is strict about size:
/// anything that is not exactly 257 bytes is rejected, so a truncated or
/// foreign file can never half-apply.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PalmaskError {
    BadSize { expected: usize, got: usize },
}

impl fmt::Display for PalmaskError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            PalmaskError::BadSize { expected, got } => {
                write!(f, ".palmask file must be exactly {expected} bytes, got {got}")
            }
        }
    }
}

impl std::error::Error for PalmaskError {}

/// A `.palmask` selection mask: 257 bytes, one per palette word. Zero keeps
/// the destination; any nonzero byte takes the source.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Palmask {
    bytes: [u8; PALMASK_WORDS],
}

impl Palmask {
    /// The default selector: everything enabled (matches Lunar Magic's
    /// default transient selector — an unmasked import).
    pub fn all() -> Self {
        Self { bytes: [PALMASK_SELECTED_BYTE; PALMASK_WORDS] }
    }

    /// Nothing selected: a masked import would change nothing.
    pub fn none() -> Self {
        Self { bytes: [0u8; PALMASK_WORDS] }
    }

    /// Strict parse: exactly 257 bytes, else rejected before anything
    /// changes.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, PalmaskError> {
        if bytes.len() != PALMASK_WORDS {
            return Err(PalmaskError::BadSize { expected: PALMASK_WORDS, got: bytes.len() });
        }
        let mut mask = Self::none();
        mask.bytes.copy_from_slice(bytes);
        Ok(mask)
    }

    /// Serialize to the 257-byte file format.
    pub fn to_bytes(&self) -> [u8; PALMASK_WORDS] {
        self.bytes
    }

    /// Whether word `word` (`0..257`) is selected (nonzero byte).
    /// Out-of-range indices are never selected.
    pub fn selected(&self, word: usize) -> bool {
        word < PALMASK_WORDS && self.bytes[word] != 0
    }

    /// Set the selection bit for one word. Out-of-range indices are
    /// ignored.
    pub fn set(&mut self, word: usize, selected: bool) {
        if word < PALMASK_WORDS {
            self.bytes[word] = if selected { PALMASK_SELECTED_BYTE } else { 0 };
        }
    }

    /// Toggle the selection bit for one word; returns the new state.
    /// Out-of-range indices return `false`.
    pub fn toggle(&mut self, word: usize) -> bool {
        if word < PALMASK_WORDS {
            let next = self.bytes[word] == 0;
            self.bytes[word] = if next { PALMASK_SELECTED_BYTE } else { 0 };
            next
        } else {
            false
        }
    }

    /// Select every word.
    pub fn select_all(&mut self) {
        self.bytes = [PALMASK_SELECTED_BYTE; PALMASK_WORDS];
    }

    /// Deselect every word.
    pub fn select_none(&mut self) {
        self.bytes = [0u8; PALMASK_WORDS];
    }

    /// Invert the whole 257-word selector.
    pub fn invert(&mut self) {
        for b in self.bytes.iter_mut() {
            *b = if *b == 0 { PALMASK_SELECTED_BYTE } else { 0 };
        }
    }

    /// Number of selected words.
    pub fn selected_count(&self) -> usize {
        self.bytes.iter().filter(|&&b| b != 0).count()
    }
}

impl Default for Palmask {
    /// Default = everything enabled, matching Lunar Magic's transient
    /// selector reset state.
    fn default() -> Self {
        Self::all()
    }
}

/// Apply a masked palette import with the recovered Lunar Magic loader
/// semantics: copy only the selected words, then clear the selected
/// row-zero indices (`0, 16, …, 240`) to the backdrop word (word 256).
///
/// All three shapes are fixed at the type level (`[u16; 257]` /
/// `Palmask`), so by the time this runs the "validate before cloning"
/// step is already satisfied — callers only reach here with strict-parsed
/// inputs.
pub fn apply_masked_import(dest: &mut [u16; PALMASK_WORDS], src: &[u16; PALMASK_WORDS], mask: &Palmask) {
    for i in 0..PALMASK_WORDS {
        if mask.selected(i) {
            dest[i] = src[i];
        }
    }
    // Row-zero colors are unsupported in the file frame of reference: the
    // ordinary loader clears them to the backdrop word, and the masked
    // variant clears exactly the selected ones.
    let backdrop = dest[PALMASK_WORDS - 1];
    for &i in ROW_ZERO_INDICES.iter() {
        if mask.selected(i) {
            dest[i] = backdrop;
        }
    }
}

/// Word index in the 257-word palette-file layout for one of this
/// editor's 36 on-screen colors (`group`: 0=BG, 1=FG, 2=sprite; `col`:
/// 0..12).
pub fn level_color_word_index(group: usize, col: usize) -> Option<usize> {
    if group < 3 && col < 12 {
        Some(group * 12 + col)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_rejects_wrong_sizes() {
        for len in [0, 1, 256, 258, 514] {
            let bytes = vec![0u8; len];
            assert_eq!(
                Palmask::from_bytes(&bytes),
                Err(PalmaskError::BadSize { expected: PALMASK_WORDS, got: len }),
                "must reject {len} bytes"
            );
        }
    }

    #[test]
    fn parse_accepts_exactly_257_bytes() {
        let bytes = vec![0x01u8; 257];
        let mask = Palmask::from_bytes(&bytes).unwrap();
        assert_eq!(mask.selected_count(), 257);
        assert_eq!(mask.to_bytes(), bytes.as_slice());
    }

    #[test]
    fn nonzero_byte_selects() {
        let mut bytes = [0u8; 257];
        bytes[5] = 0xFF;
        bytes[200] = 0x42;
        let mask = Palmask::from_bytes(&bytes).unwrap();
        assert!(mask.selected(5));
        assert!(mask.selected(200));
        assert!(!mask.selected(6));
        assert!(!mask.selected(257)); // out of range: never selected
    }

    #[test]
    fn all_selects_everything_and_none_selects_nothing() {
        let all = Palmask::all();
        assert_eq!(all.selected_count(), 257);
        let none = Palmask::none();
        assert_eq!(none.selected_count(), 0);
        assert_eq!(Palmask::default(), all);
    }

    #[test]
    fn set_toggle_invert_behave() {
        let mut mask = Palmask::none();
        assert!(mask.toggle(10));
        assert!(mask.selected(10));
        assert!(!mask.toggle(10));
        assert!(!mask.selected(10));
        mask.set(20, true);
        assert!(mask.selected(20));
        mask.set(20, false);
        assert!(!mask.selected(20));
        // Out of range is ignored.
        mask.set(999, true);
        assert_eq!(mask.selected_count(), 0);

        mask.select_all();
        assert_eq!(mask.selected_count(), 257);
        mask.invert();
        assert_eq!(mask.selected_count(), 0);
        mask.invert();
        assert_eq!(mask.selected_count(), 257);
        mask.select_none();
        assert_eq!(mask.selected_count(), 0);
    }

    #[test]
    fn masked_import_copies_only_selected_words() {
        let mut dest = [0x1111u16; 257];
        let mut src = [0u16; 257];
        for (i, w) in src.iter_mut().enumerate() {
            *w = 0x2000 + i as u16;
        }
        let mut mask = Palmask::none();
        mask.set(1, true);
        mask.set(100, true);
        apply_masked_import(&mut dest, &src, &mask);
        assert_eq!(dest[1], 0x2001);
        assert_eq!(dest[100], 0x2064);
        // Everything else is untouched — including the row-zero words,
        // which were not selected here.
        assert_eq!(dest[0], 0x1111);
        assert_eq!(dest[2], 0x1111);
        assert_eq!(dest[256], 0x1111);
    }

    #[test]
    fn masked_import_clears_selected_row_zero_indices_to_backdrop() {
        let mut dest = [0x1111u16; 257];
        dest[256] = 0x02AA; // backdrop word
        let mut src = [0u16; 257];
        for (i, w) in src.iter_mut().enumerate() {
            *w = 0x3000 + i as u16;
        }
        let mut mask = Palmask::none();
        // Select row-zero words 0, 16, 32 and one ordinary word 5.
        for i in [0usize, 16, 32, 5] {
            mask.set(i, true);
        }
        apply_masked_import(&mut dest, &src, &mask);
        // Ordinary selected word: copied from the source.
        assert_eq!(dest[5], 0x3005);
        // Selected row-zero words: cleared to the backdrop word, not the
        // source's row-zero values.
        assert_eq!(dest[0], 0x02AA);
        assert_eq!(dest[16], 0x02AA);
        assert_eq!(dest[32], 0x02AA);
        // Unselected row-zero words keep the destination.
        assert_eq!(dest[48], 0x1111);
    }

    #[test]
    fn masked_import_uses_post_apply_backdrop() {
        // When the backdrop word itself is selected, row-zero clearing
        // uses the *new* backdrop (the recovered loader clears after
        // applying).
        let mut dest = [0x1111u16; 257];
        let mut src = [0u16; 257];
        src[256] = 0x0555;
        let mut mask = Palmask::none();
        mask.set(0, true);
        mask.set(256, true);
        apply_masked_import(&mut dest, &src, &mask);
        assert_eq!(dest[256], 0x0555);
        assert_eq!(dest[0], 0x0555);
    }

    #[test]
    fn unmasked_equivalent_all_mask_is_identity_except_row_zero() {
        // An all-selected mask reproduces the ordinary loader: every word
        // copied, row-zero words cleared to the backdrop.
        let mut dest = [0x1111u16; 257];
        let mut src = [0u16; 257];
        for (i, w) in src.iter_mut().enumerate() {
            *w = i as u16;
        }
        src[256] = 0x0F0F;
        apply_masked_import(&mut dest, &src, &Palmask::all());
        for &i in ROW_ZERO_INDICES.iter() {
            assert_eq!(dest[i], 0x0F0F, "row-zero word {i}");
        }
        assert_eq!(dest[1], 1);
        assert_eq!(dest[256], 0x0F0F);
    }

    #[test]
    fn level_color_word_index_maps_36_colors() {
        assert_eq!(level_color_word_index(0, 0), Some(0));
        assert_eq!(level_color_word_index(0, 11), Some(11));
        assert_eq!(level_color_word_index(1, 0), Some(12));
        assert_eq!(level_color_word_index(2, 11), Some(35));
        assert_eq!(level_color_word_index(3, 0), None);
        assert_eq!(level_color_word_index(0, 12), None);
    }

    #[test]
    fn row_zero_indices_are_the_sixteen() {
        assert_eq!(ROW_ZERO_INDICES.len(), 16);
        assert_eq!(ROW_ZERO_INDICES[0], 0);
        assert_eq!(ROW_ZERO_INDICES[15], 240);
        for (r, &i) in ROW_ZERO_INDICES.iter().enumerate() {
            assert_eq!(i, r * 16);
        }
    }
}
