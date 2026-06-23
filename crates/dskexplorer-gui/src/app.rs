//! Top-level application state and the `eframe::App` implementation.

use std::collections::BTreeSet;
use std::path::Path;

use msx_disk::fs::map::SectorKind;
use msx_disk::recoil::{self, FnCompanions};
use msx_disk::search;
use msx_disk::view::basic;
use msx_disk::view::hex::{ascii_char, dump_to_string, HexConfig};
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

/// Top-level view: browse files, view raw sectors, or the disk map.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AppView {
    Files,
    Sectors,
    Map,
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
    /// The file currently shown in the viewer (the last one clicked).
    selected: Option<String>,
    /// All files marked for batch extract/delete (Cmd/Ctrl-click to toggle).
    selection: BTreeSet<String>,
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
    /// Paths pending a delete confirmation.
    confirm_delete: Option<Vec<String>>,
    /// Editable hex text when editing the current file's bytes.
    hex_edit: Option<String>,
    /// Top-level view selection.
    app_view: AppView,
    /// Sector shown in the Sectors view.
    current_sector: usize,
    /// Editable hex text when editing the current sector.
    sector_edit: Option<String>,
    /// Cached sector-usage map for the open disk.
    disk_map: Option<msx_disk::fs::map::DiskMap>,
    /// OS clipboard handle (kept alive so copied data persists on Linux).
    clipboard: Option<arboard::Clipboard>,
    /// Disk-wide search (Sectors view).
    disk_search_query: String,
    disk_search_is_hex: bool,
    /// Byte offsets of disk-wide matches, with the active index.
    disk_search_matches: Vec<usize>,
    disk_search_pos: usize,
    /// One-shot scroll-to-row request for the sector hex view.
    sector_scroll_row: Option<usize>,
    /// (sector, row) to highlight in the sector hex view.
    sector_highlight: Option<(usize, usize)>,
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
            selection: BTreeSet::new(),
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
            app_view: AppView::Files,
            current_sector: 0,
            sector_edit: None,
            disk_map: None,
            clipboard: None,
            disk_search_query: String::new(),
            disk_search_is_hex: false,
            disk_search_matches: Vec::new(),
            disk_search_pos: 0,
            sector_scroll_row: None,
            sector_highlight: None,
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
                self.selection.clear();
                self.content = None;
                self.current_sector = 0;
                self.sector_edit = None;
                self.disk_map = self.disk.as_ref().and_then(LoadedDisk::disk_map);
            }
            Err(e) => self.status = format!("Failed to open {}: {e}", path.display()),
        }
    }

    /// Open a file in the viewer. With `toggle` (Cmd/Ctrl-click), add or remove
    /// it from the multi-selection; otherwise it becomes the sole selection.
    fn select_file(&mut self, path: String, toggle: bool) {
        let bytes = match self.disk.as_ref().map(|d| d.read_file(&path)) {
            Some(Ok(bytes)) => bytes,
            Some(Err(e)) => {
                self.status = format!("Cannot read {path}: {e}");
                return;
            }
            None => return,
        };
        self.status = format!("{path} — {} bytes", bytes.len());
        self.view_mode = default_view_mode(&path);
        self.search_matches.clear();
        self.search_pos = 0;
        self.hex_edit = None;
        if toggle {
            if !self.selection.remove(&path) {
                self.selection.insert(path.clone());
            }
        } else {
            self.selection.clear();
            self.selection.insert(path.clone());
        }
        self.content = Some(FileContent {
            path: path.clone(),
            bytes,
        });
        self.selected = Some(path);
    }

    /// The files targeted by batch operations: the multi-selection, or the
    /// single viewed file when nothing is explicitly marked.
    fn selection_paths(&self) -> Vec<String> {
        if self.selection.is_empty() {
            self.selected.iter().cloned().collect()
        } else {
            self.selection.iter().cloned().collect()
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
        if self.disk.is_some() {
            ui.horizontal(|ui| {
                ui.selectable_value(&mut self.app_view, AppView::Files, "Files");
                ui.selectable_value(&mut self.app_view, AppView::Sectors, "Sectors");
                ui.selectable_value(&mut self.app_view, AppView::Map, "Map");
            });
        }
    }

    fn tree_panel(&mut self, ui: &mut egui::Ui) {
        if self.selection.len() > 1 {
            ui.horizontal(|ui| {
                ui.label(format!("{} files selected", self.selection.len()));
                if ui.button("Clear").clicked() {
                    self.selection.clear();
                }
            });
            ui.separator();
        }
        let mut clicked: Option<(String, bool)> = None;
        if let Some(disk) = &self.disk {
            egui::ScrollArea::vertical()
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    let show_meta = disk.dos == msx_disk::fs::DosVersion::Dos2;
                    render_entries(ui, &disk.tree, &self.selection, show_meta, &mut clicked);
                });
        } else {
            ui.weak("No disk open.");
        }
        if let Some((path, toggle)) = clicked {
            self.select_file(path, toggle);
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
                let extract_label = if self.selection.len() > 1 {
                    format!("Extract {}…", self.selection.len())
                } else {
                    "Extract…".to_string()
                };
                if ui.button(extract_label).clicked() {
                    self.extract_selected();
                }
                let copy_label = if self.view_mode == ViewMode::Screen {
                    "Copy image"
                } else {
                    "Copy"
                };
                if ui.button(copy_label).clicked() {
                    self.copy_current_view();
                }
                if self.view_mode == ViewMode::Screen && ui.button("Save PNG…").clicked() {
                    self.save_screen_png();
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
                let delete_label = if self.selection.len() > 1 {
                    format!("Delete {}", self.selection.len())
                } else {
                    "Delete".to_string()
                };
                if ui.button(delete_label).clicked() {
                    self.confirm_delete = Some(self.selection_paths());
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
            match (self.disk.as_ref(), self.content.as_ref()) {
                (Some(disk), Some(content)) => {
                    let path = content.path.clone();
                    let companions = FnCompanions(|ext: &str| disk.companion(&path, ext));
                    render_screen(ui, &mut cache, &content.path, &content.bytes, &companions);
                }
                _ => {
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
                self.selection.clear();
                self.content = None;
                self.search_matches.clear();
                self.hex_edit = None;
                self.disk_map = self.disk.as_ref().and_then(LoadedDisk::disk_map);
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
        let Some(paths) = self.confirm_delete.take() else {
            return;
        };
        if paths.is_empty() {
            return;
        }
        let msg = if paths.len() == 1 {
            format!("Deleted {}", paths[0])
        } else {
            format!("Deleted {} files", paths.len())
        };
        let result = self.disk.as_mut().unwrap().delete(&paths);
        self.after_mutation(result, msg);
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
        let Some(paths) = self.confirm_delete.clone() else {
            return;
        };
        if paths.is_empty() {
            self.confirm_delete = None;
            return;
        }
        let mut do_delete = false;
        let mut cancel = false;
        egui::Window::new("Confirm delete")
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                if let [path] = paths.as_slice() {
                    ui.label(format!("Delete \"{path}\" from the disk?"));
                } else {
                    ui.label(format!("Delete these {} files from the disk?", paths.len()));
                    egui::ScrollArea::vertical()
                        .max_height(160.0)
                        .show(ui, |ui| {
                            for path in &paths {
                                ui.monospace(path);
                            }
                        });
                }
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
        let paths = self.selection_paths();
        match paths.as_slice() {
            [] => {}
            [path] => self.extract_one(path),
            many => self.extract_many(many),
        }
    }

    /// Extract a single file via a save-as dialog (lets the user rename it).
    fn extract_one(&mut self, path: &str) {
        let Some(disk) = &self.disk else { return };
        let bytes = match disk.read_file(path) {
            Ok(bytes) => bytes,
            Err(e) => {
                self.status = format!("Cannot read {path}: {e}");
                return;
            }
        };
        let default_name = base_name(path);
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

    /// Extract several files into a chosen folder, keeping their disk names.
    fn extract_many(&mut self, paths: &[String]) {
        let Some(dir) = rfd::FileDialog::new().pick_folder() else {
            return;
        };
        let mut ok = 0usize;
        let mut failed = 0usize;
        for path in paths {
            let read = self.disk.as_ref().map(|d| d.read_file(path));
            match read {
                Some(Ok(bytes)) => {
                    if std::fs::write(dir.join(base_name(path)), &bytes).is_ok() {
                        ok += 1;
                    } else {
                        failed += 1;
                    }
                }
                _ => failed += 1,
            }
        }
        self.status = if failed == 0 {
            format!("Extracted {ok} file(s) to {}", dir.display())
        } else {
            format!(
                "Extracted {ok} file(s) to {}, {failed} failed",
                dir.display()
            )
        };
    }

    fn ensure_clipboard(&mut self) -> Option<&mut arboard::Clipboard> {
        if self.clipboard.is_none() {
            self.clipboard = arboard::Clipboard::new().ok();
        }
        self.clipboard.as_mut()
    }

    fn copy_text_to_clipboard(&mut self, text: String) {
        let result = match self.ensure_clipboard() {
            Some(cb) => cb.set_text(text).map_err(|e| e.to_string()),
            None => Err("clipboard unavailable".to_string()),
        };
        self.status = match result {
            Ok(()) => "Copied to clipboard".to_string(),
            Err(e) => format!("Clipboard error: {e}"),
        };
    }

    /// Decode the currently-selected file as an MSX image, if it is one.
    fn decode_current_screen(&self) -> Option<recoil::Image> {
        let disk = self.disk.as_ref()?;
        let content = self.content.as_ref()?;
        let path = content.path.clone();
        let companions = FnCompanions(|ext: &str| disk.companion(&path, ext));
        recoil::decode(&content.path, &content.bytes, &companions)
    }

    /// Copy the current view to the clipboard: the decoded image in Screen mode,
    /// otherwise the rendered text (hex dump / text / BASIC listing).
    fn copy_current_view(&mut self) {
        if self.view_mode == ViewMode::Screen {
            let Some(img) = self.decode_current_screen() else {
                self.status = "Nothing to copy".to_string();
                return;
            };
            let image = arboard::ImageData {
                width: img.width,
                height: img.height,
                bytes: img.to_rgba().into(),
            };
            let result = match self.ensure_clipboard() {
                Some(cb) => cb.set_image(image).map_err(|e| e.to_string()),
                None => Err("clipboard unavailable".to_string()),
            };
            self.status = match result {
                Ok(()) => "Copied image to clipboard".to_string(),
                Err(e) => format!("Clipboard error: {e}"),
            };
            return;
        }

        let Some(content) = self.content.as_ref() else {
            return;
        };
        let text = match self.view_mode {
            ViewMode::Hex => dump_to_string(
                &content.bytes,
                HexConfig {
                    bytes_per_row: self.bytes_per_row,
                    base_address: 0,
                },
            ),
            ViewMode::Text => text::to_text(
                &content.bytes,
                if self.text_show_all {
                    ControlMode::ShowAll
                } else {
                    ControlMode::Dots
                },
            ),
            ViewMode::Basic => basic::detokenize(&content.bytes),
            ViewMode::Screen => unreachable!("handled above"),
        };
        self.copy_text_to_clipboard(text);
    }

    /// Save the currently-viewed MSX image as a PNG.
    fn save_screen_png(&mut self) {
        let Some(img) = self.decode_current_screen() else {
            self.status = "Not a decodable MSX image".to_string();
            return;
        };
        let default = self
            .selected
            .as_deref()
            .and_then(|p| p.rsplit('/').next())
            .and_then(|n| n.rsplit_once('.').map(|(s, _)| s).or(Some(n)))
            .map(|stem| format!("{stem}.png"))
            .unwrap_or_else(|| "image.png".to_string());
        let Some(target) = rfd::FileDialog::new().set_file_name(&default).save_file() else {
            return;
        };
        let Some(buffer) =
            image::RgbaImage::from_raw(img.width as u32, img.height as u32, img.to_rgba())
        else {
            self.status = "Image buffer error".to_string();
            return;
        };
        self.status = match buffer.save(&target) {
            Ok(()) => format!("Saved {}", target.display()),
            Err(e) => format!("Failed to save PNG: {e}"),
        };
    }

    /// "View disk by sector" — a hex view of one sector, optionally editable.
    fn sector_panel(&mut self, ui: &mut egui::Ui) {
        let Some(sector_count) = self.disk.as_ref().map(LoadedDisk::sector_count) else {
            ui.weak("No disk open.");
            return;
        };
        if sector_count == 0 {
            ui.weak("Empty disk.");
            return;
        }
        if self.current_sector >= sector_count {
            self.current_sector = sector_count - 1;
        }
        let writable = self.disk_writable();
        ui.horizontal(|ui| {
            ui.label("Sector:");
            let mut s = self.current_sector;
            if ui
                .add(egui::DragValue::new(&mut s).range(0..=sector_count - 1))
                .changed()
            {
                self.current_sector = s;
                self.sector_edit = None;
            }
            if ui.button("\u{25C0}").clicked() && self.current_sector > 0 {
                self.current_sector -= 1;
                self.sector_edit = None;
            }
            if ui.button("\u{25B6}").clicked() && self.current_sector + 1 < sector_count {
                self.current_sector += 1;
                self.sector_edit = None;
            }
            ui.label(format!(
                "/ {sector_count}    offset {:#08X}",
                self.current_sector * 512
            ));
            if self.sector_edit.is_some() {
                ui.separator();
                if ui.button("Save sector").clicked() {
                    self.save_sector_edit();
                }
                if ui.button("Cancel").clicked() {
                    self.sector_edit = None;
                }
            } else if writable && ui.button("Edit sector").clicked() {
                if let Some(b) = self
                    .disk
                    .as_ref()
                    .and_then(|d| d.sector_bytes(self.current_sector))
                {
                    self.sector_edit = Some(format_hex_for_edit(&b));
                }
            }
        });
        ui.horizontal(|ui| {
            ui.label("Find on disk:");
            let resp = ui
                .add(egui::TextEdit::singleline(&mut self.disk_search_query).desired_width(160.0));
            ui.checkbox(&mut self.disk_search_is_hex, "Hex");
            let submit = resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            if ui.button("Find").clicked() || submit {
                self.run_disk_search();
            }
            if !self.disk_search_matches.is_empty() {
                if ui.button("\u{25C0}").clicked() {
                    self.step_disk_search(false);
                }
                if ui.button("\u{25B6}").clicked() {
                    self.step_disk_search(true);
                }
                ui.label(format!(
                    "{}/{}",
                    self.disk_search_pos + 1,
                    self.disk_search_matches.len()
                ));
            }
        });
        ui.separator();

        if self.sector_edit.is_some() {
            egui::ScrollArea::vertical()
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    if let Some(text) = self.sector_edit.as_mut() {
                        ui.add(
                            egui::TextEdit::multiline(text)
                                .code_editor()
                                .desired_width(f32::INFINITY),
                        );
                    }
                });
            return;
        }
        let bytes = self
            .disk
            .as_ref()
            .and_then(|d| d.sector_bytes(self.current_sector));
        if let Some(bytes) = bytes {
            let scroll = self.sector_scroll_row.take();
            let highlight = self
                .sector_highlight
                .filter(|(s, _)| *s == self.current_sector)
                .map(|(_, r)| r);
            render_hex(ui, &bytes, 16, scroll, highlight);
        }
    }

    fn run_disk_search(&mut self) {
        let matches = if self.disk_search_is_hex {
            match search::parse_hex(&self.disk_search_query) {
                Some(needle) => self
                    .disk
                    .as_ref()
                    .map(|d| search::find_bytes(d.data(), &needle))
                    .unwrap_or_default(),
                None => {
                    self.status = "Invalid hex pattern".to_string();
                    return;
                }
            }
        } else {
            self.disk
                .as_ref()
                .map(|d| search::find_text(d.data(), &self.disk_search_query, true))
                .unwrap_or_default()
        };
        if matches.is_empty() {
            self.status = format!("No matches for \"{}\"", self.disk_search_query);
            self.disk_search_matches.clear();
            return;
        }
        self.status = format!("{} match(es) on disk", matches.len());
        self.disk_search_matches = matches;
        self.disk_search_pos = 0;
        self.jump_to_disk_match();
    }

    fn step_disk_search(&mut self, forward: bool) {
        let n = self.disk_search_matches.len();
        if n == 0 {
            return;
        }
        self.disk_search_pos = if forward {
            (self.disk_search_pos + 1) % n
        } else {
            (self.disk_search_pos + n - 1) % n
        };
        self.jump_to_disk_match();
    }

    fn jump_to_disk_match(&mut self) {
        if let Some(&off) = self.disk_search_matches.get(self.disk_search_pos) {
            let sector = off / 512;
            let row = (off % 512) / 16;
            self.current_sector = sector;
            self.sector_edit = None;
            self.sector_scroll_row = Some(row);
            self.sector_highlight = Some((sector, row));
        }
    }

    fn save_sector_edit(&mut self) {
        let Some(edited) = self.sector_edit.clone() else {
            return;
        };
        let Some(bytes) = search::parse_hex(&edited) else {
            self.status = "Invalid hex: need whole byte pairs".to_string();
            return;
        };
        if bytes.len() != 512 {
            self.status = format!("A sector is 512 bytes (got {})", bytes.len());
            return;
        }
        let idx = self.current_sector;
        let result = self.disk.as_mut().unwrap().write_sector(idx, &bytes);
        match result {
            Ok(()) => {
                self.status = format!("Saved sector {idx}");
                self.sector_edit = None;
                self.content = None;
                self.selected = None;
                self.disk_map = self.disk.as_ref().and_then(LoadedDisk::disk_map);
            }
            Err(e) => self.status = format!("Write failed: {e}"),
        }
    }

    /// Graphical disk-usage map; the selected file's sectors are outlined.
    fn map_panel(&mut self, ui: &mut egui::Ui) {
        let Some(map) = self.disk_map.as_ref() else {
            ui.weak("No disk map available.");
            return;
        };
        let file_set: std::collections::HashSet<usize> = self
            .selected
            .as_deref()
            .map(|p| {
                self.disk
                    .as_ref()
                    .map(|d| d.file_sectors(p))
                    .unwrap_or_default()
            })
            .unwrap_or_default()
            .into_iter()
            .collect();

        ui.horizontal_wrapped(|ui| {
            for (label, color) in [
                ("Reserved", kind_color(SectorKind::Reserved)),
                ("FAT", kind_color(SectorKind::Fat)),
                ("Root", kind_color(SectorKind::RootDir)),
                ("Used", kind_color(SectorKind::DataUsed)),
                ("Free", kind_color(SectorKind::DataFree)),
            ] {
                ui.colored_label(color, "\u{25A0}");
                ui.label(label);
                ui.add_space(6.0);
            }
            if !file_set.is_empty() {
                ui.separator();
                ui.label("white outline = selected file");
            }
        });
        ui.separator();

        const COLS: usize = 64;
        const CELL: f32 = 9.0;
        let count = map.sector_count;
        let rows = count.div_ceil(COLS);
        let mut clicked = None;
        egui::ScrollArea::both().show(ui, |ui| {
            let (resp, painter) = ui.allocate_painter(
                egui::vec2(COLS as f32 * CELL, rows as f32 * CELL),
                egui::Sense::click(),
            );
            let origin = resp.rect.min;
            for i in 0..count {
                let pos = origin + egui::vec2((i % COLS) as f32 * CELL, (i / COLS) as f32 * CELL);
                let rect = egui::Rect::from_min_size(pos, egui::vec2(CELL - 1.0, CELL - 1.0));
                painter.rect_filled(rect, 0.0, kind_color(map.kinds[i]));
                if file_set.contains(&i) {
                    painter.rect_stroke(
                        rect,
                        0.0,
                        egui::Stroke::new(1.5, egui::Color32::WHITE),
                        egui::StrokeKind::Inside,
                    );
                }
            }
            if resp.clicked() {
                if let Some(p) = resp.interact_pointer_pos() {
                    let rel = p - origin;
                    if rel.x >= 0.0 && rel.y >= 0.0 {
                        let idx = (rel.y / CELL) as usize * COLS + (rel.x / CELL) as usize;
                        if idx < count {
                            clicked = Some(idx);
                        }
                    }
                }
            }
        });
        if let Some(idx) = clicked {
            self.current_sector = idx;
            self.sector_edit = None;
            self.app_view = AppView::Sectors;
        }
    }
}

/// Colour for a sector-usage category in the disk map.
fn kind_color(kind: SectorKind) -> egui::Color32 {
    match kind {
        SectorKind::Reserved => egui::Color32::from_gray(110),
        SectorKind::Fat => egui::Color32::from_rgb(80, 120, 200),
        SectorKind::RootDir => egui::Color32::from_rgb(150, 90, 180),
        SectorKind::DataUsed => egui::Color32::from_rgb(70, 160, 90),
        SectorKind::DataFree => egui::Color32::from_gray(45),
    }
}

/// Render a DOS attribute set as a fixed 4-character `RHSA` field, using `-`
/// for each absent flag (e.g. read-only + archive -> `R--A`).
fn format_attributes(attrs: msx_disk::fs::Attributes) -> String {
    let flag = |on: bool, c: char| if on { c } else { '-' };
    [
        flag(attrs.read_only, 'R'),
        flag(attrs.hidden, 'H'),
        flag(attrs.system, 'S'),
        flag(attrs.archive, 'A'),
    ]
    .into_iter()
    .collect()
}

/// Render a timestamp as `YYYY-MM-DD HH:MM`, or an empty string when absent.
fn format_timestamp(ts: Option<msx_disk::fs::Timestamp>) -> String {
    match ts {
        Some(t) => format!(
            "{:04}-{:02}-{:02} {:02}:{:02}",
            t.year, t.month, t.day, t.hour, t.minute
        ),
        None => String::new(),
    }
}

/// Build the label for a file row: name + size, plus date/time and attribute
/// columns when `show_meta` is set (MSX-DOS 2 disks).
fn format_file_row(entry: &DirEntry, show_meta: bool) -> String {
    let base = format!("{:<14} {:>8}", entry.name, entry.size);
    if show_meta {
        format!(
            "{base}  {:<16}  {}",
            format_timestamp(entry.modified),
            format_attributes(entry.attributes)
        )
    } else {
        base
    }
}

/// Recursively render the directory tree, recording a clicked file path.
fn render_entries(
    ui: &mut egui::Ui,
    entries: &[DirEntry],
    selection: &BTreeSet<String>,
    show_meta: bool,
    clicked: &mut Option<(String, bool)>,
) {
    for entry in entries {
        if entry.is_dir {
            // Plain header on DOS1; monospace with aligned date/attr columns on DOS2.
            let header: egui::WidgetText = if show_meta {
                egui::RichText::new(format!(
                    "\u{1F4C1} {:<12}  {:<16}  {}",
                    entry.name,
                    format_timestamp(entry.modified),
                    format_attributes(entry.attributes)
                ))
                .monospace()
                .into()
            } else {
                format!("\u{1F4C1} {}", entry.name).into()
            };
            egui::CollapsingHeader::new(header)
                .default_open(true)
                .show(ui, |ui| {
                    render_entries(ui, &entry.children, selection, show_meta, clicked);
                });
        } else {
            let is_selected = selection.contains(&entry.path);
            let label = format_file_row(entry, show_meta);
            if ui
                .selectable_label(is_selected, egui::RichText::new(label).monospace())
                .clicked()
            {
                let toggle = ui.input(|i| i.modifiers.command);
                *clicked = Some((entry.path.clone(), toggle));
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
/// The final path component (file name) of a slash-separated disk path.
fn base_name(path: &str) -> String {
    path.rsplit('/').next().unwrap_or("file").to_string()
}

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

/// Extensions that default to the Text view.
const TEXT_EXTENSIONS: &[&str] = &[
    "txt", "bat", "asc", "doc", "me", "ini", "cfg", "diz", "nfo", "log", "csv", "md",
];

/// Pick a sensible default view mode for a file based on its extension.
fn default_view_mode(path: &str) -> ViewMode {
    let ext = path.rsplit('.').next().unwrap_or("").to_ascii_lowercase();
    if recoil::is_supported(path) {
        ViewMode::Screen
    } else if ext == "bas" {
        ViewMode::Basic
    } else if TEXT_EXTENSIONS.contains(&ext.as_str()) {
        ViewMode::Text
    } else {
        ViewMode::Hex
    }
}

/// Render a decoded MSX graphics image, caching the GPU texture by file path.
fn render_screen(
    ui: &mut egui::Ui,
    cache: &mut Option<(String, egui::TextureHandle)>,
    path: &str,
    bytes: &[u8],
    companions: &dyn recoil::CompanionFiles,
) {
    let stale = cache.as_ref().map(|(p, _)| p != path).unwrap_or(true);
    if stale {
        let Some(img) = recoil::decode(path, bytes, companions) else {
            *cache = None;
            ui.weak("Not a recognized / decodable MSX graphics file.");
            return;
        };
        let image =
            egui::ColorImage::from_rgba_unmultiplied([img.width, img.height], &img.to_rgba());
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
        egui::CentralPanel::default().show_inside(ui, |ui| match self.app_view {
            AppView::Files => self.viewer_panel(ui),
            AppView::Sectors => self.sector_panel(ui),
            AppView::Map => self.map_panel(ui),
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
        assert_eq!(default_view_mode("README.TXT"), ViewMode::Text);
        assert_eq!(default_view_mode("AUTOEXEC.BAT"), ViewMode::Text);
        assert_eq!(default_view_mode("notes.txt"), ViewMode::Text);
    }

    fn file_entry(
        name: &str,
        size: u64,
        attributes: msx_disk::fs::Attributes,
        modified: Option<msx_disk::fs::Timestamp>,
    ) -> DirEntry {
        DirEntry {
            name: name.to_string(),
            path: name.to_string(),
            is_dir: false,
            size,
            attributes,
            modified,
            children: Vec::new(),
        }
    }

    #[test]
    fn attributes_render_as_rhsa_flags() {
        use msx_disk::fs::Attributes;
        assert_eq!(format_attributes(Attributes::default()), "----");
        assert_eq!(
            format_attributes(Attributes {
                read_only: true,
                archive: true,
                ..Attributes::default()
            }),
            "R--A"
        );
        assert_eq!(
            format_attributes(Attributes {
                read_only: true,
                hidden: true,
                system: true,
                archive: true,
            }),
            "RHSA"
        );
    }

    #[test]
    fn timestamp_formats_iso_minute_or_empty() {
        use msx_disk::fs::Timestamp;
        assert_eq!(
            format_timestamp(Some(Timestamp {
                year: 1991,
                month: 3,
                day: 25,
                hour: 14,
                minute: 30,
            })),
            "1991-03-25 14:30"
        );
        assert_eq!(format_timestamp(None), "");
    }

    #[test]
    fn file_row_omits_metadata_for_dos1() {
        let e = file_entry("GAME.COM", 1234, msx_disk::fs::Attributes::default(), None);
        assert_eq!(
            format_file_row(&e, false),
            format!("{:<14} {:>8}", "GAME.COM", 1234u64)
        );
    }

    #[test]
    fn file_row_appends_metadata_for_dos2() {
        use msx_disk::fs::{Attributes, Timestamp};
        let e = file_entry(
            "GAME.COM",
            1234,
            Attributes {
                archive: true,
                ..Attributes::default()
            },
            Some(Timestamp {
                year: 1991,
                month: 3,
                day: 25,
                hour: 14,
                minute: 30,
            }),
        );
        let row = format_file_row(&e, true);
        assert!(row.contains("1991-03-25 14:30"), "row: {row}");
        assert!(row.trim_end().ends_with("---A"), "row: {row}");
    }

    #[test]
    fn disk_search_jump_maps_offset_to_sector_and_row() {
        // Offset 1234 -> sector 2 (1024..1536), row (1234 % 512) / 16 = 210/16 = 13.
        let mut app = DskExplorerApp {
            disk_search_matches: vec![1234],
            ..Default::default()
        };
        app.jump_to_disk_match();
        assert_eq!(app.current_sector, 2);
        assert_eq!(app.sector_scroll_row, Some(13));
        assert_eq!(app.sector_highlight, Some((2, 13)));
    }

    #[test]
    fn disk_search_step_wraps_in_both_directions() {
        let mut app = DskExplorerApp {
            disk_search_matches: vec![0, 512, 1024],
            ..Default::default()
        };
        app.step_disk_search(true);
        assert_eq!(app.disk_search_pos, 1);
        app.step_disk_search(false);
        assert_eq!(app.disk_search_pos, 0);
        // Wrap backwards from first to last.
        app.step_disk_search(false);
        assert_eq!(app.disk_search_pos, 2);
        // Wrap forwards from last to first.
        app.step_disk_search(true);
        assert_eq!(app.disk_search_pos, 0);
    }

    #[test]
    fn disk_search_step_is_noop_without_matches() {
        let mut app = DskExplorerApp::default();
        app.step_disk_search(true);
        assert_eq!(app.disk_search_pos, 0);
    }

    #[test]
    fn base_name_takes_last_path_component() {
        assert_eq!(base_name("A/B/C.BIN"), "C.BIN");
        assert_eq!(base_name("ROOT.COM"), "ROOT.COM");
        assert_eq!(base_name("DIR/SUB/"), "");
    }

    #[test]
    fn selection_paths_prefers_marked_set_in_sorted_order() {
        let app = DskExplorerApp {
            selection: BTreeSet::from(["B.TXT".to_string(), "A.TXT".to_string()]),
            selected: Some("C.TXT".to_string()),
            ..Default::default()
        };
        assert_eq!(app.selection_paths(), vec!["A.TXT", "B.TXT"]);
    }

    #[test]
    fn selection_paths_falls_back_to_viewed_file() {
        let app = DskExplorerApp {
            selected: Some("ONLY.TXT".to_string()),
            ..Default::default()
        };
        assert_eq!(app.selection_paths(), vec!["ONLY.TXT"]);

        let empty = DskExplorerApp::default();
        assert!(empty.selection_paths().is_empty());
    }
}
