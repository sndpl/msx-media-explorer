//! Top-level application state and the `eframe::App` implementation.

use std::path::Path;

use msx_disk::view::basic;
use msx_disk::view::hex::ascii_char;
use msx_disk::view::text::{self, ControlMode};
use msx_disk::DirEntry;

use crate::state::LoadedDisk;

/// How the selected file's contents are shown.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ViewMode {
    Hex,
    Text,
    Basic,
}

/// The largest amount of a file rendered in the text view at once.
const MAX_TEXT_BYTES: usize = 128 * 1024;

/// Cached contents of the selected file (full bytes; views render lazily).
struct FileContent {
    path: String,
    bytes: Vec<u8>,
}

/// Root application state.
pub struct DskExplorerApp {
    disk: Option<LoadedDisk>,
    status: String,
    selected: Option<String>,
    content: Option<FileContent>,
    view_mode: ViewMode,
    bytes_per_row: usize,
    text_show_all: bool,
}

impl Default for DskExplorerApp {
    fn default() -> Self {
        DskExplorerApp {
            disk: None,
            status: "Open a disk image (or drag one in) to get started.".to_string(),
            selected: None,
            content: None,
            view_mode: ViewMode::Hex,
            bytes_per_row: 16,
            text_show_all: false,
        }
    }
}

impl DskExplorerApp {
    fn open_path(&mut self, path: &Path) {
        match LoadedDisk::open(path) {
            Ok(disk) => {
                self.status = format!(
                    "Opened {} ({:?}, {} sectors)",
                    disk.title(),
                    disk.format,
                    disk.geometry.total_sectors()
                );
                self.disk = Some(disk);
                self.selected = None;
                self.content = None;
            }
            Err(e) => self.status = format!("Failed to open {}: {e}", path.display()),
        }
    }

    fn select_file(&mut self, path: String) {
        let Some(disk) = &self.disk else { return };
        match disk.read_file(&path) {
            Ok(bytes) => {
                self.status = format!("{path} — {} bytes", bytes.len());
                self.content = Some(FileContent {
                    path: path.clone(),
                    bytes,
                });
                self.selected = Some(path);
            }
            Err(e) => self.status = format!("Cannot read {path}: {e}"),
        }
    }

    fn handle_dropped_files(&mut self, ctx: &egui::Context) {
        let dropped = ctx.input(|i| i.raw.dropped_files.clone());
        if let Some(path) = dropped.into_iter().find_map(|f| f.path) {
            self.open_path(&path);
        }
    }

    fn toolbar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            if ui.button("Open…").clicked() {
                if let Some(path) = rfd::FileDialog::new()
                    .add_filter(
                        "MSX disk images",
                        &[
                            "dsk", "di1", "ds1", "di2", "ds2", "img", "msx", "ddi", "xsa",
                        ],
                    )
                    .pick_file()
                {
                    self.open_path(&path);
                }
            }
            if let Some(disk) = &self.disk {
                ui.separator();
                ui.label(disk.title());
                if let Some(label) = &disk.label {
                    ui.separator();
                    ui.label(format!("Label: {label}"));
                }
            }
        });
    }

    fn tree_panel(&mut self, ui: &mut egui::Ui) {
        let mut clicked: Option<String> = None;
        if let Some(disk) = &self.disk {
            egui::ScrollArea::vertical()
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    render_entries(ui, &disk.tree, self.selected.as_deref(), &mut clicked);
                });
        } else {
            ui.weak("No disk open.");
        }
        if let Some(path) = clicked {
            self.select_file(path);
        }
    }

    fn viewer_panel(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.selectable_value(&mut self.view_mode, ViewMode::Hex, "Hex");
            ui.selectable_value(&mut self.view_mode, ViewMode::Text, "Text");
            ui.selectable_value(&mut self.view_mode, ViewMode::Basic, "BASIC");
            ui.separator();
            match self.view_mode {
                ViewMode::Hex => {
                    ui.label("Bytes/row:");
                    for n in [8usize, 16, 24, 32] {
                        ui.selectable_value(&mut self.bytes_per_row, n, n.to_string());
                    }
                }
                ViewMode::Text => {
                    ui.checkbox(&mut self.text_show_all, "Show all characters");
                }
                ViewMode::Basic => {}
            }
            if self.content.is_some() {
                ui.separator();
                if ui.button("Extract…").clicked() {
                    self.extract_selected();
                }
            }
        });
        ui.separator();

        match &self.content {
            None => {
                ui.weak("Select a file to view its contents.");
            }
            Some(content) => match self.view_mode {
                ViewMode::Hex => render_hex(ui, &content.bytes, self.bytes_per_row),
                ViewMode::Text => render_text(ui, &content.bytes, self.text_show_all),
                ViewMode::Basic => render_basic(ui, &content.bytes),
            },
        }
    }

    fn extract_selected(&mut self) {
        let Some(content) = &self.content else { return };
        let default_name = content
            .path
            .rsplit('/')
            .next()
            .unwrap_or("file")
            .to_string();
        let bytes = content.bytes.clone();
        if let Some(target) = rfd::FileDialog::new()
            .set_file_name(&default_name)
            .save_file()
        {
            self.status = match std::fs::write(&target, &bytes) {
                Ok(()) => format!("Extracted {} to {}", default_name, target.display()),
                Err(e) => format!("Failed to extract: {e}"),
            };
        }
    }
}

