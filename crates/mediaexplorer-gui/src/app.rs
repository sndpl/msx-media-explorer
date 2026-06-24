//! Top-level application state and the `eframe::App` implementation.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use msx_disk::fs::map::SectorKind;
use msx_disk::image::dmk::{self, Density, DmkAnalysis, DmkTrackInfo};
use msx_disk::image::geometry::{Geometry, SECTOR_SIZE};
use msx_disk::recoil::{self, FnCompanions};
use msx_disk::search;
use msx_disk::tape::TapeBlock;
use msx_disk::view::basic;
use msx_disk::view::disasm;
use msx_disk::view::hex::{ascii_char, dump_to_string, HexConfig};
use msx_disk::view::text::{self, ControlMode};
use msx_disk::{charset, DirEntry, ImageFormat, MsxCharset};

use crate::state::{humanize_bytes, LoadedDisk, LoadedTape};

/// Register GNU Unifont as a fallback font so decoded MSX glyphs (kana, accented
/// Latin, box-drawing, ...) render instead of tofu. egui's built-in fonts cover
/// only Latin; Unifont is appended *after* them in each family so ASCII keeps
/// the crisp default and only otherwise-missing glyphs fall through to it.
pub(crate) fn install_fonts(ctx: &egui::Context) {
    use std::sync::Arc;
    let mut fonts = egui::FontDefinitions::default();
    fonts.font_data.insert(
        "unifont".to_owned(),
        Arc::new(egui::FontData::from_static(include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/assets/fonts/unifont-msx.otf"
        )))),
    );
    for family in [egui::FontFamily::Monospace, egui::FontFamily::Proportional] {
        fonts
            .families
            .entry(family)
            .or_default()
            .push("unifont".to_owned());
    }
    ctx.set_fonts(fonts);
}

/// How the selected file's contents are shown.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ViewMode {
    Info,
    Hex,
    Text,
    Basic,
    /// Z80/R800 disassembly of machine-code files.
    Disasm,
    Screen,
    /// Contents of a selected `.lzh`/`.lzs`/`.pma` archive file.
    Archive,
}

/// Top-level view. Which tabs are offered depends on the open document:
/// `Files` always; `Sectors`/`Map` for disks; `Analyze` for `.dmk`; `Blocks`
/// for tapes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AppView {
    Files,
    Sectors,
    Map,
    Analyze,
    Blocks,
}

/// The largest amount of a file rendered in the text view at once.
const MAX_TEXT_BYTES: usize = 128 * 1024;

/// Extensions recognized as openable disk images (for the Open dialog filter
/// and to decide whether a dropped file should open vs. be added to the disk).
const DISK_IMAGE_EXTS: &[&str] = &[
    "dsk", "di1", "ds1", "di2", "ds2", "img", "msx", "ddi", "xsa", "dmk",
];

/// Extensions recognized as openable tape images.
const TAPE_EXTS: &[&str] = &["cas", "tsx"];

/// Cached contents of the selected file (full bytes; views render lazily).
struct FileContent {
    path: String,
    bytes: Vec<u8>,
}

/// Cached member listing for the selected archive file, computed once when the
/// file is selected (not per frame). `result` holds the members or a message
/// explaining why the archive could not be read.
struct ArchiveListing {
    result: Result<Vec<msx_disk::ArchiveEntry>, String>,
    /// The member row the user has highlighted, for the right-click menu.
    selected_member: Option<usize>,
}

/// A deferred action chosen in the archive pane, applied after the render
/// borrow of `self.archive` is released.
enum ArchiveAction {
    ExtractOne(usize),
    ExtractAll,
}

/// A file action chosen from a tree row's right-click context menu. Carries the
/// path of the row that was clicked; `tree_panel` resolves it against the
/// current selection before acting.
enum RowAction {
    Rename(String),
    Delete(String),
    Extract(String),
}

/// What the user did to a tree row this frame, collected by the row renderers
/// and applied back in `tree_panel` (the renderers only hold `&self` state).
#[derive(Default)]
struct RowEvents {
    /// A row was clicked to view it; the bool toggles multi-select (Cmd/Ctrl).
    clicked: Option<(String, bool)>,
    /// A row began an OS drag-out (the dragged row's path).
    drag_started: Option<String>,
    /// A context-menu action was chosen on a row.
    action: Option<RowAction>,
}

/// A rename in progress: the file being renamed, the new name being typed (as a
/// decoded display name), and the charset snapshot used to decode it on open and
/// re-encode it on save (so a live charset change can't corrupt the key).
struct RenameTarget {
    path: String,
    name: String,
    charset: MsxCharset,
}

/// Root application state.
pub struct MediaExplorerApp {
    disk: Option<LoadedDisk>,
    /// Open tape image, when a `.cas`/`.tsx` is loaded instead of a disk.
    tape: Option<LoadedTape>,
    /// Per-track analysis for an open `.dmk` disk.
    dmk_analysis: Option<DmkAnalysis>,
    status: String,
    /// The file currently shown in the viewer (the last one clicked).
    selected: Option<String>,
    /// All files marked for batch extract/delete (Cmd/Ctrl-click to toggle).
    selection: BTreeSet<String>,
    content: Option<FileContent>,
    /// Parsed member list for the selected archive file, when one is selected.
    archive: Option<ArchiveListing>,
    view_mode: ViewMode,
    bytes_per_row: usize,
    text_show_all: bool,
    /// Cached screen texture, keyed by the file path and forced format it was
    /// rendered from (so changing either invalidates it).
    screen_tex: Option<(String, Option<recoil::ImageFormat>, egui::TextureHandle)>,
    /// Manually-chosen graphics format that overrides extension-based decoding
    /// in the Screen view. `None` means decode by extension. Reset per file.
    forced_format: Option<recoil::ImageFormat>,
    search_query: String,
    search_is_hex: bool,
    /// Byte offsets of matches in the current file, with the active index.
    search_matches: Vec<usize>,
    search_pos: usize,
    /// One-shot request to scroll the hex view to a row.
    pending_scroll_row: Option<usize>,
    /// Rename in progress: the target file and the new name being typed.
    rename_target: Option<RenameTarget>,
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
    /// Cached filesystem geometry (clusters/sectors) for the status bar.
    disk_fs_geometry: Option<msx_disk::fs::map::FsGeometry>,
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
    /// Disk paths awaiting an OS drag-out, started once the window handle is
    /// available at the end of the frame.
    #[cfg(any(target_os = "macos", target_os = "windows"))]
    pending_drag_out: Option<Vec<String>>,
    /// MSX character set used to decode filenames and file text for display.
    charset: MsxCharset,
    /// When true, [`charset`] follows auto-detection on each disk load; a manual
    /// dropdown choice pins it (sets this false).
    charset_auto: bool,
}

/// Largest file (bytes) offered for in-app hex editing, to keep the editor
/// responsive.
const MAX_HEX_EDIT_BYTES: usize = 32 * 1024;

impl Default for MediaExplorerApp {
    fn default() -> Self {
        MediaExplorerApp {
            disk: None,
            tape: None,
            dmk_analysis: None,
            status: "Open a disk image (or drag one in) to get started.".to_string(),
            selected: None,
            selection: BTreeSet::new(),
            content: None,
            archive: None,
            view_mode: ViewMode::Hex,
            bytes_per_row: 16,
            text_show_all: false,
            screen_tex: None,
            forced_format: None,
            search_query: String::new(),
            search_is_hex: false,
            search_matches: Vec::new(),
            search_pos: 0,
            pending_scroll_row: None,
            rename_target: None,
            confirm_delete: None,
            hex_edit: None,
            app_view: AppView::Files,
            current_sector: 0,
            sector_edit: None,
            disk_map: None,
            disk_fs_geometry: None,
            clipboard: None,
            disk_search_query: String::new(),
            disk_search_is_hex: false,
            disk_search_matches: Vec::new(),
            disk_search_pos: 0,
            sector_scroll_row: None,
            sector_highlight: None,
            #[cfg(any(target_os = "macos", target_os = "windows"))]
            pending_drag_out: None,
            charset: MsxCharset::default(),
            charset_auto: true,
        }
    }
}

impl MediaExplorerApp {
    fn open_path(&mut self, path: &Path) {
        if is_tape(path) {
            self.open_tape(path);
        } else {
            self.open_disk(path);
        }
    }

    /// Reset the per-document state shared by disk and tape opens.
    fn reset_document(&mut self) {
        self.disk = None;
        self.tape = None;
        self.dmk_analysis = None;
        self.disk_map = None;
        self.disk_fs_geometry = None;
        self.selected = None;
        self.selection.clear();
        self.content = None;
        self.archive = None;
        self.current_sector = 0;
        self.sector_edit = None;
        self.app_view = AppView::Files;
    }

    fn open_disk(&mut self, path: &Path) {
        match LoadedDisk::open(path) {
            Ok(disk) => {
                self.reset_document();
                self.status = format!(
                    "Opened {} ({:?}, {} sectors)",
                    disk.title(),
                    disk.format,
                    disk.geometry.total_sectors()
                );
                // A .dmk gets a per-track analysis from the raw container bytes.
                if disk.format == ImageFormat::Dmk {
                    self.dmk_analysis =
                        std::fs::read(path).ok().and_then(|b| dmk::analyze(&b).ok());
                }
                self.disk = Some(disk);
                self.disk_map = self.disk.as_ref().and_then(LoadedDisk::disk_map);
                self.disk_fs_geometry = self.disk.as_ref().and_then(LoadedDisk::fs_geometry);
                self.autodetect_charset();
            }
            Err(e) => self.status = format!("Failed to open {}: {e}", path.display()),
        }
    }

