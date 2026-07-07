//! Top-level application state and the `eframe::App` implementation.

use std::cell::{OnceCell, RefCell};
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use msx_disk::fs::map::SectorKind;
use msx_disk::image::dmk::{self, Density, DmkAnalysis, DmkTrackInfo};
use msx_disk::image::geometry::{DiskFormat, Geometry, SECTOR_SIZE, SIZE_360K, SIZE_720K};
use msx_disk::recoil::{self, FnCompanions};
use msx_disk::search;
use msx_disk::tape::TapeBlock;
use msx_disk::view::basic;
use msx_disk::view::disasm;
use msx_disk::view::hex::{ascii_char, dump_to_string, HexConfig};
use msx_disk::view::text::{self, ControlMode};
use msx_disk::{charset, DirEntry, ImageFormat, MsxCharset};

use crate::hexlayout::{HexLayout, HexRegion};
use crate::settings::{ByteGrouping, HexViewOptions, Settings, SETTINGS_KEY};
use crate::state::{humanize_bytes, LoadedDisk, LoadedTape};
use crate::tree_nav::{self, NavKey, TreeNav};

// `MediaExplorerApp`'s methods are split across these submodules; each holds a
// focused `impl MediaExplorerApp` block. The free render/format helpers live in
// `render`/`views` and are re-exported below so every submodule reaches them
// through a single `use super::*`.
mod actions;
mod browser;
mod disk_views;
mod document;
mod hexrender;
mod info;
mod render;
mod transfer;
mod viewer;
mod views;

#[cfg(test)]
mod tests;

pub(crate) use hexrender::*;
pub(crate) use info::*;
pub(crate) use render::*;
pub(crate) use views::*;

/// Path key for the synthetic disk-root row: the empty string, distinct from
/// every real entry path and used as the "add to root" target.
const ROOT_PATH: &str = "";

/// A standard MSX disk size as a human label for the geometry-mismatch popup.
fn size_label(bytes: usize) -> String {
    match bytes {
        SIZE_360K => "360 kB (single-sided)".to_string(),
        SIZE_720K => "720 kB (double-sided)".to_string(),
        _ => format!("{} kB", bytes / 1024),
    }
}

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
pub(crate) enum ViewMode {
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
    Stats,
    Analyze,
    Blocks,
}

/// How the disk-usage Map is drawn: a flat sector grid, or a circular
/// disk-platter (concentric tracks, sector wedges, one circle per side).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MapStyle {
    Grid,
    Disk,
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
    /// CRC32 + SHA-1 of the bytes, computed once here (not per frame).
    checksums: msx_disk::Checksums,
    /// Content-derived file info for the Info pane, decoded on first view and
    /// reused across frames (not re-parsed every repaint).
    info: OnceCell<msx_disk::fileinfo::FileInfo>,
    /// The last rendered Text/Basic/Disasm listing, keyed by what it depends on.
    /// egui repaints many times per second; without this each repaint would
    /// re-detokenize/disassemble the whole file.
    rendered: RefCell<Option<(RenderKey, RenderedView)>>,
}

/// What a cached [`RenderedView`] depends on; a change recomputes the listing.
#[derive(Clone, Copy, PartialEq, Eq)]
struct RenderKey {
    mode: ViewMode,
    charset: MsxCharset,
    show_all: bool,
}

/// A decoded content listing plus an optional "showing first N kB" notice,
/// cached so it is produced once per [`RenderKey`] rather than every frame.
pub(crate) struct RenderedView {
    text: String,
    notice: Option<String>,
}

impl FileContent {
    /// The content-derived file info, decoded once and cached.
    fn file_info(&self) -> &msx_disk::fileinfo::FileInfo {
        self.info
            .get_or_init(|| msx_disk::fileinfo::describe(&self.path, &self.bytes))
    }

    /// Draw a text listing, recomputing it via `produce` only when `key` changes
    /// and otherwise reusing the cached string. Keeps immediate-mode repaints
    /// from re-decoding the whole file.
    fn show_listing(
        &self,
        ui: &mut egui::Ui,
        key: RenderKey,
        produce: impl FnOnce() -> RenderedView,
    ) {
        let mut cache = self.rendered.borrow_mut();
        let stale = match cache.as_ref() {
            Some((k, _)) => *k != key,
            None => true,
        };
        if stale {
            *cache = Some((key, produce()));
        }
        if let Some((_, view)) = cache.as_ref() {
            draw_listing(ui, view);
        }
    }
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
    /// Add host files into this directory (carries the target dir path; "" is
    /// the disk root).
    AddFiles(String),
    /// Create a new directory inside this directory (carries the parent path).
    AddDir(String),
    /// Remove this directory (carries the directory path).
    RemoveDir(String),
}

