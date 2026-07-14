use super::*;

impl MediaExplorerApp {
    pub(crate) fn toolbar(&mut self, ui: &mut egui::Ui) {
        // The document name, volume label and read-only state live in the status
        // bar; the version is on the About window (Help menu / app menu). "Save
        // as" is in the File menu; adding files/directories is in the tree's
        // right-click menu (right-click the "/" root row).
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
                    ui.selectable_value(&mut self.app_view, AppView::Stats, "Stats");
                }
                if self.dmk_analysis.is_some() {
                    ui.selectable_value(&mut self.app_view, AppView::Analyze, "Analyze");
                }
                if self.tape.is_some() {
                    ui.selectable_value(&mut self.app_view, AppView::Blocks, "Blocks");
                }
                // The MSX code page used to decode names/text now lives in the
                // "Text Encoding" menu (auto-detected on load; a manual pick pins it).
            });
        }
    }

    pub(crate) fn tree_panel(&mut self, ui: &mut egui::Ui) {
        // Filter box: a glob typed here (e.g. `*.PIC`, `img%.sc5`) live-filters
        // the file list below. Only shown once a document is open.
        if self.disk.is_some() || self.tape.is_some() {
            ui.horizontal(|ui| {
                ui.label("Filter:");
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if !self.filter.is_empty()
                        && ui
                            .button("\u{2715}")
                            .on_hover_text("Clear filter")
                            .clicked()
                    {
                        self.filter.clear();
                    }
                    ui.add(
                        egui::TextEdit::singleline(&mut self.filter)
                            // A stable id so focus survives the clear button
                            // appearing/disappearing: without it the field's
                            // auto-id shifts once the button shows after the
                            // first keystroke and egui drops keyboard focus.
                            .id(egui::Id::new("tree_filter_input"))
                            .hint_text("*.PIC")
                            .desired_width(f32::INFINITY),
                    );
                });
            });
            ui.separator();
        }
        if self.selection.len() > 1 {
            ui.horizontal(|ui| {
                ui.label(format!("{} files selected", self.selection.len()));
                if ui.button("Clear").clicked() {
                    self.selection.clear();
                }
            });
            ui.separator();
        }
        let charset = self.charset;
        // The set of paths to show under the active filter: matching files plus
        // their ancestor directories (disk), or matching keys (tape). `None`
        // means no filter; `Some(empty)` means the filter matched nothing.
        let keep = self.filter_keep_set();
        let mut events = RowEvents::default();
        // The tree viewport height, captured from the scroll area below so
        // PageUp/PageDown know how many rows fit on one screen.
        let row_height = ui.spacing().interact_size.y + ui.spacing().item_spacing.y;
        let mut viewport_height = None;
        // Scope `ctx` so its immutable borrows of `self` are released before the
        // event handling below mutates `self.cursor` / `self.collapsed`.
        {
            let ctx = TreeRender {
                selection: &self.selection,
                collapsed: &self.collapsed,
                cursor: self.cursor.as_deref(),
                scroll_to_cursor: self.scroll_to_cursor,
                writable: self.disk_writable(),
                charset,
                filter: keep.as_ref(),
            };
            if keep.as_ref().is_some_and(|k| k.is_empty()) {
                ui.weak(format!("No files match \"{}\".", self.filter));
            } else if let Some(disk) = &self.disk {
                let out = egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        if disk.is_partitioned() {
                            // An HD's partition nodes are the roots; no "/" row.
                            render_entries(ui, &disk.tree, &ctx, &mut events);
                        } else {
                            // Single-volume disk: a "/" root row holds the tree,
                            // so files/dirs can be added to the root too.
                            render_tree_with_root(ui, &disk.tree, &ctx, &mut events);
                        }
                    });
                viewport_height = Some(out.inner_rect.height());
            } else if let Some(tape) = &self.tape {
                let out = egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        render_tape_files(ui, tape, &ctx, &mut events);
                    });
                viewport_height = Some(out.inner_rect.height());
            } else {
                ui.weak("No disk open.");
            }
        }
        if let Some(height) = viewport_height {
            self.tree_page_rows = ((height / row_height) as usize).max(1);
        }
        // The cursor scroll is a one-shot; clear it once the tree has drawn.
        self.scroll_to_cursor = false;
        // Keep this frame's row rects for resolving the next drag-and-drop.
        self.drop_targets = std::mem::take(&mut events.drop_targets);
        if let Some(path) = events.toggle_dir {
            if !self.collapsed.remove(&path) {
                self.collapsed.insert(path);
            }
        }
        if let Some(path) = events.cursor_to {
            // A clicked directory (or the "/" root) clears the file viewer and
            // only highlights the folder; a clicked file is loaded below.
            if path == ROOT_PATH || self.entry_for_path(&path).is_some_and(|e| e.is_dir) {
                self.focus_directory(path);
            } else {
                self.cursor = Some(path);
            }
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
                RowAction::AddFiles(target) => self.add_files_into(&target),
                RowAction::AddDir(parent) => {
                    self.new_dir = Some(NewDirTarget {
                        parent,
                        name: String::new(),
                    });
                }
                RowAction::RemoveDir(path) => self.remove_directory(&path),
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

    /// The tree rows currently on screen, in top-to-bottom order, for keyboard
    /// navigation. A disk yields its (possibly collapsed) directory tree; a tape
    /// yields its flat file list.
    pub(crate) fn visible_tree_rows(&self) -> Vec<tree_nav::VisibleRow> {
        let keep = self.filter_keep_set();
        if keep.as_ref().is_some_and(|k| k.is_empty()) {
            return Vec::new();
        }
        if let Some(disk) = &self.disk {
            // Non-partitioned disks get a synthetic "/" root row; an HD's
            // partition nodes are themselves the roots, so no extra row.
            let root = (!disk.is_partitioned()).then_some(ROOT_PATH);
            tree_nav::flatten_visible(&disk.tree, &self.collapsed, root, keep.as_ref())
        } else if let Some(tape) = &self.tape {
            tape.entries()
                .filter(|(key, _)| keep.as_ref().is_none_or(|k| k.contains(*key)))
                .map(|(key, _)| tree_nav::VisibleRow {
                    path: key.to_string(),
                    is_dir: false,
                    depth: 0,
                    parent: None,
                    collapsible: false,
                })
                .collect()
        } else {
            Vec::new()
        }
    }

    /// The set of paths to show under the active filter: for a disk, every file
    /// whose display name matches the glob plus their ancestor directories; for a
    /// tape, the matching keys. `None` when the filter box is empty (show all);
    /// `Some(empty)` when the pattern matches nothing.
    fn filter_keep_set(&self) -> Option<BTreeSet<String>> {
        if self.filter.is_empty() {
            return None;
        }
        if let Some(disk) = &self.disk {
            return Some(tree_filter::matching_paths(
                &disk.tree,
                &self.filter,
                self.charset,
            ));
        }
        self.tape.as_ref().map(|tape| {
            tape.entries()
                .filter(|(key, _)| tree_filter::glob_match(&self.filter, key))
                .map(|(key, _)| key.to_string())
                .collect()
        })
    }

    /// Whether any modal dialog/window is open. Keyboard shortcuts stop while
    /// one is up (each dialog handles its own Escape/Enter).
    pub(crate) fn modal_open(&self) -> bool {
        self.rename_target.is_some()
            || self.confirm_delete.is_some()
            || self.new_dir.is_some()
            || self.size_fix.is_some()
            || self.show_new_disk
            || self.show_about
            || self.show_shortcuts
    }

    /// Drive the tree with the keyboard while the Files or Stats view is up:
    /// arrows move the highlight (loading a file as it lands on one) and
    /// collapse/expand directories; PageUp/PageDown/Home/End jump; Enter toggles
    /// a directory; Backspace jumps to the parent; typing letters jumps to the
    /// next matching name; and F2/Delete/Cmd+A/Cmd+E/Cmd+F/Escape/F1 trigger the
    /// corresponding actions.
    pub(crate) fn handle_tree_keys(&mut self, ctx: &egui::Context) {
        if !matches!(self.app_view, AppView::Files | AppView::Stats) {
            return;
        }
        // A modal dialog or a text field (the Find box, rename, filter) owns the
        // keyboard; don't steal keys from them. `wants_keyboard_input` is true
        // only for text entry, so a merely-focused row (which egui focuses on
        // click) does not block navigation.
        if self.modal_open() || ctx.egui_wants_keyboard_input() {
            return;
        }
        if self.handle_action_keys(ctx) {
            return;
        }
        // Consume the navigation key so the scroll area doesn't also scroll.
        let key = ctx.input_mut(|i| {
            for (k, nav) in [
                (egui::Key::ArrowDown, NavKey::Down),
                (egui::Key::ArrowUp, NavKey::Up),
                (egui::Key::ArrowLeft, NavKey::Left),
                (egui::Key::ArrowRight, NavKey::Right),
                (egui::Key::PageDown, NavKey::PageDown),
                (egui::Key::PageUp, NavKey::PageUp),
                (egui::Key::Home, NavKey::Home),
                (egui::Key::End, NavKey::End),
                (egui::Key::Enter, NavKey::Activate),
                (egui::Key::Backspace, NavKey::Parent),
            ] {
                if i.consume_key(egui::Modifiers::NONE, k) {
                    return Some(nav);
                }
            }
            None
        });
        let Some(key) = key else {
            self.handle_type_ahead(ctx);
            return;
        };
        // Drop any lingering focus ring left on a previously-clicked row, so the
        // blue cursor is the single "where am I" indicator.
        if let Some(id) = ctx.memory(|m| m.focused()) {
            ctx.memory_mut(|m| m.surrender_focus(id));
        }

        let rows = self.visible_tree_rows();
        let nav = tree_nav::navigate(
            &rows,
            self.cursor.as_deref(),
            &self.collapsed,
            key,
            self.tree_page_rows,
        );
        match nav {
            TreeNav::MoveTo(path) => self.move_cursor_to(&rows, path),
            TreeNav::SetCollapsed(path, collapsed) => {
                if collapsed {
                    self.collapsed.insert(path);
                } else {
                    self.collapsed.remove(&path);
                }
                self.scroll_to_cursor = true;
            }
            TreeNav::Nothing => {}
        }
    }

    /// Move the keyboard cursor to `path`: files load on highlight (like a
    /// click), directories only take the highlight.
    fn move_cursor_to(&mut self, rows: &[tree_nav::VisibleRow], path: String) {
        let is_file = rows.iter().any(|r| r.path == path && !r.is_dir);
        self.scroll_to_cursor = true;
        if is_file {
            self.select_file(path.clone(), false);
            self.cursor = Some(path);
        } else {
            self.focus_directory(path);
        }
    }

    /// Action shortcuts on the file tree. Returns `true` when a shortcut fired
    /// (so navigation-key handling is skipped this frame).
    fn handle_action_keys(&mut self, ctx: &egui::Context) -> bool {
        use egui::{Key, Modifiers};
        // COMMAND is Cmd on macOS and Ctrl elsewhere.
        let (rename, delete, select_all, extract, find, escape, help) = ctx.input_mut(|i| {
            (
                i.consume_key(Modifiers::NONE, Key::F2)
                    || i.consume_key(Modifiers::COMMAND, Key::R),
                i.consume_key(Modifiers::NONE, Key::Delete)
                    || i.consume_key(Modifiers::COMMAND, Key::Backspace),
                i.consume_key(Modifiers::COMMAND, Key::A),
                i.consume_key(Modifiers::COMMAND, Key::E),
                i.consume_key(Modifiers::COMMAND, Key::F),
                i.consume_key(Modifiers::NONE, Key::Escape),
                i.consume_key(Modifiers::NONE, Key::F1),
            )
        });
        // The cursor row, when it is a real directory entry (not the "/" root).
        let cursor = self
            .cursor
            .clone()
            .filter(|c| c != ROOT_PATH && !c.is_empty());
        if rename {
            if let Some(path) = cursor.filter(|_| self.disk_writable()) {
                if self.entry_for_path(&path).is_some() {
                    // Edit the decoded display name; re-encoded on save.
                    let name = charset::decode_fs_name(self.charset, &base_name(&path));
                    self.rename_target = Some(RenameTarget {
                        path,
                        name,
                        charset: self.charset,
                    });
                }
            }
        } else if delete {
            if let Some(path) = cursor.filter(|_| self.disk_writable()) {
                // Same split as the context menu: directories go through
                // remove_directory (immediate when empty, confirm otherwise).
                if self.entry_for_path(&path).is_some_and(|e| e.is_dir) {
                    self.remove_directory(&path);
                } else if self.entry_for_path(&path).is_some() {
                    self.confirm_delete = Some(self.paths_for_row(&path));
                }
            }
        } else if select_all {
            self.select_all_files();
        } else if extract {
            let paths = match &cursor {
                Some(c) => self.paths_for_row(c),
                None => self.selection_paths(),
            };
            if !paths.is_empty() {
                self.extract_paths(&paths);
            }
        } else if find {
            ctx.memory_mut(|m| m.request_focus(egui::Id::new("tree_filter_input")));
        } else if escape {
            // No modal is open here (guarded above): clear the filter first,
            // then the multi-selection.
            if !self.filter.is_empty() {
                self.filter.clear();
            } else {
                self.selection.clear();
            }
        } else if help {
            self.show_shortcuts = true;
        } else {
            return false;
        }
        true
    }

    /// Type-ahead: letters typed while the tree owns the keyboard jump the
    /// cursor to the next row whose name starts with the growing prefix; a
    /// pause resets the prefix.
    fn handle_type_ahead(&mut self, ctx: &egui::Context) {
        let typed: String = ctx.input(|i| {
            i.events
                .iter()
                .filter_map(|e| match e {
                    egui::Event::Text(t) => Some(t.as_str()),
                    _ => None,
                })
                .collect()
        });
        if typed.is_empty() {
            return;
        }
        let now = ctx.input(|i| i.time);
        if now - self.type_ahead_at > TYPE_AHEAD_TIMEOUT {
            self.type_ahead.clear();
        }
        self.type_ahead.push_str(&typed);
        self.type_ahead_at = now;
        let rows = self.visible_tree_rows();
        if let Some(path) = tree_nav::type_ahead(&rows, self.cursor.as_deref(), &self.type_ahead) {
            self.move_cursor_to(&rows, path);
        }
    }

    /// Put every visible file (not directories) into the multi-selection.
    pub(crate) fn select_all_files(&mut self) {
        for row in self.visible_tree_rows() {
            if !row.is_dir {
                self.selection.insert(row.path);
            }
        }
    }

    /// The directory entry for the currently selected file, when it lives on a
    /// disk (tape files have no `DirEntry`, so this returns `None`).
    pub(crate) fn selected_entry(&self) -> Option<&DirEntry> {
        let sel = self.selected.as_deref()?;
        self.entry_for_path(sel)
    }

    /// Find a tree entry (file or directory) by its full path; disk only.
    pub(crate) fn entry_for_path(&self, path: &str) -> Option<&DirEntry> {
        self.disk
            .as_ref()?
            .tree
            .iter()
            .flat_map(DirEntry::walk)
            .find(|e| e.path == path)
    }

    /// The directory the keyboard cursor sits on, if any. Drives the Info pane
    /// (showing folder stats instead of file contents).
    pub(crate) fn cursor_dir(&self) -> Option<&DirEntry> {
        let cursor = self.cursor.as_deref()?;
        self.entry_for_path(cursor).filter(|e| e.is_dir)
    }

    /// Move the cursor onto a directory: clear the file viewer so no file row
    /// stays highlighted and the Info pane describes the folder instead.
    pub(crate) fn focus_directory(&mut self, path: String) {
        self.selection.clear();
        self.selected = None;
        self.content = None;
        self.archive = None;
        self.cursor = Some(path);
    }
}
