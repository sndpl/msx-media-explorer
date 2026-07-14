use super::*;

/// Plain-text rendering of the File Info view, for the Copy button.
pub(crate) fn format_fileinfo_text(
    path: &str,
    bytes: &[u8],
    entry: Option<&DirEntry>,
    checksums: &msx_disk::Checksums,
) -> String {
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
    let _ = writeln!(out, "CRC32: {}", checksums.crc32_hex());
    let _ = writeln!(out, "SHA-1: {}", checksums.sha1_hex());
    out
}

/// The file name to show in the Info panel: decoded under `charset` so
/// Japanese/accented names render the same way as in the file list (which uses
/// [`DirEntry::display_name`]) rather than as the raw PUA-encoded name.
pub(crate) fn info_display_name(
    entry: Option<&DirEntry>,
    path: &str,
    charset: MsxCharset,
) -> String {
    match entry {
        Some(e) => e.display_name(charset),
        None => charset::decode_fs_name(charset, &base_name(path)),
    }
}

/// File Info view: filesystem facts plus content-derived format details.
/// The directory an add operation targets, given the row it was invoked on: a
/// directory (or the `/` root, whose path is empty) targets itself; a file
/// targets its containing folder.
pub(crate) fn add_target_dir(path: &str, is_dir: bool) -> String {
    if is_dir {
        path.to_string()
    } else {
        path.rsplit_once('/')
            .map_or("", |(parent, _)| parent)
            .to_string()
    }
}

/// Join a directory and a child name into a slash path; the empty (root)
/// directory yields the bare name.
pub(crate) fn child_path(dir: &str, name: &str) -> String {
    if dir.is_empty() {
        name.to_string()
    } else {
        format!("{dir}/{name}")
    }
}

/// Paths to delete to remove `entry` and everything under it, ordered so each
/// directory is emptied before it is removed (descendants first, `entry` last).
pub(crate) fn removal_paths(entry: &DirEntry) -> Vec<String> {
    let mut out = Vec::new();
    push_removal_paths(entry, &mut out);
    out
}

pub(crate) fn push_removal_paths(entry: &DirEntry, out: &mut Vec<String>) {
    for child in &entry.children {
        push_removal_paths(child, out);
    }
    out.push(entry.path.clone());
}

/// The drop-target directory for a pointer position: the `target` of the first
/// recorded row rect that contains `pos`, or `None` if the drop missed them all.
pub(crate) fn drop_target_at(targets: &[(egui::Rect, String)], pos: egui::Pos2) -> Option<String> {
    targets
        .iter()
        .find(|(rect, _)| rect.contains(pos))
        .map(|(_, target)| target.clone())
}

/// File and subdirectory tallies for a directory: immediate child counts plus
/// recursive totals and the summed byte size of every descendant file.
pub(crate) struct DirStats {
    pub(crate) files: usize,
    pub(crate) dirs: usize,
    pub(crate) total_files: usize,
    pub(crate) total_dirs: usize,
    pub(crate) total_bytes: u64,
}

/// Count `dir`'s immediate and recursive contents. `walk()` yields `dir` first,
/// so `skip(1)` leaves only its descendants.
pub(crate) fn directory_stats(dir: &DirEntry) -> DirStats {
    let files = dir.children.iter().filter(|e| !e.is_dir).count();
    let dirs = dir.children.iter().filter(|e| e.is_dir).count();
    let mut total_files = 0;
    let mut total_dirs = 0;
    let mut total_bytes = 0u64;
    for e in dir.walk().skip(1) {
        if e.is_dir {
            total_dirs += 1;
        } else {
            total_files += 1;
            total_bytes += e.size;
        }
    }
    DirStats {
        files,
        dirs,
        total_files,
        total_dirs,
        total_bytes,
    }
}

/// The Info pane for a selected directory: its name/path/attributes and a
/// content summary (immediate counts, plus recursive totals and size when it
/// has subdirectories). Shown in place of the file viewer when the cursor is on
/// a folder.
/// The Info pane for the disk root (`/`): whole-disk file/subdirectory counts
/// and total size. Shown when the `/` row is selected, which has no `DirEntry`.
pub(crate) fn render_root_info(ui: &mut egui::Ui, entries: &[DirEntry]) {
    let files = entries.iter().filter(|e| !e.is_dir).count();
    let dirs = entries.iter().filter(|e| e.is_dir).count();
    let mut total_files = 0;
    let mut total_dirs = 0;
    let mut total_bytes = 0u64;
    for e in entries.iter().flat_map(DirEntry::walk) {
        if e.is_dir {
            total_dirs += 1;
        } else {
            total_files += 1;
            total_bytes += e.size;
        }
    }
    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            ui.heading("Disk root");
            egui::Grid::new("root_info").num_columns(2).show(ui, |ui| {
                ui.label("Files:");
                ui.monospace(files.to_string());
                ui.end_row();
                ui.label("Subdirectories:");
                ui.monospace(dirs.to_string());
                ui.end_row();
                if total_dirs > 0 {
                    ui.label("Files (incl. nested):");
                    ui.monospace(total_files.to_string());
                    ui.end_row();
                    ui.label("Subdirectories (all):");
                    ui.monospace(total_dirs.to_string());
                    ui.end_row();
                }
                ui.label("Total size:");
                ui.monospace(format!(
                    "{} ({} bytes)",
                    humanize_bytes(total_bytes),
                    total_bytes
                ));
                ui.end_row();
            });
        });
}

