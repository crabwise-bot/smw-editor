//! Lunar Magic Layer 2 scroll modes.
//!
//! The vanilla game stores a single 4-bit "Layer 2 scroll" value in the high
//! nibble of the `$05F000` secondary-header byte. The value is an index into
//! paired (horizontal, vertical) presets (`DATA_05D710` vertical mapping,
//! `DATA_05D720` horizontal mapping in the vanilla disassembly).
//!
//! Lunar Magic 3.00 added four new rates ("Variable 2/3/4", "Slow 2") in the
//! previously blank paired slots 8-11, and renamed the rates in 3.40
//! ("Variable" -> "Medium", "Normal" -> "Constant").
//!
//! Lunar Magic 3.40 added a per-level extension byte at SNES `$06FA00`
//! (`SHCvvvvv`):
//! - `S` (bit 7): use separate horizontal/vertical settings instead of the
//!   paired preset.
//! - `H` (bit 6): horizontal auto-scroll flag. When set, the `$05F000` high
//!   nibble (`hhhh`) selects among the 12 horizontal auto-scroll modes
//!   (0-5 = Auto-Scroll Left Slow..Fast 4, 6-11 = Auto-Scroll Right
//!   Slow..Fast 4) instead of a plain speed. (Inferred from the LM 3.40
//!   scroll-name string table layout; the bit is undocumented in public
//!   references.)
//! - `C` (bit 5): auto-set the number of screens in the level.
//! - `vvvvv` (bits 0-4): vertical scroll setting (0-31) when `S` is set.
//!
//! When `S` is clear, `hhhh` is the paired preset index (0-15) and `vvvvv`
//! is ignored. A vanilla ROM has `$FF` at `$06FA00` (table not installed);
//! LM initializes the byte to `$20` (auto-set screens on) on first save.

// -------------------------------------------------------------------------------------------------
// Rate names (LM 3.40 "Change Properties in Header" dialog)
// -------------------------------------------------------------------------------------------------

/// Plain scroll speeds, indices 0-8. Shared by the paired presets and the
/// separate horizontal/vertical dropdowns.
pub const SCROLL_RATE_NAMES: [&str; 9] =
    ["None", "Constant", "Medium", "Slow", "Medium 2", "Medium 3", "Medium 4", "Slow 2", "Fast"];

/// (horizontal rate index, vertical rate index) for each paired preset 0-15.
///
/// 0-7 are the vanilla game's presets; 8-11 are the LM 3.00 additions
/// ("Variable 2/3/4" -> "Medium 2/3/4", plus "Slow 2"); 12-15 were blank
/// (H/V Scroll of None) through LM 3.00 and remain so — LM 3.40's new
/// "Fast"/"Auto-Scroll" modes live in the separate horizontal/vertical
/// settings instead of the paired list.
pub const PAIRED_SCROLL_RATES: [(u8, u8); 16] = [
    (2, 6), // 0: H Medium,     V Medium 4
    (2, 1), // 1: H Medium,     V Constant
    (1, 1), // 2: H Constant,   V Constant
    (0, 0), // 3: H None,       V None
    (1, 0), // 4: H Constant,   V None
    (2, 2), // 5: H Medium,     V Medium
    (1, 2), // 6: H Constant,   V Medium
    (0, 1), // 7: H None,       V Constant
    (0, 4), // 8: H None,       V Medium 2 (LM 3.00)
    (0, 5), // 9: H None,       V Medium 3 (LM 3.00)
    (0, 6), // A: H None,       V Medium 4 (LM 3.00)
    (0, 7), // B: H None,       V Slow 2   (LM 3.00)
    (0, 0), // C: (blank in LM)
    (0, 0), // D: (blank in LM)
    (0, 0), // E: (blank in LM)
    (0, 0), // F: (blank in LM)
];

/// Human-readable label for a paired preset, e.g. `"H-Scroll: Medium, V-Scroll: Constant"`.
pub fn paired_scroll_label(preset: u8) -> String {
    let (h, v) = PAIRED_SCROLL_RATES[(preset & 0x0F) as usize];
    format!("H-Scroll: {}, V-Scroll: {}", SCROLL_RATE_NAMES[h as usize], SCROLL_RATE_NAMES[v as usize])
}

// -------------------------------------------------------------------------------------------------
// Separate horizontal/vertical dropdowns (LM 3.40)
// -------------------------------------------------------------------------------------------------

