use super::*;

impl MediaExplorerApp {
    /// Extract the given files to the host: a save-as dialog for one, a folder
    /// picker for several.
    pub(crate) fn extract_paths(&mut self, paths: &[String]) {
        match paths {
            [] => {}
            [path] => self.extract_one(path),
            many => self.extract_many(many),
        }
    }

    /// Extract a single file via a save-as dialog (lets the user rename it).
    pub(crate) fn extract_one(&mut self, path: &str) {
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
    pub(crate) fn extract_many(&mut self, paths: &[String]) {
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
        self.status = format!(
            "Extracted {ok} member(s) to {} ({skipped} skipped, {failed} failed)",
            dir.display()
        );
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
                    self.status = format!("Drag failed: {e}");
                }
            }
            Err(e) => self.status = e,
        }
    }

    /// Extract `paths` to a per-instance temp directory (the OS drag transfers
    /// file paths, not bytes) and return their absolute locations. The pid
    /// subdirectory keeps two concurrently running instances from overwriting
    /// each other's staged files.
    #[cfg(any(target_os = "macos", target_os = "windows"))]
    pub(crate) fn stage_files_for_drag(&self, paths: &[String]) -> Result<Vec<PathBuf>, String> {
        let dir = drag_staging_root().join(std::process::id().to_string());
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
            Ok(()) => "Copied to clipboard".to_string(),
            Err(e) => format!("Clipboard error: {e}"),
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
    pub(crate) fn copy_current_view(&mut self) {
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