    /// Seed the display charset from the open disk's filenames, unless the user
    /// has pinned it via the dropdown. Best-effort; the dropdown always wins.
    fn autodetect_charset(&mut self) {
        if !self.charset_auto {
            return;
        }
        if let Some(disk) = self.disk.as_ref() {
            let sample: Vec<u8> = disk
                .tree
                .iter()
                .flat_map(DirEntry::walk)
                .flat_map(|e| e.name.chars())
                .filter_map(charset::fs_name_byte)
                .collect();
            self.charset = charset::detect(&sample);
        }
    }

    fn open_tape(&mut self, path: &Path) {
        match LoadedTape::open(path) {
            Ok(tape) => {
                self.reset_document();
                self.status = format!(
                    "Opened {} ({} — {} file(s))",
                    tape.title(),
                    tape.format.label(),
                    tape.file_count()
                );
                self.tape = Some(tape);
            }
            Err(e) => self.status = format!("Failed to open {}: {e}", path.display()),
        }
    }

    /// Read a file's bytes from whichever document is open (disk or tape).
    fn read_doc_file(&self, path: &str) -> Option<Vec<u8>> {
        if let Some(disk) = &self.disk {
            return disk.read_file(path).ok();
        }
        if let Some(tape) = &self.tape {
            return tape.read_file(path);
        }
        None
    }

