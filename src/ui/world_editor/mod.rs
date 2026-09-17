//! World Map (Overworld) Editor UI.
//!
//! The overworld tilemap is stored in WRAM at $7EC800 (Map16TilesLow) after the
//! game's init routines run. `CODE_04DC09` copies `OWL1TileData` with `MVN`,
//! so the 0x800-byte buffer stays in its packed ROM layout: 64 columns × 32 rows
//! of u8 Map16 tile IDs in row-major order. The game selects each submap by
//! changing the camera position, not by swapping to a separate L1 buffer.
//!
//! Layer 2 ($7F4000 / OWLayer2Tilemap): a 64×64 8×8-tile map stored as four
//! 32×32 screens (2 across × 2 down). Each entry is [tile_num_u8, YXPCCCTT_u8].

mod editing;
mod ow_tile_picker;

use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{Arc, Mutex},
};

use egui::{
    vec2,
    CentralPanel,
    Color32,
    CornerRadius,
    Frame,
    Key,
    PaintCallback,
    Pos2,
    Rect,
    Sense,
    SidePanel,
    Stroke,
    StrokeKind,
    Ui,
    Vec2,
    WidgetText,
};
use egui_glow::CallbackFn;
use smwe_emu::{emu::CheckedMem, rom::Rom as EmuRom, Cpu};
use smwe_render::{
    gfx_buffers::GfxBuffers,
    tile_renderer::{Tile, TileRenderer, TileUniforms},
};
use smwe_rom::{
    compression::lc_rle2,
    overworld::{L2EventEntry, L2EventKind, OWL1_TILE_DATA_SIZE, OWL1_TILE_DATA_SNES, OW_EVENT_COUNT, SUBMAP_NAMES},
    snes_utils::addr::{AddrPc, AddrSnes},
    SmwRom,
};

use crate::{
    rom_freespace::find_free_space,
    ui::{editing_mode::EditingMode, style::toggle_button, tool::DockableEditorTool},
    undo::{Undo, UndoableData},
};

// ── Layout constants ──────────────────────────────────────────────────────────

/// Game pixels per Map16 block (L1 tiles are 16×16 game pixels each).
const MAP16_PX: f32 = 16.0;

/// Visible viewport size used by the editor: 32×32 Map16 blocks = 512×512 pixels.
const SUBMAP_VIEW_X: i32 = 16;
const SUBMAP_VIEW_Y: i32 = 40;
const SUBMAP_VIEW_W: u32 = 224;
const SUBMAP_VIEW_H: u32 = 168;

/// Full BG tilemap size after the game composes the active overworld into VRAM.
const VRAM_TILE_ROWS: u32 = 64;
const VRAM_L1_TILEMAP_BASE: usize = 0x2000 * 2;
const VRAM_L2_TILEMAP_BASE: usize = 0x3000 * 2;

// ── SNES overworld tile-index helpers ─────────────────────────────────────────

const OW_L2_COLS: u32 = 64;

fn tilemap_vram_addr(base: usize, col: u32, row: u32) -> usize {
    let quadrant = ((row / 32) * 2) + (col / 32);
    let sub_row = row % 32;
    let sub_col = col % 32;
    let quadrant_offset = quadrant * 32 * 32 * 2;
    let idx = quadrant_offset + ((sub_row * 32 + sub_col) * 2);
    base + idx as usize
}

fn visible_map_size(submap: u8) -> (u32, u32) {
    if submap == 0 {
        (512, 512)
    } else {
        (SUBMAP_VIEW_W, SUBMAP_VIEW_H)
    }
}

fn visible_map_crop(submap: u8) -> (u32, u32) {
    if submap == 0 {
        (0, 0)
    } else {
        (SUBMAP_VIEW_X as u32, SUBMAP_VIEW_Y as u32)
    }
}

fn l1_vram_addr_for_map16(submap: u8, map16_x: u32, map16_y: u32) -> usize {
    let (crop_x, crop_y) = visible_map_crop(submap);
    let tile_x = (map16_x * 16 + crop_x) / 8;
    let tile_y = (map16_y * 16 + crop_y) / 8;
    tilemap_vram_addr(VRAM_L1_TILEMAP_BASE, tile_x, tile_y)
}

// ── OpenGL renderer ───────────────────────────────────────────────────────────

#[derive(Debug)]
struct OverworldRenderer {
    layer1:    TileRenderer,
    layer2:    TileRenderer,
    gfx_bufs:  GfxBuffers,
    destroyed: bool,
}

impl OverworldRenderer {
    fn new(gl: &glow::Context) -> Self {
        Self {
            layer1:    TileRenderer::new(gl),
            layer2:    TileRenderer::new(gl),
            gfx_bufs:  GfxBuffers::new(gl),
            destroyed: false,
        }
    }

    fn destroy(&mut self, gl: &glow::Context) {
        if self.destroyed {
            return;
        }
        self.gfx_bufs.destroy(gl);
        self.layer1.destroy(gl);
        self.layer2.destroy(gl);
        self.destroyed = true;
    }

    fn upload_gfx(&self, gl: &glow::Context, data: &[u8]) {
        if !self.destroyed {
            self.gfx_bufs.upload_vram(gl, data);
        }
    }

    fn upload_palette(&self, gl: &glow::Context, data: &[u8]) {
        if !self.destroyed {
            self.gfx_bufs.upload_palette(gl, data);
        }
    }

    fn set_tiles(&mut self, gl: &glow::Context, l1: Vec<Tile>, l2: Vec<Tile>) {
        if !self.destroyed {
            self.layer1.set_tiles(gl, l1);
            self.layer2.set_tiles(gl, l2);
        }
    }

    fn paint(&self, gl: &glow::Context, screen_size: Vec2, zoom: f32, offset: Vec2, draw_l1: bool, draw_l2: bool) {
        if self.destroyed {
            return;
        }
        let uniforms = TileUniforms { gfx_bufs: self.gfx_bufs, screen_size, offset, zoom };
        if draw_l2 {
            self.layer2.paint(gl, &uniforms);
        }
        if draw_l1 {
            self.layer1.paint(gl, &uniforms);
        }
    }
}

// ── Undoable overworld edit state ─────────────────────────────────────────────

/// The serialization layout is: [L1 tiles (OWL1_TILE_DATA_SIZE bytes)][L2 words as LE u16 pairs].
/// L1 is always exactly OWL1_TILE_DATA_SIZE bytes so `from_bytes` can split correctly.
#[derive(Clone)]
pub(super) struct OverworldEditState {
    pub layer1_tiles: Vec<u8>,
    pub layer2_words: Vec<u16>,
}

impl Undo for OverworldEditState {
    fn from_bytes(bytes: Vec<u8>) -> Self {
        let l1_end = OWL1_TILE_DATA_SIZE.min(bytes.len());
        let l1 = bytes[..l1_end].to_vec();
        let l2_bytes = &bytes[l1_end..];
        let layer2_words = l2_bytes.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
        Self { layer1_tiles: l1, layer2_words }
    }

    fn to_bytes(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(self.layer1_tiles.len() + self.layer2_words.len() * 2);
        out.extend_from_slice(&self.layer1_tiles);
        for &w in &self.layer2_words {
            out.extend_from_slice(&w.to_le_bytes());
        }
        out
    }

    fn size_bytes(&self) -> usize {
        self.layer1_tiles.len() + self.layer2_words.len() * 2
    }
}

// ── Editor ────────────────────────────────────────────────────────────────────

pub struct UiWorldEditor {
    gl:       Arc<glow::Context>,
    #[allow(dead_code)]
    rom:      Arc<SmwRom>,
    cpu:      Cpu,
    renderer: Arc<Mutex<OverworldRenderer>>,

    submap: u8,

    offset:         Vec2,
    zoom:           f32,
    show_grid:      bool,
    show_layer1:    bool,
    show_layer2:    bool,
    selected_tile:  Option<(u32, u32)>,
    /// Clipboard region selection (x0, y0, x1, y1 inclusive, map16-tile
    /// coords) for layer-1 copy/paste — LM v2.30 overworld clipboard flow.
    /// Set by Shift+drag in Select mode on layer 1.
    ow_sel_rect:    Option<(u32, u32, u32, u32)>,
    /// Shift+drag anchor while a region selection is being drawn.
    ow_drag_anchor: Option<(u32, u32)>,
    /// Copy origin for pastes when the pointer isn't over the canvas.
    ow_copy_origin: Option<(u32, u32)>,
    needs_center:   bool,

    // Editing state
    editing_mode:          EditingMode,
    draw_tile_num:         u8,
    draw_palette:          u8,
    draw_tile_attr:        u8,
    tile_picker:           ow_tile_picker::OwTilePicker,
    l1_tile_picker:        ow_tile_picker::OwL1TilePicker,
    edit_layer:            u8, // 1 or 2
    preview_texture:       Option<egui::TextureHandle>,
    preview_for:           Option<(u32, u32)>,
    has_edits:             bool,
    has_unsavable_changes: bool,
    pub(super) edit_state: UndoableData<OverworldEditState>,

    /// Per-event (0..smwe_rom::overworld::OW_EVENT_COUNT) preview toggle: whether
    /// this "destruction" event (castle/fortress/switch palace beaten, etc.) is
    /// considered active for preview purposes. Defaults to all-on, matching the
    /// previous blanket "activate everything" behavior.
    active_events:         Vec<bool>,
    /// Whether the Layer 2 event target markers are drawn over the map.
    show_l2_event_markers: bool,

