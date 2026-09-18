//! Lunar Magic 3.40 "Auto-Set Number of Screens" — the on-save behavior.
//!
//! LM 3.40 moved "Auto-Set Number of Screens" from General Options to the
//! "Change Properties in Header" dialog as a per-level setting, stored in
//! bit 5 (the C bit, `$20`) of the `$06FA00` Layer 2 scroll extension byte
//! (`SHCvvvvv`; see `smwe_rom::level::scroll::Layer2ScrollExt`). PR #36 wired
//! the bit through the UI and the ROM/MWL round-trip; this module implements
//! what LM actually *does* with it: on save, the level's Number of Screens
//! (primary header byte 0, `LLLLL` = screens − 1) is rewritten to the number
//! of screens the level's objects and sprites actually occupy.
//!
//! Evidence for the bit meaning: the official LM 3.40 release notes say the
//! option "makes 'Auto-Set Number of Screens' a per-level setting", and the
//! only per-level header byte added in 3.40 is `$06FA00` (SMW speedrunning
//! wiki level-data format doc); LM's initial table value is `$20`
//! (john-sparwasser/pipe-dream `LM_PARITY.md`), i.e. auto-set on, paired
//! scroll mode. The planetemu 3.40 changelog confirms the move from General
//! Options to the header dialog.
//!
//! Approximations (documented, not LM-verified): object occupancy is measured
//! by anchor tile — the editor does not track per-object pixel extents, so a
//! wide object whose tiles spill onto the next screen counts only its anchor
//! screen. Exits count by their screen number. Main/midway entrance screens
//! (secondary-header fields, not object data) are not counted. Empty levels
//! keep 1 screen; the result is clamped to the 5-bit header field (1–32
//! screens) rather than any per-mode cap, matching the editor's existing
//! Level Length slider (0–31 raw).

use smwe_rom::level::scroll::Layer2ScrollExt;

use super::{object_layer::EditableObjectLayer, sprite_layer::EditableSpriteLayer};

/// 16x16 tiles per screen along the level's long axis.
const TILES_PER_SCREEN: u32 = 16;

/// Maximum screens encodable in the primary header's 5-bit length field.
const MAX_SCREENS: u32 = 32;

/// Number of screens the level occupies, from the current editor state.
///
/// Scans layer-1 objects and exits, layer-2 objects when the level has them,
/// and sprites; takes each entry's screen index along the level's long axis
/// (X for horizontal levels, Y for vertical levels) and returns
/// `max_screen + 1`, clamped to `1..=32`.
pub(super) fn screens_used(
    l1: &EditableObjectLayer, l2: Option<&EditableObjectLayer>, sprites: &EditableSpriteLayer, vertical: bool,
) -> u32 {
    let mut max_screen = 0u32;
    let mut seen = false;
    let mut consider = |screen: u32| {
        seen = true;
        max_screen = max_screen.max(screen);
    };
    for layer in std::iter::once(l1).chain(l2) {
        for obj in &layer.objects {
            consider(axis_screen(obj.x, obj.y, vertical));
        }
        for exit in &layer.exits {
            consider(exit.screen as u32);
        }
    }
    for spr in &sprites.sprites {
        consider(axis_screen(spr.x, spr.y, vertical));
    }
    if !seen {
        return 1;
    }
    (max_screen + 1).clamp(1, MAX_SCREENS)
}

/// Screen index of an absolute tile coordinate along the level's long axis.
fn axis_screen(x: u32, y: u32, vertical: bool) -> u32 {
    if vertical {
        y / TILES_PER_SCREEN
    } else {
        x / TILES_PER_SCREEN
    }
}

/// Whether the Auto-Set Number of Screens recompute applies on this save.
///
/// The per-level C bit only takes effect once the `$06FA00` table is
/// installed, or this save installs it because the user touched a
/// scroll-extension control (`scroll_ext_dirty`). A vanilla ROM (`$FF`,
/// untouched) is never resized by the default-on checkbox.
pub(super) fn auto_set_applies(scroll_ext_raw: u8, scroll_ext_dirty: bool, auto_set: bool) -> bool {
    (Layer2ScrollExt::is_installed(scroll_ext_raw) || scroll_ext_dirty) && auto_set
}

/// Whether this save installs the `$06FA00` table: it was already installed,
/// separate H/V mode is on, or the user touched a scroll-extension control.
/// Otherwise vanilla ROMs keep `$FF` (never installed behind the user's back).
pub(super) fn scroll_ext_installs(scroll_ext_raw: u8, separate: bool, scroll_ext_dirty: bool) -> bool {
    Layer2ScrollExt::is_installed(scroll_ext_raw) || separate || scroll_ext_dirty
}

#[cfg(test)]
mod tests {
    use super::{
        super::{
            object_layer::{EditableExit, EditableObject, EditableObjectLayer},
            sprite_layer::{EditableSprite, EditableSpriteLayer},
        },
        auto_set_applies,
        screens_used,
        scroll_ext_installs,
    };

    fn obj(x: u32, y: u32) -> EditableObject {
        EditableObject { x, y, id: 0x2F, settings: 0, is_extended: false, extended_id: 0 }
    }

    fn spr(x: u32, y: u32) -> EditableSprite {
        EditableSprite { x, y, sprite_id: 0x35, extra_bits: 0 }
    }

    fn l1(objects: Vec<EditableObject>, exits: Vec<EditableExit>) -> EditableObjectLayer {
        EditableObjectLayer { objects, exits }
    }

    fn sprites(sprites: Vec<EditableSprite>) -> EditableSpriteLayer {
        EditableSpriteLayer { sprites }
    }