    /// Open a file in the viewer. With `toggle` (Cmd/Ctrl-click), add or remove
    /// it from the multi-selection; otherwise it becomes the sole selection.
    fn select_file(&mut self, path: String, toggle: bool) {
        let Some(bytes) = self.read_doc_file(&path) else {
            self.status = format!("Cannot read {path}");
            return;
        };
        self.status = format!("{path} — {} bytes", bytes.len());
        self.view_mode = default_view_mode(&path);
        self.forced_format = None;
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
        // Parse an archive's member list once, here, so the pane never re-parses
        // per frame. Non-archive files clear any previous listing.
        self.archive = if msx_disk::archive::is_archive(&path) {
            Some(ArchiveListing {
                result: msx_disk::archive::list(&bytes).map_err(|e| e.to_string()),
                selected_member: None,
            })
        } else {
            None
        };
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

    /// Files a row-level action targets: the whole multi-selection when the
    /// clicked row is part of it, otherwise just that row. Shared by the
    /// right-click menu and OS drag-out so both follow the same rule.
    fn paths_for_row(&self, path: &str) -> Vec<String> {
        if self.selection.len() > 1 && self.selection.contains(path) {
            self.selection_paths()
        } else {
            vec![path.to_string()]
        }
    }

    fn handle_dropped_files(&mut self, ctx: &egui::Context) {
        let paths: Vec<PathBuf> = ctx.input(|i| {
            i.raw
                .dropped_files
                .iter()
                .filter_map(|f| f.path.clone())
                .collect()
        });
        if paths.is_empty() {
            return;
        }
        // A single disk/tape image opens (replacing the current document);
        // anything dropped onto an open, writable disk is added to it;
        // otherwise fall back to trying to open the first dropped path.
        if let [only] = paths.as_slice() {
            if is_openable(only) {
                self.open_path(only);
                return;
            }
        }
        if self.disk_writable() {
            self.add_paths(&paths);
        } else {
            self.open_path(&paths[0]);
        }
    }

    fn toolbar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            if ui.button("Open…").clicked() {
                if let Some(path) = rfd::FileDialog::new()
                    .add_filter("MSX disk images", DISK_IMAGE_EXTS)
                    .add_filter("MSX tape images", TAPE_EXTS)
                    .pick_file()
                {
                    self.open_path(&path);
                }
            }
            ui.menu_button("New…", |ui| {
                if ui.button("720 kB (double-sided)").clicked() {
                    self.new_disk(true);
                    ui.close();
                }
                if ui.button("360 kB (single-sided)").clicked() {
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
            } else if let Some(tape) = &self.tape {
                ui.separator();
                ui.label(tape.title());
                ui.separator();
                ui.weak("read-only");
            }
        });
        if self.disk.is_some() || self.tape.is_some() {
            ui.horizontal(|ui| {
                ui.selectable_value(&mut self.app_view, AppView::Files, "Files");
                if let Some(disk) = &self.disk {
                    ui.selectable_value(&mut self.app_view, AppView::Sectors, "Sectors");
                    // The disk-usage Map is FAT12-only and whole-disk; it is
                    // disabled for partitioned hard-disk images.
                    if !disk.is_partitioned() {
                        ui.selectable_value(&mut self.app_view, AppView::Map, "Map");
                    }
                }
                if self.dmk_analysis.is_some() {
                    ui.selectable_value(&mut self.app_view, AppView::Analyze, "Analyze");
                }
                if self.tape.is_some() {
                    ui.selectable_value(&mut self.app_view, AppView::Blocks, "Blocks");
                }
                // Character-set selector: seeded by auto-detect on load, but the
                // user can pin a different MSX code page here (re-decodes live).
                if self.disk.is_some() {
                    ui.separator();
                    ui.label("Charset:");
                    let prev = self.charset;
                    egui::ComboBox::from_id_salt("charset")
                        .selected_text(self.charset.label())
                        .show_ui(ui, |ui| {
                            for &cs in MsxCharset::ALL {
                                ui.selectable_value(&mut self.charset, cs, cs.label());
                            }
                        });
                    if self.charset != prev {
                        self.charset_auto = false; // a manual choice pins the charset
                    }
                }
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
        let writable = self.disk_writable();
        let charset = self.charset;
        let mut events = RowEvents::default();
        if let Some(disk) = &self.disk {
            if disk.tree.is_empty() {
                ui.weak(empty_fat_message());
            } else {
                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        render_entries(
                            ui,
                            &disk.tree,
                            &self.selection,
                            writable,
                            charset,
                            &mut events,
                        );
                    });
            }
        } else if let Some(tape) = &self.tape {
            egui::ScrollArea::vertical()
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    render_tape_files(ui, tape, &self.selection, &mut events);
                });
        } else {
            ui.weak("No disk open.");
        }
        if let Some((path, toggle)) = events.clicked {
            self.select_file(path, toggle);
        }
        if let Some(action) = events.action {
            match action {
                RowAction::Rename(path) => {
                    // Edit the decoded display name; re-encoded on save.
                    let name = charset::decode_fs_name(self.charset, &base_name(&path));
                    self.rename_target = Some(RenameTarget {
                        path,
                        name,
                        charset: self.charset,
                    });
                }
                RowAction::Delete(path) => {
                    self.confirm_delete = Some(self.paths_for_row(&path));
                }
                RowAction::Extract(path) => {
                    let paths = self.paths_for_row(&path);
                    self.extract_paths(&paths);
                }
            }
        }
        // On macOS/Windows, a dragged row hands its file(s) to the OS drag.
        #[cfg(any(target_os = "macos", target_os = "windows"))]
        if let Some(path) = events.drag_started {
            self.pending_drag_out = Some(self.paths_for_row(&path));
        }
        #[cfg(not(any(target_os = "macos", target_os = "windows")))]
        let _ = &events.drag_started;
    }

    /// The directory entry for the currently selected file, when it lives on a
    /// disk (tape files have no `DirEntry`, so this returns `None`).
    fn selected_entry(&self) -> Option<&DirEntry> {
        let sel = self.selected.as_deref()?;
        let disk = self.disk.as_ref()?;
        disk.tree
            .iter()
            .flat_map(DirEntry::walk)
            .find(|e| e.path == sel)
    }

    fn viewer_panel(&mut self, ui: &mut egui::Ui) {
        let writable = self.disk_writable();
        ui.horizontal(|ui| {
            ui.selectable_value(&mut self.view_mode, ViewMode::Info, "Info");
            ui.selectable_value(&mut self.view_mode, ViewMode::Hex, "Hex");
            ui.selectable_value(&mut self.view_mode, ViewMode::Text, "Text");
            ui.selectable_value(&mut self.view_mode, ViewMode::Basic, "BASIC");
            ui.selectable_value(&mut self.view_mode, ViewMode::Disasm, "Disasm");
            ui.selectable_value(&mut self.view_mode, ViewMode::Screen, "Screen");
            if self.archive.is_some() {
                ui.selectable_value(&mut self.view_mode, ViewMode::Archive, "Archive");
            }
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
                ViewMode::Basic
                | ViewMode::Disasm
                | ViewMode::Screen
                | ViewMode::Info
                | ViewMode::Archive => {}
            }
            if self.content.is_some() && self.view_mode != ViewMode::Archive {
                ui.separator();
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
        });

        if self.content.is_some() && self.view_mode != ViewMode::Archive {
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

        if self.view_mode == ViewMode::Archive {
            // The archive pane needs `&mut self` (to update the highlighted
            // member and to trigger extraction), so handle it outside the
            // shared `self.content` borrow below.
            self.archive_panel(ui);
            return;
        }

        if self.view_mode == ViewMode::Screen {
            // The screen view needs the mutable texture cache, so handle it
            // outside the shared borrow of `self.content`.
            let mut cache = self.screen_tex.take();
            let mut show_picker = false;
            match (self.disk.as_ref(), self.content.as_ref()) {
                (Some(disk), Some(content)) => {
                    let path = content.path.clone();
                    let companions = FnCompanions(|ext: &str| disk.companion(&path, ext));
                    let shown = render_screen(
                        ui,
                        &mut cache,
                        &content.path,
                        &content.bytes,
                        self.forced_format,
                        &companions,
                    );
                    // Fallback-only: offer the format picker when decoding fails,
                    // and keep it visible while a manual format is active so a
                    // wrong guess can be corrected.
                    show_picker = !shown || self.forced_format.is_some();
                }
                _ => {
                    ui.weak("Select a file to view its contents.");
                }
            }
            self.screen_tex = cache;
            if show_picker {
                self.screen_format_picker(ui);
            }
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
                ViewMode::Info => render_info(
                    ui,
                    &content.path,
                    &content.bytes,
                    self.selected_entry(),
                    self.charset,
                ),
                ViewMode::Hex => render_hex(
                    ui,
                    &content.bytes,
                    self.bytes_per_row,
                    scroll_to,
                    highlight,
                    self.charset,
                ),
                ViewMode::Text => render_text(ui, &content.bytes, self.text_show_all, self.charset),
                ViewMode::Basic => render_basic(ui, &content.bytes, self.charset),
                ViewMode::Disasm => render_disasm(ui, &content.path, &content.bytes),
                ViewMode::Screen | ViewMode::Archive => unreachable!("handled above"),
            },
        }
    }

    /// Render the contents of the selected archive: a member list with a
    /// right-click "Extract…" per row and an "Extract all…" button. Reads from
    /// the cached `self.archive` listing; never re-parses here.
    fn archive_panel(&mut self, ui: &mut egui::Ui) {
        let mut action: Option<ArchiveAction> = None;
        let mut new_selection: Option<usize> = None;

        match self.archive.as_ref() {
            None => {
                ui.weak("Not an archive.");
            }
            Some(listing) => match &listing.result {
                Err(msg) => {
                    ui.colored_label(
                        egui::Color32::LIGHT_RED,
                        format!("Cannot read archive: {msg}"),
                    );
                }
                Ok(members) if members.is_empty() => {
                    ui.weak("Archive is empty.");
                }
                Ok(members) => {
                    ui.horizontal(|ui| {
                        let any = members.iter().any(|m| m.decodable && !m.is_directory);
                        if ui
                            .add_enabled(any, egui::Button::new("Extract all…"))
                            .clicked()
                        {
                            action = Some(ArchiveAction::ExtractAll);
                        }
                        ui.weak(format!("{} member(s)", members.len()));
                    });
                    ui.separator();
                    ui.monospace(format!(
                        "{:<28} {:>9} {:>8} {:<5} {}",
                        "Name", "Size", "Packed", "Meth", "Modified"
                    ));
                    egui::ScrollArea::vertical()
                        .auto_shrink([false, false])
                        .show(ui, |ui| {
                            for (i, m) in members.iter().enumerate() {
                                let is_sel = listing.selected_member == Some(i);
                                let marker = if m.is_directory {
                                    "  <dir>"
                                } else if !m.decodable {
                                    "  <unsupported>"
                                } else {
                                    ""
                                };
                                let label = format!(
                                    "{:<28} {:>9} {:>8} {:<5} {:<16}{}",
                                    truncate(&m.path, 28),
                                    m.original_size,
                                    m.compressed_size,
                                    m.method.label(),
                                    format_timestamp(m.modified),
                                    marker
                                );
                                let resp = ui.selectable_label(
                                    is_sel,
                                    egui::RichText::new(label).monospace(),
                                );
                                if resp.clicked() {
                                    new_selection = Some(i);
                                }
                                let extractable = m.decodable && !m.is_directory;
                                resp.context_menu(|ui| {
                                    if ui
                                        .add_enabled(extractable, egui::Button::new("Extract…"))
                                        .clicked()
                                    {
                                        action = Some(ArchiveAction::ExtractOne(i));
                                        ui.close();
                                    }
                                });
                            }
                        });
                }
            },
        }

        if let Some(i) = new_selection {
            if let Some(listing) = self.archive.as_mut() {
                listing.selected_member = Some(i);
            }
        }
        match action {
            Some(ArchiveAction::ExtractOne(i)) => self.extract_archive_member(i),
            Some(ArchiveAction::ExtractAll) => self.extract_archive_all(),
            None => {}
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
                self.disk_fs_geometry = self.disk.as_ref().and_then(LoadedDisk::fs_geometry);
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
        if let Some(paths) = rfd::FileDialog::new().pick_files() {
            self.add_paths(&paths);
        }
    }

    /// Read each path from the filesystem and add it to the open disk under a
    /// sanitized 8.3 name. Used by both the Add dialog and drag-in.
    fn add_paths(&mut self, paths: &[PathBuf]) {
        if !self.disk_writable() {
            self.status = "This image is read-only (.xsa or no source file).".to_string();
            return;
        }
        let mut files = Vec::new();
        for path in paths {
            match std::fs::read(path) {
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
        let Some(target) = self.rename_target.take() else {
            return;
        };
        // Sanitize the decoded name, then re-encode it to the PUA fatfs key that
        // `rename` expects; the status message keeps the readable form.
        let display = sanitize_8_3(&target.name, |c| is_rename_char(c, target.charset));
        let new_base = charset::encode_fs_name(target.charset, &display)
            .unwrap_or_else(|| display.clone());
        let new_path = match target.path.rsplit_once('/') {
            Some((parent, _)) => format!("{parent}/{new_base}"),
            None => new_base,
        };
        let result = self.disk.as_mut().unwrap().rename(&target.path, &new_path);
        self.after_mutation(result, format!("Renamed to {display}"));
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
        // Decode names for display only; the stored `paths` (PUA-encoded) stay
        // the keys used for the actual delete.
        let charset = self.charset;
        egui::Window::new("Confirm delete")
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                if let [path] = paths.as_slice() {
                    let shown = charset::decode_fs_name(charset, path);
                    ui.label(format!("Delete \"{shown}\" from the disk?"));
                } else {
                    ui.label(format!("Delete these {} files from the disk?", paths.len()));
                    egui::ScrollArea::vertical()
                        .max_height(160.0)
                        .show(ui, |ui| {
                            for path in &paths {
                                ui.monospace(charset::decode_fs_name(charset, path));
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

    /// Render the rename modal if a rename is pending (triggered from the
    /// tree's right-click menu).
    fn rename_dialog(&mut self, ctx: &egui::Context) {
        let Some(target) = self.rename_target.as_mut() else {
            return;
        };
        let charset = target.charset;
        let old = charset::decode_fs_name(charset, &base_name(&target.path));
        let mut apply = false;
        let mut cancel = false;
        egui::Window::new("Rename file")
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                ui.label(format!("Rename \"{old}\" to:"));
                let resp =
                    ui.add(egui::TextEdit::singleline(&mut target.name).desired_width(160.0));
                // Restrict to a valid 8.3 name (charset-aware) as the user types.
                if resp.changed() {
                    target.name = normalize_msx_input(&target.name, charset);
                }
                // Focus the field the first frame the modal appears.
                if ui.memory(|m| m.focused().is_none()) {
                    resp.request_focus();
                }
                ui.small("8-character name, optional 3-character extension.");
                let valid = !msx_name_stem(&target.name).is_empty();
                let enter =
                    valid && resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    apply = ui.add_enabled(valid, egui::Button::new("Rename")).clicked();
                    cancel = ui.button("Cancel").clicked();
                });
                apply |= enter;
            });
        if apply {
            self.apply_rename();
        } else if cancel {
            self.rename_target = None;
        }
    }

    /// Extract the given files to the host: a save-as dialog for one, a folder
    /// picker for several.
    fn extract_paths(&mut self, paths: &[String]) {
        match paths {
            [] => {}
            [path] => self.extract_one(path),
            many => self.extract_many(many),
        }
    }

    /// Extract a single file via a save-as dialog (lets the user rename it).
    fn extract_one(&mut self, path: &str) {
        let Some(bytes) = self.read_doc_file(path) else {
            self.status = format!("Cannot read {path}");
            return;
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
            match self.read_doc_file(path) {
                Some(bytes) if std::fs::write(dir.join(base_name(path)), &bytes).is_ok() => {
                    ok += 1;
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

    /// Extract one archive member (decompressed) via a save-as dialog.
    fn extract_archive_member(&mut self, index: usize) {
        let (default_name, extracted) = {
            let Some(content) = self.content.as_ref() else {
                return;
            };
            let Some(listing) = self.archive.as_ref() else {
                return;
            };
            let Ok(members) = &listing.result else {
                return;
            };
            let Some(member) = members.get(index) else {
                return;
            };
            (
                base_name(&member.path),
                msx_disk::archive::extract(&content.bytes, index),
            )
        };
        match extracted {
            Ok(data) => {
                if let Some(target) = rfd::FileDialog::new()
                    .set_file_name(&default_name)
                    .save_file()
                {
                    let crc_warn = if self
                        .content
                        .as_ref()
                        .and_then(|c| msx_disk::archive::crc_ok(&c.bytes, index))
                        == Some(false)
                    {
                        " (warning: CRC mismatch)"
                    } else {
                        ""
                    };
                    self.status = match std::fs::write(&target, &data) {
                        Ok(()) => format!(
                            "Extracted {} ({} bytes){} to {}",
                            default_name,
                            data.len(),
                            crc_warn,
                            target.display()
                        ),
                        Err(e) => format!("Failed to extract: {e}"),
                    };
                }
            }
            Err(e) => self.status = format!("Cannot decompress {default_name}: {e}"),
        }
    }

    /// Extract every decodable member into a chosen folder, preserving each
    /// member's subdirectory path under it.
    fn extract_archive_all(&mut self) {
        let Some(dir) = rfd::FileDialog::new().pick_folder() else {
            return;
        };
        let (mut ok, mut skipped, mut failed) = (0usize, 0usize, 0usize);
        {
            let Some(content) = self.content.as_ref() else {
                return;
            };
            let Some(listing) = self.archive.as_ref() else {
                return;
            };
            let Ok(members) = &listing.result else {
                return;
            };
            for (i, m) in members.iter().enumerate() {
                if m.is_directory || !m.decodable {
                    skipped += 1;
                    continue;
                }
                match msx_disk::archive::extract(&content.bytes, i) {
                    Ok(data) => {
                        let target = dir.join(sanitize_member_path(&m.path));
                        if let Some(parent) = target.parent() {
                            let _ = std::fs::create_dir_all(parent);
                        }
                        if std::fs::write(&target, &data).is_ok() {
                            ok += 1;
                        } else {
                            failed += 1;
                        }
                    }
                    Err(_) => failed += 1,
                }
            }
        }
        self.status = format!(
            "Extracted {ok} member(s) to {} ({skipped} skipped, {failed} failed)",
            dir.display()
        );
    }

    /// If a row was dragged this frame, write the file(s) to a temp directory
    /// and hand them to the OS drag, using the window handle from `frame`.
    #[cfg(any(target_os = "macos", target_os = "windows"))]
    fn process_drag_out(&mut self, frame: &eframe::Frame) {
        let Some(paths) = self.pending_drag_out.take() else {
            return;
        };
        match self.stage_files_for_drag(&paths) {
            Ok(staged) => {
                if let Err(e) = crate::dnd::start_file_drag(frame, staged) {
                    self.status = format!("Drag failed: {e}");
                }
            }
            Err(e) => self.status = e,
        }
    }

    /// Extract `paths` to a temp directory (the OS drag transfers file paths,
    /// not bytes) and return their absolute locations.
    #[cfg(any(target_os = "macos", target_os = "windows"))]
    fn stage_files_for_drag(&self, paths: &[String]) -> Result<Vec<PathBuf>, String> {
        let dir = std::env::temp_dir().join("mediaexplorer-dragout");
        std::fs::create_dir_all(&dir).map_err(|e| format!("temp dir: {e}"))?;
        let mut staged = Vec::with_capacity(paths.len());
        for path in paths {
            let bytes = self
                .read_doc_file(path)
                .ok_or_else(|| format!("cannot read {path}"))?;
            let target = dir.join(base_name(path));
            std::fs::write(&target, &bytes)
                .map_err(|e| format!("write {}: {e}", target.display()))?;
            staged.push(target);
        }
        Ok(staged)
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

    /// Decode the currently-selected file as an MSX image, if it is one,
    /// honoring any manually-forced format.
    fn decode_current_screen(&self) -> Option<recoil::Image> {
        let disk = self.disk.as_ref()?;
        let content = self.content.as_ref()?;
        let path = content.path.clone();
        let companions = FnCompanions(|ext: &str| disk.companion(&path, ext));
        decode_screen(
            self.forced_format,
            &content.path,
            &content.bytes,
            &companions,
        )
    }

    /// A fallback picker letting the user force a graphics format when a file's
    /// extension is unknown or its bytes don't decode under it. Shown in the
    /// Screen view only when auto-decoding fails or a format is already forced.
    fn screen_format_picker(&mut self, ui: &mut egui::Ui) {
        ui.separator();
        ui.horizontal(|ui| {
            ui.label("Try decoding as:");
            let selected = self
                .forced_format
                .map(|f| f.label())
                .unwrap_or("Auto (from extension)");
            egui::ComboBox::from_id_salt("screen_format")
                .selected_text(selected)
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut self.forced_format, None, "Auto (from extension)");
                    for &fmt in recoil::ImageFormat::all() {
                        ui.selectable_value(&mut self.forced_format, Some(fmt), fmt.label());
                    }
                });
        });
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
            ViewMode::Info => {
                format_fileinfo_text(&content.path, &content.bytes, self.selected_entry())
            }
            ViewMode::Hex => dump_to_string(
                &content.bytes,
                HexConfig {
                    bytes_per_row: self.bytes_per_row,
                    base_address: 0,
                },
                self.charset,
            ),
            ViewMode::Text => text::to_text(
                &content.bytes,
                if self.text_show_all {
                    ControlMode::ShowAll
                } else {
                    ControlMode::Dots
                },
                self.charset,
            ),
            ViewMode::Basic => basic::detokenize(&content.bytes, self.charset),
            ViewMode::Disasm => {
                let img = disasm::locate(&content.path, &content.bytes);
                disasm::disassemble(img.code, img.origin, img.exec)
            }
            // Screen is handled above; the Copy button is hidden in Archive mode.
            ViewMode::Screen | ViewMode::Archive => return,
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
            render_hex(ui, &bytes, 16, scroll, highlight, self.charset);
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

    /// Bottom status bar. With a disk open it shows the physical disk type plus
    /// BPB-derived filesystem facts; with a tape open, tape facts; otherwise
    /// just the transient status text.
    fn status_bar(&self, ui: &mut egui::Ui) {
        if let Some(disk) = &self.disk {
            let geo = disk.geometry;
            ui.label(egui::RichText::new(describe_geometry(geo)).weak());
            ui.horizontal(|ui| {
                ui.label(format!(
                    "Size: {}",
                    humanize_bytes(geo.total_bytes() as u64)
                ));
                if disk.is_partitioned() {
                    ui.separator();
                    ui.label(format!("Sectors: {}", geo.total_sectors()));
                    ui.separator();
                    ui.label(format!("{} partitions", disk.tree.len()));
                } else if let Some(fs) = self.disk_fs_geometry {
                    ui.separator();
                    ui.label(format!("Free: {}", humanize_bytes(fs.free_bytes())));
                    ui.separator();
                    ui.label(format!("Clusters: {}", fs.cluster_count));
                    ui.separator();
                    ui.label(format!("Sectors/cluster: {}", fs.sectors_per_cluster));
                    ui.separator();
                    ui.label(format!("Bytes/sector: {}", fs.bytes_per_sector));
                    ui.separator();
                    ui.label(format!("Sectors: {}", fs.total_sectors));
                } else {
                    ui.separator();
                    ui.label(format!("Sectors: {}", geo.total_sectors()));
                }
                if let Some(label) = &disk.label {
                    ui.separator();
                    ui.label(format!("Vol: {label}"));
                }
                ui.separator();
                ui.label(&self.status);
            });
        } else if let Some(tape) = &self.tape {
            ui.horizontal(|ui| {
                ui.label(format!("Tape ({})", tape.format.label()));
                ui.separator();
                ui.label(format!("Files: {}", tape.file_count()));
                ui.separator();
                ui.label(format!("Data: {} bytes", tape.total_bytes()));
                ui.separator();
                ui.label(&self.status);
            });
        } else {
            ui.label(&self.status);
        }
    }

    /// The DMK per-track analysis table (only shown for `.dmk` disks).
    fn analyze_panel(&mut self, ui: &mut egui::Ui) {
        let Some(analysis) = &self.dmk_analysis else {
            ui.weak("No DMK analysis available.");
            return;
        };
        let standard = analysis
            .track_infos
            .iter()
            .filter(|t| t.is_standard())
            .count();
        ui.label(format!(
            "{} tracks x {} side(s){} — {}/{} tracks standard (9x512 MFM)",
            analysis.tracks,
            analysis.sides,
            if analysis.write_protected {
                ", write-protected"
            } else {
                ""
            },
            standard,
            analysis.track_infos.len(),
        ));
        ui.separator();
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                ui.monospace("Track Side Sectors Size Dens CRC   Notes");
                for t in &analysis.track_infos {
                    ui.monospace(format_dmk_track_row(t));
                }
            });
    }

    /// The tape block overview (only shown for tapes).
    fn blocks_panel(&mut self, ui: &mut egui::Ui) {
        let Some(tape) = &self.tape else {
            ui.weak("No tape open.");
            return;
        };
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                for (i, block) in tape.tape.blocks.iter().enumerate() {
                    render_tape_block(ui, i + 1, block);
                    ui.separator();
                }
            });
    }
}

/// Format one DMK track as a fixed-width row for the analysis table.
fn format_dmk_track_row(t: &DmkTrackInfo) -> String {
    let count = t.sectors.len();
    let size = t.sectors.first().map(|s| s.size).unwrap_or(0);
    let dens = match t.sectors.first().map(|s| s.density) {
        Some(Density::Mfm) => "MFM",
        Some(Density::Fm) => "FM",
        None => "-",
    };
    let bad = t
        .sectors
        .iter()
        .filter(|s| !s.id_crc_ok || !s.data_crc_ok)
        .count();
    let crc = if count == 0 {
        "-".to_string()
    } else if bad == 0 {
        "ok".to_string()
    } else {
        format!("{bad} bad")
    };
    let notes = if count == 0 {
        "empty"
    } else if t.is_standard() {
        "standard"
    } else {
        "non-standard"
    };
    format!(
        "{:<5} {:<4} {:<7} {:<4} {:<4} {:<5} {notes}",
        t.track, t.side, count, size, dens, crc
    )
}

/// Render one tape block in the block overview, in the style of TSX viewers.
fn render_tape_block(ui: &mut egui::Ui, number: usize, block: &TapeBlock) {
    match block {
        TapeBlock::CustomInfo { id, text } => {
            ui.monospace(format!("{number:>3}  #35 Custom info"));
            ui.label(format!("       {id}: {text}"));
        }
        TapeBlock::ArchiveInfo(pairs) => {
            ui.monospace(format!("{number:>3}  #32 Archive info"));
            for (field, value) in pairs {
                ui.label(format!("       {field}: {value}"));
            }
        }
        TapeBlock::Msx {
            header: Some(h),
            data,
        } => {
            ui.monospace(format!(
                "{number:>3}  #4B MSX block   {} HEADER ({} bytes)",
                h.kind.label(),
                data.len()
            ));
            ui.label(format!("       Found: {}", h.name));
        }
        TapeBlock::Msx { header: None, data } => {
            ui.monospace(format!(
                "{number:>3}  #4B MSX block   ({} bytes)",
                data.len()
            ));
        }
        TapeBlock::Other { id, len } => {
            ui.monospace(format!("{number:>3}  #{id:02X} block       ({len} bytes)"));
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

/// Character width of a full file row from `format_file_row`:
/// `name(14) + ' ' + size(8) + "  " + date(16) + "  " + attrs(4)` = 47. MSX 8.3
/// names never exceed the 14-wide name field, so every row is exactly this wide.
const FILE_ROW_CHARS: usize = 47;

/// Default width for the file tree panel: wide enough that a full monospace file
/// row fits on one line, plus room for the folder indent, scrollbar, and margins.
fn files_panel_default_width(ui: &egui::Ui) -> f32 {
    let font = egui::TextStyle::Monospace.resolve(ui.style());
    let sample = "0".repeat(FILE_ROW_CHARS);
    let galley = ui
        .painter()
        .layout_no_wrap(sample, font, egui::Color32::WHITE);
    galley.rect.width() + 64.0
}

/// Build the label for a file row: name + size + date/time + attributes. The
/// date/time and attribute columns are always shown (MSX-DOS 1 disks carry them
/// too); an absent timestamp renders as a blank date column. The name is decoded
/// for display under `charset`.
fn format_file_row(entry: &DirEntry, charset: MsxCharset) -> String {
    format!(
        "{:<14} {:>8}  {:<16}  {}",
        entry.display_name(charset),
        entry.size,
        format_timestamp(entry.modified),
        format_attributes(entry.attributes)
    )
}

/// Message shown in the Files panel when a disk mounts but its FAT directory
/// holds no entries (a blank disk, or a sector-based / non-DOS disk that stores
/// no files in a FAT). Points the user at the raw views.
fn empty_fat_message() -> &'static str {
    "This disk's FAT directory is empty — no files to list.\n\n\
     It may be a blank disk or a sector-based (non-DOS) disk. \
     Use the Sectors or Map view to inspect its raw contents."
}

/// Recursively render the directory tree, recording a clicked file path.
fn render_entries(
    ui: &mut egui::Ui,
    entries: &[DirEntry],
    selection: &BTreeSet<String>,
    writable: bool,
    charset: MsxCharset,
    events: &mut RowEvents,
) {
    for entry in entries {
        if entry.is_dir {
            // Monospace header with aligned date/attribute columns; an absent
            // timestamp (e.g. synthetic partition nodes) renders blank.
            let header: egui::WidgetText = egui::RichText::new(format!(
                "\u{1F4C1} {:<12}  {:<16}  {}",
                entry.display_name(charset),
                format_timestamp(entry.modified),
                format_attributes(entry.attributes)
            ))
            .monospace()
            .into();
            egui::CollapsingHeader::new(header)
                .default_open(true)
                .show(ui, |ui| {
                    render_entries(ui, &entry.children, selection, writable, charset, events);
                });
        } else {
            let is_selected = selection.contains(&entry.path);
            let label = format_file_row(entry, charset);
            let resp = ui
                .selectable_label(is_selected, egui::RichText::new(label).monospace())
                .interact(egui::Sense::click_and_drag());
            if resp.clicked() {
                let toggle = ui.input(|i| i.modifiers.command);
                events.clicked = Some((entry.path.clone(), toggle));
            }
            if resp.drag_started() {
                events.drag_started = Some(entry.path.clone());
            }
            resp.context_menu(|ui| {
                if writable {
                    if ui.button("Rename…").clicked() {
                        events.action = Some(RowAction::Rename(entry.path.clone()));
                        ui.close();
                    }
                    if ui.button("Delete").clicked() {
                        events.action = Some(RowAction::Delete(entry.path.clone()));
                        ui.close();
                    }
                    ui.separator();
                }
                if ui.button("Extract…").clicked() {
                    events.action = Some(RowAction::Extract(entry.path.clone()));
                    ui.close();
                }
            });
        }
    }
}

/// Render the file list for an open tape: each derived file as a selectable,
/// draggable row showing its name, kind, and size.
fn render_tape_files(
    ui: &mut egui::Ui,
    tape: &LoadedTape,
    selection: &BTreeSet<String>,
    events: &mut RowEvents,
) {
    if tape.file_count() == 0 {
        ui.weak("Tape has no recognizable files.");
        return;
    }
    for (key, file) in tape.entries() {
        let is_selected = selection.contains(key);
        let label = format!(
            "{:<14} {:<7} {:>8}",
            key,
            file.kind.label(),
            file.data.len()
        );
        let resp = ui
            .selectable_label(is_selected, egui::RichText::new(label).monospace())
            .interact(egui::Sense::click_and_drag());
        if resp.clicked() {
            let toggle = ui.input(|i| i.modifiers.command);
            events.clicked = Some((key.to_string(), toggle));
        }
        if resp.drag_started() {
            events.drag_started = Some(key.to_string());
        }
        resp.context_menu(|ui| {
            if ui.button("Extract…").clicked() {
                events.action = Some(RowAction::Extract(key.to_string()));
                ui.close();
            }
        });
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
    charset: MsxCharset,
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
            line.extend(chunk.iter().map(|&b| ascii_char(b, charset)));
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
    path.rsplit(['/', '\\'])
        .next()
        .unwrap_or("file")
        .to_string()
}

/// Truncate `s` to at most `max` characters, marking elision with a trailing
/// ellipsis so the monospace member table stays aligned.
fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let mut out: String = s.chars().take(max.saturating_sub(1)).collect();
        out.push('\u{2026}');
        out
    }
}

/// Turn a stored archive member path into a safe relative host path: split on
/// both separators and drop empty, `.`, and `..` components. This preserves
/// subdirectories while preventing path traversal outside the chosen folder.
fn sanitize_member_path(path: &str) -> PathBuf {
    let mut out = PathBuf::new();
    for part in path.split(['/', '\\']) {
        if part.is_empty() || part == "." || part == ".." {
            continue;
        }
        out.push(part);
    }
    if out.as_os_str().is_empty() {
        out.push("extracted");
    }
    out
}

/// A human-readable description of the physical disk format, e.g.
/// `3.5" Double Sided, Double Density (2DD): 2 sides × 80 tracks × 9
/// sectors/track × 512 bytes/sector = 737280 bytes (720 kB)`.
fn describe_geometry(geo: Geometry) -> String {
    let bytes = geo.total_bytes();
    let name = match (geo.sides, geo.tracks, geo.sectors_per_track) {
        (1, 80, 9) => Some("3.5\" Single Sided, Double Density (1DD)"),
        (2, 80, 9) => Some("3.5\" Double Sided, Double Density (2DD)"),
        (1, 40, 9) => Some("5.25\" Single Sided, Double Density (SS,DD)"),
        (2, 40, 9) => Some("5.25\" Double Sided, Double Density (DS,DD)"),
        _ => None,
    };
    let arithmetic = format!(
        "{} side{} \u{00D7} {} tracks \u{00D7} {} sectors/track \u{00D7} {} bytes/sector = {bytes} bytes ({} kB)",
        geo.sides,
        if geo.sides == 1 { "" } else { "s" },
        geo.tracks,
        geo.sectors_per_track,
        SECTOR_SIZE,
        bytes / 1024,
    );
    match name {
        Some(n) => format!("{n}: {arithmetic}"),
        None => arithmetic,
    }
}

/// Whether `path`'s lowercased extension is in `exts`.
fn ext_in(path: &Path, exts: &[&str]) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase())
        .is_some_and(|e| exts.contains(&e.as_str()))
}

/// Whether `path`'s extension marks it as an openable disk image.
fn is_disk_image(path: &Path) -> bool {
    ext_in(path, DISK_IMAGE_EXTS)
}

/// Whether `path`'s extension marks it as a tape image.
fn is_tape(path: &Path) -> bool {
    ext_in(path, TAPE_EXTS)
}

/// Whether `path` can be opened as a document (disk or tape).
fn is_openable(path: &Path) -> bool {
    is_disk_image(path) || is_tape(path)
}

/// Punctuation allowed in an MSX (FAT 8.3) filename, besides ASCII
/// alphanumerics. Shared by [`sanitize_msx_name`] and [`normalize_msx_input`].
const MSX_NAME_PUNCT: &str = "_-!#$%&@^{}()~'";

/// True if `c` is allowed in an MSX 8.3 filename (ASCII rule only).
fn is_msx_name_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || MSX_NAME_PUNCT.contains(c)
}

