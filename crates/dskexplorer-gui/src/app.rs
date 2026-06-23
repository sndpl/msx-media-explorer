//! Top-level application state and the `eframe::App` implementation.

use std::path::Path;

use msx_disk::search;
use msx_disk::view::basic;
use msx_disk::view::hex::ascii_char;
use msx_disk::view::screen::{self, ScreenMode};
use msx_disk::view::text::{self, ControlMode};
use msx_disk::DirEntry;

use crate::state::LoadedDisk;

/// How the selected file's contents are shown.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ViewMode {
    Hex,
    Text,
    Basic,
    Screen,
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
    /// Cached screen texture, keyed by the file path it was rendered from.
    screen_tex: Option<(String, egui::TextureHandle)>,
    search_query: String,
    search_is_hex: bool,
    /// Byte offsets of matches in the current file, with the active index.
    search_matches: Vec<usize>,
    search_pos: usize,
    /// One-shot request to scroll the hex view to a row.
    pending_scroll_row: Option<usize>,
    /// New name being typed when renaming the selected file.
    renaming: Option<String>,
    /// Path pending a delete confirmation.
    confirm_delete: Option<String>,
    /// Editable hex text when editing the current file's bytes.
    hex_edit: Option<String>,
}

/// Largest file (bytes) offered for in-app hex editing, to keep the editor
/// responsive.
const MAX_HEX_EDIT_BYTES: usize = 32 * 1024;

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
            screen_tex: None,
            search_query: String::new(),
            search_is_hex: false,
            search_matches: Vec::new(),
            search_pos: 0,
            pending_scroll_row: None,
            renaming: None,
            confirm_delete: None,
            hex_edit: None,
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
                self.view_mode = default_view_mode(&path);
                self.search_matches.clear();
                self.search_pos = 0;
                self.hex_edit = None;
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
            ui.menu_button("New…", |ui| {
                if ui.button("720 KB (double-sided)").clicked() {
                    self.new_disk(true);
                    ui.close();
                }
                if ui.button("360 KB (single-sided)").clicked() {
                    self.new_disk(false);
                    ui.close();
                }
            });
            if self.disk.is_some() {
                if ui.button("Save as .dsk…").clicked() {
                    self.save_as_dsk();
                }
                if ui.button("Save as .xsa…").clicked() {
                    self.save_as_xsa();
                }
            }
            let writable = self.disk_writable();
            if writable && ui.button("Add files…").clicked() {
                self.add_files_dialog();
            }
            if let Some(disk) = &self.disk {
                ui.separator();
                ui.label(disk.title());
                if let Some(label) = &disk.label {
                    ui.separator();
                    ui.label(format!("Label: {label}"));
                }
                if !disk.writable() {
                    ui.separator();
                    ui.weak("read-only");
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
        let writable = self.disk_writable();
        ui.horizontal(|ui| {
            ui.selectable_value(&mut self.view_mode, ViewMode::Hex, "Hex");
            ui.selectable_value(&mut self.view_mode, ViewMode::Text, "Text");
            ui.selectable_value(&mut self.view_mode, ViewMode::Basic, "BASIC");
            ui.selectable_value(&mut self.view_mode, ViewMode::Screen, "Screen");
            ui.separator();
            match self.view_mode {
                ViewMode::Hex => {
                    if self.hex_edit.is_some() {
                        if ui.button("Save edits").clicked() {
                            self.save_hex_edit();
                        }
                        if ui.button("Cancel").clicked() {
                            self.hex_edit = None;
                        }
                    } else {
                        ui.label("Bytes/row:");
                        for n in [8usize, 16, 24, 32] {
                            ui.selectable_value(&mut self.bytes_per_row, n, n.to_string());
                        }
                        let editable = writable
                            && self
                                .content
                                .as_ref()
                                .is_some_and(|c| c.bytes.len() <= MAX_HEX_EDIT_BYTES);
                        if editable && ui.button("Edit hex").clicked() {
                            let text = format_hex_for_edit(&self.content.as_ref().unwrap().bytes);
                            self.hex_edit = Some(text);
                        }
                    }
                }
                ViewMode::Text => {
                    ui.checkbox(&mut self.text_show_all, "Show all characters");
                }
                ViewMode::Basic | ViewMode::Screen => {}
            }
            if self.content.is_some() {
                ui.separator();
                if ui.button("Extract…").clicked() {
                    self.extract_selected();
                }
            }

            let file_selected = self.selected.is_some() && self.content.is_some();
            if self.renaming.is_some() {
                let mut apply = false;
                let mut cancel = false;
                if let Some(name) = self.renaming.as_mut() {
                    ui.separator();
                    ui.label("New name:");
                    ui.add(egui::TextEdit::singleline(name).desired_width(120.0));
                    apply = ui.button("Apply").clicked();
                    cancel = ui.button("Cancel").clicked();
                }
                if apply {
                    self.apply_rename();
                } else if cancel {
                    self.renaming = None;
                }
            } else if writable && file_selected {
                ui.separator();
                if ui.button("Rename").clicked() {
                    let base = self
                        .selected
                        .as_deref()
                        .and_then(|p| p.rsplit('/').next())
                        .unwrap_or("")
                        .to_string();
                    self.renaming = Some(base);
                }
                if ui.button("Delete").clicked() {
                    self.confirm_delete = self.selected.clone();
                }
            }
        });

        if self.content.is_some() {
            ui.horizontal(|ui| {
                ui.label("Find:");
                let resp =
                    ui.add(egui::TextEdit::singleline(&mut self.search_query).desired_width(180.0));
                ui.checkbox(&mut self.search_is_hex, "Hex");
                let submit = resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                if ui.button("Find").clicked() || submit {
                    self.run_search();
                }
                if !self.search_matches.is_empty() {
                    if ui.button("◀").clicked() {
                        self.step_search(false);
                    }
                    if ui.button("▶").clicked() {
                        self.step_search(true);
                    }
                    ui.label(format!(
                        "{}/{}",
                        self.search_pos + 1,
                        self.search_matches.len()
                    ));
                }
            });
        }
        ui.separator();

        if self.view_mode == ViewMode::Hex && self.hex_edit.is_some() {
            egui::ScrollArea::vertical()
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    if let Some(text) = self.hex_edit.as_mut() {
                        ui.add(
                            egui::TextEdit::multiline(text)
                                .code_editor()
                                .desired_width(f32::INFINITY),
                        );
                    }
                });
            return;
        }

        if self.view_mode == ViewMode::Screen {
            // The screen view needs the mutable texture cache, so handle it
            // outside the shared borrow of `self.content`.
            let mut cache = self.screen_tex.take();
            match &self.content {
                Some(content) => render_screen(ui, &mut cache, &content.path, &content.bytes),
                None => {
                    ui.weak("Select a file to view its contents.");
                }
            }
            self.screen_tex = cache;
            return;
        }

        let scroll_to = self.pending_scroll_row.take();
        let highlight = self
            .search_matches
            .get(self.search_pos)
            .map(|&o| o / self.bytes_per_row.max(1));

        match &self.content {
            None => {
                ui.weak("Select a file to view its contents.");
            }
            Some(content) => match self.view_mode {
                ViewMode::Hex => {
                    render_hex(ui, &content.bytes, self.bytes_per_row, scroll_to, highlight)
                }
                ViewMode::Text => render_text(ui, &content.bytes, self.text_show_all),
                ViewMode::Basic => render_basic(ui, &content.bytes),
                ViewMode::Screen => unreachable!("handled above"),
            },
        }
    }

    fn run_search(&mut self) {
        let Some(content) = &self.content else { return };
        let matches = if self.search_is_hex {
            match search::parse_hex(&self.search_query) {
                Some(needle) => search::find_bytes(&content.bytes, &needle),
                None => {
                    self.status = "Invalid hex pattern".to_string();
                    return;
                }
            }
        } else {
            search::find_text(&content.bytes, &self.search_query, true)
        };

        if matches.is_empty() {
            self.status = format!("No matches for \"{}\"", self.search_query);
            self.search_matches.clear();
            return;
        }
        self.status = format!("{} match(es)", matches.len());
        self.search_matches = matches;
        self.search_pos = 0;
        self.view_mode = ViewMode::Hex;
        self.jump_to_current_match();
    }

    fn step_search(&mut self, forward: bool) {
        if self.search_matches.is_empty() {
            return;
        }
        let n = self.search_matches.len();
        self.search_pos = if forward {
            (self.search_pos + 1) % n
        } else {
            (self.search_pos + n - 1) % n
        };
        self.jump_to_current_match();
    }

    fn jump_to_current_match(&mut self) {
        if let Some(&offset) = self.search_matches.get(self.search_pos) {
            self.pending_scroll_row = Some(offset / self.bytes_per_row.max(1));
        }
    }

    fn disk_writable(&self) -> bool {
        self.disk
            .as_ref()
            .map(LoadedDisk::writable)
            .unwrap_or(false)
    }

    /// Update state after a write operation completes.
    fn after_mutation(&mut self, result: msx_disk::Result<()>, ok_msg: String) {
        match result {
            Ok(()) => {
                self.status = ok_msg;
                self.selected = None;
                self.content = None;
                self.search_matches.clear();
                self.hex_edit = None;
            }
            Err(e) => self.status = format!("Write failed: {e}"),
        }
    }

    fn new_disk(&mut self, double_sided: bool) {
        let bytes = match msx_disk::fs::write::create_blank(double_sided) {
            Ok(b) => b,
            Err(e) => {
                self.status = format!("Could not create disk: {e}");
                return;
            }
        };
        let default = if double_sided {
            "blank720.dsk"
        } else {
            "blank360.dsk"
        };
        if let Some(path) = rfd::FileDialog::new().set_file_name(default).save_file() {
            match std::fs::write(&path, &bytes) {
                Ok(()) => self.open_path(&path),
                Err(e) => self.status = format!("Failed to write {}: {e}", path.display()),
            }
        }
    }

    fn save_as_dsk(&mut self) {
        self.save_converted("dsk", LoadedDisk::to_dsk_bytes);
    }

    fn save_as_xsa(&mut self) {
        self.save_converted("xsa", LoadedDisk::to_xsa_bytes);
    }

    /// Shared helper for "Save as <ext>": derive a default name, run `encode`,
    /// and write to a chosen path.
    fn save_converted(&mut self, ext: &str, encode: fn(&LoadedDisk) -> Vec<u8>) {
        let Some((bytes, default)) = self.disk.as_ref().map(|d| {
            let title = d.title();
            let stem = title.rsplit_once('.').map(|(s, _)| s).unwrap_or(&title);
            (encode(d), format!("{stem}.{ext}"))
        }) else {
            return;
        };
        if let Some(path) = rfd::FileDialog::new().set_file_name(&default).save_file() {
            self.status = match std::fs::write(&path, &bytes) {
                Ok(()) => format!("Saved {}", path.display()),
                Err(e) => format!("Failed to save: {e}"),
            };
        }
    }

    fn add_files_dialog(&mut self) {
        if !self.disk_writable() {
            self.status = "This image is read-only (.xsa or no source file).".to_string();
            return;
        }
        let Some(paths) = rfd::FileDialog::new().pick_files() else {
            return;
        };
        let mut files = Vec::new();
        for path in paths {
            match std::fs::read(&path) {
                Ok(bytes) => {
                    let raw = path
                        .file_name()
                        .map(|n| n.to_string_lossy().into_owned())
                        .unwrap_or_default();
                    files.push((sanitize_msx_name(&raw), bytes));
                }
                Err(e) => {
                    self.status = format!("Could not read {}: {e}", path.display());
                    return;
                }
            }
        }
        let count = files.len();
        let result = self.disk.as_mut().unwrap().add_files(&files);
        self.after_mutation(result, format!("Added {count} file(s)"));
    }

    fn apply_rename(&mut self) {
        let (Some(old_path), Some(new_name)) = (self.selected.clone(), self.renaming.take()) else {
            return;
        };
        let new_base = sanitize_msx_name(&new_name);
        let new_path = match old_path.rsplit_once('/') {
            Some((parent, _)) => format!("{parent}/{new_base}"),
            None => new_base.clone(),
        };
        let result = self.disk.as_mut().unwrap().rename(&old_path, &new_path);
        self.after_mutation(result, format!("Renamed to {new_base}"));
    }

    fn confirm_delete_now(&mut self) {
        let Some(path) = self.confirm_delete.take() else {
            return;
        };
        let result = self
            .disk
            .as_mut()
            .unwrap()
            .delete(std::slice::from_ref(&path));
        self.after_mutation(result, format!("Deleted {path}"));
    }

    fn save_hex_edit(&mut self) {
        let (Some(edited), Some(path)) = (self.hex_edit.clone(), self.selected.clone()) else {
            return;
        };
        let Some(bytes) = search::parse_hex(&edited) else {
            self.status =
                "Invalid hex: need whole byte pairs (0-9, A-F), whitespace ignored".to_string();
            return;
        };
        let result = self
            .disk
            .as_mut()
            .unwrap()
            .add_files(&[(path.clone(), bytes)]);
        self.after_mutation(result, format!("Saved edits to {path}"));
    }

    /// Render the delete-confirmation modal if a deletion is pending.
    fn delete_confirmation(&mut self, ctx: &egui::Context) {
        let Some(path) = self.confirm_delete.clone() else {
            return;
        };
        let mut do_delete = false;
        let mut cancel = false;
        egui::Window::new("Confirm delete")
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                ui.label(format!("Delete \"{path}\" from the disk?"));
                ui.label("This rewrites the image file on disk.");
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if ui.button("Delete").clicked() {
                        do_delete = true;
                    }
                    if ui.button("Cancel").clicked() {
                        cancel = true;
                    }
                });
            });
        if do_delete {
            self.confirm_delete_now();
        } else if cancel {
            self.confirm_delete = None;
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
///
/// `scroll_to_row` requests a one-shot scroll (e.g. to a search hit), and
/// `highlight_row` tints the current match row.
fn render_hex(
    ui: &mut egui::Ui,
    bytes: &[u8],
    bytes_per_row: usize,
    scroll_to_row: Option<usize>,
    highlight_row: Option<usize>,
) {
    let bpr = bytes_per_row.max(1);
    let total_rows = bytes.len().div_ceil(bpr);
    let row_height = ui.text_style_height(&egui::TextStyle::Monospace);
    let mut area = egui::ScrollArea::vertical().auto_shrink([false, false]);
    if let Some(row) = scroll_to_row {
        // Center the target row in the viewport where possible.
        let offset = (row as f32 * row_height - 80.0).max(0.0);
        area = area.vertical_scroll_offset(offset);
    }
    area.show_rows(ui, row_height, total_rows, |ui, range| {
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
            if highlight_row == Some(row) {
                ui.monospace(egui::RichText::new(line).background_color(egui::Color32::DARK_BLUE));
            } else {
                ui.monospace(line);
            }
        }
    });
}

