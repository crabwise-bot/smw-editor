//! Lunar Magic parity: Tools > "Scan for Undefined Exits..." (LM v1.50 menu,
//! v1.60 toolbar button).
//!
//! The scan walks all 512 levels through the real emulator, which takes on
//! the order of two minutes, so it runs on a worker thread behind a
//! progress bar; the results window lists every screen whose exit-enabled
//! tiles have no configured destination (missing screen-exit record, or an
//! exit that resolves to one of the TEST levels `$000`/`$100`).

use std::sync::{
    atomic::{AtomicBool, Ordering},
    mpsc::{self, Receiver},
    Arc,
};

use egui::{Context, RichText, ScrollArea, Window};

use crate::{
    exit_scan::{scan_undefined_exits_with_progress, ExitScanReport, UndefinedExitKind},
    ui::UiMainWindow,
};

/// Messages from the scan worker thread to the UI thread.
enum ExitScanMsg {
    /// Number of levels scanned so far (out of 512).
    Progress(u32),
    /// The scan finished; `Err` carries a displayable message.
    Done(Result<ExitScanReport, String>),
}

struct RunningScan {
    rx:      Receiver<ExitScanMsg>,
    cancel:  Arc<AtomicBool>,
    scanned: u32,
}

enum ExitScanState {
    Idle,
    Running(RunningScan),
    Done(ExitScanReport),
    Failed(String),
}

/// State for the Tools > Scan for Undefined Exits window. Owned by
/// [`UiMainWindow`].
pub struct ExitScanUi {
    state: ExitScanState,
}

impl ExitScanUi {
    pub fn new() -> Self {
        ExitScanUi { state: ExitScanState::Idle }
    }

    pub fn is_running(&self) -> bool {
        matches!(self.state, ExitScanState::Running(_))
    }

    pub fn has_report(&self) -> bool {
        matches!(self.state, ExitScanState::Done(_))
    }

    fn set_failed(&mut self, msg: String) {
        self.state = ExitScanState::Failed(msg);
    }

    /// Start a scan over `rom_bytes` (already merged with unsaved tab edits
    /// by the caller). No-op while a scan is already running.
    pub fn start(&mut self, rom_bytes: Vec<u8>) {
        if self.is_running() {
            return;
        }
        let (tx, rx) = mpsc::channel();
        let cancel = Arc::new(AtomicBool::new(false));
        let cancel_flag = Arc::clone(&cancel);
        std::thread::Builder::new()
            .name("exit-scan".to_string())
            .spawn(move || {
                let mut last_sent = 0u32;
                let result = scan_undefined_exits_with_progress(&rom_bytes, &mut |n| {
                    // The channel is unbounded but the UI only needs coarse
                    // progress; send every 4 levels.
                    if n - last_sent >= 4 || n >= 512 {
                        last_sent = n;
                        if tx.send(ExitScanMsg::Progress(n)).is_err() {
                            return false;
                        }
                    }
                    !cancel_flag.load(Ordering::Relaxed)
                })
                .map_err(|e| e.to_string());
                let _ = tx.send(ExitScanMsg::Done(result));
            })
            .expect("failed to spawn exit-scan thread");
        self.state = ExitScanState::Running(RunningScan { rx, cancel, scanned: 0 });
    }

    /// Drop a running scan; the worker thread notices the cancel flag (or the
    /// closed channel) and stops at the next level boundary.
    pub fn cancel(&mut self) {
        if let ExitScanState::Running(running) = &self.state {
            running.cancel.store(true, Ordering::Relaxed);
        }
        self.state = ExitScanState::Idle;
    }

    /// Drain pending worker messages. Called every frame while the window is
    /// open.
    fn poll(&mut self) {
        let running = match &mut self.state {
            ExitScanState::Running(r) => r,
            _ => return,
        };
        while let Ok(msg) = running.rx.try_recv() {
            match msg {
                ExitScanMsg::Progress(n) => running.scanned = n,
                ExitScanMsg::Done(result) => {
                    self.state = match result {
                        Ok(report) => ExitScanState::Done(report),
                        Err(e) if e == "scan cancelled" => ExitScanState::Idle,
                        Err(e) => ExitScanState::Failed(e),
                    };
                    return;
                }
            }
        }
    }