/// True if `c` is allowed in a rename under `charset`. ASCII follows the 8.3
/// rule; a high glyph (kana, accented Latin, ...) is allowed when the charset
/// can encode it, so non-ASCII names survive editing instead of being stripped.
fn is_rename_char(c: char, charset: MsxCharset) -> bool {
    if (c as u32) < 0x80 {
        is_msx_name_char(c)
    } else {
        charset::encode_byte(charset, c).is_some()
    }
}

/// Coerce `name` into a valid 8.3 shape: ASCII-uppercase, keep only characters
/// for which `allowed` holds, cap the stem at 8 and extension at 3 characters
/// (counted as characters, since one MSX byte is one glyph). An empty stem
/// becomes `FILE`.
fn sanitize_8_3(name: &str, allowed: impl Fn(char) -> bool) -> String {
    let upper: String = name.chars().map(|c| c.to_ascii_uppercase()).collect();
    let (stem, ext) = match upper.rsplit_once('.') {
        Some((s, e)) => (s, e),
        None => (upper.as_str(), ""),
    };
    let keep = |s: &str, max: usize| -> String {
        s.chars().filter(|c| allowed(*c)).take(max).collect()
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

/// Coerce a dropped host filename into a valid ASCII 8.3 MSX name.
fn sanitize_msx_name(name: &str) -> String {
    sanitize_8_3(name, is_msx_name_char)
}

/// Restrict free-form rename input to a valid 8.3 shape *as it is typed*:
/// ASCII-uppercase, only characters allowed under `charset`, at most one `.`
/// separator, an 8-char stem and a 3-char extension. Unlike [`sanitize_8_3`] it
/// leaves an empty stem empty (so the field can be cleared) and keeps a trailing
/// `.` so the extension can still be typed.
fn normalize_msx_input(raw: &str, charset: MsxCharset) -> String {
    let mut stem = String::new();
    let mut ext = String::new();
    let mut in_ext = false;
    for c in raw.chars() {
        let c = c.to_ascii_uppercase();
        if c == '.' {
            // The first dot starts the extension; later dots are ignored.
            in_ext = true;
        } else if is_rename_char(c, charset) {
            if in_ext {
                if ext.chars().count() < 3 {
                    ext.push(c);
                }
            } else if stem.chars().count() < 8 {
                stem.push(c);
            }
        }
    }
    if in_ext {
        format!("{stem}.{ext}")
    } else {
        stem
    }
}

/// The stem (pre-extension) part of a normalized 8.3 name; empty means the name
/// has no usable base and cannot be applied.
fn msx_name_stem(name: &str) -> &str {
    name.split('.').next().unwrap_or("")
}

/// Extensions that default to the Text view.
const TEXT_EXTENSIONS: &[&str] = &[
    "txt", "bat", "asc", "doc", "me", "ini", "cfg", "diz", "nfo", "log", "csv", "md",
];

/// Pick a sensible default view mode for a file based on its extension.
fn default_view_mode(path: &str) -> ViewMode {
    let ext = path.rsplit('.').next().unwrap_or("").to_ascii_lowercase();
    if msx_disk::archive::is_archive(path) {
        ViewMode::Archive
    } else if recoil::is_supported(path) {
        ViewMode::Screen
    } else if ext == "bas" {
        ViewMode::Basic
    } else if msx_disk::fileinfo::music::is_music_ext(&ext) {
        ViewMode::Info
    } else if matches!(ext.as_str(), "com" | "cpm" | "bin") {
        ViewMode::Disasm
    } else if TEXT_EXTENSIONS.contains(&ext.as_str()) {
        ViewMode::Text
    } else {
        ViewMode::Hex
    }
}

/// Decode the selected file as an MSX image: forcing `forced` when set,
/// otherwise classifying by file extension. The single seam shared by the
/// Screen view, Copy image, and Save PNG so they never diverge.
fn decode_screen(
    forced: Option<recoil::ImageFormat>,
    path: &str,
    bytes: &[u8],
    companions: &dyn recoil::CompanionFiles,
) -> Option<recoil::Image> {
    match forced {
        Some(fmt) => recoil::decode_as(fmt, bytes, companions),
        None => recoil::decode(path, bytes, companions),
    }
}

/// The message shown in the Screen view when decoding fails. It names the
/// forced format so a failed manual attempt reads differently per format (and
/// differently from the initial auto-detect failure), giving explicit feedback
/// that an attempt was made.
fn screen_decode_failed_message(forced: Option<recoil::ImageFormat>) -> String {
    match forced {
        Some(fmt) => format!(
            "Could not decode as {}. Try another format below.",
            fmt.label()
        ),
        None => {
            "Not a recognized MSX graphics file. Pick a format below to try decoding it anyway."
                .to_string()
        }
    }
}

/// Render a decoded MSX graphics image, caching the GPU texture by `(path,
/// forced format)`. Returns whether an image was shown (false on decode
/// failure, so the caller can offer the format picker).
fn render_screen(
    ui: &mut egui::Ui,
    cache: &mut Option<(String, Option<recoil::ImageFormat>, egui::TextureHandle)>,
    path: &str,
    bytes: &[u8],
    forced: Option<recoil::ImageFormat>,
    companions: &dyn recoil::CompanionFiles,
) -> bool {
    let stale = cache
        .as_ref()
        .map(|(p, f, _)| p != path || *f != forced)
        .unwrap_or(true);
    if stale {
        let Some(img) = decode_screen(forced, path, bytes, companions) else {
            *cache = None;
            let msg = screen_decode_failed_message(forced);
            if forced.is_some() {
                // A manual attempt that failed: make it prominent (theme-aware
                // warning colour) so the result of the pick is unmistakable.
                let color = ui.visuals().warn_fg_color;
                ui.colored_label(color, msg);
            } else {
                ui.weak(msg);
            }
            return false;
        };
        let image =
            egui::ColorImage::from_rgba_unmultiplied([img.width, img.height], &img.to_rgba());
        let texture = ui.ctx().load_texture(
            format!("screen:{path}"),
            image,
            egui::TextureOptions::NEAREST,
        );
        *cache = Some((path.to_string(), forced, texture));
    }

    if let Some((_, _, texture)) = cache {
        // Confirm which forced format produced the image, so a successful manual
        // attempt is acknowledged (not just the initial extension-based decode).
        if let Some(fmt) = forced {
            ui.colored_label(
                egui::Color32::from_rgb(0x3c, 0xa8, 0x4b),
                format!("Decoded as {}.", fmt.label()),
            );
        }
        egui::ScrollArea::both().show(ui, |ui| {
            let size = texture.size_vec2() * 2.0; // 2x nearest-neighbour zoom
            ui.image(egui::load::SizedTexture::new(texture.id(), size));
        });
    }
    cache.is_some()
}

/// Detokenized MSX-BASIC listing.
fn render_basic(ui: &mut egui::Ui, bytes: &[u8], charset: MsxCharset) {
    let listing = basic::detokenize(bytes, charset);
    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            ui.add(egui::Label::new(egui::RichText::new(listing).monospace()).wrap());
        });
}