    #[test]
    fn empty_level_keeps_one_screen() {
        assert_eq!(screens_used(&l1(vec![], vec![]), None, &sprites(vec![]), false), 1);
        assert_eq!(screens_used(&l1(vec![], vec![]), None, &sprites(vec![]), true), 1);
    }

    #[test]
    fn horizontal_uses_x_axis() {
        // Object on screen 2 (tile x = 2*16+3) -> 3 screens.
        let l = l1(vec![obj(35, 20)], vec![]);
        assert_eq!(screens_used(&l, None, &sprites(vec![]), false), 3);
        // Same tile Y on a horizontal level does not add screens.
        let l = l1(vec![obj(3, 400)], vec![]);
        assert_eq!(screens_used(&l, None, &sprites(vec![]), false), 1);
    }

    #[test]
    fn vertical_uses_y_axis() {
        // Object at tile y = 3*16+1 -> 4 screens; x is ignored.
        let l = l1(vec![obj(500, 49)], vec![]);
        assert_eq!(screens_used(&l, None, &sprites(vec![]), true), 4);
    }

    #[test]
    fn sprites_and_exits_count() {
        let l = l1(vec![], vec![EditableExit { screen: 5, midway: false, secondary: false, id: 0 }]);
        let s = sprites(vec![spr(7 * 16 + 2, 10)]);
        assert_eq!(screens_used(&l, None, &s, false), 8);
    }

    #[test]
    fn layer2_objects_count() {
        let l1l = l1(vec![obj(5, 5)], vec![]);
        let l2l = l1(vec![obj(9 * 16, 3)], vec![]);
        assert_eq!(screens_used(&l1l, Some(&l2l), &sprites(vec![]), false), 10);
    }

    #[test]
    fn clamps_to_header_field_range() {
        // Far beyond the 5-bit field: clamps to 32, never 0.
        let l = l1(vec![obj(100 * 16, 0)], vec![]);
        assert_eq!(screens_used(&l, None, &sprites(vec![]), false), 32);
    }

    #[test]
    fn anchor_tile_decides_not_extent() {
        // Tile x=15 is the last tile of screen 0, even though a wide object
        // anchored there would spill into screen 1 (documented approximation).
        let l = l1(vec![obj(15, 10)], vec![]);
        assert_eq!(screens_used(&l, None, &sprites(vec![]), false), 1);
        let l = l1(vec![obj(16, 10)], vec![]);
        assert_eq!(screens_used(&l, None, &sprites(vec![]), false), 2);
    }

    #[test]
    fn gate_never_resizes_untouched_vanilla_rom() {
        // $FF = table never installed; the default-on checkbox alone must
        // not resize a vanilla ROM.
        assert!(!auto_set_applies(0xFF, false, true));
        assert!(!scroll_ext_installs(0xFF, false, false));
    }

    #[test]
    fn gate_touching_controls_installs_and_applies() {
        // Touching any scroll-extension control installs the table, so the
        // Auto-Set Screens checkbox takes effect on vanilla ROMs.
        assert!(auto_set_applies(0xFF, true, true));
        assert!(scroll_ext_installs(0xFF, false, true));
        // Separate H/V mode installs even without other touches.
        assert!(scroll_ext_installs(0xFF, true, false));
    }

    #[test]
    fn gate_installed_table_applies_without_touch() {
        // LM-installed table ($20 = initial value, auto-set on): applies.
        assert!(auto_set_applies(0x20, false, true));
        assert!(scroll_ext_installs(0x20, false, false));
        // ...but only when the per-level bit is actually set: an installed
        // table with C clear decodes to auto_set=false, so nothing applies.
        assert!(!auto_set_applies(0x00, false, false));
        assert!(auto_set_applies(0x00, false, true)); // 0x00 != $FF: installed
    }

    /// Scan the real ROM and report levels where the occupied screen count
    /// differs from the declared header length — the cases where Auto-Set
    /// Number of Screens visibly changes something. Informational only.
    /// Run with `ROM_PATH=/path/to/smw.smc cargo test --lib -- --ignored
    /// auto_screens_scan`.
    #[test]
    #[ignore]
    fn auto_screens_scan() {
        use smwe_rom::{level::Layer2Data, SmwRom};

        use super::super::{object_layer::EditableObjectLayer, sprite_layer::EditableSpriteLayer};

        let rom_path = std::env::var("ROM_PATH").expect("set ROM_PATH");
        let rom = SmwRom::from_file(rom_path).expect("parse ROM");
        let mut diffs = 0;
        for (idx, level) in rom.levels.iter().enumerate() {
            let vertical = level.secondary_header.vertical_level();
            let l1l = EditableObjectLayer::from_level(level);
            let l2l = match &level.layer2 {
                Layer2Data::Objects { objects, .. } => Some(EditableObjectLayer::from_object_layer(objects, vertical)),
                Layer2Data::Background(_) => None,
            };
            let spr = EditableSpriteLayer::from_level(level);
            let used = screens_used(&l1l, l2l.as_ref(), &spr, vertical);
            let declared = level.primary_header.level_length() as u32 + 1;
            if used != declared {
                diffs += 1;
                if diffs <= 15 {
                    println!(
                        "level {idx:03X}: declared {declared} screens, occupies {used} ({} objects, {} sprites{})",
                        l1l.objects.len(),
                        spr.sprites.len(),
                        if vertical { ", vertical" } else { "" },
                    );
                }
            }
        }
        println!("{diffs} levels differ out of {}", rom.levels.len());
    }
}
