use super::*;

/// Format one DMK track as a fixed-width row for the analysis table.
pub(crate) fn format_dmk_track_row(t: &DmkTrackInfo) -> String {
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
pub(crate) fn render_tape_block(ui: &mut egui::Ui, number: usize, block: &TapeBlock) {
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
pub(crate) fn kind_color(kind: SectorKind) -> egui::Color32 {
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
pub(crate) fn format_attributes(attrs: msx_disk::fs::Attributes) -> String {
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
pub(crate) fn format_timestamp(ts: Option<msx_disk::fs::Timestamp>) -> String {
    match ts {
        Some(t) => format!(
            "{:04}-{:02}-{:02} {:02}:{:02}",
            t.year, t.month, t.day, t.hour, t.minute
        ),
        None => String::new(),
    }
}

/// Monospace character width of a file row's name field, including the gap to
/// the data columns. The columns are painted at this fixed pixel offset (see
/// [`tree_row_cols`]) so a name whose glyphs fall back to a different font
/// (kana names render through Unifont, whose advance differs from the primary
/// monospace font) cannot shift them.
pub(crate) const NAME_FIELD_CHARS: usize = 15;

/// Character width of a full file row: `name(14) + ' ' + size(8) + "  " +
/// date(16) + "  " + attrs(4)` = 47. MSX 8.3 names never exceed the 14-wide
/// name field, so every row is exactly this wide.
pub(crate) const FILE_ROW_CHARS: usize = 47;

/// Default width for the file tree panel: wide enough that a full monospace file
/// row fits on one line, plus room for the folder indent, scrollbar, and margins.
pub(crate) fn files_panel_default_width(ui: &egui::Ui) -> f32 {
    mono_width(ui, &"0".repeat(FILE_ROW_CHARS)) + 64.0
}

/// Pixel width of `s` laid out in the monospace font (for fixed column offsets).
fn mono_width(ui: &egui::Ui, s: &str) -> f32 {
    let font = egui::TextStyle::Monospace.resolve(ui.style());
    ui.painter()
        .layout_no_wrap(s.to_owned(), font, egui::Color32::WHITE)
        .rect
        .width()
}

/// Build a file row's data columns: size + date/time + attributes. The name is
/// drawn separately (see [`tree_row_cols`]). The date/time and attribute
/// columns are always shown (MSX-DOS 1 disks carry them too); an absent
/// timestamp renders as a blank date column.
pub(crate) fn file_row_columns(entry: &DirEntry) -> String {
    format!(
        "{:>8}  {:<16}  {}",
        entry.size,
        format_timestamp(entry.modified),
        format_attributes(entry.attributes)
    )
}

/// Message shown in the Files panel when a disk mounts but its FAT directory
/// holds no entries (a blank disk, or a sector-based / non-DOS disk that stores
/// no files in a FAT). Points the user at the raw views.
pub(crate) fn empty_fat_message() -> &'static str {
    "This disk's FAT directory is empty — no files to list.\n\n\
     It may be a blank disk or a sector-based (non-DOS) disk. \
     Use the Sectors or Map view to inspect its raw contents."
}

/// Read-only context threaded through [`render_entries`]: what is selected /
/// collapsed / highlighted, plus the display settings. Keeps the recursive
/// renderer to a few arguments.
pub(crate) struct TreeRender<'a> {
    pub(crate) selection: &'a BTreeSet<String>,
    pub(crate) collapsed: &'a BTreeSet<String>,
    /// The keyboard-cursor row's path, highlighted and scrolled into view.
    pub(crate) cursor: Option<&'a str>,
    /// One-shot: scroll the cursor row into view this frame.
    pub(crate) scroll_to_cursor: bool,
    pub(crate) writable: bool,
    pub(crate) charset: MsxCharset,
}

/// A selectable tree row drawn with an explicit, stable `id` rather than egui's
/// positional auto-id.
///
/// Each row's id is derived from its (unique) file path, so it stays attached
/// to the same file as the tree reflows. Collapsing/expanding a directory
/// shifts the rows below it; with positional ids a row's rect would inherit the
/// id of whatever row previously sat there, tripping egui's debug
/// `warn_if_rect_changes_id` overlay (a one-frame red border). A stable id
/// keeps "same rect, new id" from happening. Mirrors `selectable_label`'s look.
pub(crate) fn tree_row(
    ui: &mut egui::Ui,
    id: egui::Id,
    selected: bool,
    text: egui::RichText,
    sense: egui::Sense,
) -> egui::Response {
    tree_row_cols(ui, id, selected, text, None, sense)
}

/// [`tree_row`] with the row split into a name part and a data-columns part
/// painted at a fixed pixel offset (`cols` = offset from the text origin, and
/// the columns text). Fixed pixel columns — rather than one space-padded
/// string — keep the size/date/attribute columns aligned even when the name's
/// glyphs come from a fallback font with a different advance width (kana
/// filenames). The name is truncated at the column start so it can never run
/// underneath the columns.
pub(crate) fn tree_row_cols(
    ui: &mut egui::Ui,
    id: egui::Id,
    selected: bool,
    text: egui::RichText,
    cols: Option<(f32, egui::RichText)>,
    sense: egui::Sense,
) -> egui::Response {
    let padding = ui.spacing().button_padding;
    let (wrap, max_width) = match &cols {
        Some((col_x, _)) => (egui::TextWrapMode::Truncate, *col_x),
        None => (egui::TextWrapMode::Extend, f32::INFINITY),
    };
    let galley = egui::WidgetText::from(text).into_galley(
        ui,
        Some(wrap),
        max_width,
        egui::TextStyle::Button,
    );
    let cols_galley = cols.map(|(col_x, text)| {
        let g = egui::WidgetText::from(text).into_galley(
            ui,
            Some(egui::TextWrapMode::Extend),
            f32::INFINITY,
            egui::TextStyle::Button,
        );
        (col_x, g)
    });
    let content = match &cols_galley {
        Some((col_x, g)) => egui::vec2(
            (col_x + g.size().x).max(galley.size().x),
            galley.size().y.max(g.size().y),
        ),
        None => galley.size(),
    };
    let mut desired = content + 2.0 * padding;
    desired.y = desired.y.max(ui.spacing().interact_size.y);
    let (_, rect) = ui.allocate_space(desired);
    let response = ui.interact(rect, id, sense);
    if ui.is_rect_visible(rect) {
        let visuals = ui.style().interact_selectable(&response, selected);
        if selected || response.hovered() || response.has_focus() {
            ui.painter().rect(
                rect,
                visuals.corner_radius,
                visuals.weak_bg_fill,
                visuals.bg_stroke,
                egui::StrokeKind::Inside,
            );
        }
        let text_pos = egui::pos2(
            rect.min.x + padding.x,
            rect.center().y - 0.5 * galley.size().y,
        );
        ui.painter()
            .galley(text_pos, galley, visuals.fg_stroke.color);
        if let Some((col_x, g)) = cols_galley {
            let pos = egui::pos2(
                rect.min.x + padding.x + col_x,
                rect.center().y - 0.5 * g.size().y,
            );
            ui.painter().galley(pos, g, visuals.fg_stroke.color);
        }
    }
    response
}

/// The shared "Add files here… / Add directory…" entries, targeting `dir`
/// ("" = disk root). Used by file, directory, and root-row context menus.
pub(crate) fn add_menu_items(ui: &mut egui::Ui, events: &mut RowEvents, dir: &str) {
    if ui.button("Add files here…").clicked() {
        events.action = Some(RowAction::AddFiles(dir.to_string()));
        ui.close();
    }
    if ui.button("Add directory…").clicked() {
        events.action = Some(RowAction::AddDir(dir.to_string()));
        ui.close();
    }
}

/// Render the disk's `/` root row followed by its (indented) contents. The root
/// is selectable and a right-click/drop target for adding to the disk root, but
/// not collapsible (a disk has one root), so it carries no expand triangle and
/// is always shown expanded. An empty disk still shows `/` plus a hint.
pub(crate) fn render_tree_with_root(
    ui: &mut egui::Ui,
    entries: &[DirEntry],
    ctx: &TreeRender,
    events: &mut RowEvents,
) {
    let is_cursor = ctx.cursor == Some(ROOT_PATH);
    let id = ui.make_persistent_id(ROOT_PATH);
    let header = egui::RichText::new("/").monospace();
    let resp = tree_row(ui, id, is_cursor, header, egui::Sense::click());
    events.drop_targets.push((resp.rect, ROOT_PATH.to_string()));
    if is_cursor && ctx.scroll_to_cursor {
        resp.scroll_to_me(Some(egui::Align::Center));
    }
    if resp.clicked() {
        events.cursor_to = Some(ROOT_PATH.to_string());
    }
    if ctx.writable {
        resp.context_menu(|ui| add_menu_items(ui, events, ROOT_PATH));
    }
    ui.indent(ROOT_PATH, |ui| {
        if entries.is_empty() {
            ui.weak(empty_fat_message());
        } else {
            render_entries(ui, entries, ctx, events);
        }
    });
}

pub(crate) fn render_entries(
    ui: &mut egui::Ui,
    entries: &[DirEntry],
    ctx: &TreeRender,
    events: &mut RowEvents,
) {
    for entry in entries {
        let is_cursor = ctx.cursor == Some(entry.path.as_str());
        // Stable, path-keyed id so the row keeps its identity across reflows.
        let id = ui.make_persistent_id(&entry.path);
        // Where a file dropped on (or "add" invoked from) this row should land:
        // a directory targets itself, a file its parent folder.
        let target = add_target_dir(&entry.path, entry.is_dir);
        if entry.is_dir {
            // App-managed expansion (so the keyboard can drive it): a leading
            // triangle shows the state, and clicking the header toggles it.
            // The date/attribute columns are painted at a fixed pixel offset
            // (measured from a constant sample prefix, so expanded/collapsed
            // rows align too); an absent timestamp (e.g. synthetic partition
            // nodes) renders blank.
            let expanded = !ctx.collapsed.contains(&entry.path);
            let arrow = if expanded { '\u{25BC}' } else { '\u{25B6}' };
            let header = egui::RichText::new(format!(
                "{arrow} \u{1F4C1} {}",
                entry.display_name(ctx.charset)
            ))
            .monospace();
            let cols = egui::RichText::new(format!(
                "{:<16}  {}",
                format_timestamp(entry.modified),
                format_attributes(entry.attributes)
            ))
            .monospace();
            let col_x = mono_width(ui, &format!("\u{25BC} \u{1F4C1} {}", "0".repeat(14)));
            let resp = tree_row_cols(
                ui,
                id,
                is_cursor,
                header,
                Some((col_x, cols)),
                egui::Sense::click(),
            );
            events.drop_targets.push((resp.rect, target.clone()));
            if is_cursor && ctx.scroll_to_cursor {
                resp.scroll_to_me(Some(egui::Align::Center));
            }
            if resp.clicked() {
                events.toggle_dir = Some(entry.path.clone());
                events.cursor_to = Some(entry.path.clone());
            }
            if ctx.writable {
                resp.context_menu(|ui| {
                    add_menu_items(ui, events, &target);
                    ui.separator();
                    if ui.button("Rename…").clicked() {
                        events.action = Some(RowAction::Rename(entry.path.clone()));
                        ui.close();
                    }
                    if ui.button("Remove directory").clicked() {
                        events.action = Some(RowAction::RemoveDir(entry.path.clone()));
                        ui.close();
                    }
                });
            }
            if expanded {
                ui.indent(&entry.path, |ui| {
                    render_entries(ui, &entry.children, ctx, events);
                });
            }
        } else {
            let is_selected = is_cursor || ctx.selection.contains(&entry.path);
            let name = egui::RichText::new(entry.display_name(ctx.charset)).monospace();
            let cols = egui::RichText::new(file_row_columns(entry)).monospace();
            let col_x = mono_width(ui, &"0".repeat(NAME_FIELD_CHARS));
            let resp = tree_row_cols(
                ui,
                id,
                is_selected,
                name,
                Some((col_x, cols)),
                egui::Sense::click_and_drag(),
            );
            events.drop_targets.push((resp.rect, target.clone()));
            if is_cursor && ctx.scroll_to_cursor {
                resp.scroll_to_me(Some(egui::Align::Center));
            }
            if resp.clicked() {
                let toggle = ui.input(|i| i.modifiers.command);
                events.clicked = Some((entry.path.clone(), toggle));
                events.cursor_to = Some(entry.path.clone());
            }
            if resp.drag_started() {
                events.drag_started = Some(entry.path.clone());
            }
            resp.context_menu(|ui| {
                if ctx.writable {
                    add_menu_items(ui, events, &target);
                    ui.separator();
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
pub(crate) fn render_tape_files(
    ui: &mut egui::Ui,
    tape: &LoadedTape,
    ctx: &TreeRender,
    events: &mut RowEvents,
) {
    if tape.file_count() == 0 {
        ui.weak("Tape has no recognizable files.");
        return;
    }
    for (key, file) in tape.entries() {
        let is_cursor = ctx.cursor == Some(key);
        let is_selected = is_cursor || ctx.selection.contains(key);
        // Same fixed-pixel columns as the disk tree (see `tree_row_cols`): a
        // name with fallback-font glyphs must not shift the kind/size columns.
        let id = ui.make_persistent_id(key);
        let name = egui::RichText::new(key).monospace();
        let cols = egui::RichText::new(format!("{:<7} {:>8}", file.kind.label(), file.data.len()))
            .monospace();
        let col_x = mono_width(ui, &"0".repeat(NAME_FIELD_CHARS));
        let resp = tree_row_cols(
            ui,
            id,
            is_selected,
            name,
            Some((col_x, cols)),
            egui::Sense::click_and_drag(),
        );
        if is_cursor && ctx.scroll_to_cursor {
            resp.scroll_to_me(Some(egui::Align::Center));
        }
        if resp.clicked() {
            let toggle = ui.input(|i| i.modifiers.command);
            events.clicked = Some((key.to_string(), toggle));
            events.cursor_to = Some(key.to_string());
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