/// The most code we disassemble at once: the full Z80 16-bit address space.
/// Beyond this, displayed addresses would wrap and banked ROMs need mapping we
/// don't have, so the listing is capped with a notice.
const MAX_DISASM_BYTES: usize = 64 * 1024;

/// Z80/R800 disassembly listing. The load address and code window come from the
/// file's type and any BSAVE header (see [`disasm::locate`]).
fn render_disasm(ui: &mut egui::Ui, path: &str, bytes: &[u8]) {
    let img = disasm::locate(path, bytes);
    let shown = &img.code[..img.code.len().min(MAX_DISASM_BYTES)];
    let listing = disasm::disassemble(shown, img.origin, img.exec);
    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            if img.code.len() > MAX_DISASM_BYTES {
                ui.weak(format!(
                    "Showing first {} kB of {} kB.",
                    MAX_DISASM_BYTES / 1024,
                    img.code.len() / 1024
                ));
            }
            ui.add(egui::Label::new(egui::RichText::new(listing).monospace()).wrap());
        });
}

/// Plain-text rendering of the File Info view, for the Copy button.
fn format_fileinfo_text(path: &str, bytes: &[u8], entry: Option<&DirEntry>) -> String {
    use std::fmt::Write;
    let info = msx_disk::fileinfo::describe(path, bytes);
    let mut out = String::new();
    let _ = writeln!(out, "Name: {}", base_name(path));
    let size = entry.map(|e| e.size).unwrap_or(bytes.len() as u64);
    let _ = writeln!(out, "Size: {size} bytes");
    if let Some(e) = entry {
        let ts = format_timestamp(e.modified);
        if !ts.is_empty() {
            let _ = writeln!(out, "Modified: {ts}");
        }
        let _ = writeln!(out, "Attributes: {}", format_attributes(e.attributes));
    }
    if let Some(desc) = info.description {
        let _ = writeln!(out, "Description: {desc}");
    }
    if let Some(b) = info.bload {
        let _ = writeln!(
            out,
            "BSAVE header: start=0x{:04X} end=0x{:04X} exec=0x{:04X} ({} bytes)",
            b.start,
            b.end,
            b.exec,
            b.data_len()
        );
    }
    if let Some(g) = &info.graphics {
        let _ = writeln!(out, "Graphics: {}", g.label);
    }
    if let Some(m) = &info.music {
        let _ = writeln!(out, "Music format: {}", m.format);
        if let Some(t) = &m.title {
            let _ = writeln!(out, "Title: {t}");
        }
        if let Some(a) = &m.author {
            let _ = writeln!(out, "Author: {a}");
        }
        if let Some(p) = m.positions {
            let _ = writeln!(out, "Positions: {p}");
        }
        if let Some(c) = m.channels {
            let _ = writeln!(out, "Channels: {c}");
        }
        if let Some(s) = m.subsongs {
            if s > 0 {
                let _ = writeln!(out, "Subsongs: {s}");
            }
        }
        for (k, v) in &m.extra {
            let _ = writeln!(out, "{k}: {v}");
        }
    }
    out
}