    /// Per-tile (index into `layer1_tiles`) level-number overrides. Absent
    /// entries use the vanilla scan-order-derived level number unchanged.
    /// Applying these requires patching a single ROM instruction operand to
    /// read a custom table instead of the WRAM-computed one — see
    /// `smwe_rom::overworld::LEVEL_NUMBER_PATCH_OPERAND_SNES` for why this
    /// doesn't need new ASM code, just different data.
    custom_level_numbers:  HashMap<usize, u8>,
    level_numbers_dirty:   bool,
    /// Vanilla level names decoded from the ROM (93 entries, index =
    /// translevel). Used as the base for custom name edits.
    vanilla_level_names:   Vec<String>,
    /// Custom level names by translevel. Absent entries use the vanilla name.
    custom_level_names:    HashMap<u8, String>,
    /// True if any level name has been customized (requires the name-table
    /// relocation patch on save).
    level_names_dirty:     bool,
    /// Level-name text field buffer, synced to `level_name_for`.
    level_name_edit:       String,
    /// Translevel the name field (and error) currently belong to.
    level_name_for:        Option<u8>,
    /// Validation error from the last rejected name edit, if any.
    level_name_error:      Option<String>,
    /// Event-ownership table (`$05D608` events-by-translevel): raw byte per
    /// translevel (`0x00`–`0x5C`), `$FF` = no event. Edited via the
    /// event-ownership panel; written back in place on save.
    event_ownership:       Vec<u8>,
    /// True if any event-ownership assignment has been changed.
    event_ownership_dirty: bool,
    /// Last time the overworld animated tiles were ticked.
    last_anim_tick:        std::time::Instant,
}

impl UiWorldEditor {
    pub fn new(gl: Arc<glow::Context>, rom: Arc<SmwRom>, rom_path: PathBuf) -> Self {
        let renderer = Arc::new(Mutex::new(OverworldRenderer::new(&gl)));

        let raw = std::fs::read(&rom_path).expect("cannot read ROM for emulator");
        let rom_bytes = if raw.len() % 0x400 == 0x200 { raw[0x200..].to_vec() } else { raw };
        let mut emu_rom = EmuRom::new(rom_bytes);
        emu_rom.load_symbols(include_str!("../../../symbols/SMW_U.sym"));
        let cpu = Cpu::new(CheckedMem::new(Arc::new(emu_rom)));

        let source_layer1_tiles = rom.overworld.layer1_tiles.clone();
        let edit_state =
            UndoableData::new(OverworldEditState { layer1_tiles: source_layer1_tiles, layer2_words: Vec::new() });
        // Decode vanilla level names before `rom` is moved into the struct.
        let vanilla_level_names =
            smwe_rom::overworld::level_names::decode_all(rom.rom_bytes(), 0, false).unwrap_or_default();
        // Decode the vanilla event-ownership table ($05D608) the same way.
        let event_ownership = smwe_rom::overworld::event_ownership::EventOwnership::parse(rom.rom_bytes(), 0)
            .map(|eo| eo.table)
            .unwrap_or_else(|_| {
                vec![
                    smwe_rom::overworld::event_ownership::NO_EVENT;
                    smwe_rom::overworld::event_ownership::EVENT_OWNERSHIP_COUNT
                ]
            });
        let mut editor = Self {
            gl,
            rom,
            cpu,
            renderer,
            submap: 0,
            offset: Vec2::ZERO,
            zoom: 2.0,
            show_grid: false,
            show_layer1: true,
            show_layer2: true,
            selected_tile: None,
            ow_sel_rect: None,
            ow_drag_anchor: None,
            ow_copy_origin: None,
            needs_center: false,
            editing_mode: EditingMode::Select,
            draw_tile_num: 0x00,
            draw_palette: 0,
            draw_tile_attr: 0x00,
            tile_picker: ow_tile_picker::OwTilePicker::new(),
            l1_tile_picker: ow_tile_picker::OwL1TilePicker::new(),
            edit_layer: 1,
            preview_texture: None,
            preview_for: None,
            has_edits: false,
            has_unsavable_changes: false,
            edit_state,
            active_events: vec![true; smwe_rom::overworld::OW_EVENT_COUNT],
            show_l2_event_markers: true,
            custom_level_numbers: HashMap::new(),
            level_numbers_dirty: false,
            vanilla_level_names,
            custom_level_names: HashMap::new(),
            level_names_dirty: false,
            level_name_edit: String::new(),
            level_name_for: None,
            level_name_error: None,
            event_ownership,
            event_ownership_dirty: false,
            last_anim_tick: std::time::Instant::now(),
        };
        editor.load_submap();
        editor
    }

    fn load_submap(&mut self) {
        apply_active_events_to_wram(&mut self.cpu, &self.active_events);
        smwe_emu::emu::load_overworld(&mut self.cpu, self.submap);

        let mut r = self.renderer.lock().expect("Cannot lock overworld renderer");
        r.upload_palette(&self.gl, &self.cpu.mem.cgram);
        r.upload_gfx(&self.gl, &self.cpu.mem.vram);

        let l2_scroll_x = i16::from_le_bytes(self.cpu.mem.load_u16(0x001E).to_le_bytes()) as i32;
        let l2_scroll_y = i16::from_le_bytes(self.cpu.mem.load_u16(0x0020).to_le_bytes()) as i32;

        let l1 = build_bg_tiles(&self.cpu.mem.vram, VRAM_L1_TILEMAP_BASE, self.submap, l2_scroll_x, l2_scroll_y);
        let l2 = build_bg_tiles(&self.cpu.mem.vram, VRAM_L2_TILEMAP_BASE, self.submap, l2_scroll_x, l2_scroll_y);

        log::info!("Loaded submap {}: L1={} tiles, L2={} tiles", self.submap, l1.len(), l2.len());

        r.set_tiles(&self.gl, l1, l2);

        self.tile_picker.rebuild(&self.cpu.mem.vram, &self.cpu.mem.cgram, VRAM_L1_TILEMAP_BASE, VRAM_L2_TILEMAP_BASE);
        self.l1_tile_picker.rebuild(&mut self.cpu);

        self.offset = Vec2::ZERO;
        self.selected_tile = None;
        self.needs_center = true;
        self.has_edits = false;
        self.has_unsavable_changes = false;
        let layer2_words = read_overworld_l2_words(&self.cpu);
        self.edit_state.write(|s| {
            s.layer2_words = layer2_words;
        });
        self.edit_state.clear_stack();
    }
}

impl DockableEditorTool for UiWorldEditor {
    fn title(&self) -> WidgetText {
        "World Map Editor".into()
    }

    fn update(&mut self, ui: &mut Ui) {
        SidePanel::left("world_editor.left_panel").resizable(false).show_inside(ui, |ui| self.left_panel(ui));
        CentralPanel::default().frame(Frame::NONE.inner_margin(0.)).show_inside(ui, |ui| self.central_panel(ui));
    }

    fn on_closed(&mut self) {
        self.renderer.lock().expect("Cannot lock overworld renderer").destroy(&self.gl);
    }

    fn has_unsaved_changes(&self) -> bool {
        self.has_edits
    }

    fn on_save_succeeded(&mut self) {
        self.has_edits = false;
        self.event_ownership_dirty = false;
    }

