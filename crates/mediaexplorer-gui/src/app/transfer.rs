use super::*;

/// One planned host write from [`MediaExplorerApp::plan_extraction`]: a path
/// relative to the chosen destination, the disk path to read the bytes from, and
/// the file's original FAT modification time.
pub(crate) struct Planned {
    pub(crate) rel: PathBuf,
    pub(crate) disk_path: String,
    pub(crate) modified: Option<msx_disk::fs::Timestamp>,
}

/// Write an extracted file to `target`, then restore its original directory-
/// entry modification time so the host copy keeps its disk date instead of
/// "now". Setting the time is best-effort: any failure is ignored so it can
/// never fail the extraction itself.
pub(crate) fn write_extracted(
    target: &Path,
    bytes: &[u8],
    modified: Option<msx_disk::fs::Timestamp>,
) -> std::io::Result<()> {
    std::fs::write(target, bytes)?;
    if let Some(time) = modified.and_then(|t| t.to_system_time()) {
        let _ = std::fs::OpenOptions::new()
            .write(true)
            .open(target)
            .and_then(|f| f.set_modified(time));
    }
    Ok(())
}

impl MediaExplorerApp {
    /// Extract the given rows to the host. A single file uses a save-as dialog
    /// (so it can be renamed); a directory or any multi-selection uses a folder
    /// picker and writes the tree recursively, preserving subdirectories.
    pub(crate) fn extract_paths(&mut self, paths: &[String]) {
        let is_single_file = matches!(paths, [p]
            if self.entry_for_path(p).map(|e| !e.is_dir).unwrap_or(true));
        match paths {
            [] => {}
            [path] if is_single_file => self.extract_one(path),
            many => self.extract_tree(many),
        }
    }

    /// Extract a single file via a save-as dialog (lets the user rename it).
    pub(crate) fn extract_one(&mut self, path: &str) {
        let Some(bytes) = self.read_doc_file(path) else {
            self.status = t!("status.cannot_read", path => path).to_string();
            return;
        };
        let modified = self.entry_for_path(path).and_then(|e| e.modified);
        // A host-safe default (control bytes -> pictures, reserved -> `_`), from
        // the decoded name so kana comes out right; the dialog lets it be edited.
        let default_name = msx_disk::hostname::safe_component(
            &self
                .entry_for_path(path)
                .map(|e| e.display_name(self.charset))
                .unwrap_or_else(|| base_name(path)),
        );
        if let Some(target) = rfd::FileDialog::new()
            .set_file_name(&default_name)
            .save_file()
        {
            self.status = match write_extracted(&target, &bytes, modified) {
                Ok(()) => {
                    t!("status.extracted_one_to", name => default_name, dir => target.display())
                        .to_string()
                }
                Err(e) => t!("status.extract_failed", error => e).to_string(),
            };
        }
    }

    /// Extract `paths` (files and/or directories) into a chosen folder,
    /// recreating each directory's subtree under it.
    pub(crate) fn extract_tree(&mut self, paths: &[String]) {
        let planned = self.plan_extraction(paths);
        if planned.is_empty() {
            self.status = t!("status.nothing_to_extract").to_string();
            return;
        }
        let Some(dir) = rfd::FileDialog::new().pick_folder() else {
            return;
        };
        let (ok, failed) = self.write_extractions(&dir, &planned);
        self.status = if failed == 0 {
            tn!("status.extracted_to", ok, dir => dir.display()).to_string()
        } else {
            tn!("status.extracted_to_failed", ok, dir => dir.display(), failed => failed)
                .to_string()
        };
    }

    /// Flatten `paths` into the files to write on the host. A directory expands
    /// into its whole subtree, each file's destination rooted at the directory's
    /// own name (`UTILS/SUB/X.BIN`); a plain file maps to its base name. Names
    /// are decoded for the host under the active charset. Paths not found in a
    /// disk tree (e.g. tape files) are treated as a single flat file.
    pub(crate) fn plan_extraction(&self, paths: &[String]) -> Vec<Planned> {
        let mut out = Vec::new();
        for path in paths {
            match self.entry_for_path(path) {
                Some(entry) => self.collect_entry(entry, Path::new(""), &mut out),
                None => out.push(Planned {
                    rel: PathBuf::from(msx_disk::hostname::safe_component(&base_name(path))),
                    disk_path: path.clone(),
                    modified: None,
                }),
            }
        }
        out
    }