pub(crate) fn render_directory_info(ui: &mut egui::Ui, dir: &DirEntry, charset: MsxCharset) {
    let stats = directory_stats(dir);
    let yes_no = |b: bool| if b { "yes" } else { "no" };
    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            ui.heading("Directory");
            egui::Grid::new("dir_info").num_columns(2).show(ui, |ui| {
                ui.label("Name:");
                ui.monospace(dir.display_name(charset));
                ui.end_row();
                ui.label("Path:");
                ui.monospace(charset::decode_fs_name(charset, &dir.path));
                ui.end_row();
                let ts = format_timestamp(dir.modified);
                if !ts.is_empty() {
                    ui.label("Modified:");
                    ui.monospace(ts);
                    ui.end_row();
                }
            });

            let a = dir.attributes;
            ui.add_space(4.0);
            ui.label("Attributes");
            egui::Grid::new("dir_attrs").num_columns(2).show(ui, |ui| {
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

            ui.add_space(8.0);
            ui.heading("Contents");
            egui::Grid::new("dir_contents")
                .num_columns(2)
                .show(ui, |ui| {
                    ui.label("Files:");
                    ui.monospace(stats.files.to_string());
                    ui.end_row();
                    ui.label("Subdirectories:");
                    ui.monospace(stats.dirs.to_string());
                    ui.end_row();
                    // Recursive totals only add information when there are nested
                    // folders; for a flat directory they equal the immediate counts.
                    if stats.total_dirs > 0 {
                        ui.label("Files (incl. nested):");
                        ui.monospace(stats.total_files.to_string());
                        ui.end_row();
                        ui.label("Subdirectories (all):");
                        ui.monospace(stats.total_dirs.to_string());
                        ui.end_row();
                    }
                    ui.label("Total size:");
                    ui.monospace(format!(
                        "{} ({} bytes)",
                        humanize_bytes(stats.total_bytes),
                        stats.total_bytes
                    ));
                    ui.end_row();
                });
        });
}

pub(crate) fn render_info(
    ui: &mut egui::Ui,
    path: &str,
    bytes: &[u8],
    entry: Option<&DirEntry>,
    charset: MsxCharset,
    checksums: &msx_disk::Checksums,
    info: &msx_disk::fileinfo::FileInfo,
) {
    let yes_no = |b: bool| if b { "yes" } else { "no" };

    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            ui.heading("File");
            egui::Grid::new("info_file").num_columns(2).show(ui, |ui| {
                ui.label("Name:");
                ui.monospace(charset::display_control_safe(&info_display_name(
                    entry, path, charset,
                )));
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

            // Crafted directory entries whose name is control characters (they
            // beep/clear the screen when the directory is listed on a real MSX):
            // explain which codes are present and what they do.
            let raw_name = entry.map(|e| e.name.as_str()).unwrap_or(path);
            let name_bytes: Vec<u8> = raw_name.chars().filter_map(charset::fs_name_byte).collect();
            let mut controls: Vec<u8> = name_bytes
                .iter()
                .copied()
                .filter(|&b| b < 0x20 || b == 0x7F)
                .collect();
            controls.sort_unstable();
            controls.dedup();
            if !controls.is_empty() {
                ui.add_space(8.0);
                ui.heading("Control characters");
                ui.label(
                    "This filename contains control characters that manipulate an MSX terminal \
                     when the directory is listed (e.g. via the BASIC files command):",
                );
                egui::Grid::new("info_controls")
                    .num_columns(2)
                    .show(ui, |ui| {
                        for b in &controls {
                            ui.monospace(format!("{b:02X}"));
                            ui.label(
                                charset::control_char_effect(*b).unwrap_or("control character"),
                            );
                            ui.end_row();
                        }
                    });
                let hex = name_bytes
                    .iter()
                    .map(|b| format!("{b:02X}"))
                    .collect::<Vec<_>>()
                    .join(" ");
                ui.add_space(2.0);
                ui.horizontal(|ui| {
                    ui.label("Raw bytes:");
                    ui.monospace(hex);
                });
            }

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

            if let Some(b) = &info.bload {
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

            ui.add_space(8.0);
            ui.heading("Checksums");
            egui::Grid::new("info_checksums")
                .num_columns(2)
                .show(ui, |ui| {
                    kv_row(ui, "CRC32", checksums.crc32_hex());
                    kv_row(ui, "SHA-1", checksums.sha1_hex());
                });
        });
}

/// Text view, capped to a sane size for responsiveness.
pub(crate) fn produce_text(bytes: &[u8], show_all: bool, charset: MsxCharset) -> RenderedView {
    let shown = &bytes[..bytes.len().min(MAX_TEXT_BYTES)];
    let mode = if show_all {
        ControlMode::ShowAll
    } else {
        ControlMode::Dots
    };
    RenderedView {
        text: text::to_text(shown, mode, charset),
        notice: (bytes.len() > MAX_TEXT_BYTES).then(|| size_notice(MAX_TEXT_BYTES, bytes.len())),
    }
}