    fn save_to_rom(&self, rom_bytes: &mut [u8], has_smc_header: bool) -> anyhow::Result<()> {
        if self.has_unsavable_changes {
            anyhow::bail!("Overworld edits currently only modify composed VRAM and cannot be serialized to ROM yet");
        }
        let header_offset = usize::from(has_smc_header) * 0x200;
        let start = AddrPc::try_from_lorom(OWL1_TILE_DATA_SNES)?.as_index() + header_offset;
        let end = start + OWL1_TILE_DATA_SIZE;
        let dst = rom_bytes
            .get_mut(start..end)
            .ok_or_else(|| anyhow::anyhow!("Overworld layer 1 ROM write range out of bounds"))?;
        self.edit_state.read(|s| dst.copy_from_slice(&s.layer1_tiles));

        let (tile_compressed, attr_compressed, l2_len) = self.edit_state.read(|s| {
            let tile_stream: Vec<u8> = s.layer2_words.iter().map(|w| (*w & 0x00FF) as u8).collect();
            let attr_stream: Vec<u8> = s.layer2_words.iter().map(|w| (*w >> 8) as u8).collect();
            (lc_rle2::compress_pass(&tile_stream), lc_rle2::compress_pass(&attr_stream), s.layer2_words.len())
        });

        write_overworld_l2_stream(
            rom_bytes,
            has_smc_header,
            AddrPc::try_from_lorom(AddrSnes(0x04A533))?.as_index(),
            l2_len,
            &tile_compressed,
            "OWTileNumbers",
        )?;
        write_overworld_l2_stream(
            rom_bytes,
            has_smc_header,
            AddrPc::try_from_lorom(AddrSnes(0x04C02B))?.as_index(),
            l2_len,
            &attr_compressed,
            "OWTilemap",
        )?;

        // ── Custom per-tile level-number assignment ─────────────────────────
        // Only touches the ROM if the user has actually overridden a level
        // number: leaving this alone keeps overworld behavior byte-for-byte
        // vanilla (translevel/scan-order-derived) for hacks that don't use it.
        //
        // Known limitation: each save that has overrides allocates a fresh
        // freespace table rather than reusing/growing a previously-patched
        // one, so repeated saves with active overrides accumulate small
        // (0x800-byte) orphaned regions in ROM. Harmless but wasteful; a
        // follow-up could detect and reuse an already-owned table in place.
        if !self.custom_level_numbers.is_empty() {
            let tiles = self.edit_state.read(|s| s.layer1_tiles.clone());
            let mut table = vec![0u8; tiles.len()];
            for (idx, slot) in table.iter_mut().enumerate() {
                if let Some(vanilla_level_num) = smwe_rom::overworld::level_number_for_index(&tiles, idx) {
                    let effective = self.custom_level_numbers.get(&idx).copied().unwrap_or(vanilla_level_num);
                    *slot = smwe_rom::overworld::encode_custom_level_number(effective).ok_or_else(|| {
                        anyhow::anyhow!(
                            "Level number {effective:#04X} at tile index {idx} exceeds the maximum \
                             assignable value ({:#04X})",
                            smwe_rom::overworld::MAX_ASSIGNABLE_LEVEL_NUMBER
                        )
                    })?;
                }
            }

            let table_pc = find_free_space(rom_bytes, table.len(), 0x008000, header_offset).ok_or_else(|| {
                anyhow::anyhow!("No free space for the custom level-number table ({} bytes)", table.len())
            })?;
            rom_bytes[table_pc + header_offset..table_pc + header_offset + table.len()].copy_from_slice(&table);

            let table_snes = AddrSnes::try_from_lorom(AddrPc(table_pc as u32))?;
            let patch_pc = AddrPc::try_from_lorom(smwe_rom::overworld::LEVEL_NUMBER_PATCH_OPERAND_SNES)?.as_index()
                + header_offset;
            let bytes = table_snes.0.to_le_bytes();
            rom_bytes[patch_pc..patch_pc + 3].copy_from_slice(&bytes[..3]);
        }

        // ── Custom level names ──────────────────────────────────────────────
        // Encodes all 93 names (vanilla + overrides) into the relocated
        // fragment tables and applies the patch. Only touches the ROM if the
        // user has actually customized a name.
        if !self.custom_level_names.is_empty() {
            use smwe_rom::overworld::level_names as ln;
            // Start from vanilla names decoded at load; apply overrides.
            let mut names = self.vanilla_level_names.clone();
            // Ensure 93 entries (in case decode failed at load).
            names.resize(ln::LEVEL_NAMES_COUNT, String::new());
            for (&translevel, custom) in &self.custom_level_names {
                if (translevel as usize) < names.len() {
                    names[translevel as usize] = custom.clone();
                }
            }
            let encoded = ln::encode_names(&names).map_err(|e| anyhow::anyhow!("Cannot encode level names: {e}"))?;
            let header_offset = usize::from(has_smc_header) * 0x200;
            ln::apply_to_rom(rom_bytes, header_offset, &encoded)
                .map_err(|e| anyhow::anyhow!("Cannot apply level-name patch: {e}"))?;
        }

        // ── Event ownership (which event each level triggers) ────────────────
        // In-place write of the 93-byte `$05D608` table; only touches the ROM
        // if the user actually changed an assignment.
        if self.event_ownership_dirty {
            use smwe_rom::overworld::event_ownership as eo;
            let ownership = eo::EventOwnership { table: self.event_ownership.clone() };
            ownership
                .apply_to_rom(rom_bytes, header_offset)
                .map_err(|e| anyhow::anyhow!("Cannot apply event ownership edits: {e}"))?;
        }

        Ok(())
    }
}

// ── UI ────────────────────────────────────────────────────────────────────────

impl UiWorldEditor {
    fn source_l1_offset(&self) -> usize {
        if self.submap == 0 {
            0
        } else {
            0x400
        }
    }

    fn source_l1_index_for_view(&self, map16_x: u32, map16_y: u32) -> Option<usize> {
        let (crop_x, crop_y) = visible_map_crop(self.submap);
        let src_col = ((map16_x * 16 + crop_x) / 16) as usize;
        let src_row = ((map16_y * 16 + crop_y) / 16) as usize;
        if src_col >= 32 || src_row >= 32 {
            return None;
        }
        Some(self.source_l1_offset() + ow_l1_addr(src_col as u32, src_row as u32))
    }

    pub(super) fn source_l1_tile_at_view(&self, map16_x: u32, map16_y: u32) -> Option<u8> {
        let idx = self.source_l1_index_for_view(map16_x, map16_y)?;
        self.edit_state.read(|s| s.layer1_tiles.get(idx).copied())
    }

    pub(super) fn set_source_l1_tile_at_view(&mut self, map16_x: u32, map16_y: u32, tile_id: u8) {
        let Some(idx) = self.source_l1_index_for_view(map16_x, map16_y) else {
            return;
        };
        self.edit_state.write(|s| {
            if let Some(slot) = s.layer1_tiles.get_mut(idx) {
                *slot = tile_id;
            }
        });
        self.has_edits = true;
        self.apply_source_l1_tile_to_vram(map16_x, map16_y, tile_id);
    }

    fn apply_source_l1_tile_to_vram(&mut self, map16_x: u32, map16_y: u32, tile_id: u8) {
        let (crop_x, crop_y) = visible_map_crop(self.submap);
        let src_col = (map16_x * 16 + crop_x) / 16;
        let src_row = (map16_y * 16 + crop_y) / 16;
        self.write_source_l1_block_words(src_col, src_row, tile_id);
    }

    pub(super) fn write_source_l1_block_words(&mut self, src_col: u32, src_row: u32, tile_id: u8) {
        let sub_tiles = source_l1_subtiles(&mut self.cpu, tile_id);
        let base_tile_x = src_col * 2;
        let base_tile_y = src_row * 2;
        let offsets = [(0u32, 0u32), (1u32, 0u32), (0u32, 1u32), (1u32, 1u32)];
        for (word, (dx, dy)) in sub_tiles.into_iter().zip(offsets) {
            let addr = tilemap_vram_addr(VRAM_L1_TILEMAP_BASE, base_tile_x + dx, base_tile_y + dy);
            if addr + 1 < self.cpu.mem.vram.len() {
                let [lo, hi] = word.to_le_bytes();
                self.cpu.mem.vram[addr] = lo;
                self.cpu.mem.vram[addr + 1] = hi;
            }
        }
    }

    /// Per-event ("destruction event": castle/fortress/switch palace beaten,
    /// etc.) preview toggles. Toggling any checkbox reloads the current submap
    /// with the new `OWEventsActivated` bits, so the real emulated game code
    /// applies (or doesn't apply) that event's reveal-tile swap.
    fn events_panel(&mut self, ui: &mut Ui) {
        ui.collapsing("Events (preview)", |ui| {
            ui.horizontal(|ui| {
                if ui.button("All on").clicked() {
                    self.active_events.iter_mut().for_each(|e| *e = true);
                    self.load_submap();
                }
                if ui.button("All off").clicked() {
                    self.active_events.iter_mut().for_each(|e| *e = false);
                    self.load_submap();
                }
            });
            egui::ScrollArea::vertical().max_height(200.0).show(ui, |ui| {
                let mut changed = false;
                for (i, active) in self.active_events.iter_mut().enumerate() {
                    let offset = self.rom.overworld_events.tile_offsets.get(i).copied().unwrap_or(0);
                    if offset == 0 {
                        continue; // unused event slot
                    }
                    changed |= ui.checkbox(active, format!("Event {i:3} (tile offset {offset:#06X})")).changed();
                }
                if changed {
                    self.load_submap();
                }
            });
        });

        ui.collapsing("Layer 2 events", |ui| {
            ui.checkbox(&mut self.show_l2_event_markers, "Show target markers on map");
            let l2 = &self.rom.overworld_l2_events;
            let events_with_l2: Vec<usize> = (0..OW_EVENT_COUNT)
                .filter(|&e| {
                    !l2.entries_for_event(e).unwrap_or(0..0).is_empty() || !l2.silent_l2_events_for(e as u8).is_empty()
                })
                .collect();
            ui.label(format!(
                "{} table entries · {} events with L2 data · {} silent L2 rows",
                l2.entry_count(),
                events_with_l2.len(),
                l2.silent_events.iter().filter(|s| s.is_l2).count(),
            ));
            ui.label("Markers follow the event checkboxes above. The animated L2 sequence itself runs in-game;");
            ui.label("this panel shows where each event's Layer 2 tiles land.");
            egui::ScrollArea::vertical().max_height(220.0).show(ui, |ui| {
                for event in events_with_l2 {
                    let range = l2.entries_for_event(event).unwrap_or(0..0);
                    let silent = l2.silent_l2_events_for(event as u8);
                    let header = if range.is_empty() {
                        format!("Event {event}: silent row only")
                    } else {
                        format!("Event {event}: entries {}..{}", range.start, range.end)
                    };
                    ui.collapsing(header, |ui| {
                        for idx in range {
                            if let Some(entry) = l2.entries.get(idx) {
                                ui.monospace(format!("[{idx:3}] {}", describe_l2_entry(entry)));
                            }
                        }
                        for s in &silent {
                            ui.monospace(format!("[silent] {}", describe_l2_entry(&s.as_entry())));
                        }
                    });
                }
            });
        });
    }