/// The file name to show in the Info panel: decoded under `charset` so
/// Japanese/accented names render the same way as in the file list (which uses
/// [`DirEntry::display_name`]) rather than as the raw PUA-encoded name.
fn info_display_name(entry: Option<&DirEntry>, path: &str, charset: MsxCharset) -> String {
    match entry {
        Some(e) => e.display_name(charset),
        None => charset::decode_fs_name(charset, &base_name(path)),
    }
}

/// File Info view: filesystem facts plus content-derived format details.
fn render_info(
    ui: &mut egui::Ui,
    path: &str,
    bytes: &[u8],
    entry: Option<&DirEntry>,
    charset: MsxCharset,
) {
    let info = msx_disk::fileinfo::describe(path, bytes);
    let yes_no = |b: bool| if b { "yes" } else { "no" };

    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            ui.heading("File");
            egui::Grid::new("info_file").num_columns(2).show(ui, |ui| {
                ui.label("Name:");
                ui.monospace(info_display_name(entry, path, charset));
                ui.end_row();
                let size = entry.map(|e| e.size).unwrap_or(bytes.len() as u64);
                ui.label("Size:");
                ui.monospace(format!("{size} bytes"));
                ui.end_row();
                if let Some(ts) = entry.map(|e| format_timestamp(e.modified)) {
                    if !ts.is_empty() {
                        ui.label("Modified:");
                        ui.monospace(ts);
                        ui.end_row();
                    }
                }
            });

            if let Some(e) = entry {
                let a = e.attributes;
                ui.add_space(4.0);
                ui.label("Attributes");
                egui::Grid::new("info_attrs").num_columns(2).show(ui, |ui| {
                    for (name, set) in [
                        ("Read-only", a.read_only),
                        ("Hidden", a.hidden),
                        ("System", a.system),
                        ("Archive", a.archive),
                    ] {
                        ui.label(format!("{name}:"));
                        ui.monospace(yes_no(set));
                        ui.end_row();
                    }
                });
            }

            if let Some(desc) = info.description {
                ui.add_space(8.0);
                ui.heading("Description");
                ui.label(desc);
            }

            if let Some(b) = info.bload {
                ui.add_space(8.0);
                ui.heading("Binary (BSAVE) header");
                egui::Grid::new("info_bload").num_columns(2).show(ui, |ui| {
                    for (name, addr) in [("Start", b.start), ("End", b.end), ("Exec", b.exec)] {
                        ui.label(format!("{name}:"));
                        ui.monospace(format!("0x{addr:04X}"));
                        ui.end_row();
                    }
                    ui.label("Length:");
                    ui.monospace(format!("{} bytes", b.data_len()));
                    ui.end_row();
                });
            }

            if let Some(g) = &info.graphics {
                ui.add_space(8.0);
                ui.heading("Graphics");
                ui.label(g.label.as_str());
            }

            if let Some(m) = &info.music {
                ui.add_space(8.0);
                ui.heading("Music");
                egui::Grid::new("info_music").num_columns(2).show(ui, |ui| {
                    ui.label("Format:");
                    ui.monospace(m.format);
                    ui.end_row();
                    if let Some(t) = &m.title {
                        ui.label("Title:");
                        ui.monospace(t.as_str());
                        ui.end_row();
                    }
                    if let Some(a) = &m.author {
                        ui.label("Author:");
                        ui.monospace(a.as_str());
                        ui.end_row();
                    }
                    if let Some(p) = m.positions {
                        ui.label("Positions:");
                        ui.monospace(p.to_string());
                        ui.end_row();
                    }
                    if let Some(c) = m.channels {
                        ui.label("Channels:");
                        ui.monospace(c.to_string());
                        ui.end_row();
                    }
                    if let Some(s) = m.subsongs {
                        if s > 0 {
                            ui.label("Subsongs:");
                            ui.monospace(s.to_string());
                            ui.end_row();
                        }
                    }
                    for (k, v) in &m.extra {
                        ui.label(format!("{k}:"));
                        ui.monospace(v.as_str());
                        ui.end_row();
                    }
                });
            }
        });
}