/// Format bytes as editable hex: 16 space-separated pairs per line.
fn format_hex_for_edit(bytes: &[u8]) -> String {
    use std::fmt::Write;
    let mut out = String::new();
    for (i, b) in bytes.iter().enumerate() {
        if i > 0 {
            out.push(if i % 16 == 0 { '\n' } else { ' ' });
        }
        let _ = write!(out, "{b:02X}");
    }
    out
}

/// Coerce a host filename into an MSX-DOS 8.3 uppercase name.
fn sanitize_msx_name(name: &str) -> String {
    let upper = name.to_uppercase();
    let (stem, ext) = match upper.rsplit_once('.') {
        Some((s, e)) => (s, e),
        None => (upper.as_str(), ""),
    };
    let keep = |s: &str, max: usize| -> String {
        s.chars()
            .filter(|c| c.is_ascii_alphanumeric() || "_-!#$%&@^{}()~'".contains(*c))
            .take(max)
            .collect()
    };
    let mut stem = keep(stem, 8);
    if stem.is_empty() {
        stem = "FILE".to_string();
    }
    let ext = keep(ext, 3);
    if ext.is_empty() {
        stem
    } else {
        format!("{stem}.{ext}")
    }
}

/// Pick a sensible default view mode for a file based on its extension.
fn default_view_mode(path: &str) -> ViewMode {
    let ext = path.rsplit('.').next().unwrap_or("").to_ascii_lowercase();
    if ScreenMode::from_extension(&ext).is_some() {
        ViewMode::Screen
    } else if ext == "bas" {
        ViewMode::Basic
    } else {
        ViewMode::Hex
    }
}