    /// Event-ownership editor: which event each level (translevel) triggers when
    /// beaten — the `$05D608` events-by-translevel table. The game reads
    /// `DATA_05D608[TranslevelNo]` into `OverworldEvent` on level completion;
    /// Lunar Magic has no UI for choosing these assignments.
    fn event_ownership_panel(&mut self, ui: &mut Ui) {
        use smwe_rom::overworld::event_ownership as eo;
        ui.collapsing("Event ownership (by level)", |ui| {
            ui.label("Which event triggers when each level is beaten ($05D608).");
            ui.add_space(4.0);
            egui::ScrollArea::vertical().max_height(240.0).show(ui, |ui| {
                for tl in 0..eo::EVENT_OWNERSHIP_COUNT {
                    let name = self
                        .custom_level_names
                        .get(&(tl as u8))
                        .cloned()
                        .or_else(|| self.vanilla_level_names.get(tl).cloned())
                        .unwrap_or_default();
                    let cur: Option<u8> = match self.event_ownership.get(tl).copied() {
                        Some(b) if b != eo::NO_EVENT => Some(b),
                        _ => None,
                    };
                    let mut new = cur;
                    ui.horizontal(|ui| {
                        ui.label(format!("0x{tl:02X} {name}"));
                        egui::ComboBox::from_id_salt(("world_editor.event_ownership", tl))
                            .selected_text(event_option_label(cur, &self.rom))
                            .show_ui(ui, |ui| {
                                ui.selectable_value(&mut new, None, event_option_label(None, &self.rom));
                                for e in 0..smwe_rom::overworld::OW_EVENT_COUNT as u8 {
                                    ui.selectable_value(&mut new, Some(e), event_option_label(Some(e), &self.rom));
                                }
                            });
                    });
                    if new != cur {
                        if let Some(slot) = self.event_ownership.get_mut(tl) {
                            *slot = new.unwrap_or(eo::NO_EVENT);
                        }
                        self.event_ownership_dirty = true;
                        self.has_edits = true;
                    }
                }
            });
        });
    }

