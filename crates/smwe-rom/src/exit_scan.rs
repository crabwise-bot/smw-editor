//! Lunar Magic parity: "Scan for Undefined Exits".
//!
//! LM v1.50 (September 24, 2002) added a menu item "to scan for
//! exit-enabled objects that lead to the bonus game levels, and included an
//! option to do the scan automatically whenever you save a level to the
//! ROM. This may help people who accidentally use exit-enabled objects
//! without setting them up to go anywhere." LM v3.30 (May 1, 2021) added the
//! "Scan for Undefined Exits" toolbar button for the same scan; LM v3.31
//! fixed it to also check horizontal exit pipe tiles (and to only count one
//! of the Boss Door tiles, since the others don't act as a door).
//!
//! "Undefined" here has LM's concrete meaning: a screen exit whose resolved
//! level destination is one of the two TEST levels (`0x000` / `0x100` — the
//! "bonus game levels"). A freshly placed 4-byte screen-exit object is all
//! zeros, so its destination reads as level `0x000`; the scan catches exits
//! the author never configured. Exits routed through a secondary exit that
//! the editor knows leaves to the overworld (LM v3.00 option) have no level
//! destination and are excluded — matching LM's v3.0x fix ("didn't exclude
//! exits to the overworld when checking the level destination").

use thiserror::Error;

use crate::{
    level::{
        object_layer::ExitObject,
        secondary_entrance::{SecondaryEntrance, SecondaryExitExtData},
    },
    snes_utils::rom::Rom,
    RomError,
};

/// The two "bonus game"/TEST levels. An exit object that was never given a
/// destination points at one of these (a fresh 4-byte exit record is all
/// zeros, i.e. direct destination `0x000`).
pub const UNDEFINED_EXIT_DESTINATIONS: [u16; 2] = [0x000, 0x100];

#[derive(Debug, Error)]
pub enum ExitScanError {
    #[error("reading secondary exit {0:#05X}: {1}")]
    SecondaryExitRead(u16, RomError),
}

/// Where a screen-exit object resolves to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ResolvedExitDestination {
    /// The exit leaves to the overworld (LM v3.00 secondary-exit option):
    /// it has no level destination, so the undefined-exit scan skips it.
    Overworld,
    /// The level the exit leads to (`0x000`–`0x1FF`).
    Level(u16),
}

/// Resolve a 4-byte screen-exit object to its destination.
///
/// * `secondary_exit() == false` → the destination field is a direct level
///   number.
/// * `secondary_exit() == true` → the destination field is a secondary-exit
///   table index; the entry's own destination level is used. Entries the
///   editor knows send the player to the overworld resolve to
///   [`ResolvedExitDestination::Overworld`].
///
/// The destination field is 9 bits, so secondary-exit indices are always
/// `0x000`–`0x1FF` (inside the vanilla table); LM v2.50's expanded
/// `0x200`–`0x2000` entries cannot be referenced by this encoding.
pub fn resolve_exit_destination(
    rom: &Rom, ext: &SecondaryExitExtData, exit: &ExitObject,
) -> Result<ResolvedExitDestination, ExitScanError> {
    if !exit.secondary_exit() {
        return Ok(ResolvedExitDestination::Level(exit.destination_level()));
    }
    let index = exit.destination_level();
    if ext.options_for(index).exit_to_overworld.is_some() {
        return Ok(ResolvedExitDestination::Overworld);
    }
    let entry = SecondaryEntrance::read_from_rom(rom, index as usize)
        .map_err(|e| ExitScanError::SecondaryExitRead(index, e))?;
    Ok(ResolvedExitDestination::Level(entry.destination_level()))
}