/// What the user did to a tree row this frame, collected by the row renderers
/// and applied back in `tree_panel` (the renderers only hold `&self` state).
#[derive(Default)]
pub(crate) struct RowEvents {
    /// A row was clicked to view it; the bool toggles multi-select (Cmd/Ctrl).
    clicked: Option<(String, bool)>,
    /// A directory header was clicked to toggle its expanded/collapsed state.
    toggle_dir: Option<String>,
    /// A row (file or directory) became the keyboard cursor via a click.
    cursor_to: Option<String>,
    /// A row began an OS drag-out (the dragged row's path).
    drag_started: Option<String>,
    /// A context-menu action was chosen on a row.
    action: Option<RowAction>,
    /// Screen rect of each rendered row paired with the directory a file
    /// dropped on it should be added to, for spatial drag-and-drop targeting.
    drop_targets: Vec<(egui::Rect, String)>,
}

/// A rename in progress: the file being renamed, the new name being typed (as a
/// decoded display name), and the charset snapshot used to decode it on open and
/// re-encode it on save (so a live charset change can't corrupt the key).
struct RenameTarget {
    path: String,
    name: String,
    charset: MsxCharset,
}

/// A new-directory dialog in progress: the parent directory it will be created
/// in ("" = disk root) and the name being typed.
struct NewDirTarget {
    parent: String,
    name: String,
}

/// A named byte offset the user can jump back to in the hex view.
struct Bookmark {
    offset: usize,
    name: String,
}

/// Per-hex-view interaction state: the byte cursor, an inclusive selection
/// range, the go-to-offset input, and bookmarks. Kept separately for the file
/// hex view and the sector hex view so they don't clobber each other.
#[derive(Default)]
pub(crate) struct HexUiState {
    /// The anchored byte (selection start / data-inspector origin).
    cursor: Option<usize>,
    /// The selected byte range, inclusive and normalized (`lo <= hi`).
    selection: Option<(usize, usize)>,
    /// Text in the go-to-offset box.
    goto_input: String,
    /// Named offsets the user has saved.
    bookmarks: Vec<Bookmark>,
    /// The byte where the current click-drag began.
    drag_anchor: Option<usize>,
    /// Which column the active selection was made in; decides whether
    /// Cmd/Ctrl+C copies the hex bytes or their ASCII rendering.
    region: HexRegion,
}

impl HexUiState {
    /// Clear the transient selection/cursor (keeps bookmarks and goto input).
    fn clear_selection(&mut self) {
        self.cursor = None;
        self.selection = None;
        self.drag_anchor = None;
    }
}

/// A read-only snapshot of a [`HexUiState`] passed to [`render_hex`] for
/// painting the selection and cursor.
#[derive(Clone, Copy, Default)]
pub(crate) struct HexSelection {
    cursor: Option<usize>,
    selection: Option<(usize, usize)>,
}

/// A pointer gesture recognized in the hex view, applied by the caller to its
/// [`HexUiState`].
pub(crate) enum HexGesture {
    /// A click-drag began at this byte, in the given column.
    DragStart { byte: usize, region: HexRegion },
    /// A click-drag extended to this byte.
    DragTo(usize),
    /// A click landed on this byte; `shift` extends the selection from the
    /// cursor. `region` records which column was clicked.
    Click {
        byte: usize,
        shift: bool,
        region: HexRegion,
    },
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
    /// How the Map tab is drawn (sector grid vs circular disk platter).
    map_style: MapStyle,
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
    /// Hex power-inspection state for the file viewer's Hex tab.
    hex: HexUiState,
    /// Hex power-inspection state for the Sectors view.
    sector_hex: HexUiState,
    /// Whether the data inspector panel is shown beneath the hex views.
    show_inspector: bool,
    /// Whether the About window is open.
    show_about: bool,
    /// Whether the "New disk" type-chooser popup is open.
    show_new_disk: bool,
    /// Cached app-icon texture for the About window, decoded on first open.
    about_icon: Option<egui::TextureHandle>,
    /// Persisted settings: recent files and hex-view display options.
    settings: Settings,
    /// Native macOS menu handles, built once at startup. `None` until the menu
    /// is created (and always on non-macOS, which uses an in-window menu bar).
    #[cfg(target_os = "macos")]
    mac_menu: Option<crate::macos::MacMenu>,
    /// Highlighted tree row (file or directory) for keyboard navigation. Kept
    /// separate from [`selected`](Self::selected), which is always a file.
    cursor: Option<String>,
    /// Directory paths the user has collapsed in the tree; empty means all
    /// expanded. This is the source of truth for the tree's open/closed state.
    collapsed: BTreeSet<String>,
    /// One-shot request to scroll the cursor row into view after a key move.
    scroll_to_cursor: bool,
    /// New-directory dialog state, when the user is naming a directory to add.
    new_dir: Option<NewDirTarget>,
    /// A detected boot-sector/image-size mismatch awaiting the user's decision.
    size_fix: Option<msx_disk::fs::sizefix::SizeMismatch>,
    /// Last frame's tree-row rects with their add-target directories, used to
    /// resolve which folder a drag-and-drop landed on.
    drop_targets: Vec<(egui::Rect, String)>,
}