/// Render a decoded MSX screen image, caching the GPU texture by file path.
fn render_screen(
    ui: &mut egui::Ui,
    cache: &mut Option<(String, egui::TextureHandle)>,
    path: &str,
    bytes: &[u8],
) {
    let Some(mode) = ScreenMode::from_filename(path) else {
        ui.weak("Not a recognized MSX screen file (.SC2/.SC5/.SC7/.SC8/.SCC).");
        return;
    };

    let stale = cache.as_ref().map(|(p, _)| p != path).unwrap_or(true);
    if stale {
        let bmp = screen::render(mode, bytes);
        let image = egui::ColorImage::from_rgba_unmultiplied([bmp.width, bmp.height], &bmp.rgba);
        let texture = ui.ctx().load_texture(
            format!("screen:{path}"),
            image,
            egui::TextureOptions::NEAREST,
        );
        *cache = Some((path.to_string(), texture));
    }

    if let Some((_, texture)) = cache {
        egui::ScrollArea::both().show(ui, |ui| {
            let size = texture.size_vec2() * 2.0; // 2x nearest-neighbour zoom
            ui.image(egui::load::SizedTexture::new(texture.id(), size));
        });
    }
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

        self.delete_confirmation(ui.ctx());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_edit_format_parse_roundtrip() {
        let bytes: Vec<u8> = (0u8..50).collect();
        let text = format_hex_for_edit(&bytes);
        assert!(text.contains('\n'), "should wrap at 16 bytes");
        assert_eq!(msx_disk::search::parse_hex(&text), Some(bytes));
    }

    #[test]
    fn sanitize_makes_msx_83_names() {
        assert_eq!(sanitize_msx_name("my long file.text"), "MYLONGFI.TEX");
        assert_eq!(sanitize_msx_name("a.b"), "A.B");
        assert_eq!(sanitize_msx_name(""), "FILE");
        assert_eq!(sanitize_msx_name("game.com"), "GAME.COM");
    }

    #[test]
    fn default_view_mode_by_extension() {
        assert_eq!(default_view_mode("PIC.SC8"), ViewMode::Screen);
        assert_eq!(default_view_mode("PROG.BAS"), ViewMode::Basic);
        assert_eq!(default_view_mode("DATA.BIN"), ViewMode::Hex);
    }
}