/// LM's definition of an "undefined" exit destination: one of the TEST
/// levels, where unconfigured exits point by default.
pub fn is_undefined_destination(destination: u16) -> bool {
    UNDEFINED_EXIT_DESTINATIONS.contains(&destination)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::level::object_layer::{ObjectInstance, ObjectLayer};

    fn exit_object(bytes: [u8; 4]) -> ExitObject {
        // Extended object 0x00 decodes to a 4-byte ExitObject.
        let layer_bytes = [bytes[0], bytes[1], 0x00, bytes[3], 0xFF];
        let (layer, _) = ObjectLayer::parse(&layer_bytes).unwrap().1;
        match &layer.objects()[0] {
            ObjectInstance::Extended(crate::level::object_layer::ExtendedInstance::Exit(e)) => e.clone(),
            other => panic!("expected ExitObject, got {other:?}"),
        }
    }

    #[test]
    fn fresh_exit_object_is_undefined() {
        // A freshly placed screen exit is all zeros: direct destination
        // level 0x000, which the scan flags.
        let exit = exit_object([0x00, 0x00, 0x00, 0x00]);
        assert!(!exit.secondary_exit());
        assert_eq!(exit.destination_level(), 0x000);
        assert!(is_undefined_destination(exit.destination_level()));
    }

    #[test]
    fn direct_destination_decodes() {
        // Screen 3, direct destination 0x105: bytes per the ExitObject bit
        // layout (screen in byte 0 low 5 bits, dest hi bit in byte 1 bit 0,
        // dest lo byte in byte 3).
        let exit = exit_object([0x03, 0x01, 0x00, 0x05]);
        assert!(!exit.secondary_exit());
        assert_eq!(exit.screen_number(), 3);
        assert_eq!(exit.destination_level(), 0x105);
        assert!(!is_undefined_destination(0x105));
    }

    #[test]
    fn secondary_exit_flag_decodes() {
        // Same but with the secondary-exit flag (byte 1 bit 1) set:
        // destination field becomes secondary-exit index 0x105.
        let exit = exit_object([0x03, 0x03, 0x00, 0x05]);
        assert!(exit.secondary_exit());
        assert_eq!(exit.destination_level(), 0x105);
    }

    #[test]
    fn undefined_destinations_are_exactly_the_test_levels() {
        assert!(is_undefined_destination(0x000));
        assert!(is_undefined_destination(0x100));
        assert!(!is_undefined_destination(0x001));
        assert!(!is_undefined_destination(0x0FF));
        assert!(!is_undefined_destination(0x101));
        assert!(!is_undefined_destination(0x1FF));
    }

    /// Real-ROM tests: need `ROM_PATH` pointing at a headerless SMW ROM.
    #[test]
    #[ignore]
    fn vanilla_secondary_exit_zero_resolves() {
        let path = std::env::var("ROM_PATH").expect("ROM_PATH not set");
        let rom = Rom::new(std::fs::read(&path).unwrap()).unwrap();
        let ext = SecondaryExitExtData::parse(&std::fs::read(&path).unwrap()).unwrap_or_default();
        // Screen 0, via secondary exit 0x000 (flag set, index 0).
        let exit = exit_object([0x00, 0x02, 0x00, 0x00]);
        let resolved = resolve_exit_destination(&rom, &ext, &exit).unwrap();
        // Whatever vanilla secondary exit 0 points at, the resolution must
        // succeed and agree with a direct read of the table.
        let entry = SecondaryEntrance::read_from_rom(&rom, 0).unwrap();
        assert_eq!(resolved, ResolvedExitDestination::Level(entry.destination_level()));
    }

    #[test]
    #[ignore]
    fn vanilla_level_105_exits_are_defined() {
        // Level 0x105 (Yoshi's Island 1) has fully configured exits; none
        // may resolve to a TEST level through the real ROM data.
        let path = std::env::var("ROM_PATH").expect("ROM_PATH not set");
        let raw = std::fs::read(&path).unwrap();
        let rom = Rom::new(raw.clone()).unwrap();
        let ext = SecondaryExitExtData::parse(&raw).unwrap_or_default();
        let level = crate::level::Level::parse(&rom, 0x105).unwrap();
        let mut exits = 0;
        for obj in level.layer1.objects() {
            if let ObjectInstance::Extended(crate::level::object_layer::ExtendedInstance::Exit(e)) = obj {
                exits += 1;
                let resolved = resolve_exit_destination(&rom, &ext, e).unwrap();
                match resolved {
                    ResolvedExitDestination::Overworld => {}
                    ResolvedExitDestination::Level(d) => {
                        assert!(!is_undefined_destination(d), "level 105 exit leads to {d:#05X}")
                    }
                }
            }
        }
        assert!(exits > 0, "level 0x105 should have screen exits");
    }
}
