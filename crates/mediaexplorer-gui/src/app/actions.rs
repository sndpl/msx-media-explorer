use super::*;

impl MediaExplorerApp {
    /// Run the Find for the current view: raw file bytes in the Hex view, the
    /// rendered listing in Text/BASIC/Disasm (so matches land in what the user
    /// is actually reading). The other views have no Find UI.
    pub(crate) fn run_search(&mut self) {
        match self.view_mode {
            ViewMode::Hex => self.run_hex_search(),
            ViewMode::Text | ViewMode::Basic | ViewMode::Disasm => self.run_listing_search(),
            ViewMode::Info | ViewMode::Screen | ViewMode::Archive => {}
        }
    }

    fn run_hex_search(&mut self) {
        let Some(content) = &self.content else { return };
        let matches = if self.search_is_hex {
            match search::parse_hex(&self.search_query) {
                Some(needle) => search::find_bytes(&content.bytes, &needle),
                None => {
                    self.status = t!("disk.invalid_hex_pattern").to_string();
                    return;
                }
            }
        } else {
            search::find_text(&content.bytes, &self.search_query, true)
        };
        self.search_match_lines.clear();
        if matches.is_empty() {
            self.status = t!("status.no_matches", query => self.search_query.clone()).to_string();
            self.search_matches.clear();
            return;
        }
        self.status = tn!("status.matches", matches.len()).to_string();
        self.search_matches = matches;
        self.search_pos = 0;
        self.jump_to_current_match();
    }

    fn run_listing_search(&mut self) {
        let Some(text) = self.current_listing_text() else {
            return;
        };
        let matches = search::find_text(text.as_bytes(), &self.search_query, true);
        if matches.is_empty() {
            self.status = t!("status.no_matches", query => self.search_query.clone()).to_string();
            self.search_matches.clear();
            self.search_match_lines.clear();
            return;
        }
        self.status = tn!("status.matches", matches.len()).to_string();
        self.search_match_len = self.search_query.len();
        self.search_match_lines = matches
            .iter()
            .map(|&o| text[..o].bytes().filter(|&b| b == b'\n').count())
            .collect();
        self.search_matches = matches;
        self.search_pos = 0;
        self.jump_to_current_match();
    }

    /// The text the current listing view renders, produced with the same
    /// functions the viewer uses (so match offsets line up exactly).
    fn current_listing_text(&self) -> Option<String> {
        let content = self.content.as_ref()?;
        Some(match self.view_mode {
            ViewMode::Text => produce_text(&content.bytes, self.text_show_all, self.charset).text,
            ViewMode::Basic => produce_basic(&content.bytes, self.charset).text,
            ViewMode::Disasm => produce_disasm(&content.path, &content.bytes).text,
            _ => return None,
        })
    }

    pub(crate) fn step_search(&mut self, forward: bool) {
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

    pub(crate) fn jump_to_current_match(&mut self) {
        if let Some(&offset) = self.search_matches.get(self.search_pos) {
            self.pending_scroll_row = Some(if self.view_mode == ViewMode::Hex {
                offset / self.settings.hex.bytes_per_row.max(1)
            } else {
                // Listing views scroll by line (precomputed per match).
                self.search_match_lines
                    .get(self.search_pos)
                    .copied()
                    .unwrap_or(0)
            });
        }
    }

    pub(crate) fn disk_writable(&self) -> bool {
        self.disk
            .as_ref()
            .map(LoadedDisk::writable)
            .unwrap_or(false)
    }

    /// The active UI language: the user's explicit choice, else the OS locale,
    /// else English.
    pub(crate) fn active_language(&self) -> Lang {
        self.settings.language.unwrap_or_else(|| {
            sys_locale::get_locale()
                .and_then(|tag| Lang::from_locale_tag(&tag))
                .unwrap_or(Lang::English)
        })
    }

    /// Switch the UI language: persist the choice, apply it, and (on macOS)
    /// rebuild the native menu, whose labels are baked at build time. The
    /// in-window egui menu re-renders itself, so it needs nothing here.
    pub(crate) fn set_language(&mut self, lang: Lang, ctx: &egui::Context) {
        if self.settings.language == Some(lang) {
            return;
        }
        self.settings.language = Some(lang);
        rust_i18n::set_locale(lang.code());
        #[cfg(target_os = "macos")]
        {
            self.mac_menu = Some(crate::macos::build_menu(ctx, &self.settings));
        }
        ctx.request_repaint();
    }

    /// Update state after a write operation completes.
    pub(crate) fn after_mutation(&mut self, result: msx_disk::Result<()>, ok_msg: String) {
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
            Err(e) => self.status = t!("status.write_failed", error => e).to_string(),
        }
    }

