use super::*;

impl MediaExplorerApp {
    pub(crate) fn toolbar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            if self.disk.is_some() {
                if ui.button("Save as .dsk…").clicked() {
                    self.save_as_dsk();
                }
                if ui.button("Save as .xsa…").clicked() {
                    self.save_as_xsa();
                }
            }
            // Adding files / directories lives in the tree's right-click menu
            // (right-click the "/" root row to add to the disk root).
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
            // Always-visible version, parked at the right edge; click for the
            // About window.
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let clicked = ui
                    .add(
                        egui::Label::new(egui::RichText::new(format!("v{VERSION}")).weak())
                            .sense(egui::Sense::click()),
                    )
                    .on_hover_text("About MSX Media Explorer")
                    .clicked();
                if clicked {
                    self.show_about = true;
                }
            });
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
                    ui.selectable_value(&mut self.app_view, AppView::Stats, "Stats");
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

    pub(crate) fn tree_panel(&mut self, ui: &mut egui::Ui) {
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
        let mut events = RowEvents::default();
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
            };
            if let Some(disk) = &self.disk {
                egui::ScrollArea::vertical()
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
            } else if let Some(tape) = &self.tape {
                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        render_tape_files(ui, tape, &ctx, &mut events);
                    });
            } else {
                ui.weak("No disk open.");
            }
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
        if let Some(disk) = &self.disk {
            // Non-partitioned disks get a synthetic "/" root row; an HD's
            // partition nodes are themselves the roots, so no extra row.
            let root = (!disk.is_partitioned()).then_some(ROOT_PATH);
            tree_nav::flatten_visible(&disk.tree, &self.collapsed, root)
        } else if let Some(tape) = &self.tape {
            tape.entries()
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

    /// Drive the tree with the arrow keys while the Files or Stats view is up:
    /// Up/Down move the highlight one row (loading a file as it lands on one),
    /// Left/Right collapse/expand directories or step to the parent/first child.
    pub(crate) fn handle_tree_keys(&mut self, ctx: &egui::Context) {
        if !matches!(self.app_view, AppView::Files | AppView::Stats) {
            return;
        }
        // A modal dialog or a text field (the Find box, rename) owns the
        // keyboard; don't steal arrow keys from them. `wants_keyboard_input` is
        // true only for text entry, so a merely-focused row (which egui focuses
        // on click) does not block navigation.
        if self.rename_target.is_some()
            || self.confirm_delete.is_some()
            || self.new_dir.is_some()
            || self.size_fix.is_some()
        {
            return;
        }
        if ctx.egui_wants_keyboard_input() {
            return;
        }
        // Consume the arrow key so the scroll area doesn't also scroll on it.
        let key = ctx.input_mut(|i| {
            for (k, nav) in [
                (egui::Key::ArrowDown, NavKey::Down),
                (egui::Key::ArrowUp, NavKey::Up),
                (egui::Key::ArrowLeft, NavKey::Left),
                (egui::Key::ArrowRight, NavKey::Right),
            ] {
                if i.consume_key(egui::Modifiers::NONE, k) {
                    return Some(nav);
                }
            }
            None
        });
        let Some(key) = key else {
            return;
        };
        // Drop any lingering focus ring left on a previously-clicked row, so the
        // blue cursor is the single "where am I" indicator.
        if let Some(id) = ctx.memory(|m| m.focused()) {
            ctx.memory_mut(|m| m.surrender_focus(id));
        }

        let rows = self.visible_tree_rows();
        match tree_nav::navigate(&rows, self.cursor.as_deref(), &self.collapsed, key) {
            TreeNav::MoveTo(path) => {
                let is_file = rows.iter().any(|r| r.path == path && !r.is_dir);
                self.scroll_to_cursor = true;
                if is_file {
                    // Load-on-highlight: landing on a file previews it like a click.
                    self.select_file(path.clone(), false);
                    self.cursor = Some(path);
                } else {
                    self.focus_directory(path);
                }
            }
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