    fn left_panel(&mut self, ui: &mut Ui) {
        egui::ScrollArea::vertical().show(ui, |ui| {
            ui.heading("Overworld");
            ui.add_space(4.0);

            // Submap selector
            ui.horizontal(|ui| {
                ui.label("Submap");
                let prev = self.submap;
                egui::ComboBox::from_id_salt("world_editor.submap")
                    .selected_text(SUBMAP_NAMES.get(self.submap as usize).copied().unwrap_or("Submap"))
                    .show_ui(ui, |ui| {
                        for (i, name) in SUBMAP_NAMES.iter().enumerate() {
                            ui.selectable_value(&mut self.submap, i as u8, *name);
                        }
                    });
                if self.submap != prev {
                    self.load_submap();
                }
            });

            ui.separator();

            // Zoom
            ui.add(egui::Slider::new(&mut self.zoom, 0.5..=8.0).step_by(0.25).text("Zoom"));
            if ui.button("Reset View").clicked() {
                self.offset = Vec2::ZERO;
                self.zoom = 2.0;
            }

            ui.separator();

            ui.checkbox(&mut self.show_layer1, "Show Layer 1");
            ui.checkbox(&mut self.show_layer2, "Show Layer 2");
            ui.checkbox(&mut self.show_grid, "Show Grid");
            ui.small("Clipboard: Shift+drag on layer 1 selects a region • Ctrl+C copies • Ctrl+V pastes.");

            ui.separator();
            self.events_panel(ui);

            ui.separator();
            self.event_ownership_panel(ui);

            // ── Editing mode toolbar ────────────────────────────────
            ui.separator();
            ui.label("Mode:");
            ui.horizontal(|ui| {
                let modes = [
                    ("Select [1]", EditingMode::Select),
                    ("Draw [2]", EditingMode::Draw),
                    ("Erase [3]", EditingMode::Erase),
                ];
                for (label, mode) in modes {
                    if toggle_button(ui, label, self.editing_mode == mode) {
                        self.editing_mode = mode;
                    }
                }
            });

            // ── Layer selector ────────────────────────────────────────
            ui.horizontal(|ui| {
                ui.label("Layer:");
                let layers = [("L1", 1u8), ("L2", 2u8)];
                for (label, layer) in layers {
                    if toggle_button(ui, label, self.edit_layer == layer) {
                        self.edit_layer = layer;
                        self.preview_texture = None; // Force preview refresh
                    }
                }
            });

            // ── Draw mode tile picker ───────────────────────────────
            if self.editing_mode == EditingMode::Draw {
                ui.separator();
                ui.label("Paint tile:");
                ui.horizontal(|ui| {
                    let label = if self.edit_layer == 1 { "Tile ID" } else { "Tile" };
                    ui.label(format!("{label}: {:#04X}", self.draw_tile_num));
                    let mut t = self.draw_tile_num as u16;
                    if ui
                        .add(egui::Slider::new(&mut t, 0..=0xFF).show_value(false).hexadecimal(2, false, false))
                        .changed()
                    {
                        self.draw_tile_num = t as u8;
                    }
                });
                if self.edit_layer == 2 {
                    ui.horizontal(|ui| {
                        ui.label("Palette:");
                        let mut p = self.draw_palette as u16;
                        if ui.add(egui::Slider::new(&mut p, 0..=7)).changed() {
                            self.draw_palette = p as u8;
                        }
                    });

                    // VRAM tile picker grid
                    let tex = self.tile_picker.texture(ui.ctx());
                    let tex_size = tex.size();
                    let max_w = ui.available_width().min(300.0);
                    let display_w = max_w;
                    let display_h = display_w;
                    let (rect, resp) = ui.allocate_exact_size(vec2(display_w, display_h), egui::Sense::click());
                    ui.painter().image(
                        tex.id(),
                        rect,
                        Rect::from_min_size(egui::pos2(0.0, 0.0), vec2(1.0, 1.0)),
                        Color32::WHITE,
                    );

                    if resp.clicked_by(egui::PointerButton::Primary) {
                        if let Some(pos) = resp.interact_pointer_pos() {
                            let rel = pos - rect.min;
                            let px = rel.x / display_w * tex_size[0] as f32;
                            let py = rel.y / display_h * tex_size[1] as f32;
                            if let Some((tile_num, pal)) = self.tile_picker.tile_at_pixel(px, py) {
                                self.draw_tile_num = tile_num;
                                self.draw_palette = pal;
                            }
                        }
                    }

                    if let Some((col, row)) = self.tile_picker.tile_grid_pos(self.draw_tile_num, self.draw_palette) {
                        let tile_screen = display_w / (tex_size[0] as f32 / 16.0);
                        let sel_rect = Rect::from_min_size(
                            rect.min + vec2(col as f32 * tile_screen, row as f32 * tile_screen),
                            vec2(tile_screen, tile_screen),
                        );
                        ui.painter().rect_stroke(
                            sel_rect,
                            egui::CornerRadius::ZERO,
                            egui::Stroke::new(2.0_f32, Color32::YELLOW),
                            egui::StrokeKind::Outside,
                        );
                    }
                } else {
                    // Visual L1 tile picker grid
                    let tex = self.l1_tile_picker.texture(ui.ctx());
                    let tex_size = tex.size();
                    let max_w = ui.available_width().min(300.0);
                    let (rect, resp) = ui.allocate_exact_size(vec2(max_w, max_w), egui::Sense::click());
                    ui.painter().image(
                        tex.id(),
                        rect,
                        Rect::from_min_size(egui::pos2(0.0, 0.0), vec2(1.0, 1.0)),
                        Color32::WHITE,
                    );
                    if resp.clicked_by(egui::PointerButton::Primary) {
                        if let Some(pos) = resp.interact_pointer_pos() {
                            let rel = pos - rect.min;
                            let px = rel.x / max_w * tex_size[0] as f32;
                            let py = rel.y / max_w * tex_size[1] as f32;
                            if let Some(tile_id) = self.l1_tile_picker.block_at_pixel(px, py) {
                                self.draw_tile_num = tile_id;
                                self.preview_texture = None; // Invalidate preview cache
                            }
                        }
                    }
                    // Selection highlight
                    let (col, row) = self.l1_tile_picker.block_grid_pos(self.draw_tile_num);
                    let tile_screen = max_w / ow_tile_picker::L1_COLS as f32;
                    let sel_rect = Rect::from_min_size(
                        rect.min + vec2(col as f32 * tile_screen, row as f32 * tile_screen),
                        vec2(tile_screen, tile_screen),
                    );
                    ui.painter().rect_stroke(
                        sel_rect,
                        egui::CornerRadius::ZERO,
                        egui::Stroke::new(2.0_f32, Color32::YELLOW),
                        egui::StrokeKind::Outside,
                    );
                }
            }

            ui.separator();

            // ── Tile preview ────────────────────────────────────
            let draw_mode = self.editing_mode == EditingMode::Draw;
            if draw_mode {
                if self.edit_layer == 1 {
                    ui.label(format!("Paint tile ID: {:#04X}", self.draw_tile_num));
                } else {
                    ui.label(format!("Paint: {:#04X} pal {}", self.draw_tile_num, self.draw_palette));
                }
                let cache_key = (self.draw_tile_num as u32 | 0x100, self.draw_palette as u32);
                if self.preview_for != Some(cache_key) {
                    let image = if self.edit_layer == 1 {
                        render_source_l1_tile_preview(&mut self.cpu, self.draw_tile_num)
                    } else {
                        render_single_tile_preview(
                            &self.cpu.mem.vram,
                            &self.cpu.mem.cgram,
                            self.draw_tile_num,
                            self.draw_palette,
                        )
                    };
                    let handle = ui.ctx().load_texture(
                        format!("ow_draw_preview_{}", self.draw_tile_num),
                        image,
                        egui::TextureOptions::NEAREST,
                    );
                    self.preview_texture = Some(handle);
                    self.preview_for = Some(cache_key);
                }
                if let Some(ref tex) = self.preview_texture {
                    let display_size = 64.0;
                    let (rect, _) = ui.allocate_exact_size(vec2(display_size, display_size), egui::Sense::hover());
                    ui.painter().image(
                        tex.id(),
                        rect,
                        Rect::from_min_size(egui::pos2(0.0, 0.0), vec2(1.0, 1.0)),
                        Color32::WHITE,
                    );
                }
            } else if let Some((x, y)) = self.selected_tile {
                ui.label(format!("Selected: ({x}, {y}) [L{}]", self.edit_layer));
                let tilemap_base = if self.edit_layer == 2 { VRAM_L2_TILEMAP_BASE } else { VRAM_L1_TILEMAP_BASE };
                if self.edit_layer == 1 {
                    if let Some(tile_id) = self.source_l1_tile_at_view(x, y) {
                        ui.monospace(format!("  Source tile ID: {tile_id:#04X}"));
                    }
                    if let Some(idx) = self.source_l1_index_for_view(x, y) {
                        let tiles = self.edit_state.read(|s| s.layer1_tiles.clone());
                        if let Some(vanilla_level_num) = smwe_rom::overworld::level_number_for_index(&tiles, idx) {
                            let translevel = smwe_rom::overworld::translevel_for_index(&tiles, idx).unwrap_or(0);
                            ui.monospace(format!(
                                "  Level tile: #{vanilla_level_num:03X} (translevel {translevel:#04X})"
                            ));
                            ui.small(
                                "  vanilla number is order-derived — moving/inserting level tiles elsewhere \
                                 renumbers it unless overridden below",
                            );

                            let current = self.custom_level_numbers.get(&idx).copied().unwrap_or(vanilla_level_num);
                            let mut new_num = current as i32;
                            ui.horizontal(|ui| {
                                ui.label("  Assign level:");
                                let changed = ui
                                    .add(
                                        egui::Slider::new(
                                            &mut new_num,
                                            0..=smwe_rom::overworld::MAX_ASSIGNABLE_LEVEL_NUMBER as i32,
                                        )
                                        .hexadecimal(2, false, false),
                                    )
                                    .changed();
                                if changed {
                                    if new_num as u8 == vanilla_level_num {
                                        self.custom_level_numbers.remove(&idx);
                                    } else {
                                        self.custom_level_numbers.insert(idx, new_num as u8);
                                    }
                                    self.level_numbers_dirty = true;
                                    self.has_edits = true;
                                }
                            });
                            if self.custom_level_numbers.contains_key(&idx) {
                                ui.colored_label(
                                    egui::Color32::from_rgb(220, 160, 60),
                                    "  Overridden — needs the level-number patch on save",
                                );
                            }

                            // ── Level name editor ───────────────────────────
                            // Translevel indexes into the 93-entry name table.
                            let translevel_u8 = (translevel & 0xFF) as u8;
                            let vanilla_name =
                                self.vanilla_level_names.get(translevel as usize).cloned().unwrap_or_default();
                            // Keep the text buffer synced: selecting a
                            // different level tile re-decodes it.
                            if self.level_name_for != Some(translevel_u8) {
                                self.level_name_edit = self
                                    .custom_level_names
                                    .get(&translevel_u8)
                                    .cloned()
                                    .unwrap_or_else(|| vanilla_name.clone());
                                self.level_name_for = Some(translevel_u8);
                                self.level_name_error = None;
                            }
                            ui.horizontal(|ui| {
                                ui.label("  Level name:");
                                let resp = ui.add(
                                    egui::TextEdit::singleline(&mut self.level_name_edit)
                                        .desired_width(200.0)
                                        .hint_text(&vanilla_name),
                                );
                                if resp.changed() {
                                    use smwe_rom::overworld::level_names as ln;
                                    let trimmed = self.level_name_edit.trim().to_string();
                                    if trimmed.is_empty() || trimmed.to_uppercase() == vanilla_name.to_uppercase() {
                                        self.custom_level_names.remove(&translevel_u8);
                                        self.level_name_error = None;
                                        self.level_names_dirty = true;
                                        self.has_edits = true;
                                    } else {
                                        match ln::check_name(&trimmed) {
                                            Ok(normalized) => {
                                                self.custom_level_names.insert(translevel_u8, normalized);
                                                self.level_name_error = None;
                                                self.level_names_dirty = true;
                                                self.has_edits = true;
                                            }
                                            Err(e) => {
                                                // Refuse over-budget/invalid
                                                // input; the field keeps the
                                                // rejected text so the user
                                                // can fix it.
                                                self.level_name_error = Some(e.to_string());
                                            }
                                        }
                                    }
                                }
                            });
                            // Byte-budget feedback, mirroring the message-box
                            // editor: the game draws at most MAX_NAME_CHARS
                            // tiles per name (CODE_049D07's $26-byte stripe).
                            {
                                use smwe_rom::overworld::level_names as ln;
                                let used = self.level_name_edit.trim().chars().count();
                                let budget_color = if self.level_name_error.is_some() || used > ln::MAX_NAME_CHARS {
                                    egui::Color32::from_rgb(220, 60, 60)
                                } else {
                                    ui.style().visuals.text_color()
                                };
                                ui.colored_label(
                                    budget_color,
                                    format!("  Name encodes to {used} / {} tiles", ln::MAX_NAME_CHARS),
                                );
                                if let Some(err) = self.level_name_error.as_ref() {
                                    ui.colored_label(egui::Color32::from_rgb(220, 60, 60), format!("  {err}"));
                                }
                            }
                            if self.custom_level_names.contains_key(&translevel_u8) {
                                ui.colored_label(
                                    egui::Color32::from_rgb(220, 160, 60),
                                    "  Custom name — needs the name-table relocation patch on save",
                                );
                                ui.small("  A–Z 0–9 space # ' supported");
                            }
                        }
                    }
                } else {
                    let (crop_x, crop_y) = visible_map_crop(self.submap);
                    let tile_x = (x * 16 + crop_x) / 8;
                    let tile_y = (y * 16 + crop_y) / 8;
                    let addr = tilemap_vram_addr(tilemap_base, tile_x, tile_y);
                    let sub0 = u16::from_le_bytes([self.cpu.mem.vram[addr], self.cpu.mem.vram[addr + 1]]);
                    let tile_num = (sub0 & 0x3FF) as u32;
                    let pal = ((sub0 >> 10) & 0x7) as u32;
                    let flip_x = (sub0 & 0x4000) != 0;
                    let flip_y = (sub0 & 0x8000) != 0;
                    ui.monospace(format!("  TL vram #{tile_num:03X}  pal {pal}"));
                    if flip_x || flip_y {
                        ui.monospace(format!("  flip x={flip_x} y={flip_y}"));
                    }
                }

                let cache_key = ((x & 0xFFFF) | ((y & 0xFFFF) << 16), 0u32);
                if self.preview_for != Some(cache_key) {
                    let image = if self.edit_layer == 1 {
                        let tile_id = self.source_l1_tile_at_view(x, y).unwrap_or(0);
                        render_source_l1_tile_preview(&mut self.cpu, tile_id)
                    } else {
                        render_ow_block_preview(
                            &self.cpu.mem.vram,
                            &self.cpu.mem.cgram,
                            self.submap,
                            x,
                            y,
                            tilemap_base,
                        )
                    };
                    let handle =
                        ui.ctx().load_texture(format!("ow_preview_{x}_{y}"), image, egui::TextureOptions::NEAREST);
                    self.preview_texture = Some(handle);
                    self.preview_for = Some(cache_key);
                }
                if let Some(ref tex) = self.preview_texture {
                    let display_size = 64.0;
                    let (rect, _) = ui.allocate_exact_size(vec2(display_size, display_size), egui::Sense::hover());
                    ui.painter().image(
                        tex.id(),
                        rect,
                        Rect::from_min_size(egui::pos2(0.0, 0.0), vec2(1.0, 1.0)),
                        Color32::WHITE,
                    );
                }
            } else {
                ui.label("Selected: (none)");
            }
        });
    }