    pub(crate) fn new_disk(&mut self, format: DiskFormat) {
        // New disks get a real MSX-DOS 2 boot block (VOL_ID + serial), so
        // MSX-DOS 2's UNDEL and disk cache work; DOS 1 machines boot it too.
        let bytes = match msx_disk::fs::write::create_blank(format).and_then(|blank| {
            msx_disk::fs::write::set_boot_block(
                &blank,
                msx_disk::fs::DosVersion::Dos2,
                new_disk_volume_serial(),
            )
        }) {
            Ok(b) => b,
            Err(e) => {
                self.status = t!("status.create_disk_failed", error => e).to_string();
                return;
            }
        };
        let default = format.default_file_name();
        if let Some(path) = rfd::FileDialog::new().set_file_name(default).save_file() {
            match std::fs::write(&path, &bytes) {
                Ok(()) => self.open_path(&path),
                Err(e) => {
                    self.status = t!("status.write_file_failed", path => path.display(), error => e)
                        .to_string()
                }
            }
        }
    }

    /// Launch a detached second instance of the app, so two disks can be open
    /// side by side (e.g. to drag files from one to the other). Spawning the
    /// current executable works both for the dev binary and from inside a
    /// packaged `.app`; the child outlives the dropped handle.
    pub(crate) fn open_new_window(&mut self) {
        let spawned = std::env::current_exe()
            .and_then(|exe| std::process::Command::new(exe).spawn().map(|_| ()));
        if let Err(e) = spawned {
            self.status = t!("status.new_window_failed", error => e).to_string();
        }
    }

    /// Open the system file picker for a disk or tape image.
    ///
    /// Images in the wild often have no extension or an unrecognized one
    /// (`DiskImage::open` falls back to magic-byte sniffing), so the dialog
    /// must never make files unselectable. On Windows and Linux an
    /// "All files" filter is offered alongside the MSX ones (rfd maps `*` to
    /// `*.*` / a match-all glob there). On macOS rfd merges every filter's
    /// extensions into a single `setAllowedFileTypes:` list — `*` is not a
    /// wildcard in that API, and there is no filter dropdown to switch to —
    /// so the panel is left unrestricted instead.
    pub(crate) fn open_dialog(&mut self) {
        let dialog = rfd::FileDialog::new();
        #[cfg(not(target_os = "macos"))]
        let dialog = dialog
            .add_filter(t!("file_dialog.disk_images"), DISK_IMAGE_EXTS)
            .add_filter(t!("file_dialog.tape_images"), TAPE_EXTS)
            .add_filter(t!("file_dialog.all_files"), &["*"]);
        if let Some(path) = dialog.pick_file() {
            self.open_path(&path);
        }
    }

    /// Record a successfully opened image in the recent-files list and refresh
    /// the native Recent submenu.
    pub(crate) fn record_recent(&mut self, path: &Path) {
        self.settings.push_recent(path);
        self.refresh_recent_menu();
    }

    /// Open the recent-files entry at `index`, pruning it if it has since gone
    /// missing.
    pub(crate) fn open_recent(&mut self, index: usize) {
        let Some(path) = self.settings.recent.get(index).cloned() else {
            return;
        };
        if path.exists() {
            self.open_path(&path);
        } else {
            self.status = t!("status.file_gone", path => path.display()).to_string();
            self.settings.recent.retain(|p| p != &path);
            self.refresh_recent_menu();
        }
    }

    /// Forget all recent files.
    pub(crate) fn clear_recent(&mut self) {
        self.settings.clear_recent();
        self.refresh_recent_menu();
    }

    /// Rebuild the native Recent submenu from the current list (no-op when the
    /// native menu is absent).
    pub(crate) fn refresh_recent_menu(&mut self) {
        #[cfg(target_os = "macos")]
        if let Some(menu) = self.mac_menu.as_mut() {
            menu.rebuild_recent(&self.settings.recent);
        }
    }

    /// Close the open disk/tape, returning to the empty state.
    pub(crate) fn close_document(&mut self) {
        self.reset_document();
        self.status = t!("status.get_started").to_string();
        self.search_query.clear();
        self.search_matches.clear();
        self.search_pos = 0;
    }