/// Application version (from Cargo.toml), shown in the toolbar and About window.
const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Short git hash this binary was built from; empty when built without a git
/// checkout. Set by `build.rs`.
const GIT_HASH: &str = match option_env!("GIT_HASH") {
    Some(hash) => hash,
    None => "",
};

/// Commit date (`YYYY-MM-DD`) of the build; empty when built without a git
/// checkout. Set by `build.rs`.
const BUILD_DATE: &str = match option_env!("BUILD_DATE") {
    Some(date) => date,
    None => "",
};

/// App icon (256x256 PNG): decoded lazily for the in-app About window, and on
/// macOS also used as the application/About-panel icon (see `macos` module).
pub(crate) const ICON_PNG: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/assets/icons/128x128@2x.png"
));

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
            map_style: MapStyle::Grid,
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
            hex: HexUiState::default(),
            sector_hex: HexUiState::default(),
            show_inspector: false,
            show_about: false,
            show_new_disk: false,
            about_icon: None,
            settings: Settings::default(),
            #[cfg(target_os = "macos")]
            mac_menu: None,
            cursor: None,
            collapsed: BTreeSet::new(),
            scroll_to_cursor: false,
            new_dir: None,
            size_fix: None,
            drop_targets: Vec::new(),
        }
    }
}

impl eframe::App for MediaExplorerApp {
    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        // Another instance may have saved since this one loaded; union its
        // recent-files list (best-effort) so concurrent instances don't
        // clobber each other's. Other prefs stay last-writer-wins.
        if let Some(disk) = crate::settings::read_from_storage_file(crate::APP_NAME) {
            self.settings.merge_recent(&disk.recent);
        }
        eframe::set_value(storage, SETTINGS_KEY, &self.settings);
    }

    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        // macOS: act on any native menu item activated since the last frame.
        #[cfg(target_os = "macos")]
        for id in crate::macos::take_menu_events() {
            self.handle_menu_event(&id);
        }
        // Keep the native Text Encoding menu's checks/enabled state in sync; the
        // charset can change outside a menu event (auto-detect on disk load).
        #[cfg(target_os = "macos")]
        if let Some(menu) = self.mac_menu.as_ref() {
            menu.sync_encoding(self.charset, self.charset_auto, self.disk.is_some());
            menu.sync_save_items(
                self.disk
                    .as_ref()
                    .is_some_and(LoadedDisk::can_convert_to_dsk),
                self.disk
                    .as_ref()
                    .is_some_and(LoadedDisk::can_convert_to_xsa),
            );
        }

        self.handle_dropped_files(ui.ctx());
        self.handle_tree_keys(ui.ctx());

        // Non-macOS: an in-window menu bar driving the same actions/state (the
        // native bar is used on macOS instead).
        #[cfg(not(target_os = "macos"))]
        egui::Panel::top("menubar").show_inside(ui, |ui| {
            self.menu_bar(ui);
        });

        egui::Panel::top("toolbar").show_inside(ui, |ui| {
            ui.add_space(4.0);
            self.toolbar(ui);
            ui.add_space(4.0);
        });
        if self.settings.show_status_bar {
            egui::Panel::bottom("status").show_inside(ui, |ui| {
                ui.add_space(2.0);
                self.status_bar(ui);
                ui.add_space(2.0);
            });
        }
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
            AppView::Stats => self.stats_panel(ui),
            AppView::Analyze => self.analyze_panel(ui),
            AppView::Blocks => self.blocks_panel(ui),
        });

        self.delete_confirmation(ui.ctx());
        self.rename_dialog(ui.ctx());
        self.new_dir_dialog(ui.ctx());
        self.new_disk_dialog(ui.ctx());
        self.size_fix_dialog(ui.ctx());
        self.about_dialog(ui.ctx());

        #[cfg(any(target_os = "macos", target_os = "windows"))]
        self.process_drag_out(frame);
        #[cfg(not(any(target_os = "macos", target_os = "windows")))]
        let _ = frame;
    }
}
