//! Lunar Magic parity: "Scan for Undefined Exits" (LM v1.50 menu item,
//! v3.30 toolbar button).
//!
//! The scan walks every level (`0x000`–`0x1FF`), finds screens that contain
//! exit-enabled tiles (doors, exit pipes, etc. — the same predicate as the
//! "Mark exit-enabled tiles" view, LM v3.31), and checks each screen's
//! 4-byte screen-exit object. A screen is reported when:
//!
//! * it has exit-enabled tiles but **no** screen-exit object (the exit was
//!   never set up — exactly what LM's v1.50 changelog describes: "people
//!   who accidentally use exit-enabled objects without setting them up to
//!   go anywhere"), or
//! * its screen exit resolves — directly or through a secondary exit — to
//!   one of the TEST levels (`0x000`/`0x100`, the "bonus game levels"), which
//!   is where an unconfigured exit points by default.
//!
//! Exits routed through a secondary exit that leaves to the overworld (LM
//! v3.00 option) have no level destination and are excluded, matching LM's
//! v3.0x fix. Reports are per screen (like LM's dialog), not per tile, so
//! multi-tile doors collapse to one row.

use std::collections::{HashMap, HashSet};

use anyhow::{Context, Result};
use smwe_rom::{
    block_behavior::is_exit_enabled,
    exit_scan::{is_undefined_destination, resolve_exit_destination, ResolvedExitDestination},
    level::{
        object_layer::{ExitObject, ExtendedInstance, ObjectInstance},
        secondary_entrance::SecondaryExitExtData,
        Level,
        LEVEL_COUNT,
    },
    map16_expanded::{act_as_of, read_acts_table},
    snes_utils::rom::Rom,
};

use crate::level_png_export::{block_at, level_geom_of, load_level_cpu, screen_at, LevelGeom, BLOCK_MAP_BASE};

/// One row of the scan report.
#[derive(Debug, Clone)]
pub struct UndefinedExit {
    /// Translevel (`0x000`–`0x1FF`).
    pub level:  u16,
    /// Screen within the level holding the exit-enabled tiles.
    pub screen: u8,
    /// Why the screen was flagged.
    pub kind:   UndefinedExitKind,
}

/// Why a screen was flagged by the scan.
#[derive(Debug, Clone)]
pub enum UndefinedExitKind {
    /// The screen has exit-enabled tiles but no 4-byte screen-exit object:
    /// entering them goes nowhere.
    NoExitRecord,
    /// The screen's exit leads to a TEST level (`0x000`/`0x100`).
    UndefinedDestination {
        /// `Some(i)` when the destination came through secondary exit `i`;
        /// `None` for a direct level destination.
        via_secondary: Option<u16>,
        destination:   u16,
    },
}

/// Full result of scanning the ROM's levels.
#[derive(Debug)]
pub struct ExitScanReport {
    pub findings:       Vec<UndefinedExit>,
    /// Levels successfully scanned (`0x000`–`0x1FF`).
    pub levels_scanned: u32,
    /// Levels skipped because their data failed to parse (corrupt or
    /// third-party layout the parser doesn't understand).
    pub levels_skipped: u32,
}

/// Screens in `level` containing at least one exit-enabled tile.
///
/// Mirrors the "Mark exit-enabled tiles" overlay exactly (same block maps,
/// same acts-like resolution, same Layer 2 handling in level mode `0x01`).
fn exit_screens(cpu: &mut smwe_emu::Cpu, acts: &HashMap<u16, u16>, g: &LevelGeom) -> HashSet<u8> {
    let l2_active = g.level_mode == 0x01 && g.has_layer2;
    let l2_off = g.scr_len * g.scr_size;
    let (tw, th) = (g.width / 16, g.height / 16);
    let mut screens = HashSet::new();
    for ty in 0..th {
        for tx in 0..tw {
            let id = block_at(cpu, g, tx, ty, BLOCK_MAP_BASE);
            let enabled = id != 0 && is_exit_enabled(act_as_of(acts, id), g.level_mode);
            let enabled = enabled
                || (l2_active && {
                    let id2 = block_at(cpu, g, tx, ty, BLOCK_MAP_BASE + l2_off);
                    id2 != 0 && is_exit_enabled(act_as_of(acts, id2), g.level_mode)
                });
            if enabled {
                screens.insert(screen_at(g, tx, ty));
            }
        }
    }
    screens
}