/// Vertical dropdown labels, indices 0-31 (stored in `$06FA00` bits 0-4).
pub const VSCROLL_NAMES: [&str; 32] = [
    "None",
    "Constant",
    "Medium",
    "Slow",
    "Medium 2",
    "Medium 3",
    "Medium 4",
    "Slow 2",
    "Fast",
    "Not Used 1",
    "Not Used 2",
    "Not Used 3",
    "Not Used 4",
    "Not Used 5",
    "Not Used 6",
    "Not Used 7",
    "Auto-Scroll Up Slow",
    "Auto-Scroll Up Medium",
    "Auto-Scroll Up Fast",
    "Auto-Scroll Up Fast 2",
    "Auto-Scroll Up Fast 3",
    "Auto-Scroll Up Fast 4",
    "Auto-Scroll Down Slow",
    "Auto-Scroll Down Medium",
    "Auto-Scroll Down Fast",
    "Auto-Scroll Down Fast 2",
    "Auto-Scroll Down Fast 3",
    "Auto-Scroll Down Fast 4",
    "Not Used 8",
    "Not Used 9",
    "Not Used A",
    "Not Used B",
];

/// A horizontal dropdown entry: the `$06FA00` H-bit and the `$05F000` high
/// nibble (`hhhh`) it encodes to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HScrollEntry {
    /// Value of the `$06FA00` H bit (bit 6): horizontal auto-scroll flag.
    pub h_bit: bool,
    /// `$05F000` high nibble (0-15).
    pub hhhh:  u8,
    /// Display label.
    pub name:  &'static str,
}

/// Horizontal dropdown entries: plain speeds (`hhhh` 0-15, H bit clear) plus
/// the 12 horizontal auto-scroll modes (`hhhh` 0-11, H bit set).
pub const HSCROLL_ENTRIES: [HScrollEntry; 28] = [
    HScrollEntry { h_bit: false, hhhh: 0, name: "None" },
    HScrollEntry { h_bit: false, hhhh: 1, name: "Constant" },
    HScrollEntry { h_bit: false, hhhh: 2, name: "Medium" },
    HScrollEntry { h_bit: false, hhhh: 3, name: "Slow" },
    HScrollEntry { h_bit: false, hhhh: 4, name: "Medium 2" },
    HScrollEntry { h_bit: false, hhhh: 5, name: "Medium 3" },
    HScrollEntry { h_bit: false, hhhh: 6, name: "Medium 4" },
    HScrollEntry { h_bit: false, hhhh: 7, name: "Slow 2" },
    HScrollEntry { h_bit: false, hhhh: 8, name: "Fast" },
    HScrollEntry { h_bit: false, hhhh: 9, name: "Not Used 1" },
    HScrollEntry { h_bit: false, hhhh: 10, name: "Not Used 2" },
    HScrollEntry { h_bit: false, hhhh: 11, name: "Not Used 3" },
    HScrollEntry { h_bit: false, hhhh: 12, name: "Not Used 4" },
    HScrollEntry { h_bit: false, hhhh: 13, name: "Not Used 5" },
    HScrollEntry { h_bit: false, hhhh: 14, name: "Not Used 6" },
    HScrollEntry { h_bit: false, hhhh: 15, name: "Not Used 7" },
    HScrollEntry { h_bit: true, hhhh: 0, name: "Auto-Scroll Left Slow" },
    HScrollEntry { h_bit: true, hhhh: 1, name: "Auto-Scroll Left Medium" },
    HScrollEntry { h_bit: true, hhhh: 2, name: "Auto-Scroll Left Fast" },
    HScrollEntry { h_bit: true, hhhh: 3, name: "Auto-Scroll Left Fast 2" },
    HScrollEntry { h_bit: true, hhhh: 4, name: "Auto-Scroll Left Fast 3" },
    HScrollEntry { h_bit: true, hhhh: 5, name: "Auto-Scroll Left Fast 4" },
    HScrollEntry { h_bit: true, hhhh: 6, name: "Auto-Scroll Right Slow" },
    HScrollEntry { h_bit: true, hhhh: 7, name: "Auto-Scroll Right Medium" },
    HScrollEntry { h_bit: true, hhhh: 8, name: "Auto-Scroll Right Fast" },
    HScrollEntry { h_bit: true, hhhh: 9, name: "Auto-Scroll Right Fast 2" },
    HScrollEntry { h_bit: true, hhhh: 10, name: "Auto-Scroll Right Fast 3" },
    HScrollEntry { h_bit: true, hhhh: 11, name: "Auto-Scroll Right Fast 4" },
];

/// Find the horizontal dropdown entry for an (`h_bit`, `hhhh`) pair.
pub fn hscroll_entry(h_bit: bool, hhhh: u8) -> Option<&'static HScrollEntry> {
    HSCROLL_ENTRIES.iter().find(|e| e.h_bit == h_bit && e.hhhh == (hhhh & 0x0F))
}

// -------------------------------------------------------------------------------------------------
// $06FA00 extension byte
// -------------------------------------------------------------------------------------------------

/// Value of `$06FA00` in a ROM where LM never installed the table.
pub const SCROLL_EXT_UNINSTALLED: u8 = 0xFF;

