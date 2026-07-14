use super::*;

impl MediaExplorerApp {
    /// Build the app, restoring persisted settings and (on macOS) installing
    /// the native menu bar.
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let mut app = Self::default();
        if let Some(storage) = cc.storage {
            if let Some(settings) = eframe::get_value::<Settings>(storage, SETTINGS_KEY) {
                app.settings = settings;
            }
        }
        // Set the active UI locale before any strings are produced (the macOS
        // menu below is built from it): the explicit choice, else the OS locale,
        // else English.
        rust_i18n::set_locale(app.active_language().code());
        #[cfg(target_os = "macos")]
        {
            app.mac_menu = Some(crate::macos::build_menu(&cc.egui_ctx, &app.settings));
        }
        // Sweep drag-staging dirs abandoned by earlier runs (per-pid, so a
        // second live instance is never touched).
        #[cfg(any(target_os = "macos", target_os = "windows"))]
        super::transfer::clean_stale_drag_staging();
        app
    }

    pub(crate) fn open_path(&mut self, path: &Path) {
        if is_tape(path) {
            self.open_tape(path);
        } else {
            self.open_disk(path);
        }
    }

    /// Reset the per-document state shared by disk and tape opens.
    pub(crate) fn reset_document(&mut self) {
        self.disk = None;
        self.tape = None;
        self.dmk_analysis = None;
        self.disk_map = None;
        self.disk_fs_geometry = None;
        self.selected = None;
        self.selection.clear();
        self.cursor = None;
        self.collapsed.clear();
        self.filter.clear();
        self.scroll_to_cursor = false;
        self.type_ahead.clear();
        self.new_dir = None;
        self.size_fix = None;
        self.drop_targets.clear();
        self.content = None;
        self.archive = None;
        self.current_sector = 0;
        self.sector_edit = None;
        self.app_view = AppView::Files;
        self.hex = HexUiState::default();
        self.sector_hex = HexUiState::default();
    }

    pub(crate) fn open_disk(&mut self, path: &Path) {
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
                // Flag a boot-sector/image-size mismatch for the user to repair.
                self.size_fix = self.disk.as_ref().and_then(LoadedDisk::size_mismatch);
                self.record_recent(path);
            }
            Err(e) => self.status = format!("Failed to open {}: {e}", path.display()),
        }
    }

    /// Seed the display charset from the open disk's filenames, unless the user
    /// has pinned it via the dropdown. Best-effort; the dropdown always wins.
    pub(crate) fn autodetect_charset(&mut self) {
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

    pub(crate) fn open_tape(&mut self, path: &Path) {
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
                self.record_recent(path);
            }
            Err(e) => self.status = format!("Failed to open {}: {e}", path.display()),
        }
    }

    /// Read a file's bytes from whichever document is open (disk or tape).
    pub(crate) fn read_doc_file(&self, path: &str) -> Option<Vec<u8>> {
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
    pub(crate) fn select_file(&mut self, path: String, toggle: bool) {
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
        // A byte selection / bookmarks are meaningless across files.
        self.hex = HexUiState::default();
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
        let checksums = msx_disk::Checksums::of(&bytes);
        self.content = Some(FileContent {
            path: path.clone(),
            bytes,
            checksums,
            info: OnceCell::new(),
            rendered: RefCell::new(None),
        });
        self.selected = Some(path);
    }

    /// The files targeted by batch operations: the multi-selection, or the
    /// single viewed file when nothing is explicitly marked.
    pub(crate) fn selection_paths(&self) -> Vec<String> {
        if self.selection.is_empty() {
            self.selected.iter().cloned().collect()
        } else {
            self.selection.iter().cloned().collect()
        }
    }

    /// Files a row-level action targets: the whole multi-selection when the
    /// clicked row is part of it, otherwise just that row. Shared by the
    /// right-click menu and OS drag-out so both follow the same rule.
    pub(crate) fn paths_for_row(&self, path: &str) -> Vec<String> {
        if self.selection.len() > 1 && self.selection.contains(path) {
            self.selection_paths()
        } else {
            vec![path.to_string()]
        }
    }

    pub(crate) fn handle_dropped_files(&mut self, ctx: &egui::Context) {
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
            // Spatial drop: add into whichever folder row the pointer is over
            // (a file row → its folder, the "/" root or empty space → root).
            let pos = ctx.input(|i| i.pointer.hover_pos().or_else(|| i.pointer.interact_pos()));
            let target = pos
                .and_then(|p| drop_target_at(&self.drop_targets, p))
                .unwrap_or_default();
            self.add_paths_into(&paths, &target);
        } else {
            self.open_path(&paths[0]);
        }
    }
}