    /// Append `entry` (and, for a directory, its descendants) to `out`, with each
    /// file's host path built under `prefix` from charset-decoded names.
    fn collect_entry(&self, entry: &DirEntry, prefix: &Path, out: &mut Vec<Planned>) {
        let rel = prefix.join(msx_disk::hostname::safe_component(
            &entry.display_name(self.charset),
        ));
        if entry.is_dir {
            for child in &entry.children {
                self.collect_entry(child, &rel, out);
            }
        } else {
            out.push(Planned {
                rel,
                disk_path: entry.path.clone(),
                modified: entry.modified,
            });
        }
    }

    /// Write every planned file under `dest`, creating parent directories and
    /// preserving timestamps. Returns `(written, failed)`.
    pub(crate) fn write_extractions(&self, dest: &Path, planned: &[Planned]) -> (usize, usize) {
        let (mut ok, mut failed) = (0usize, 0usize);
        for item in planned {
            let Some(bytes) = self.read_doc_file(&item.disk_path) else {
                failed += 1;
                continue;
            };
            let target = dest.join(&item.rel);
            if let Some(parent) = target.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            // Suffix a duplicate name so byte-identical entries don't overwrite.
            let target = msx_disk::hostname::free_target(&target);
            if write_extracted(&target, &bytes, item.modified).is_ok() {
                ok += 1;
            } else {
                failed += 1;
            }
        }
        (ok, failed)
    }