/// Screen-exit objects (`ExtendedInstance::Exit`) in the level's Layer 1
/// object stream, keyed by screen number.
fn exit_records(level: &Level) -> HashMap<u8, ExitObject> {
    let mut map = HashMap::new();
    for obj in level.layer1.objects() {
        if let ObjectInstance::Extended(ExtendedInstance::Exit(e)) = obj {
            // Later records win, matching how the game processes the
            // object stream in order.
            map.insert(e.screen_number(), e.clone());
        }
    }
    map
}

/// Scan every level for undefined exits.
///
/// `rom_bytes` is the raw ROM image (SMC header included if present);
/// callers merge unsaved tab edits first, like the PNG export does, so the
/// scan sees the current editor state.
///
/// `progress` is called after each level with the number of levels scanned
/// so far; returning `false` aborts the scan early (reported as an error).
pub fn scan_undefined_exits_with_progress(
    rom_bytes: &[u8], progress: &mut dyn FnMut(u32) -> bool,
) -> Result<ExitScanReport> {
    let stripped: &[u8] = if rom_bytes.len() % 0x400 == 0x200 { &rom_bytes[0x200..] } else { rom_bytes };
    let rom = Rom::new(stripped.to_vec()).context("parsing ROM image")?;
    let ext = SecondaryExitExtData::parse(rom_bytes).unwrap_or_default();
    let acts = read_acts_table(stripped, 0).unwrap_or_default();

    let mut findings = Vec::new();
    let mut levels_scanned = 0u32;
    let mut levels_skipped = 0u32;

    for level in 0..LEVEL_COUNT as u16 {
        let parsed = match Level::parse(&rom, level as u32) {
            Ok(l) => l,
            Err(_) => {
                levels_skipped += 1;
                continue;
            }
        };
        let records = exit_records(&parsed);

        let mut cpu = match load_level_cpu(stripped, level) {
            Ok(c) => c,
            Err(_) => {
                levels_skipped += 1;
                continue;
            }
        };
        let g = level_geom_of(&mut cpu);

        for screen in exit_screens(&mut cpu, &acts, &g) {
            match records.get(&screen) {
                None => findings.push(UndefinedExit { level, screen, kind: UndefinedExitKind::NoExitRecord }),
                Some(exit) => {
                    let resolved = match resolve_exit_destination(&rom, &ext, exit) {
                        Ok(r) => r,
                        Err(_) => {
                            levels_skipped += 1;
                            continue;
                        }
                    };
                    match resolved {
                        ResolvedExitDestination::Overworld => {}
                        ResolvedExitDestination::Level(dest) if is_undefined_destination(dest) => {
                            findings.push(UndefinedExit {
                                level,
                                screen,
                                kind: UndefinedExitKind::UndefinedDestination {
                                    via_secondary: exit.secondary_exit().then_some(exit.destination_level()),
                                    destination:   dest,
                                },
                            });
                        }
                        ResolvedExitDestination::Level(_) => {}
                    }
                }
            }
        }
        levels_scanned += 1;
        if !progress(u32::from(level) + 1) {
            anyhow::bail!("scan cancelled");
        }
    }

    findings.sort_by_key(|f| (f.level, f.screen));
    Ok(ExitScanReport { findings, levels_scanned, levels_skipped })
}

/// Convenience wrapper for [`scan_undefined_exits_with_progress`] with no
/// progress reporting and no cancellation.
pub fn scan_undefined_exits(rom_bytes: &[u8]) -> Result<ExitScanReport> {
    scan_undefined_exits_with_progress(rom_bytes, &mut |_| true)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Real-ROM test: the scan must run clean over the whole vanilla ROM
    /// and report only genuine findings (the vanilla game ships with no
    /// undefined exits, so the report should be small and human-reviewable).
    #[test]
    #[ignore]
    fn real_rom_scan_runs_clean() {
        let rom_path = std::env::var("ROM_PATH").expect("ROM_PATH must point at a real SMW ROM");
        let rom_bytes = std::fs::read(&rom_path).expect("cannot read ROM");
        let report = scan_undefined_exits(&rom_bytes).expect("scan failed");
        assert_eq!(report.levels_scanned, LEVEL_COUNT as u32, "every level should scan");
        assert_eq!(report.levels_skipped, 0, "no level should fail to parse/decompress");
        for f in &report.findings {
            eprintln!("finding: level {:03X} screen {}: {:?}", f.level, f.screen, f.kind);
        }
    }
}