    /// The "New disk" format chooser: pick one of the standard MSX floppy
    /// formats, then fall through to [`new_disk`](Self::new_disk) (save dialog
    /// + open).
    pub(crate) fn new_disk_dialog(&mut self, ctx: &egui::Context) {
        if !self.show_new_disk {
            return;
        }
        let mut choice: Option<DiskFormat> = None;
        let mut cancel = ctx.input(|i| i.key_pressed(egui::Key::Escape));
        egui::Window::new(t!("dialog.new_disk_title"))
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                ui.label(t!("dialog.new_disk_prompt"));
                ui.add_space(8.0);
                // Format names are technical MSX sizes — not localized.
                for format in DiskFormat::ALL {
                    if ui.button(format.label()).clicked() {
                        choice = Some(format);
                    }
                }
                ui.add_space(8.0);
                cancel |= ui.button(t!("button.cancel")).clicked();
            });
        if let Some(format) = choice {
            self.show_new_disk = false;
            self.new_disk(format);
        } else if cancel {
            self.show_new_disk = false;
        }
    }

    /// Act on a native macOS menu item, by its id, then reflect any toggle
    /// changes back into the menu's checkmarks.
    #[cfg(target_os = "macos")]
    pub(crate) fn handle_menu_event(&mut self, id: &str, ctx: &egui::Context) {
        match id {
            "app.about" => self.show_about = true,
            "help.shortcuts" => self.show_shortcuts = true,
            "file.new" => self.show_new_disk = true,
            "file.new_window" => self.open_new_window(),
            "file.open" => self.open_dialog(),
            "file.save_dsk" => self.save_as_dsk(),
            "file.save_xsa" => self.save_as_xsa(),
            "file.save_sav" => self.save_as_sav(),
            "file.close" => self.close_document(),
            "recent.clear" => self.clear_recent(),
            "view.line_numbers" => self.settings.hex.show_line_numbers ^= true,
            "view.hex" => self.settings.hex.show_hex ^= true,
            "view.ascii" => self.settings.hex.show_ascii ^= true,
            "view.status_bar" => self.settings.show_status_bar ^= true,
            "view.columns" => self.settings.hex.show_columns ^= true,
            "view.hide_nulls" => self.settings.hex.hide_null_bytes ^= true,
            "view.lnf.hex" => self.settings.hex.line_number_hex = true,
            "view.lnf.dec" => self.settings.hex.line_number_hex = false,
            "view.group.none" => self.settings.hex.grouping = ByteGrouping::None,
            "encoding.auto" => {
                self.charset_auto = true;
                self.autodetect_charset();
            }
            _ if id.starts_with("encoding.") => {
                if let Some(i) = id
                    .strip_prefix("encoding.")
                    .and_then(|n| n.parse::<usize>().ok())
                {
                    if let Some(&cs) = MsxCharset::ALL.get(i) {
                        self.charset = cs;
                        self.charset_auto = false;
                    }
                }
            }
            _ if id.starts_with("lang.") => {
                if let Some(&lang) = id
                    .strip_prefix("lang.")
                    .and_then(|n| n.parse::<usize>().ok())
                    .and_then(|i| Lang::ALL.get(i))
                {
                    self.set_language(lang, ctx);
                }
            }
            _ if id.starts_with("recent.") => {
                if let Some(i) = id.strip_prefix("recent.").and_then(|n| n.parse().ok()) {
                    self.open_recent(i);
                }
            }
            _ if id.starts_with("view.bpr.") => {
                if let Some(n) = id.strip_prefix("view.bpr.").and_then(|n| n.parse().ok()) {
                    self.settings.hex.bytes_per_row = n;
                }
            }
            _ if id.starts_with("view.group.") => {
                if let Some(n) = id.strip_prefix("view.group.").and_then(|n| n.parse().ok()) {
                    self.settings.hex.grouping = ByteGrouping::Of(n);
                }
            }
            _ => {}
        }
        if let Some(menu) = self.mac_menu.as_ref() {
            menu.sync_checks(&self.settings);
        }
    }

    /// In-window menu bar for platforms without a native one (non-macOS). Drives
    /// the same settings and actions as the native menu.
    #[cfg_attr(target_os = "macos", allow(dead_code))]
    pub(crate) fn menu_bar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.menu_button(t!("menu.file"), |ui| {
                if ui.button(t!("menu.new_disk")).clicked() {
                    self.show_new_disk = true;
                    ui.close();
                }
                if ui.button(t!("menu.new_window")).clicked() {
                    self.open_new_window();
                    ui.close();
                }
                if ui.button(t!("menu.open")).clicked() {
                    self.open_dialog();
                    ui.close();
                }
                ui.menu_button(t!("menu.open_recent"), |ui| {
                    if self.settings.recent.is_empty() {
                        ui.add_enabled(false, egui::Button::new(t!("menu.no_recent_files")));
                    } else {
                        // Collect the click during the borrow of `recent`, then act
                        // after it ends — avoids cloning the list every frame.
                        let mut chosen: Option<usize> = None;
                        for (i, path) in self.settings.recent.iter().enumerate() {
                            let label = path
                                .file_name()
                                .map(|n| n.to_string_lossy().into_owned())
                                .unwrap_or_else(|| path.to_string_lossy().into_owned());
                            if ui.button(label).clicked() {
                                chosen = Some(i);
                                ui.close();
                            }
                        }
                        ui.separator();
                        let clear = ui.button(t!("menu.clear_menu")).clicked();
                        if clear {
                            ui.close();
                        }
                        if let Some(i) = chosen {
                            self.open_recent(i);
                        } else if clear {
                            self.clear_recent();
                        }
                    }
                });
                ui.separator();
                // Save-as is format conversion, so each is only offered when it
                // would change the format (and never for a partitioned HD image).
                let can_dsk = self
                    .disk
                    .as_ref()
                    .is_some_and(LoadedDisk::can_convert_to_dsk);
                if ui
                    .add_enabled(can_dsk, egui::Button::new(t!("menu.save_as_dsk")))
                    .clicked()
                {
                    self.save_as_dsk();
                    ui.close();
                }
                let can_xsa = self
                    .disk
                    .as_ref()
                    .is_some_and(LoadedDisk::can_convert_to_xsa);
                if ui
                    .add_enabled(can_xsa, egui::Button::new(t!("menu.save_as_xsa")))
                    .clicked()
                {
                    self.save_as_xsa();
                    ui.close();
                }
                let can_sav = self
                    .disk
                    .as_ref()
                    .is_some_and(LoadedDisk::can_convert_to_sav);
                if ui
                    .add_enabled(can_sav, egui::Button::new(t!("menu.save_as_sav")))
                    .clicked()
                {
                    self.save_as_sav();
                    ui.close();
                }
                ui.separator();
                if ui.button(t!("menu.close")).clicked() {
                    self.close_document();
                    ui.close();
                }
            });
            ui.menu_button(t!("menu.view"), |ui| {
                ui.checkbox(
                    &mut self.settings.hex.show_line_numbers,
                    t!("menu.line_numbers"),
                );
                ui.checkbox(&mut self.settings.hex.show_hex, t!("menu.hexadecimal"));
                ui.checkbox(&mut self.settings.hex.show_ascii, t!("menu.plain_text"));
                ui.checkbox(&mut self.settings.show_status_bar, t!("menu.status_bar"));
                ui.checkbox(&mut self.settings.hex.show_columns, t!("menu.columns"));
                ui.separator();
                ui.menu_button(t!("menu.bytes_per_row"), |ui| {
                    for n in crate::settings::ROW_SIZES {
                        ui.radio_value(&mut self.settings.hex.bytes_per_row, n, n.to_string());
                    }
                });
                ui.menu_button(t!("menu.line_number_format"), |ui| {
                    ui.radio_value(
                        &mut self.settings.hex.line_number_hex,
                        false,
                        t!("menu.decimal"),
                    );
                    ui.radio_value(
                        &mut self.settings.hex.line_number_hex,
                        true,
                        t!("menu.hexadecimal"),
                    );
                });
                ui.menu_button(t!("menu.byte_grouping"), |ui| {
                    ui.radio_value(
                        &mut self.settings.hex.grouping,
                        ByteGrouping::None,
                        t!("menu.none"),
                    );
                    for n in ByteGrouping::SIZES {
                        ui.radio_value(
                            &mut self.settings.hex.grouping,
                            ByteGrouping::Of(n),
                            n.to_string(),
                        );
                    }
                });
                ui.separator();
                ui.checkbox(
                    &mut self.settings.hex.hide_null_bytes,
                    t!("menu.hide_null_bytes"),
                );
            });
            ui.menu_button(t!("menu.text_encoding"), |ui| {
                // The charset only applies to an open disk; mirror the old
                // dropdown's "only when a disk is loaded" gating.
                ui.add_enabled_ui(self.disk.is_some(), |ui| {
                    if ui.radio(self.charset_auto, t!("menu.automatic")).clicked() {
                        self.charset_auto = true;
                        self.autodetect_charset();
                        ui.close();
                    }
                    ui.separator();
                    // MSX code-page names are proper region names — not localized.
                    for &cs in MsxCharset::ALL {
                        let selected = !self.charset_auto && self.charset == cs;
                        if ui.selectable_label(selected, cs.label()).clicked() {
                            self.charset = cs;
                            self.charset_auto = false;
                            ui.close();
                        }
                    }
                });
            });
            ui.menu_button(t!("menu.language"), |ui| {
                let active = self.active_language();
                for &lang in Lang::ALL {
                    // Language names are shown in their own language, as usual.
                    if ui
                        .selectable_label(active == lang, lang.native_name())
                        .clicked()
                    {
                        self.set_language(lang, ui.ctx());
                        ui.close();
                    }
                }
            });
            ui.menu_button(t!("menu.help"), |ui| {
                if ui.button(t!("menu.keyboard_shortcuts")).clicked() {
                    self.show_shortcuts = true;
                    ui.close();
                }
                if ui.button(t!("menu.about")).clicked() {
                    self.show_about = true;
                    ui.close();
                }
            });
        });
    }

    pub(crate) fn save_as_dsk(&mut self) {
        self.save_converted("dsk", |d| Ok(d.to_dsk_bytes()));
    }

    pub(crate) fn save_as_xsa(&mut self) {
        self.save_converted("xsa", |d| Ok(d.to_xsa_bytes()));
    }

    pub(crate) fn save_as_sav(&mut self) {
        self.save_converted("sav", LoadedDisk::to_sav_bytes);
    }

    /// Shared helper for "Save as <ext>": derive a default name, run `encode`,
    /// and write to a chosen path.
    pub(crate) fn save_converted(
        &mut self,
        ext: &str,
        encode: fn(&LoadedDisk) -> msx_disk::Result<Vec<u8>>,
    ) {
        let Some((encoded, default)) = self.disk.as_ref().map(|d| {
            let title = d.title();
            let stem = title.rsplit_once('.').map(|(s, _)| s).unwrap_or(&title);
            (encode(d), format!("{stem}.{ext}"))
        }) else {
            return;
        };
        let bytes = match encoded {
            Ok(b) => b,
            Err(e) => {
                self.status = t!("status.convert_failed", ext => ext, error => e).to_string();
                return;
            }
        };
        if let Some(path) = rfd::FileDialog::new().set_file_name(&default).save_file() {
            self.status = match std::fs::write(&path, &bytes) {
                Ok(()) => t!("status.saved", path => path.display()).to_string(),
                Err(e) => t!("status.save_failed", error => e).to_string(),
            };
        }
    }

    /// Pick host files and add them into `target` ("" = disk root), from the
    /// tree's "Add files here…" menu.
    pub(crate) fn add_files_into(&mut self, target: &str) {
        if !self.disk_writable() {
            self.status = t!("status.read_only_image").to_string();
            return;
        }
        if let Some(paths) = rfd::FileDialog::new().pick_files() {
            self.add_paths_into(&paths, target);
        }
    }

    /// Read each host path and add it into `target` ("" = disk root) under a
    /// sanitized 8.3 name. Used by the right-click menu and drag-in.
    pub(crate) fn add_paths_into(&mut self, paths: &[PathBuf], target: &str) {
        if !self.disk_writable() {
            self.status = t!("status.read_only_image").to_string();
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
                    files.push((child_path(target, &sanitize_msx_name(&raw)), bytes));
                }
                Err(e) => {
                    self.status =
                        t!("status.could_not_read", path => path.display(), error => e).to_string();
                    return;
                }
            }
        }
        let count = files.len();
        let result = self.disk.as_mut().unwrap().add_files(&files);
        let dest = if target.is_empty() { "/" } else { target };
        let msg = tn!("status.added_files", count, dest => dest).to_string();
        self.after_mutation(result, msg);
    }

    /// Remove a directory: empty ones go immediately; non-empty ones open the
    /// confirmation modal with the recursive (descendants-first) delete list.
    pub(crate) fn remove_directory(&mut self, path: &str) {
        let plan = self
            .entry_for_path(path)
            .map(|dir| (dir.children.is_empty(), removal_paths(dir)));
        let Some((empty, paths)) = plan else {
            return;
        };
        if empty {
            let result = self.disk.as_mut().unwrap().delete(&[path.to_string()]);
            self.after_mutation(result, t!("status.removed", path => path).to_string());
        } else {
            self.confirm_delete = Some(paths);
        }
    }

    /// Create the directory named in the new-directory dialog. The typed name is
    /// normalized to an 8.3 display name and re-encoded to its on-disk (PUA) key,
    /// mirroring [`apply_rename`](Self::apply_rename) so kana names round-trip.
    pub(crate) fn apply_new_dir(&mut self) {
        let Some(target) = self.new_dir.take() else {
            return;
        };
        let charset = self.charset;
        let display = sanitize_8_3(&target.name, |c| is_rename_char(c, charset));
        if msx_name_stem(&display).is_empty() {
            self.status = t!("status.enter_dir_name").to_string();
            return;
        }
        let base = charset::encode_fs_name(charset, &display).unwrap_or_else(|| display.clone());
        let path = child_path(&target.parent, &base);
        let result = self.disk.as_mut().unwrap().create_dir(&path);
        self.after_mutation(
            result,
            t!("status.created_directory", name => display).to_string(),
        );
    }

    pub(crate) fn apply_rename(&mut self) {
        let Some(target) = self.rename_target.take() else {
            return;
        };
        // Sanitize the decoded name, then re-encode it to the PUA fatfs key that
        // `rename` expects; the status message keeps the readable form.
        let display = sanitize_8_3(&target.name, |c| is_rename_char(c, target.charset));
        let new_base =
            charset::encode_fs_name(target.charset, &display).unwrap_or_else(|| display.clone());
        let new_path = match target.path.rsplit_once('/') {
            Some((parent, _)) => format!("{parent}/{new_base}"),
            None => new_base,
        };
        let result = self.disk.as_mut().unwrap().rename(&target.path, &new_path);
        self.after_mutation(result, t!("status.renamed_to", name => display).to_string());
    }

    pub(crate) fn confirm_delete_now(&mut self) {
        let Some(paths) = self.confirm_delete.take() else {
            return;
        };
        if paths.is_empty() {
            return;
        }
        let msg = tn!(
            "status.deleted",
            paths.len(),
            name => paths.first().map(String::as_str).unwrap_or_default()
        )
        .to_string();
        let result = self.disk.as_mut().unwrap().delete(&paths);
        self.after_mutation(result, msg);
    }

    pub(crate) fn save_hex_edit(&mut self) {
        let (Some(edited), Some(path)) = (self.hex_edit.clone(), self.selected.clone()) else {
            return;
        };
        let Some(bytes) = search::parse_hex(&edited) else {
            self.status = t!("status.invalid_hex_bytes").to_string();
            return;
        };
        let result = self
            .disk
            .as_mut()
            .unwrap()
            .add_files(&[(path.clone(), bytes)]);
        self.after_mutation(result, t!("status.saved_edits", path => path).to_string());
    }

    /// Render the delete-confirmation modal if a deletion is pending.
    pub(crate) fn delete_confirmation(&mut self, ctx: &egui::Context) {
        let Some(paths) = self.confirm_delete.clone() else {
            return;
        };
        if paths.is_empty() {
            self.confirm_delete = None;
            return;
        }
        // Enter confirms, Escape cancels (the buttons stay for the mouse).
        let mut do_delete = ctx.input(|i| i.key_pressed(egui::Key::Enter));
        let mut cancel = ctx.input(|i| i.key_pressed(egui::Key::Escape));
        // Decode names for display only; the stored `paths` (PUA-encoded) stay
        // the keys used for the actual delete.
        let charset = self.charset;
        egui::Window::new(t!("dialog.confirm_delete_title"))
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                if let [path] = paths.as_slice() {
                    let shown = charset::decode_fs_name(charset, path);
                    ui.label(t!("dialog.delete_one", name => shown));
                } else {
                    ui.label(t!("dialog.delete_many", count => paths.len()));
                    egui::ScrollArea::vertical()
                        .max_height(160.0)
                        .show(ui, |ui| {
                            for path in &paths {
                                ui.monospace(charset::decode_fs_name(charset, path));
                            }
                        });
                }
                ui.label(t!("dialog.delete_rewrites"));
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if ui.button(t!("button.delete")).clicked() {
                        do_delete = true;
                    }
                    if ui.button(t!("button.cancel")).clicked() {
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
    pub(crate) fn rename_dialog(&mut self, ctx: &egui::Context) {
        let Some(target) = self.rename_target.as_mut() else {
            return;
        };
        let charset = target.charset;
        let old = charset::decode_fs_name(charset, &base_name(&target.path));
        let mut apply = false;
        let mut cancel = ctx.input(|i| i.key_pressed(egui::Key::Escape));
        egui::Window::new(t!("dialog.rename_title"))
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                ui.label(t!("dialog.rename_prompt", name => old));
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
                ui.small(t!("dialog.name_hint"));
                let valid = !msx_name_stem(&target.name).is_empty();
                let enter =
                    valid && resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    apply = ui
                        .add_enabled(valid, egui::Button::new(t!("button.rename")))
                        .clicked();
                    cancel |= ui.button(t!("button.cancel")).clicked();
                });
                apply |= enter;
            });
        if apply {
            self.apply_rename();
        } else if cancel {
            self.rename_target = None;
        }
    }

    /// Render the new-directory modal if one is pending (from the tree's
    /// right-click "Add directory…").
    pub(crate) fn new_dir_dialog(&mut self, ctx: &egui::Context) {
        let charset = self.charset;
        let Some(target) = self.new_dir.as_mut() else {
            return;
        };
        let parent_label = if target.parent.is_empty() {
            "/".to_string()
        } else {
            charset::decode_fs_name(charset, &target.parent)
        };
        let mut apply = false;
        let mut cancel = ctx.input(|i| i.key_pressed(egui::Key::Escape));
        egui::Window::new(t!("dialog.new_dir_title"))
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                ui.label(t!("dialog.new_dir_prompt", parent => parent_label));
                let resp =
                    ui.add(egui::TextEdit::singleline(&mut target.name).desired_width(160.0));
                if resp.changed() {
                    target.name = normalize_msx_input(&target.name, charset);
                }
                if ui.memory(|m| m.focused().is_none()) {
                    resp.request_focus();
                }
                ui.small(t!("dialog.name_hint"));
                let warn = ui.visuals().warn_fg_color;
                ui.small(egui::RichText::new(t!("dialog.new_dir_warning")).color(warn));
                let valid = !msx_name_stem(&target.name).is_empty();
                let enter =
                    valid && resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    apply = ui
                        .add_enabled(valid, egui::Button::new(t!("button.create")))
                        .clicked();
                    cancel |= ui.button(t!("button.cancel")).clicked();
                });
                apply |= enter;
            });
        if apply {
            self.apply_new_dir();
        } else if cancel {
            self.new_dir = None;
        }
    }

    /// Render the boot-sector/image-size mismatch popup, if one was detected on
    /// open. Applies the single safe, recommended repair on confirmation.
    pub(crate) fn size_fix_dialog(&mut self, ctx: &egui::Context) {
        use msx_disk::fs::sizefix::SizeFix;
        let Some(m) = self.size_fix.as_ref() else {
            return;
        };
        let real = size_label(m.real_bytes);
        let action = match m.fix {
            SizeFix::TruncateImage => t!("dialog.size_fix_truncate", size => real),
            SizeFix::PadImage => t!("dialog.size_fix_pad", size => real),
            SizeFix::FixBootSector => t!("dialog.size_fix_fixboot", size => real),
        };
        let mut apply = false;
        let mut ignore = ctx.input(|i| i.key_pressed(egui::Key::Escape));
        egui::Window::new(t!("dialog.size_fix_title"))
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                egui::Grid::new("size_fix_facts")
                    .num_columns(2)
                    .show(ui, |ui| {
                        ui.label(t!("dialog.size_fix_declared"));
                        ui.monospace(size_label(m.declared_bytes));
                        ui.end_row();
                        ui.label(t!("dialog.size_fix_image"));
                        ui.monospace(size_label(m.image_bytes));
                        ui.end_row();
                        ui.label(t!("dialog.size_fix_real"));
                        ui.monospace(&real);
                        ui.end_row();
                    });
                ui.add_space(6.0);
                ui.label(action);
                ui.small(t!("dialog.delete_rewrites"));
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    apply = ui.button(t!("button.apply_fix")).clicked();
                    ignore |= ui.button(t!("button.ignore")).clicked();
                });
            });
        if apply {
            self.apply_size_fix();
        } else if ignore {
            self.size_fix = None;
        }
    }

    pub(crate) fn apply_size_fix(&mut self) {
        let Some(m) = self.size_fix.take() else {
            return;
        };
        let result = self.disk.as_mut().unwrap().apply_size_fix(&m);
        self.after_mutation(
            result,
            t!("status.fixed_geometry", size => size_label(m.real_bytes)).to_string(),
        );
        // Re-check the (reloaded) disk; a successful repair leaves it consistent.
        self.size_fix = self.disk.as_ref().and_then(LoadedDisk::size_mismatch);
    }

    /// The About window: app name, version, repo, and license. Cross-platform,
    /// so the version is reachable the same way on every OS (the native macOS
    /// "About" panel only fills in icon/version for the packaged `.app`).
    pub(crate) fn about_dialog(&mut self, ctx: &egui::Context) {
        if !self.show_about {
            return;
        }
        let icon = self
            .about_icon
            .get_or_insert_with(|| load_about_icon(ctx))
            .clone();
        let mut open = true;
        egui::Window::new(t!("dialog.about_title"))
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .open(&mut open)
            .show(ctx, |ui| {
                ui.vertical_centered(|ui| {
                    ui.add_space(12.0);
                    ui.add(
                        egui::Image::new(egui::load::SizedTexture::new(
                            icon.id(),
                            egui::vec2(96.0, 96.0),
                        ))
                        .corner_radius(egui::CornerRadius::same(20)),
                    );
                    ui.add_space(12.0);
                    // Product name is a brand — not localized.
                    ui.label(
                        egui::RichText::new("MSX Media Explorer")
                            .size(24.0)
                            .strong(),
                    );
                    ui.add_space(8.0);
                    ui.label(egui::RichText::new(t!("dialog.about_tagline")).weak());
                    ui.add_space(18.0);
                    about_meta_grid(ui);
                    ui.add_space(12.0);
                });
            });
        if !open || ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            self.show_about = false;
        }
    }

    /// The keyboard-shortcuts help window (F1, or Help menu). A static cheat
    /// sheet; `Cmd` rows show the platform's real modifier.
    pub(crate) fn shortcuts_dialog(&mut self, ctx: &egui::Context) {
        if !self.show_shortcuts {
            return;
        }
        let cmd = if cfg!(target_os = "macos") {
            "Cmd"
        } else {
            "Ctrl"
        };
        let delete = if cfg!(target_os = "macos") {
            format!("Delete / {cmd}+Backspace")
        } else {
            "Delete".to_string()
        };
        // Key combos stay as-is (universal); the descriptions are translated.
        let rows: Vec<(String, String)> = vec![
            ("Up / Down".into(), t!("shortcut.move").into_owned()),
            ("Left / Right".into(), t!("shortcut.collapse").into_owned()),
            ("PageUp / PageDown".into(), t!("shortcut.page").into_owned()),
            ("Home / End".into(), t!("shortcut.home_end").into_owned()),
            ("Enter".into(), t!("shortcut.activate").into_owned()),
            ("Backspace".into(), t!("shortcut.parent").into_owned()),
            (
                "A\u{2013}Z, 0\u{2013}9 \u{2026}".into(),
                t!("shortcut.type_ahead").into_owned(),
            ),
            (format!("F2 / {cmd}+R"), t!("shortcut.rename").into_owned()),
            (delete, t!("shortcut.delete").into_owned()),
            (format!("{cmd}+A"), t!("shortcut.select_all").into_owned()),
            (format!("{cmd}+E"), t!("shortcut.extract").into_owned()),
            (format!("{cmd}+F"), t!("shortcut.filter").into_owned()),
            ("Escape".into(), t!("shortcut.escape").into_owned()),
            ("F1".into(), t!("shortcut.help").into_owned()),
        ];
        let mut open = true;
        egui::Window::new(t!("dialog.shortcuts_title"))
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .open(&mut open)
            .show(ctx, |ui| {
                egui::Grid::new("shortcuts")
                    .num_columns(2)
                    .spacing([16.0, 6.0])
                    .show(ui, |ui| {
                        for (keys, action) in &rows {
                            ui.monospace(keys);
                            ui.label(action);
                            ui.end_row();
                        }
                    });
            });
        if !open || ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            self.show_shortcuts = false;
        }
    }
}

/// Volume serial for a freshly created disk's DOS 2 boot sector, derived from
/// the clock like MSX-DOS 2's FORMAT does (uniqueness, not secrecy, is the goal).
fn new_disk_volume_serial() -> u32 {
    use std::time::{SystemTime, UNIX_EPOCH};
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    (now.as_secs() as u32) ^ now.subsec_nanos()
}
