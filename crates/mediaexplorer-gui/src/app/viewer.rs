use super::*;

impl MediaExplorerApp {
    pub(crate) fn viewer_panel(&mut self, ui: &mut egui::Ui) {
        // The "/" root row has no DirEntry; show whole-disk stats for it.
        if self.cursor.as_deref() == Some(ROOT_PATH) {
            if let Some(disk) = &self.disk {
                render_root_info(ui, &disk.tree);
                return;
            }
        }
        // A directory has no file content or per-file tabs; show its info pane.
        if let Some(dir) = self.cursor_dir() {
            render_directory_info(ui, dir, self.charset);
            return;
        }
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
                        // Bytes-per-row now lives in the View menu.
                        let editable = writable
                            && self
                                .content
                                .as_ref()
                                .is_some_and(|c| c.bytes.len() <= MAX_HEX_EDIT_BYTES);
                        // The separator only divides "Edit hex" from "Go to:";
                        // without the button it would double up with the one
                        // after the view tabs.
                        if editable {
                            if ui.button("Edit hex").clicked() {
                                let text =
                                    format_hex_for_edit(&self.content.as_ref().unwrap().bytes);
                                self.hex_edit = Some(text);
                            }
                            ui.separator();
                        }
                        ui.label("Go to:");
                        let resp = ui.add(
                            egui::TextEdit::singleline(&mut self.hex.goto_input)
                                .desired_width(70.0)
                                .hint_text("hex"),
                        );
                        let submit =
                            resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                        if ui.button("Go").clicked() || submit {
                            self.goto_hex_offset();
                        }
                        ui.checkbox(&mut self.show_inspector, "Inspector");
                        self.hex_bookmarks_menu(ui);
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
            // The Screen view exposes Copy image / Save PNG as right-click
            // options on the image itself, so it carries no toolbar copy button.
            if self.content.is_some()
                && !matches!(self.view_mode, ViewMode::Archive | ViewMode::Screen)
            {
                ui.separator();
                if ui.button("Copy").clicked() {
                    self.copy_current_view();
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
            let mut action = None;
            match (self.disk.as_ref(), self.content.as_ref()) {
                (Some(disk), Some(content)) => {
                    let path = content.path.clone();
                    let companions = FnCompanions(|ext: &str| disk.companion(&path, ext));
                    let render = render_screen(
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
                    show_picker = !render.shown || self.forced_format.is_some();
                    action = render.action;
                }
                _ => {
                    ui.weak("Select a file to view its contents.");
                }
            }
            self.screen_tex = cache;
            if show_picker {
                self.screen_format_picker(ui);
            }
            // The image's right-click menu is built while `self.content`/`disk`
            // are borrowed above; run any chosen action now that those borrows
            // (and the texture cache) have been released.
            match action {
                Some(ScreenAction::CopyImage) => self.copy_current_view(),
                Some(ScreenAction::SavePng) => self.save_screen_png(),
                None => {}
            }
            return;
        }

        let scroll_to = self.pending_scroll_row.take();
        let highlight = self
            .search_matches
            .get(self.search_pos)
            .map(|&o| o / self.settings.hex.bytes_per_row.max(1));

        if self.view_mode == ViewMode::Hex {
            // The hex view needs `&mut self` (selection state + inspector panel),
            // so handle it outside the shared `self.content` borrow below.
            self.hex_view(ui, scroll_to, highlight);
            return;
        }

        let charset = self.charset;
        let show_all = self.text_show_all;
        let mode = self.view_mode;
        match &self.content {
            None => {
                ui.weak("Select a file to view its contents.");
            }
            Some(content) => match mode {
                ViewMode::Info => render_info(
                    ui,
                    &content.path,
                    &content.bytes,
                    self.selected_entry(),
                    charset,
                    &content.checksums,
                    content.file_info(),
                ),
                ViewMode::Text => {
                    let key = RenderKey {
                        mode,
                        charset,
                        show_all,
                    };
                    content
                        .show_listing(ui, key, || produce_text(&content.bytes, show_all, charset));
                }
                ViewMode::Basic => {
                    let key = RenderKey {
                        mode,
                        charset,
                        show_all,
                    };
                    content.show_listing(ui, key, || produce_basic(&content.bytes, charset));
                }
                ViewMode::Disasm => {
                    let key = RenderKey {
                        mode,
                        charset,
                        show_all,
                    };
                    content.show_listing(ui, key, || produce_disasm(&content.path, &content.bytes));
                }
                ViewMode::Screen | ViewMode::Archive | ViewMode::Hex => {
                    unreachable!("handled above")
                }
            },
        }
    }

    /// The Hex view: an optional data-inspector panel docked at the bottom and
    /// the interactive hex dump filling the rest. Selection gestures update
    /// [`MediaExplorerApp::hex`].
    pub(crate) fn hex_view(
        &mut self,
        ui: &mut egui::Ui,
        scroll_to: Option<usize>,
        highlight: Option<usize>,
    ) {
        if self.content.is_none() {
            ui.weak("Select a file to view its contents.");
            return;
        }
        let charset = self.charset;
        let bpr = self.settings.hex.bytes_per_row.max(1);
        if self.show_inspector {
            let origin = self
                .hex
                .cursor
                .or(self.hex.selection.map(|(s, _)| s))
                .unwrap_or(0);
            let bytes = &self.content.as_ref().unwrap().bytes;
            egui::Panel::bottom("hex_inspector")
                .resizable(true)
                .default_size(170.0)
                .show_inside(ui, |ui| render_inspector(ui, bytes, origin, charset));
        }
        let sel = HexSelection {
            cursor: self.hex.cursor,
            selection: self.hex.selection,
        };
        let opts = self.settings.hex;
        let gesture = {
            let bytes = &self.content.as_ref().unwrap().bytes;
            render_hex(ui, bytes, bpr, scroll_to, highlight, charset, sel, &opts)
        };
        if let Some(g) = gesture {
            apply_hex_gesture(&mut self.hex, g);
        }
        // Cmd/Ctrl+C copies the byte selection, in the representation of the
        // column it was made in (hex digits vs ASCII). Skipped while a text box
        // (Go to / Find) holds the keyboard, so its own copy still works.
        if self.hex.selection.is_some()
            && !ui.ctx().egui_wants_keyboard_input()
            && copy_event_pending(ui.ctx())
        {
            let ascii = self.hex.region == HexRegion::Ascii;
            if let Some(text) = self.selected_hex_text(ascii) {
                self.copy_text_to_clipboard(text);
            }
        }
    }

    /// Jump the file hex view to the offset typed in the go-to box.
    pub(crate) fn goto_hex_offset(&mut self) {
        let Some(off) = parse_offset(&self.hex.goto_input) else {
            self.status = "Enter a hex offset, e.g. 1A0".to_string();
            return;
        };
        let len = self.content.as_ref().map(|c| c.bytes.len()).unwrap_or(0);
        if off >= len {
            self.status = format!("Offset 0x{off:X} is past the end ({len} bytes)");
            return;
        }
        self.hex.cursor = Some(off);
        self.hex.selection = None;
        self.pending_scroll_row = Some(off / self.settings.hex.bytes_per_row.max(1));
        self.status = format!("Jumped to 0x{off:06X}");
    }

    /// The selected bytes of the file hex view, as a hex or ASCII string.
    pub(crate) fn selected_hex_text(&self, ascii: bool) -> Option<String> {
        let content = self.content.as_ref()?;
        let (lo, hi) = self.hex.selection?;
        let hi = hi.min(content.bytes.len().saturating_sub(1));
        let slice = content.bytes.get(lo..=hi)?;
        Some(format_selected_bytes(slice, ascii, self.charset))
    }

    /// The selected bytes of the current sector, as a hex or ASCII string.
    pub(crate) fn sector_selection_text(&self, ascii: bool) -> Option<String> {
        let (lo, hi) = self.sector_hex.selection?;
        let bytes = self.disk.as_ref()?.sector_slice(self.current_sector)?;
        let hi = hi.min(bytes.len().saturating_sub(1));
        let slice = bytes.get(lo..=hi)?;
        Some(format_selected_bytes(slice, ascii, self.charset))
    }

    /// The Bookmarks menu for the file hex view: add the cursor offset, jump to
    /// a saved one, or delete it.
    pub(crate) fn hex_bookmarks_menu(&mut self, ui: &mut egui::Ui) {
        ui.menu_button("Bookmarks", |ui| {
            if let Some(cur) = self.hex.cursor {
                if ui.button(format!("Add bookmark at 0x{cur:06X}")).clicked() {
                    self.hex.bookmarks.push(Bookmark {
                        offset: cur,
                        name: format!("0x{cur:06X}"),
                    });
                    ui.close();
                }
            } else {
                ui.weak("Click a byte to set the cursor first.");
            }
            if self.hex.bookmarks.is_empty() {
                return;
            }
            ui.separator();
            let mut goto = None;
            let mut remove = None;
            for (i, bm) in self.hex.bookmarks.iter().enumerate() {
                ui.horizontal(|ui| {
                    if ui.button(&bm.name).clicked() {
                        goto = Some(bm.offset);
                        ui.close();
                    }
                    if ui.small_button("\u{2715}").clicked() {
                        remove = Some(i);
                    }
                });
            }
            if let Some(off) = goto {
                self.hex.cursor = Some(off);
                self.hex.selection = None;
                self.pending_scroll_row = Some(off / self.settings.hex.bytes_per_row.max(1));
            }
            if let Some(i) = remove {
                self.hex.bookmarks.remove(i);
            }
        });
    }

    /// Whole-disk statistics view: image checksums plus per-volume counts,
    /// file-type breakdown, largest files, and FAT-chain integrity.
    pub(crate) fn stats_panel(&mut self, ui: &mut egui::Ui) {
        let Some(disk) = self.disk.as_ref() else {
            ui.weak("No disk open.");
            return;
        };
        let mut copy: Option<String> = None;
        let mut jump: Option<String> = None;
        let charset = self.charset;
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                ui.heading("Image checksums");
                checksum_grid(ui, "stats_image_cks", disk.checksums(), &mut copy);

                if disk.is_partitioned() {
                    for i in 0..disk.partition_count() {
                        ui.add_space(10.0);
                        let title = disk
                            .tree
                            .get(i)
                            .map(|n| n.name.clone())
                            .unwrap_or_else(|| format!("Partition {}", i + 1));
                        egui::CollapsingHeader::new(title)
                            .id_salt(("partition_stats", i))
                            .default_open(i == 0)
                            .show(ui, |ui| match disk.volume_stats(i) {
                                Some(s) => render_disk_stats(
                                    ui,
                                    s,
                                    i,
                                    &format!("P{}/", i + 1),
                                    charset,
                                    &mut jump,
                                ),
                                None => {
                                    ui.weak("No statistics for this partition.");
                                }
                            });
                    }
                } else {
                    ui.add_space(10.0);
                    match disk.stats() {
                        Some(s) => render_disk_stats(ui, s, 0, "", charset, &mut jump),
                        None => {
                            ui.weak("No filesystem statistics (no FAT BPB).");
                        }
                    }
                }
            });
        if let Some(t) = copy {
            self.copy_text_to_clipboard(t);
        }
        if let Some(path) = jump {
            self.app_view = AppView::Files;
            self.select_file(path, false);
        }
    }

    /// Render the contents of the selected archive: a member list with a
    /// right-click "Extract…" per row and an "Extract all…" button. Reads from
    /// the cached `self.archive` listing; never re-parses here.
    pub(crate) fn archive_panel(&mut self, ui: &mut egui::Ui) {
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
}