    fn summary_text(report: &ExitScanReport) -> String {
        let n = report.findings.len();
        let mut s = format!(
            "Scanned {} levels — {} undefined exit{} found.",
            report.levels_scanned,
            n,
            if n == 1 { "" } else { "s" }
        );
        if report.levels_skipped > 0 {
            s.push_str(&format!(
                " ({} level{} skipped: unparseable data)",
                report.levels_skipped,
                if report.levels_skipped == 1 { "" } else { "s" }
            ));
        }
        s
    }

    fn finding_text(f: &crate::exit_scan::UndefinedExit) -> String {
        match &f.kind {
            UndefinedExitKind::NoExitRecord => {
                format!("Level ${:03X}, screen {} — exit-enabled tiles but no screen exit defined", f.level, f.screen)
            }
            UndefinedExitKind::UndefinedDestination { via_secondary, destination } => {
                let via = via_secondary.map(|i| format!(" via secondary exit ${i:03X}")).unwrap_or_default();
                format!(
                    "Level ${:03X}, screen {} — exit leads to level ${destination:03X} (TEST){via}",
                    f.level, f.screen
                )
            }
        }
    }
}

impl UiMainWindow {
    /// Tools > Scan for Undefined Exits... — start the scan (ROM bytes with
    /// unsaved tab edits merged, like the PNG export) and open the window.
    pub(crate) fn open_exit_scan(&mut self) {
        self.show_exit_scan_dialog = true;
        if !self.exit_scan.is_running() && !self.exit_scan.has_report() {
            match self.rom_bytes_with_tab_edits() {
                Ok(bytes) => self.exit_scan.start(bytes),
                Err(e) => self.exit_scan.set_failed(format!("Could not read ROM: {e:#}")),
            }
        }
    }

    /// Re-run the scan over the current ROM image (with tab edits merged).
    fn rescan_exits(&mut self) {
        match self.rom_bytes_with_tab_edits() {
            Ok(bytes) => self.exit_scan.start(bytes),
            Err(e) => self.exit_scan.set_failed(format!("Could not read ROM: {e:#}")),
        }
    }

    pub(crate) fn exit_scan_window(&mut self, ctx: &Context) {
        self.exit_scan.poll();
        let mut open = true;
        let mut rescan = false;
        Window::new("Scan for Undefined Exits").collapsible(false).resizable(true).show(ctx, |ui| {
            match &self.exit_scan.state {
                ExitScanState::Running(running) => {
                    let scanned = running.scanned;
                    ui.label(format!("Scanning levels… {scanned} / 512"));
                    ui.add(egui::ProgressBar::new(scanned as f32 / 512.0).show_percentage());
                    ui.label("Each level is decompressed through the real emulator; this takes a couple of minutes.");
                    // Keep the progress bar moving while the worker runs.
                    ctx.request_repaint();
                    ui.horizontal(|ui| {
                        if ui.button("Cancel").clicked() {
                            self.exit_scan.cancel();
                        }
                    });
                }
                ExitScanState::Failed(msg) => {
                    ui.label(RichText::new(format!("Scan failed: {msg}")).color(egui::Color32::RED));
                    ui.horizontal(|ui| {
                        if ui.button("Re-scan").clicked() {
                            rescan = true;
                        }
                        if ui.button("Close").clicked() {
                            open = false;
                        }
                    });
                }
                ExitScanState::Idle => {
                    ui.label("No scan has been run yet.");
                    ui.horizontal(|ui| {
                        if ui.button("Scan now").clicked() {
                            rescan = true;
                        }
                        if ui.button("Close").clicked() {
                            open = false;
                        }
                    });
                }
                ExitScanState::Done(report) => {
                    ui.label(ExitScanUi::summary_text(report));
                    ui.label(
                        "LM v1.50: exits whose destination was never set up point at the TEST levels ($000/$100).",
                    );
                    ui.separator();
                    if report.findings.is_empty() {
                        ui.label("✓ No undefined exits found.");
                    } else {
                        ScrollArea::vertical().max_height(320.0).show(ui, |ui| {
                            for f in &report.findings {
                                ui.label(format!("⚠ {}", ExitScanUi::finding_text(f)));
                            }
                        });
                    }
                    ui.separator();
                    ui.horizontal(|ui| {
                        if ui.button("Re-scan").clicked() {
                            rescan = true;
                        }
                        if ui.button("Close").clicked() {
                            open = false;
                        }
                    });
                }
            }
        });
        if rescan {
            self.rescan_exits();
        }
        if !open {
            self.show_exit_scan_dialog = false;
        }
    }
}