/// Recursively render the directory tree, recording a clicked file path.
fn render_entries(
    ui: &mut egui::Ui,
    entries: &[DirEntry],
    selected: Option<&str>,
    clicked: &mut Option<String>,
) {
    for entry in entries {
        if entry.is_dir {
            egui::CollapsingHeader::new(format!("\u{1F4C1} {}", entry.name))
                .default_open(true)
                .show(ui, |ui| {
                    render_entries(ui, &entry.children, selected, clicked);
                });
        } else {
            let is_selected = selected == Some(entry.path.as_str());
            let label = format!("{:<14} {:>8}", entry.name, entry.size);
            if ui
                .selectable_label(is_selected, egui::RichText::new(label).monospace())
                .clicked()
            {
                *clicked = Some(entry.path.clone());
            }
        }
    }
}

/// Virtualized hex view: only the visible rows are formatted each frame.
fn render_hex(ui: &mut egui::Ui, bytes: &[u8], bytes_per_row: usize) {
    let bpr = bytes_per_row.max(1);
    let total_rows = bytes.len().div_ceil(bpr);
    let row_height = ui.text_style_height(&egui::TextStyle::Monospace);
    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show_rows(ui, row_height, total_rows, |ui, range| {
            for row in range {
                let offset = row * bpr;
                let chunk = &bytes[offset..(offset + bpr).min(bytes.len())];
                let mut line = format!("{offset:06X}  ");
                for col in 0..bpr {
                    match chunk.get(col) {
                        Some(b) => line.push_str(&format!("{b:02X} ")),
                        None => line.push_str("   "),
                    }
                }
                line.push(' ');
                line.extend(chunk.iter().map(|&b| ascii_char(b)));
                ui.monospace(line);
            }
        });
}

/// Detokenized MSX-BASIC listing.
fn render_basic(ui: &mut egui::Ui, bytes: &[u8]) {
    let listing = basic::detokenize(bytes);
    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            ui.add(egui::Label::new(egui::RichText::new(listing).monospace()).wrap());
        });
}

/// Text view, capped to a sane size for responsiveness.
fn render_text(ui: &mut egui::Ui, bytes: &[u8], show_all: bool) {
    let shown = &bytes[..bytes.len().min(MAX_TEXT_BYTES)];
    let mode = if show_all {
        ControlMode::ShowAll
    } else {
        ControlMode::Dots
    };
    let rendered = text::to_text(shown, mode);
    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            if bytes.len() > MAX_TEXT_BYTES {
                ui.weak(format!(
                    "Showing first {} KB of {} KB.",
                    MAX_TEXT_BYTES / 1024,
                    bytes.len() / 1024
                ));
            }
            ui.add(egui::Label::new(egui::RichText::new(rendered).monospace()).wrap());
        });
}

impl eframe::App for DskExplorerApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.handle_dropped_files(ui.ctx());

        egui::Panel::top("toolbar").show_inside(ui, |ui| {
            ui.add_space(4.0);
            self.toolbar(ui);
            ui.add_space(4.0);
        });
        egui::Panel::bottom("status").show_inside(ui, |ui| {
            ui.add_space(2.0);
            ui.label(&self.status);
            ui.add_space(2.0);
        });
        egui::Panel::left("tree")
            .resizable(true)
            .default_size(280.0)
            .show_inside(ui, |ui| {
                self.tree_panel(ui);
            });
        egui::CentralPanel::default().show_inside(ui, |ui| {
            self.viewer_panel(ui);
        });
    }
}