    /// Extract one archive member (decompressed) via a save-as dialog.
    pub(crate) fn extract_archive_member(&mut self, index: usize) {
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
                        t!("status.crc_mismatch_suffix").into_owned()
                    } else {
                        String::new()
                    };
                    self.status = match std::fs::write(&target, &data) {
                        Ok(()) => t!(
                            "status.extracted_member",
                            name => default_name,
                            count => data.len(),
                            warn => crc_warn,
                            dir => target.display(),
                        )
                        .to_string(),
                        Err(e) => t!("status.extract_failed", error => e).to_string(),
                    };
                }
            }
            Err(e) => {
                self.status =
                    t!("status.cannot_decompress", name => default_name, error => e).to_string()
            }
        }
    }

    /// Extract every decodable member into a chosen folder, preserving each
    /// member's subdirectory path under it.
    pub(crate) fn extract_archive_all(&mut self) {
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
        self.status = tn!(
            "status.extracted_members",
            ok,
            dir => dir.display(),
            skipped => skipped,
            failed => failed,
        )
        .to_string();
    }

    /// If a row was dragged this frame, write the file(s) to a temp directory
    /// and hand them to the OS drag, using the window handle from `frame`.
    #[cfg(any(target_os = "macos", target_os = "windows"))]
    pub(crate) fn process_drag_out(&mut self, frame: &eframe::Frame) {
        let Some(paths) = self.pending_drag_out.take() else {
            return;
        };
        match self.stage_files_for_drag(&paths) {
            Ok(staged) => {
                if let Err(e) = crate::dnd::start_file_drag(frame, staged) {
                    self.status = t!("status.drag_failed", error => e).to_string();
                }
            }
            Err(e) => self.status = e,
        }
    }

    /// Stage `paths` (files and/or directories) into a per-instance temp
    /// directory — the OS drag transfers file paths, not bytes — and return the
    /// top-level staged entries to hand to the drag: the folder itself for a
    /// directory, the file for a file, so a folder is dropped with its whole
    /// subtree. The pid subdirectory keeps two concurrently running instances
    /// from overwriting each other's staged files.
    #[cfg(any(target_os = "macos", target_os = "windows"))]
    pub(crate) fn stage_files_for_drag(&self, paths: &[String]) -> Result<Vec<PathBuf>, String> {
        let dir = drag_staging_root().join(std::process::id().to_string());
        std::fs::create_dir_all(&dir).map_err(|e| format!("temp dir: {e}"))?;
        let planned = self.plan_extraction(paths);
        for item in &planned {
            let bytes = self
                .read_doc_file(&item.disk_path)
                .ok_or_else(|| format!("cannot read {}", item.disk_path))?;
            let target = dir.join(&item.rel);
            if let Some(parent) = target.parent() {
                std::fs::create_dir_all(parent).map_err(|e| format!("temp dir: {e}"))?;
            }
            write_extracted(&target, &bytes, item.modified)
                .map_err(|e| format!("write {}: {e}", target.display()))?;
        }
        // Hand the OS the top-level staged entries (a folder or a file), not the
        // individual leaf files, de-duplicated in first-seen order.
        let mut roots = Vec::new();
        for item in &planned {
            if let Some(first) = item.rel.components().next() {
                let root = dir.join(first);
                if !roots.contains(&root) {
                    roots.push(root);
                }
            }
        }
        Ok(roots)
    }

    pub(crate) fn ensure_clipboard(&mut self) -> Option<&mut arboard::Clipboard> {
        if self.clipboard.is_none() {
            self.clipboard = arboard::Clipboard::new().ok();
        }
        self.clipboard.as_mut()
    }

    pub(crate) fn copy_text_to_clipboard(&mut self, text: String) {
        let result = match self.ensure_clipboard() {
            Some(cb) => cb.set_text(text).map_err(|e| e.to_string()),
            None => Err("clipboard unavailable".to_string()),
        };
        self.status = match result {
            Ok(()) => t!("status.copied_to_clipboard").to_string(),
            Err(e) => t!("status.clipboard_error", error => e).to_string(),
        };
    }

    /// Decode the currently-selected file as an MSX image, if it is one,
    /// honoring any manually-forced format.
    pub(crate) fn decode_current_screen(&self) -> Option<recoil::Image> {
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
    pub(crate) fn screen_format_picker(&mut self, ui: &mut egui::Ui) {
        ui.separator();
        ui.horizontal(|ui| {
            ui.label(t!("screen.try_decoding_as"));
            let auto_label = t!("screen.auto_from_ext");
            let selected = self
                .forced_format
                .map(|f| f.label().to_string())
                .unwrap_or_else(|| auto_label.to_string());
            egui::ComboBox::from_id_salt("screen_format")
                .selected_text(selected)
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut self.forced_format, None, auto_label);
                    for &fmt in recoil::ImageFormat::all() {
                        ui.selectable_value(&mut self.forced_format, Some(fmt), fmt.label());
                    }
                });
        });
    }

    /// Copy the current view to the clipboard: the decoded image in Screen mode,
    /// otherwise the rendered text (hex dump / text / BASIC listing).
    pub(crate) fn copy_current_view(&mut self) {
        if self.view_mode == ViewMode::Screen {
            let Some(img) = self.decode_current_screen() else {
                self.status = t!("status.nothing_to_copy").to_string();
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
                Ok(()) => t!("status.copied_image").to_string(),
                Err(e) => t!("status.clipboard_error", error => e).to_string(),
            };
            return;
        }

        let Some(content) = self.content.as_ref() else {
            return;
        };
        let text = match self.view_mode {
            ViewMode::Info => format_fileinfo_text(
                &content.path,
                &content.bytes,
                self.selected_entry(),
                &content.checksums,
            ),
            ViewMode::Hex => {
                dump_to_string(&content.bytes, hex_config(&self.settings.hex), self.charset)
            }
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
    pub(crate) fn save_screen_png(&mut self) {
        let Some(img) = self.decode_current_screen() else {
            self.status = t!("status.not_decodable").to_string();
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
            self.status = t!("status.image_buffer_error").to_string();
            return;
        };
        self.status = match buffer.save(&target) {
            Ok(()) => t!("status.saved", path => target.display()).to_string(),
            Err(e) => t!("status.save_png_failed", error => e).to_string(),
        };
    }
}

/// Root of the drag-out staging area; each instance stages into a pid-named
/// subdirectory of it.
#[cfg(any(target_os = "macos", target_os = "windows"))]
fn drag_staging_root() -> PathBuf {
    std::env::temp_dir().join("mediaexplorer-dragout")
}

/// Best-effort cleanup of drag-staging directories left behind by previous
/// runs (staged files are never deleted after a drag — the OS may still be
/// copying them). Only subdirectories untouched for over a day are removed:
/// far longer than any live drag, so a concurrently running instance's staging
/// is never disturbed even if its pid was recycled. All errors are ignored.
#[cfg(any(target_os = "macos", target_os = "windows"))]
pub(crate) fn clean_stale_drag_staging() {
    let Ok(entries) = std::fs::read_dir(drag_staging_root()) else {
        return;
    };
    const DAY: std::time::Duration = std::time::Duration::from_secs(24 * 60 * 60);
    for entry in entries.flatten() {
        let stale = entry
            .metadata()
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| t.elapsed().ok())
            .is_some_and(|age| age > DAY);
        if stale {
            let _ = std::fs::remove_dir_all(entry.path());
        }
    }
}