/// Text view, capped to a sane size for responsiveness.
fn render_text(ui: &mut egui::Ui, bytes: &[u8], show_all: bool, charset: MsxCharset) {
    let shown = &bytes[..bytes.len().min(MAX_TEXT_BYTES)];
    let mode = if show_all {
        ControlMode::ShowAll
    } else {
        ControlMode::Dots
    };
    let rendered = text::to_text(shown, mode, charset);
    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            if bytes.len() > MAX_TEXT_BYTES {
                ui.weak(format!(
                    "Showing first {} kB of {} kB.",
                    MAX_TEXT_BYTES / 1024,
                    bytes.len() / 1024
                ));
            }
            ui.add(egui::Label::new(egui::RichText::new(rendered).monospace()).wrap());
        });
}

impl eframe::App for MediaExplorerApp {
    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        self.handle_dropped_files(ui.ctx());

        egui::Panel::top("toolbar").show_inside(ui, |ui| {
            ui.add_space(4.0);
            self.toolbar(ui);
            ui.add_space(4.0);
        });
        egui::Panel::bottom("status").show_inside(ui, |ui| {
            ui.add_space(2.0);
            self.status_bar(ui);
            ui.add_space(2.0);
        });
        let tree_width = files_panel_default_width(ui);
        egui::Panel::left("tree")
            .resizable(true)
            // `min_size` (not `default_size`) so a persisted-narrower width from
            // an earlier run is clamped back up to one full row; the user can
            // still drag the panel wider.
            .default_size(tree_width)
            .min_size(tree_width)
            .show_inside(ui, |ui| {
                self.tree_panel(ui);
            });
        egui::CentralPanel::default().show_inside(ui, |ui| match self.app_view {
            AppView::Files => self.viewer_panel(ui),
            AppView::Sectors => self.sector_panel(ui),
            AppView::Map => self.map_panel(ui),
            AppView::Analyze => self.analyze_panel(ui),
            AppView::Blocks => self.blocks_panel(ui),
        });

        self.delete_confirmation(ui.ctx());
        self.rename_dialog(ui.ctx());

        #[cfg(any(target_os = "macos", target_os = "windows"))]
        self.process_drag_out(frame);
        #[cfg(not(any(target_os = "macos", target_os = "windows")))]
        let _ = frame;
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
    fn normalize_input_enforces_83_shape() {
        let intl = MsxCharset::International;
        // Uppercases, drops disallowed characters, caps stem and extension.
        assert_eq!(normalize_msx_input("my long file.text", intl), "MYLONGFI.TEX");
        assert_eq!(normalize_msx_input("game.com", intl), "GAME.COM");
        // Only the first dot separates; later dots are dropped.
        assert_eq!(normalize_msx_input("a.b.c", intl), "A.BC");
        // Spaces and other illegal characters are filtered out.
        assert_eq!(normalize_msx_input("a b/c?", intl), "ABC");
    }

    #[test]
    fn normalize_input_allows_in_progress_typing() {
        let intl = MsxCharset::International;
        // Empty stays empty (the field can be cleared) instead of "FILE".
        assert_eq!(normalize_msx_input("", intl), "");
        // A trailing dot is kept so the extension can still be typed.
        assert_eq!(normalize_msx_input("GAME.", intl), "GAME.");
    }

    /// Three half-width katakana (bytes 0xB1..=0xB3) as decoded display glyphs.
    fn sample_kana() -> String {
        (0xB1u8..=0xB3)
            .map(|b| charset::decode_byte(MsxCharset::Japanese, b))
            .collect()
    }

    #[test]
    fn rename_normalize_keeps_japanese_glyphs() {
        let kana = sample_kana();
        let out = normalize_msx_input(&format!("{kana}.bas"), MsxCharset::Japanese);
        assert_eq!(out, format!("{kana}.BAS"));
    }

    #[test]
    fn rename_normalize_drops_glyphs_outside_charset() {
        // Kana isn't representable in International, so it is stripped; ASCII stays.
        let out = normalize_msx_input(&format!("{}AB.bas", sample_kana()), MsxCharset::International);
        assert_eq!(out, "AB.BAS");
    }

    #[test]
    fn rename_round_trips_japanese_name_to_fatfs_key() {
        // The on-disk (PUA) key decodes for display, sanitizes, and re-encodes
        // to the exact same key — proving a rename preserves Japanese names.
        let key = "\u{F0B1}\u{F0B2}\u{F0B3}.BAS";
        let display = charset::decode_fs_name(MsxCharset::Japanese, key);
        let sane = sanitize_8_3(&display, |c| is_rename_char(c, MsxCharset::Japanese));
        let encoded = charset::encode_fs_name(MsxCharset::Japanese, &sane).unwrap();
        assert_eq!(encoded, key);
    }

    #[test]
    fn msx_name_stem_detects_applicable_names() {
        assert_eq!(msx_name_stem("GAME.COM"), "GAME");
        assert_eq!(msx_name_stem("GAME"), "GAME");
        // No stem means nothing to apply.
        assert!(msx_name_stem("").is_empty());
        assert!(msx_name_stem(".COM").is_empty());
    }

    /// A synthetic, minimal SCREEN 2 BSAVE buffer (single top-left pixel), used
    /// to exercise format selection without a real disk fixture.
    fn synthetic_sc2() -> Vec<u8> {
        let mut buf = vec![0u8; 14343];
        buf[0] = 0xfe;
        buf[3] = 0xff;
        buf[4] = 0x37;
        buf[7] = 0x80;
        buf[0x2007] = 0xf1;
        buf
    }

    #[test]
    fn screen_failure_message_distinguishes_forced_attempt() {
        // A failed manual attempt names the format, so picking a type that does
        // not decode produces a visibly different message (proving an attempt
        // was made) rather than the constant "not recognized" placeholder.
        let forced = screen_decode_failed_message(Some(recoil::ImageFormat::Screen8));
        assert!(
            forced.contains("SCREEN 8"),
            "forced-failure message should name the format: {forced}"
        );
        assert!(
            forced.to_lowercase().contains("could not"),
            "should read as a failed attempt: {forced}"
        );

        // The auto (no forced format) message is distinct and points at the
        // picker.
        let auto = screen_decode_failed_message(None);
        assert!(!auto.contains("SCREEN 8"));
        assert!(
            auto.to_lowercase().contains("pick a format"),
            "auto message should point at the picker: {auto}"
        );
    }

    #[test]
    fn decode_screen_uses_extension_when_no_format_forced() {
        let buf = synthetic_sc2();
        assert!(decode_screen(None, "pic.sc2", &buf, &recoil::NoCompanions).is_some());
        // An unknown extension can't be classified, so auto-decode fails.
        assert!(decode_screen(None, "pic.dat", &buf, &recoil::NoCompanions).is_none());
    }

    #[test]
    fn decode_screen_forced_format_overrides_extension() {
        let buf = synthetic_sc2();
        // Forcing SCREEN 2 decodes a file whose extension is unrecognized.
        assert!(decode_screen(
            Some(recoil::ImageFormat::Screen2),
            "pic.dat",
            &buf,
            &recoil::NoCompanions
        )
        .is_some());
        // Forcing a mismatched format on a recognized .sc2 fails (these are not
        // SCREEN 8 bytes), proving the forced format wins over the extension.
        assert!(decode_screen(
            Some(recoil::ImageFormat::Screen8),
            "pic.sc2",
            &buf,
            &recoil::NoCompanions
        )
        .is_none());
    }