    fn central_panel(&mut self, ui: &mut Ui) {
        let available = vec2(ui.available_width(), ui.available_height());
        let (view_rect, resp) = ui.allocate_exact_size(available, Sense::click_and_drag());
        let painter = ui.painter_at(view_rect);

        // ── Auto-center on submap load ─────────────────────────────────────
        if self.needs_center {
            self.needs_center = false;
            let z = self.zoom;
            let (map_px_w, map_px_h) = visible_map_size(self.submap);
            self.offset =
                vec2((view_rect.width() / z - map_px_w as f32) * 0.5, (view_rect.height() / z - map_px_h as f32) * 0.5);
        }

        // ── Input handling moved below (needs the canvas origin) ──

        let zoom_delta = ui.input(|i| i.zoom_delta());
        let wheel_delta = ui.input(|i| i.raw_scroll_delta.y);
        if resp.contains_pointer() {
            let factor = if (zoom_delta - 1.0).abs() > f32::EPSILON {
                zoom_delta
            } else if wheel_delta.abs() > f32::EPSILON {
                (wheel_delta * 0.005).exp()
            } else {
                1.0
            };
            if factor != 1.0 {
                self.zoom = (self.zoom * factor).clamp(0.25, 16.0);
            }
        }

        // ── Background ───────────────────────────────────────────────────────
        painter.rect_filled(view_rect, CornerRadius::ZERO, Color32::from_rgb(16, 16, 20));

        let z = self.zoom;
        let (map_px_w, map_px_h) = visible_map_size(self.submap);
        let map16_cols = map_px_w.div_ceil(16);
        let map16_rows = map_px_h.div_ceil(16);
        let map16_sz = MAP16_PX * z;
        let canvas_w = map_px_w as f32 * z;
        let canvas_h = map_px_h as f32 * z;
        let origin = view_rect.min + self.offset * z;
        let ow_rect = Rect::from_min_size(origin, vec2(canvas_w, canvas_h));

        // ── Input ────────────────────────────────────────────────────────────
        // Shift+drag on layer 1 in Select mode draws a clipboard region
        // (LM v2.30 overworld copy) instead of panning.
        let shift = ui.input(|i| i.modifiers.shift);
        let region_dragging = shift
            && self.edit_layer == 1
            && matches!(self.editing_mode, EditingMode::Select | EditingMode::Probe)
            && resp.dragged_by(egui::PointerButton::Primary);
        let is_pan = resp.dragged_by(egui::PointerButton::Middle)
            || (resp.dragged_by(egui::PointerButton::Primary) && !region_dragging);
        if is_pan {
            self.offset += resp.drag_delta() / self.zoom;
        }
        if region_dragging {
            let tile_at = |pos: Pos2| -> (u32, u32) {
                let rel = (pos - origin) / map16_sz;
                (
                    rel.x.floor().clamp(0.0, map16_cols as f32 - 1.0) as u32,
                    rel.y.floor().clamp(0.0, map16_rows as f32 - 1.0) as u32,
                )
            };
            if resp.drag_started_by(egui::PointerButton::Primary) {
                if let Some(pos) = resp.interact_pointer_pos().or_else(|| resp.hover_pos()) {
                    let (ax, ay) = tile_at(pos);
                    self.ow_drag_anchor = Some((ax, ay));
                    self.ow_sel_rect = Some((ax, ay, ax, ay));
                    self.selected_tile = None;
                }
            } else if let (Some((ax, ay)), Some(pos)) = (self.ow_drag_anchor, resp.hover_pos()) {
                let (cx, cy) = tile_at(pos);
                self.ow_sel_rect = Some((ax.min(cx), ay.min(cy), ax.max(cx), ay.max(cy)));
            }
        } else if resp.drag_stopped_by(egui::PointerButton::Primary) {
            self.ow_drag_anchor = None;
        }

        // ── GL render ────────────────────────────────────────────────────────
        {
            let renderer = Arc::clone(&self.renderer);
            let draw_l1 = self.show_layer1;
            let draw_l2 = self.show_layer2;
            let ppp = ui.ctx().pixels_per_point();
            let screen_sz = view_rect.size() * ppp;
            let gl_offset = self.offset;
            let gl_zoom = z * ppp;

            // ── Overworld animated tiles ─────────────────────────────
            // SMW advances each animated tile slot once every 8 game-frames at
            // 60 fps, so each distinct animation frame shows for ~133ms.  We tick
            // at the same interval to match the real game's visual speed.
            const ANIM_INTERVAL: std::time::Duration = std::time::Duration::from_millis(133);
            if self.last_anim_tick.elapsed() >= ANIM_INTERVAL {
                self.last_anim_tick = std::time::Instant::now();
                smwe_emu::emu::advance_ow_anim_frame(&mut self.cpu);
                let r = self.renderer.lock().expect("Cannot lock overworld renderer");
                r.upload_gfx(&self.gl, &self.cpu.mem.vram);
            }
            ui.ctx().request_repaint_after(ANIM_INTERVAL);

            ui.painter().add(PaintCallback {
                rect:     view_rect,
                callback: Arc::new(CallbackFn::new(move |_info, painter| {
                    let r = renderer.lock().expect("Cannot lock overworld renderer");
                    r.paint(painter.gl().as_ref(), screen_sz, gl_zoom, gl_offset, draw_l1, draw_l2);
                })),
            });
        }

        // ── Border around canvas ──────────────────────────────────────────────
        painter.rect_stroke(
            ow_rect,
            CornerRadius::ZERO,
            Stroke::new(2.0_f32, Color32::from_white_alpha(140)),
            StrokeKind::Outside,
        );

        // ── Grid (Map16 block grid, aligned to L1) ───────────────────────────
        if self.show_grid || ui.input(|i| i.modifiers.shift_only()) {
            let stroke = Stroke::new(0.5_f32, Color32::from_white_alpha(25));
            let start_col = ((view_rect.min.x - origin.x) / map16_sz).floor() as i32;
            let end_col = ((view_rect.max.x - origin.x) / map16_sz).ceil() as i32;
            for c in start_col..=end_col {
                let px = origin.x + c as f32 * map16_sz;
                painter.vline(px, view_rect.y_range(), stroke);
            }
            let start_row = ((view_rect.min.y - origin.y) / map16_sz).floor() as i32;
            let end_row = ((view_rect.max.y - origin.y) / map16_sz).ceil() as i32;
            for r in start_row..=end_row {
                let py = origin.y + r as f32 * map16_sz;
                painter.hline(view_rect.x_range(), py, stroke);
            }
        }

        // ── Layer 2 event target markers ──────────────────────────────────────
        // Cyan ring = VRAM tile stream, orange ring = tilemap copy. Follows the
        // event checkboxes in the left panel (only active events are drawn).
        if self.show_l2_event_markers {
            let (crop_x, crop_y) = visible_map_crop(self.submap);
            let tile_sz = 8.0 * z;
            let l2 = &self.rom.overworld_l2_events;
            for (event, active) in self.active_events.iter().enumerate() {
                if !active {
                    continue;
                }
                let mark = |entry: &L2EventEntry| {
                    let (col, row) = entry.target_tile();
                    let sx = origin.x + (col as f32 * 8.0 - crop_x as f32) * z;
                    let sy = origin.y + (row as f32 * 8.0 - crop_y as f32) * z;
                    let center = egui::pos2(sx + tile_sz * 0.5, sy + tile_sz * 0.5);
                    if !view_rect.contains(center) {
                        return;
                    }
                    let color = l2_marker_color(entry.kind());
                    painter.circle_stroke(center, tile_sz * 0.45, Stroke::new(1.5_f32, color));
                    painter.circle_filled(center, 1.5, color);
                };
                if let Some(range) = l2.entries_for_event(event) {
                    for idx in range {
                        if let Some(entry) = l2.entries.get(idx) {
                            mark(entry);
                        }
                    }
                }
                for s in l2.silent_l2_events_for(event as u8) {
                    mark(&s.as_entry());
                }
            }
        }

        // ── Hover / click (Map16 block granularity) ───────────────────────────
        if let Some(cursor) = resp.hover_pos() {
            let rel = (cursor - origin) / map16_sz;
            let tx = rel.x.floor() as i32;
            let ty = rel.y.floor() as i32;
            if (0..map16_cols as i32).contains(&tx) && (0..map16_rows as i32).contains(&ty) {
                let x = tx as u32;
                let y = ty as u32;
                let addr = l1_vram_addr_for_map16(self.submap, x, y);
                let tile_id = u16::from_le_bytes([self.cpu.mem.vram[addr], self.cpu.mem.vram[addr + 1]]) & 0x03FF;
                let tile_rect =
                    Rect::from_min_size(origin + vec2(x as f32 * map16_sz, y as f32 * map16_sz), Vec2::splat(map16_sz));
                painter.rect_stroke(
                    tile_rect,
                    CornerRadius::ZERO,
                    Stroke::new(1.0_f32, Color32::WHITE),
                    StrokeKind::Outside,
                );

                if resp.clicked_by(egui::PointerButton::Primary)
                    && (self.editing_mode == EditingMode::Select || ui.input(|i| i.modifiers.alt))
                    && !ui.input(|i| i.modifiers.shift)
                {
                    self.selected_tile = Some((x, y));
                    // A plain click replaces the clipboard region selection.
                    self.ow_sel_rect = None;
                }

                painter.text(
                    view_rect.right_bottom() - vec2(6.0, 6.0),
                    egui::Align2::RIGHT_BOTTOM,
                    format!("({tx},{ty})  L1={tile_id:#05x}  {:.0}%", z * 100.0),
                    egui::FontId::monospace(10.0),
                    Color32::from_white_alpha(170),
                );
            }
        }

        // ── Editing interaction ─────────────────────────────────────
        // Shift suppresses plain-click selection while a clipboard region
        // is being drawn (see the region-select block above).
        self.handle_editing_interaction(&resp, origin, map16_sz, shift);

        // ── Keyboard shortcuts ──────────────────────────────────────
        ui.input_mut(|input| {
            if input.consume_shortcut(&egui::KeyboardShortcut::new(egui::Modifiers::COMMAND, Key::Z)) {
                self.handle_undo();
            }
            if input.consume_shortcut(&egui::KeyboardShortcut::new(egui::Modifiers::COMMAND, Key::Y)) {
                self.handle_redo();
            }
            if input.key_pressed(egui::Key::Num1) {
                self.editing_mode = EditingMode::Select;
            }
            if input.key_pressed(egui::Key::Num2) {
                self.editing_mode = EditingMode::Draw;
            }
            if input.key_pressed(egui::Key::Num3) {
                self.editing_mode = EditingMode::Erase;
            }
        });

        // ── Selected tile highlight ───────────────────────────────────────────
        if let Some((x, y)) = self.selected_tile {
            let r = Rect::from_min_size(origin + vec2(x as f32 * map16_sz, y as f32 * map16_sz), Vec2::splat(map16_sz));
            painter.rect_stroke(
                r,
                CornerRadius::ZERO,
                Stroke::new(2.0_f32, Color32::from_rgb(255, 220, 0)),
                StrokeKind::Outside,
            );
        }

        // ── Clipboard region selection highlight ────────────────────────────
        if let Some((x0, y0, x1, y1)) = self.ow_sel_rect {
            let r = Rect::from_min_max(
                origin + vec2(x0 as f32 * map16_sz, y0 as f32 * map16_sz),
                origin + vec2((x1 + 1) as f32 * map16_sz, (y1 + 1) as f32 * map16_sz),
            );
            painter.rect_stroke(
                r,
                CornerRadius::ZERO,
                Stroke::new(2.0_f32, Color32::from_rgb(80, 200, 255)),
                StrokeKind::Outside,
            );
        }

        // ── Clipboard: copy/paste overworld layer-1 tiles ───────────────
        // Lunar Magic v2.30 lets you copy/paste between the background
        // editor and the overworld. Ctrl+C copies the Shift+drag region (or
        // the selected tile); paste arrives as Event::Paste directly on
        // Ctrl+V — drain it here (a focused text widget keeps its own paste).
        let ow_widget_focused = ui.ctx().memory(|m| m.focused().is_some());
        if !ow_widget_focused {
            if ui.input(|i| i.events.contains(&egui::Event::Copy)) {
                self.ow_clipboard_copy(ui.ctx());
            }
            if let Some(text) = crate::ui::clipboard::take_paste_text(ui.ctx()) {
                // Paste at the hovered tile, falling back to the copy origin.
                let anchor = resp
                    .hover_pos()
                    .map(|pos| {
                        let rel = (pos - origin) / map16_sz;
                        (rel.x.floor().max(0.0) as u32, rel.y.floor().max(0.0) as u32)
                    })
                    .or(self.ow_copy_origin);
                self.ow_clipboard_paste(&text, anchor);
            }
        }
    }
}