/// Value LM writes on first install (auto-set screens on, paired mode).
pub const SCROLL_EXT_INITIAL: u8 = 0x20;

/// Decoded LM 3.40 Layer 2 scroll extension byte (`SHCvvvvv`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Layer2ScrollExt {
    /// S bit: separate horizontal/vertical settings.
    pub separate:         bool,
    /// H bit: horizontal auto-scroll flag (see `HSCROLL_ENTRIES`).
    pub h_auto:           bool,
    /// C bit: auto-set the number of screens in the level.
    pub auto_set_screens: bool,
    /// Vertical scroll setting (0-31), meaningful when `separate`.
    pub vscroll:          u8,
}

impl Layer2ScrollExt {
    pub fn decode(byte: u8) -> Self {
        Self {
            separate:         (byte & 0x80) != 0,
            h_auto:           (byte & 0x40) != 0,
            auto_set_screens: (byte & 0x20) != 0,
            vscroll:          byte & 0x1F,
        }
    }

    pub fn encode(self) -> u8 {
        ((self.separate as u8) << 7)
            | ((self.h_auto as u8) << 6)
            | ((self.auto_set_screens as u8) << 5)
            | (self.vscroll & 0x1F)
    }

    /// Whether the table looks installed (anything other than erased `$FF`).
    pub fn is_installed(byte: u8) -> bool {
        byte != SCROLL_EXT_UNINSTALLED
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paired_labels_match_vanilla_and_lm_additions() {
        // Vanilla presets (verified against SMWDisX DATA_05D710/DATA_05D720).
        assert_eq!(paired_scroll_label(0), "H-Scroll: Medium, V-Scroll: Medium 4");
        assert_eq!(paired_scroll_label(1), "H-Scroll: Medium, V-Scroll: Constant");
        assert_eq!(paired_scroll_label(2), "H-Scroll: Constant, V-Scroll: Constant");
        assert_eq!(paired_scroll_label(3), "H-Scroll: None, V-Scroll: None");
        assert_eq!(paired_scroll_label(4), "H-Scroll: Constant, V-Scroll: None");
        assert_eq!(paired_scroll_label(5), "H-Scroll: Medium, V-Scroll: Medium");
        assert_eq!(paired_scroll_label(6), "H-Scroll: Constant, V-Scroll: Medium");
        assert_eq!(paired_scroll_label(7), "H-Scroll: None, V-Scroll: Constant");
        // LM 3.00 additions.
        assert_eq!(paired_scroll_label(8), "H-Scroll: None, V-Scroll: Medium 2");
        assert_eq!(paired_scroll_label(9), "H-Scroll: None, V-Scroll: Medium 3");
        assert_eq!(paired_scroll_label(10), "H-Scroll: None, V-Scroll: Medium 4");
        assert_eq!(paired_scroll_label(11), "H-Scroll: None, V-Scroll: Slow 2");
    }

    #[test]
    fn scroll_ext_round_trip() {
        let ext = Layer2ScrollExt {
            separate:         true,
            h_auto:           true,
            auto_set_screens: true,
            vscroll:          21,
        };
        let byte = ext.encode();
        assert_eq!(byte, 0x80 | 0x40 | 0x20 | 21);
        assert_eq!(Layer2ScrollExt::decode(byte), ext);
        // LM's initial value: auto-set screens, paired mode.
        let init = Layer2ScrollExt::decode(SCROLL_EXT_INITIAL);
        assert!(!init.separate && !init.h_auto && init.auto_set_screens && init.vscroll == 0);
        assert!(!Layer2ScrollExt::is_installed(SCROLL_EXT_UNINSTALLED));
        assert!(Layer2ScrollExt::is_installed(SCROLL_EXT_INITIAL));
    }

    #[test]
    fn hscroll_entries_cover_auto_scroll() {
        // 16 plain speeds + 12 horizontal auto-scrolls.
        assert_eq!(HSCROLL_ENTRIES.len(), 28);
        let left_fast = hscroll_entry(true, 2).unwrap();
        assert_eq!(left_fast.name, "Auto-Scroll Left Fast");
        let right_fast4 = hscroll_entry(true, 11).unwrap();
        assert_eq!(right_fast4.name, "Auto-Scroll Right Fast 4");
        assert_eq!(hscroll_entry(false, 8).unwrap().name, "Fast");
        assert!(hscroll_entry(true, 12).is_none());
    }

    #[test]
    fn vscroll_names_cover_auto_scroll() {
        assert_eq!(VSCROLL_NAMES[16], "Auto-Scroll Up Slow");
        assert_eq!(VSCROLL_NAMES[27], "Auto-Scroll Down Fast 4");
        assert_eq!(VSCROLL_NAMES[8], "Fast");
    }
}