    #[test]
    fn default_view_mode_by_extension() {
        assert_eq!(default_view_mode("PIC.SC8"), ViewMode::Screen);
        assert_eq!(default_view_mode("PROG.BAS"), ViewMode::Basic);
        assert_eq!(default_view_mode("DATA.BIN"), ViewMode::Disasm);
        assert_eq!(default_view_mode("GAME.COM"), ViewMode::Disasm);
        assert_eq!(default_view_mode("TOOL.cpm"), ViewMode::Disasm);
        assert_eq!(default_view_mode("README.TXT"), ViewMode::Text);
        assert_eq!(default_view_mode("AUTOEXEC.BAT"), ViewMode::Text);
        assert_eq!(default_view_mode("notes.txt"), ViewMode::Text);
        assert_eq!(default_view_mode("SONG.MBM"), ViewMode::Info);
        assert_eq!(default_view_mode("TUNE.mod"), ViewMode::Info);
        assert_eq!(default_view_mode("track.pt3"), ViewMode::Info);
        assert_eq!(default_view_mode("GAME.LZH"), ViewMode::Archive);
        assert_eq!(default_view_mode("util.lha"), ViewMode::Archive);
        assert_eq!(default_view_mode("DEMO.LZS"), ViewMode::Archive);
        assert_eq!(default_view_mode("SNOOPY.pma"), ViewMode::Archive);
    }

    #[test]
    fn sanitize_member_path_preserves_subdirs_and_blocks_traversal() {
        assert_eq!(sanitize_member_path("FILE.BIN"), PathBuf::from("FILE.BIN"));
        assert_eq!(
            sanitize_member_path("SUB/DIR/FILE.BIN"),
            PathBuf::from("SUB/DIR/FILE.BIN")
        );
        // Backslash separators (MS-DOS) are normalized.
        assert_eq!(
            sanitize_member_path("SUB\\FILE.BIN"),
            PathBuf::from("SUB/FILE.BIN")
        );
        // `..` and leading separators cannot escape the chosen folder.
        assert_eq!(
            sanitize_member_path("../../etc/passwd"),
            PathBuf::from("etc/passwd")
        );
        assert_eq!(sanitize_member_path("/abs/path"), PathBuf::from("abs/path"));
        assert_eq!(sanitize_member_path("../.."), PathBuf::from("extracted"));
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
    fn file_row_always_includes_date_and_attributes() {
        use msx_disk::fs::{Attributes, Timestamp};
        // DOS1 disks carry timestamps too, so a file with one always shows it.
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
        let row = format_file_row(&e, MsxCharset::International);
        assert!(row.starts_with(&format!("{:<14} {:>8}", "GAME.COM", 1234u64)));
        assert!(row.contains("1991-03-25 14:30"), "row: {row}");
        assert!(row.trim_end().ends_with("---A"), "row: {row}");
    }

    #[test]
    fn info_name_decodes_under_charset_like_the_file_list() {
        // A PUA-encoded high byte (0xB1 -> U+F0B1) must decode to the same glyph
        // the file list shows, not render as the raw PUA name.
        let name = "\u{F0B1}.BIN";
        let e = file_entry(name, 10, msx_disk::fs::Attributes::default(), None);
        let shown = info_display_name(Some(&e), "DIR/\u{F0B1}.BIN", MsxCharset::Japanese);
        // Matches the file list's decoded name...
        assert_eq!(shown, e.display_name(MsxCharset::Japanese));
        // ...and is no longer the raw, undecoded PUA string.
        assert_ne!(shown, name);
    }

    #[test]
    fn info_name_falls_back_to_path_base_when_no_entry() {
        // With no DirEntry, the path's base name is still decoded under charset.
        let shown = info_display_name(None, "DIR/\u{F0B1}.BIN", MsxCharset::Japanese);
        assert_eq!(shown, charset::decode_fs_name(MsxCharset::Japanese, "\u{F0B1}.BIN"));
    }

    #[test]
    fn file_row_width_matches_panel_sizing_constant() {
        use msx_disk::fs::{Attributes, Timestamp};
        // A maximal file row must be exactly FILE_ROW_CHARS wide, so the panel's
        // default width (derived from that constant) fits a full row on one line.
        let e = file_entry(
            "ABCDEFGH.IJK",
            99_999_999,
            Attributes {
                read_only: true,
                hidden: true,
                system: true,
                archive: true,
            },
            Some(Timestamp {
                year: 9999,
                month: 12,
                day: 31,
                hour: 23,
                minute: 59,
            }),
        );
        let row = format_file_row(&e, MsxCharset::International);
        assert_eq!(row.chars().count(), FILE_ROW_CHARS, "row: {row:?}");
    }

    #[test]
    fn empty_fat_message_names_fat_and_points_to_raw_views() {
        let msg = empty_fat_message();
        assert!(msg.to_lowercase().contains("fat"), "msg: {msg}");
        assert!(msg.contains("Sectors"), "msg: {msg}");
    }

    #[test]
    fn file_row_keeps_columns_when_timestamp_absent() {
        // No timestamp -> blank date column, but the attribute column remains.
        let e = file_entry("GAME.COM", 1234, msx_disk::fs::Attributes::default(), None);
        let row = format_file_row(&e, MsxCharset::International);
        assert!(row.starts_with(&format!("{:<14} {:>8}", "GAME.COM", 1234u64)));
        assert!(row.trim_end().ends_with("----"), "row: {row}");
    }

    #[test]
    fn disk_search_jump_maps_offset_to_sector_and_row() {
        // Offset 1234 -> sector 2 (1024..1536), row (1234 % 512) / 16 = 210/16 = 13.
        let mut app = MediaExplorerApp {
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
        let mut app = MediaExplorerApp {
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
        let mut app = MediaExplorerApp::default();
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
    fn describe_geometry_names_standard_formats() {
        let desc = describe_geometry(Geometry::DS_720K);
        assert!(
            desc.starts_with("3.5\" Double Sided, Double Density (2DD)"),
            "{desc}"
        );
        assert!(desc.contains("2 sides"), "{desc}");
        assert!(desc.contains("80 tracks"), "{desc}");
        assert!(desc.contains("= 737280 bytes (720 kB)"), "{desc}");

        let desc360 = describe_geometry(Geometry::SS_360K);
        assert!(
            desc360.starts_with("3.5\" Single Sided, Double Density (1DD)"),
            "{desc360}"
        );
        assert!(desc360.contains("1 side "), "singular: {desc360}");
        assert!(desc360.contains("(360 kB)"), "{desc360}");
    }

    #[test]
    fn describe_geometry_falls_back_to_arithmetic_for_unknown() {
        let odd = Geometry {
            sides: 2,
            tracks: 35,
            sectors_per_track: 8,
        };
        let desc = describe_geometry(odd);
        // No named form factor, just the arithmetic.
        assert!(!desc.contains('"'), "{desc}");
        assert!(
            desc.starts_with("2 sides × 35 tracks × 8 sectors/track"),
            "{desc}"
        );
    }

    #[test]
    fn is_disk_image_matches_known_extensions_case_insensitively() {
        assert!(is_disk_image(Path::new("GAME.DSK")));
        assert!(is_disk_image(Path::new("game.xsa")));
        assert!(is_disk_image(Path::new("raw.DMK")));
        assert!(is_disk_image(Path::new("/tmp/disk.Di2")));
        assert!(!is_disk_image(Path::new("README.TXT")));
        assert!(!is_disk_image(Path::new("noext")));
        // Tapes are not disk images, but are still openable documents.
        assert!(!is_disk_image(Path::new("tape.cas")));
    }

    #[test]
    fn tape_and_openable_extensions() {
        assert!(is_tape(Path::new("game.cas")));
        assert!(is_tape(Path::new("GAME.TSX")));
        assert!(!is_tape(Path::new("disk.dsk")));
        // Openable covers both disks and tapes.
        assert!(is_openable(Path::new("disk.dmk")));
        assert!(is_openable(Path::new("tape.tsx")));
        assert!(!is_openable(Path::new("notes.txt")));
    }

    #[test]
    fn selection_paths_prefers_marked_set_in_sorted_order() {
        let app = MediaExplorerApp {
            selection: BTreeSet::from(["B.TXT".to_string(), "A.TXT".to_string()]),
            selected: Some("C.TXT".to_string()),
            ..Default::default()
        };
        assert_eq!(app.selection_paths(), vec!["A.TXT", "B.TXT"]);
    }

    #[test]
    fn selection_paths_falls_back_to_viewed_file() {
        let app = MediaExplorerApp {
            selected: Some("ONLY.TXT".to_string()),
            ..Default::default()
        };
        assert_eq!(app.selection_paths(), vec!["ONLY.TXT"]);

        let empty = MediaExplorerApp::default();
        assert!(empty.selection_paths().is_empty());
    }

    #[test]
    fn paths_for_row_targets_only_an_unselected_row() {
        let app = MediaExplorerApp {
            selection: BTreeSet::from(["A.TXT".to_string(), "B.TXT".to_string()]),
            ..Default::default()
        };
        // Right-clicking a row outside the selection acts on that row alone.
        assert_eq!(app.paths_for_row("C.TXT"), vec!["C.TXT"]);
    }

    #[test]
    fn paths_for_row_expands_to_whole_multiselection() {
        let app = MediaExplorerApp {
            selection: BTreeSet::from(["B.TXT".to_string(), "A.TXT".to_string()]),
            ..Default::default()
        };
        // Right-clicking a row inside a multi-selection acts on the whole set.
        assert_eq!(app.paths_for_row("A.TXT"), vec!["A.TXT", "B.TXT"]);
    }

    #[test]
    fn paths_for_row_single_selected_row_acts_on_itself() {
        let app = MediaExplorerApp {
            selection: BTreeSet::from(["A.TXT".to_string()]),
            ..Default::default()
        };
        assert_eq!(app.paths_for_row("A.TXT"), vec!["A.TXT"]);
    }
}