// ── Tile list builders ────────────────────────────────────────────────────────

fn build_bg_tiles(vram: &[u8], tilemap_base: usize, submap: u8, scroll_x: i32, scroll_y: i32) -> Vec<Tile> {
    let mut tiles = Vec::with_capacity((OW_L2_COLS * VRAM_TILE_ROWS) as usize);
    let (crop_x, crop_y, view_w, view_h) = if submap == 0 {
        (0, 0, 512, 512)
    } else {
        (SUBMAP_VIEW_X, SUBMAP_VIEW_Y, SUBMAP_VIEW_W as i32, SUBMAP_VIEW_H as i32)
    };

    for row in 0..VRAM_TILE_ROWS {
        for col in 0..OW_L2_COLS {
            let addr = tilemap_vram_addr(tilemap_base, col, row);
            let t0 = vram[addr] as u16;
            let t1 = vram[addr + 1] as u16;
            let tile_num = t0 | ((t1 & 3) << 8);
            let palette = (t1 >> 2) & 7;
            let flip_x = (t1 & 0x40) != 0;
            let flip_y = (t1 & 0x80) != 0;
            let px = (col * 8) as i32 - scroll_x - crop_x;
            let py = (row * 8) as i32 - scroll_y - crop_y;
            if px <= -8 || py <= -8 || px >= view_w || py >= view_h {
                continue;
            }

            let t = tile_num | (palette << 10) | ((flip_x as u16) << 14) | ((flip_y as u16) << 15);
            tiles.push(ow_tile(px.max(0) as u32, py.max(0) as u32, t));
        }
    }
    tiles
}

fn ow_tile(x: u32, y: u32, t: u16) -> Tile {
    let t32 = t as u32;
    let tile = t32 & 0x3FF;
    let pal = (t32 >> 10) & 0x7;
    let scale = 8u32;
    let params = scale | (pal << 8) | (t32 & 0xC000);
    Tile([x, y, tile, params])
}

/// Label for an event-ownership combo option: the event number plus the
/// overworld tile it reveals (when the event has a reveal-tile entry), so the
/// user can pick events by what they visibly do.
fn event_option_label(event: Option<u8>, rom: &SmwRom) -> String {
    match event {
        None => "None (no event)".to_string(),
        Some(e) => {
            let off = rom.overworld_events.tile_offsets.get(e as usize).copied().unwrap_or(0);
            if off == 0 {
                format!("Event {e}")
            } else {
                format!("Event {e} — reveals tile {off:#06X}")
            }
        }
    }
}

/// Write `active_events` (indices 0..OW_EVENT_COUNT) into the emulated
/// `OWEventsActivated` WRAM table ($1F02-$1F60, 8 events/byte, MSB-first bit
/// order per SMWDisX `bank_04.asm` `DATA_04E44B`), so that when the real game
/// code runs `load_overworld` it applies exactly the reveal-tile swaps
/// (`CODE_04DA49`) for the events the user has toggled on.
fn apply_active_events_to_wram(cpu: &mut Cpu, active_events: &[bool]) {
    for byte_idx in 0..15u32 {
        let mut byte = 0u8;
        for bit in 0..8u32 {
            let event_num = (byte_idx * 8 + bit) as usize;
            if active_events.get(event_num).copied().unwrap_or(false) {
                byte |= 0x80 >> bit;
            }
        }
        cpu.mem.store_u8(0x1F02 + byte_idx, byte);
    }
}

/// One-line human description of a Layer 2 event table entry for the events
/// panel, e.g. `stream 36 tiles -> (12,7)` or `tilemap copy $7F8000+0x0900 -> (6,15)`.
fn describe_l2_entry(entry: &L2EventEntry) -> String {
    let (col, row) = entry.target_tile();
    match entry.kind() {
        L2EventKind::TileStream(n) => format!("stream {n} tiles -> ({col},{row})"),
        L2EventKind::TilemapCopy(off) => format!("tilemap copy WRAM+{off:#06X} -> ({col},{row})"),
    }
}

/// Marker color for a Layer 2 event entry on the map: cyan for VRAM tile
/// streams, orange for tilemap copies.
fn l2_marker_color(kind: L2EventKind) -> Color32 {
    match kind {
        L2EventKind::TileStream(_) => Color32::from_rgb(0, 220, 255),
        L2EventKind::TilemapCopy(_) => Color32::from_rgb(255, 170, 0),
    }
}

fn read_overworld_l2_words(cpu: &Cpu) -> Vec<u16> {
    let base = (0x7F4000 - 0x7E0000) as usize;
    let bytes = &cpu.mem.wram[base..base + 0x2000];
    bytes.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect()
}

fn write_overworld_l2_stream(
    rom_bytes: &mut [u8], has_smc_header: bool, start_pc_no_header: usize, output_len: usize, compressed: &[u8],
    label: &str,
) -> anyhow::Result<()> {
    let header_offset = usize::from(has_smc_header) * 0x200;
    let start = start_pc_no_header + header_offset;
    let old_size = lc_rle2::compressed_size_for_output(
        rom_bytes.get(start..).ok_or_else(|| anyhow::anyhow!("{label} ROM source start out of bounds"))?,
        output_len,
    );
    if compressed.len() <= old_size {
        let dst = rom_bytes
            .get_mut(start..start + old_size)
            .ok_or_else(|| anyhow::anyhow!("{label} ROM write range out of bounds"))?;
        dst[..compressed.len()].copy_from_slice(compressed);
        dst[compressed.len()..].fill(0);
    } else {
        let new_pc = find_free_space(rom_bytes, compressed.len(), 0x008000, header_offset)
            .ok_or_else(|| anyhow::anyhow!("{label} no free space found for {} bytes", compressed.len()))?;

        if let Some(dst) = rom_bytes.get_mut(start..start + old_size) {
            dst.fill(0xFF);
        }

        let new_file = new_pc + header_offset;
        rom_bytes
            .get_mut(new_file..new_file + compressed.len())
            .ok_or_else(|| anyhow::anyhow!("{label} new location write out of bounds"))?
            .copy_from_slice(compressed);

        let old_snes = AddrSnes::try_from_lorom(AddrPc(start_pc_no_header as u32))?.0;
        let new_snes = AddrSnes::try_from_lorom(AddrPc(new_pc as u32))?.0;
        patch_snes_pointer(rom_bytes, old_snes, new_snes, label)?;
    }
    Ok(())
}

fn patch_snes_pointer(rom_bytes: &mut [u8], old_snes: u32, new_snes: u32, label: &str) -> anyhow::Result<()> {
    let old_bytes = old_snes.to_le_bytes();
    let new_bytes = new_snes.to_le_bytes();
    let matches: Vec<usize> = rom_bytes
        .windows(3)
        .enumerate()
        .filter_map(|(offset, window)| (window == &old_bytes[..3]).then_some(offset))
        .collect();
    let [offset] = matches.as_slice() else {
        anyhow::bail!(
            "{label} expected exactly one pointer to SNES ${old_snes:06X}, found {}; refusing to repoint",
            matches.len()
        );
    };
    rom_bytes[*offset..*offset + 3].copy_from_slice(&new_bytes[..3]);
    log::info!("{label} repointed from SNES ${old_snes:06X} to ${new_snes:06X}");
    Ok(())
}

fn ow_l1_addr(col: u32, row: u32) -> usize {
    let x_part = (col & 0x0F) | ((col & 0x10) << 4);
    let y_part = ((row & 0x0F) << 4) | ((row & 0x10) << 5);
    (x_part + y_part) as usize
}

fn source_l1_subtiles(cpu: &mut Cpu, tile_id: u8) -> [u16; 4] {
    let ptr_base = 0x7E0FBEu32;
    let char_bank = 0x05_0000u32;
    let char_ptr = cpu.mem.load_u16(ptr_base + tile_id as u32 * 2) as u32;
    let gfx_addr = char_bank | char_ptr;
    [
        cpu.mem.load_u16(gfx_addr),
        cpu.mem.load_u16(gfx_addr + 2),
        cpu.mem.load_u16(gfx_addr + 4),
        cpu.mem.load_u16(gfx_addr + 6),
    ]
}

fn render_source_l1_tile_preview(cpu: &mut Cpu, tile_id: u8) -> egui::ColorImage {
    let sub_tiles = source_l1_subtiles(cpu, tile_id);
    let mut pixels = vec![0u8; 16 * 16 * 4];
    let offsets = [(0u32, 0u32), (8u32, 0u32), (0u32, 8u32), (8u32, 8u32)];
    for (sub_tile, (x0, y0)) in sub_tiles.into_iter().zip(offsets) {
        let tile_num = (sub_tile & 0x03FF) as usize;
        let pal = ((sub_tile >> 10) & 0x7) as usize;
        let flip_x = (sub_tile & 0x4000) != 0;
        let flip_y = (sub_tile & 0x8000) != 0;
        render_preview_tile(&cpu.mem.vram, &cpu.mem.cgram, tile_num, pal, flip_x, flip_y, x0, y0, 16, &mut pixels);
    }
    egui::ColorImage::from_rgba_unmultiplied([16, 16], &pixels)
}

#[allow(clippy::too_many_arguments)]
fn render_preview_tile(
    vram: &[u8], cgram: &[u8], tile_num: usize, pal: usize, flip_x: bool, flip_y: bool, x0: u32, y0: u32, width: usize,
    pixels: &mut [u8],
) {
    let tile_base = tile_num * 32;
    for ty_px in 0..8u32 {
        for tx_px in 0..8u32 {
            let px = if flip_x { 7 - tx_px } else { tx_px };
            let py = if flip_y { 7 - ty_px } else { ty_px };
            let row_off = tile_base + (py as usize) * 2;
            if row_off + 17 >= vram.len() {
                continue;
            }
            let b0 = vram[row_off];
            let b1 = vram[row_off + 1];
            let b2 = vram[row_off + 16];
            let b3 = vram[row_off + 17];
            let bit = 7 - px as usize;
            let color_idx =
                (((b0 >> bit) & 1) | (((b1 >> bit) & 1) << 1) | (((b2 >> bit) & 1) << 2) | (((b3 >> bit) & 1) << 3))
                    as usize;
            if color_idx == 0 {
                continue;
            }
            let pal_idx = pal * 16 + color_idx;
            let off_color = pal_idx * 2;
            if off_color + 1 >= cgram.len() {
                continue;
            }
            let lo = cgram[off_color] as u16;
            let hi = cgram[off_color + 1] as u16;
            let rgb = lo | (hi << 8);
            let r = ((rgb & 0x1F) << 3) as u8;
            let g = (((rgb >> 5) & 0x1F) << 3) as u8;
            let b = (((rgb >> 10) & 0x1F) << 3) as u8;
            let px_abs = x0 + tx_px;
            let py_abs = y0 + ty_px;
            let off = ((py_abs as usize) * width + px_abs as usize) * 4;
            if off + 3 < pixels.len() {
                pixels[off] = r;
                pixels[off + 1] = g;
                pixels[off + 2] = b;
                pixels[off + 3] = 255;
            }
        }
    }
}

fn render_ow_block_preview(
    vram: &[u8], cgram: &[u8], submap: u8, map16_x: u32, map16_y: u32, tilemap_base: usize,
) -> egui::ColorImage {
    let (crop_x, crop_y) = visible_map_crop(submap);
    let base_tile_x = (map16_x * 16 + crop_x) / 8;
    let base_tile_y = (map16_y * 16 + crop_y) / 8;
    let mut pixels = vec![0u8; 16 * 16 * 4];

    let sub_positions = [(0u32, 0u32), (1, 0), (0, 1), (1, 1)];
    for (dx, dy) in sub_positions {
        let tx = base_tile_x + dx;
        let ty = base_tile_y + dy;
        let addr = tilemap_vram_addr(tilemap_base, tx, ty);
        if addr + 1 >= vram.len() {
            continue;
        }
        let t0 = vram[addr] as u16;
        let t1 = vram[addr + 1] as u16;
        let tile_num = (t0 | ((t1 & 3) << 8)) as usize;
        let pal = ((t1 >> 2) & 7) as usize;
        let flip_x = (t1 & 0x40) != 0;
        let flip_y = (t1 & 0x80) != 0;

        let tile_base = tile_num * 32;
        let x0 = dx * 8;
        let y0 = dy * 8;
        for ty_px in 0..8u32 {
            for tx_px in 0..8u32 {
                let px = if flip_x { 7 - tx_px } else { tx_px };
                let py = if flip_y { 7 - ty_px } else { ty_px };
                let row_off = tile_base + (py as usize) * 2;
                if row_off + 17 >= vram.len() {
                    continue;
                }
                let b0 = vram[row_off];
                let b1 = vram[row_off + 1];
                let b2 = vram[row_off + 16];
                let b3 = vram[row_off + 17];
                let bit = 7 - px as usize;
                let color_idx = (((b0 >> bit) & 1)
                    | (((b1 >> bit) & 1) << 1)
                    | (((b2 >> bit) & 1) << 2)
                    | (((b3 >> bit) & 1) << 3)) as usize;
                if color_idx == 0 {
                    continue;
                }
                let pal_idx = pal * 16 + color_idx;
                let off_color = pal_idx * 2;
                if off_color + 1 >= cgram.len() {
                    continue;
                }
                let lo = cgram[off_color] as u16;
                let hi = cgram[off_color + 1] as u16;
                let rgb = lo | (hi << 8);
                let r = ((rgb & 0x1F) << 3) as u8;
                let g = (((rgb >> 5) & 0x1F) << 3) as u8;
                let b = (((rgb >> 10) & 0x1F) << 3) as u8;

                let px_abs = x0 + tx_px;
                let py_abs = y0 + ty_px;
                let off = ((py_abs as usize) * 16 + px_abs as usize) * 4;
                if off + 3 < pixels.len() {
                    pixels[off] = r;
                    pixels[off + 1] = g;
                    pixels[off + 2] = b;
                    pixels[off + 3] = 255;
                }
            }
        }
    }
    egui::ColorImage::from_rgba_unmultiplied([16, 16], &pixels)
}

fn render_single_tile_preview(vram: &[u8], cgram: &[u8], tile_num: u8, pal: u8) -> egui::ColorImage {
    let mut pixels = vec![0u8; 16 * 16 * 4];
    let tile_base = (tile_num as usize) * 32;
    for ty in 0..8u32 {
        for tx in 0..8u32 {
            let row_off = tile_base + (ty as usize) * 2;
            if row_off + 17 >= vram.len() {
                continue;
            }
            let b0 = vram[row_off];
            let b1 = vram[row_off + 1];
            let b2 = vram[row_off + 16];
            let b3 = vram[row_off + 17];
            let bit = 7 - tx as usize;
            let color_idx =
                (((b0 >> bit) & 1) | (((b1 >> bit) & 1) << 1) | (((b2 >> bit) & 1) << 2) | (((b3 >> bit) & 1) << 3))
                    as usize;
            if color_idx == 0 {
                continue;
            }
            let pal_idx = (pal as usize) * 16 + color_idx;
            let off_color = pal_idx * 2;
            if off_color + 1 >= cgram.len() {
                continue;
            }
            let lo = cgram[off_color] as u16;
            let hi = cgram[off_color + 1] as u16;
            let rgb = lo | (hi << 8);
            let r = ((rgb & 0x1F) << 3) as u8;
            let g = (((rgb >> 5) & 0x1F) << 3) as u8;
            let b = (((rgb >> 10) & 0x1F) << 3) as u8;
            for dy in 0..2u32 {
                for dx in 0..2u32 {
                    let px = tx * 2 + dx;
                    let py = ty * 2 + dy;
                    let off = ((py as usize) * 16 + px as usize) * 4;
                    if off + 3 < pixels.len() {
                        pixels[off] = r;
                        pixels[off + 1] = g;
                        pixels[off + 2] = b;
                        pixels[off + 3] = 255;
                    }
                }
            }
        }
    }
    egui::ColorImage::from_rgba_unmultiplied([16, 16], &pixels)
}
