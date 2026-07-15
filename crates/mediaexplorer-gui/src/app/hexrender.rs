use super::*;

/// Virtualized hex view: only the visible rows are formatted each frame.
///
/// `scroll_to_row` requests a one-shot scroll (e.g. to a search hit), and
/// `highlight_row` tints the current match row.
/// Render a virtualized hex dump and report any byte selection gesture.
///
/// Each visible block is painted into a single interactive region whose byte
/// cells are hit-tested, so clicks and drags map to byte offsets. `scroll_to_row`
/// and `highlight_row` keep the search behavior; `sel` paints the current
/// selection and cursor.
// A painting primitive: data, layout options, search-navigation and selection
// are all distinct inputs, so the argument count is inherent rather than a
// smell worth bundling into a struct.
#[allow(clippy::too_many_arguments)]
pub(crate) fn render_hex(
    ui: &mut egui::Ui,
    bytes: &[u8],
    bytes_per_row: usize,
    scroll_to_row: Option<usize>,
    highlight_row: Option<usize>,
    charset: MsxCharset,
    sel: HexSelection,
    opts: &HexViewOptions,
) -> Option<HexGesture> {
    let bpr = bytes_per_row.max(1);
    let total_rows = bytes.len().div_ceil(bpr);
    let font_id = egui::TextStyle::Monospace.resolve(ui.style());
    let row_height = ui.text_style_height(&egui::TextStyle::Monospace);
    // Monospace advance width, measured from a single laid-out glyph.
    let char_w = ui
        .painter()
        .layout_no_wrap("0".to_string(), font_id.clone(), egui::Color32::WHITE)
        .rect
        .width();
    let text_color = ui.visuals().text_color();
    let sel_color = ui.visuals().selection.bg_fill;
    let cursor_stroke = egui::Stroke::new(1.0_f32, ui.visuals().strong_text_color());

    let layout = HexLayout::new(opts, bpr, bytes.len());
    let content_width = layout.total_cols() as f32 * char_w;
    let cell_x = |origin_x: f32, col: usize| origin_x + col as f32 * char_w;

    // Non-scrolling column-offset header.
    if opts.show_columns {
        let width = content_width.max(ui.available_width());
        let (resp, painter) =
            ui.allocate_painter(egui::vec2(width, row_height), egui::Sense::hover());
        let mut buf = vec![' '; layout.total_cols()];
        if layout.show_line_numbers() {
            place(&mut buf, 0, "Offset");
        }
        if layout.show_hex() {
            for col in 0..bpr {
                if let Some(at) = layout.hex_cell_col(col) {
                    let label = if opts.line_number_hex {
                        format!("{:02X}", col & 0xFF)
                    } else {
                        format!("{:02}", col % 100)
                    };
                    place(&mut buf, at, &label);
                }
            }
        }
        let header: String = buf.into_iter().collect();
        painter.text(
            resp.rect.min,
            egui::Align2::LEFT_TOP,
            header,
            font_id.clone(),
            ui.visuals().weak_text_color(),
        );
    }

    let mut gesture = None;
    let mut area = egui::ScrollArea::vertical().auto_shrink([false, false]);
    if let Some(row) = scroll_to_row {
        // Center the target row in the viewport where possible.
        let offset = (row as f32 * row_height - 80.0).max(0.0);
        area = area.vertical_scroll_offset(offset);
    }
    area.show_rows(ui, row_height, total_rows, |ui, range| {
        let visible = range.len();
        let width = content_width.max(ui.available_width());
        let (resp, painter) = ui.allocate_painter(
            egui::vec2(width, visible as f32 * row_height),
            egui::Sense::click_and_drag(),
        );
        let origin = resp.rect.min;

        for (i, row) in range.clone().enumerate() {
            let y = origin.y + i as f32 * row_height;
            let offset = row * bpr;
            let chunk = &bytes[offset..(offset + bpr).min(bytes.len())];

            // Search-match row tint (behind everything). A weak tint of the
            // theme's selection color keeps the row's text readable in both
            // light and dark themes (a fixed DARK_BLUE swallowed dark text).
            if highlight_row == Some(row) {
                painter.rect_filled(
                    egui::Rect::from_min_size(
                        egui::pos2(origin.x, y),
                        egui::vec2(width, row_height),
                    ),
                    0.0,
                    sel_color.gamma_multiply(0.4),
                );
            }
            // Per-byte selection background and cursor outline.
            for col in 0..chunk.len() {
                let byte = offset + col;
                let selected = sel
                    .selection
                    .is_some_and(|(lo, hi)| (lo..=hi).contains(&byte));
                if selected {
                    if let Some(c) = layout.hex_cell_col(col) {
                        painter.rect_filled(
                            egui::Rect::from_min_size(
                                egui::pos2(cell_x(origin.x, c), y),
                                egui::vec2(char_w * 2.0, row_height),
                            ),
                            0.0,
                            sel_color,
                        );
                    }
                    if let Some(c) = layout.ascii_cell_col(col) {
                        painter.rect_filled(
                            egui::Rect::from_min_size(
                                egui::pos2(cell_x(origin.x, c), y),
                                egui::vec2(char_w, row_height),
                            ),
                            0.0,
                            sel_color,
                        );
                    }
                }
                if sel.cursor == Some(byte) {
                    if let Some(c) = layout.hex_cell_col(col) {
                        painter.rect_stroke(
                            egui::Rect::from_min_size(
                                egui::pos2(cell_x(origin.x, c), y),
                                egui::vec2(char_w * 2.0, row_height),
                            ),
                            0.0,
                            cursor_stroke,
                            egui::StrokeKind::Inside,
                        );
                    }
                }
            }

            // Build the address + hex part into a fixed-width char buffer so
            // every glyph lands on the character column the geometry uses for
            // hit-testing; these are always single-width ASCII, so one text
            // run is safe.
            let mut buf = vec![' '; layout.total_cols()];
            if layout.show_line_numbers() {
                place(&mut buf, 0, &layout.format_addr(offset));
            }
            if layout.show_hex() {
                for (col, &b) in chunk.iter().enumerate() {
                    if opts.hide_null_bytes && b == 0 {
                        continue;
                    }
                    if let Some(at) = layout.hex_cell_col(col) {
                        place(&mut buf, at, &format!("{b:02X}"));
                    }
                }
            }
            // The ASCII gutter: decoded high bytes (kana, semigraphics) come
            // from the Unifont fallback whose advance differs from the primary
            // monospace font, so putting them in the row's single text run
            // would drift the glyphs off the cells the geometry (selection,
            // click mapping) uses. Rows whose gutter is pure ASCII join the
            // single run (the common, cheap case); others are painted one
            // glyph per cell, shrinking glyphs wider than a cell to fit, so
            // every row stays the same width.
            let gutter: Vec<char> = if layout.show_ascii() {
                chunk
                    .iter()
                    .map(|&b| {
                        if opts.hide_null_bytes && b == 0 {
                            ' '
                        } else {
                            ascii_char(b, charset)
                        }
                    })
                    .collect()
            } else {
                Vec::new()
            };
            let uniform = gutter.iter().all(char::is_ascii);
            if uniform {
                for (col, &ch) in gutter.iter().enumerate() {
                    if let Some(at) = layout.ascii_cell_col(col) {
                        if let Some(slot) = buf.get_mut(at) {
                            *slot = ch;
                        }
                    }
                }
            }
            let line: String = buf.into_iter().collect();
            painter.text(
                egui::pos2(origin.x, y),
                egui::Align2::LEFT_TOP,
                line,
                font_id.clone(),
                text_color,
            );
            if !uniform {
                for (col, &ch) in gutter.iter().enumerate() {
                    if ch == ' ' {
                        continue;
                    }
                    let Some(at) = layout.ascii_cell_col(col) else {
                        continue;
                    };
                    let x = cell_x(origin.x, at);
                    let galley =
                        painter.layout_no_wrap(ch.to_string(), font_id.clone(), text_color);
                    if galley.rect.width() <= char_w * 1.02 {
                        painter.galley(egui::pos2(x, y), galley, text_color);
                    } else {
                        let shrunk = egui::FontId::new(
                            font_id.size * (char_w / galley.rect.width()),
                            font_id.family.clone(),
                        );
                        painter.text(
                            egui::pos2(x + 0.5 * char_w, y + 0.5 * row_height),
                            egui::Align2::CENTER_CENTER,
                            ch,
                            shrunk,
                            text_color,
                        );
                    }
                }
            }
        }

        // Map a pointer interaction to a byte and a gesture.
        if let Some(pos) = resp.interact_pointer_pos() {
            let rel_row = ((pos.y - origin.y) / row_height).floor();
            let local_x = pos.x - origin.x;
            if rel_row >= 0.0 && (rel_row as usize) < visible && char_w > 0.0 && local_x >= 0.0 {
                let row = range.start + rel_row as usize;
                if let Some((col, region)) = layout.byte_at_char((local_x / char_w) as usize) {
                    let byte = row * bpr + col;
                    if byte < bytes.len() {
                        gesture = if resp.drag_started() {
                            Some(HexGesture::DragStart { byte, region })
                        } else if resp.dragged() {
                            Some(HexGesture::DragTo(byte))
                        } else if resp.clicked() {
                            let shift = ui.input(|i| i.modifiers.shift);
                            Some(HexGesture::Click {
                                byte,
                                shift,
                                region,
                            })
                        } else {
                            None
                        };
                    }
                }
            }
        }
    });
    gesture
}

/// Overwrite characters in a monospace row buffer starting at character column
/// `at`, clipping anything past the buffer's end.
pub(crate) fn place(buf: &mut [char], at: usize, s: &str) {
    for (i, c) in s.chars().enumerate() {
        if let Some(slot) = buf.get_mut(at + i) {
            *slot = c;
        }
    }
}

/// Build a clipboard [`HexConfig`] that mirrors the on-screen view options.
pub(crate) fn hex_config(opts: &HexViewOptions) -> HexConfig {
    HexConfig {
        bytes_per_row: opts.bytes_per_row.max(1),
        base_address: 0,
        show_line_numbers: opts.show_line_numbers,
        line_number_hex: opts.line_number_hex,
        show_hex: opts.show_hex,
        show_ascii: opts.show_ascii,
        grouping: opts.grouping.size(),
        hide_null_bytes: opts.hide_null_bytes,
    }
}

/// Normalize two byte offsets into an inclusive `(lo, hi)` range.
pub(crate) fn normalize(a: usize, b: usize) -> (usize, usize) {
    (a.min(b), a.max(b))
}

/// Whether the platform copy shortcut (Cmd+C on macOS, Ctrl+C elsewhere) fired
/// this frame. egui-winit emits [`egui::Event::Copy`] for it, so this matches
/// the same gesture that copies selected text in a label or text field.
pub(crate) fn copy_event_pending(ctx: &egui::Context) -> bool {
    ctx.input(|i| i.events.iter().any(|e| matches!(e, egui::Event::Copy)))
}

/// Apply a hex gesture to a view's interaction state.
pub(crate) fn apply_hex_gesture(hs: &mut HexUiState, gesture: HexGesture) {
    match gesture {
        HexGesture::DragStart { byte, region } => {
            hs.drag_anchor = Some(byte);
            hs.cursor = Some(byte);
            hs.selection = None;
            hs.region = region;
        }
        HexGesture::DragTo(b) => {
            if let Some(a) = hs.drag_anchor {
                hs.selection = Some(normalize(a, b));
                hs.cursor = Some(a.min(b));
            }
        }
        HexGesture::Click {
            byte,
            shift,
            region,
        } => {
            if shift {
                let anchor = hs.cursor.unwrap_or(byte);
                hs.selection = Some(normalize(anchor, byte));
            } else {
                hs.cursor = Some(byte);
                hs.selection = None;
            }
            hs.drag_anchor = None;
            hs.region = region;
        }
    }
}

/// Parse a hexadecimal byte offset (with or without a `0x` prefix).
pub(crate) fn parse_offset(s: &str) -> Option<usize> {
    let t = s.trim();
    let t = t
        .strip_prefix("0x")
        .or_else(|| t.strip_prefix("0X"))
        .unwrap_or(t);
    if t.is_empty() {
        return None;
    }
    usize::from_str_radix(t, 16).ok()
}

/// A two-column label/value grid row.
pub(crate) fn kv_row(
    ui: &mut egui::Ui,
    key: impl Into<egui::WidgetText>,
    value: impl Into<String>,
) {
    ui.label(key);
    ui.monospace(value.into());
    ui.end_row();
}

/// The BSAVE-header grid, shared by the Info view and the data inspector.
pub(crate) fn bload_grid(ui: &mut egui::Ui, b: &msx_disk::fileinfo::BloadHeader, id_salt: &str) {
    egui::Grid::new(id_salt).num_columns(2).show(ui, |ui| {
        for (name, addr) in [
            (t!("info.start"), b.start),
            (t!("info.end"), b.end),
            (t!("info.exec"), b.exec),
        ] {
            kv_row(ui, format!("{name}:"), format!("0x{addr:04X}"));
        }
        kv_row(
            ui,
            t!("info.length"),
            t!("info.bytes", count => b.data_len()).into_owned(),
        );
    });
}

/// The data inspector: interpret the bytes at `origin` as common primitive
/// types and decode MSX structures (BSAVE header, boot-sector BPB, FCB) when the
/// bytes there parse as one.
pub(crate) fn render_inspector(
    ui: &mut egui::Ui,
    bytes: &[u8],
    origin: usize,
    charset: MsxCharset,
) {
    ui.horizontal(|ui| {
        ui.strong(t!("inspector.title"));
        ui.weak(format!("@ 0x{origin:06X}"));
    });
    ui.separator();
    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            let at = bytes.get(origin..).unwrap_or(&[]);
            if at.is_empty() {
                ui.weak(t!("inspector.cursor_past_end"));
                return;
            }
            egui::Grid::new("inspector_prims")
                .num_columns(2)
                .show(ui, |ui| {
                    let b = at[0];
                    kv_row(ui, "u8", b.to_string());
                    kv_row(ui, "i8", (b as i8).to_string());
                    kv_row(ui, "hex", format!("0x{b:02X}"));
                    kv_row(ui, "binary", format!("{b:08b}"));
                    kv_row(ui, "char", ascii_char(b, charset).to_string());
                    if at.len() >= 2 {
                        let v = u16::from_le_bytes([at[0], at[1]]);
                        kv_row(ui, "u16 (LE)", v.to_string());
                        kv_row(ui, "i16 (LE)", (v as i16).to_string());
                    }
                    if at.len() >= 3 {
                        let v = (at[0] as u32) | (at[1] as u32) << 8 | (at[2] as u32) << 16;
                        kv_row(ui, "u24 (LE)", v.to_string());
                    }
                    if at.len() >= 4 {
                        let v = u32::from_le_bytes([at[0], at[1], at[2], at[3]]);
                        kv_row(ui, "u32 (LE)", v.to_string());
                    }
                });

            if let Some(h) = msx_disk::fileinfo::bload::parse(at) {
                ui.add_space(6.0);
                ui.strong(t!("inspector.bsave_header"));
                bload_grid(ui, &h, "inspect_bload");
            }
            if let Some(bpb) = msx_disk::fs::map::Bpb::parse(at) {
                ui.add_space(6.0);
                ui.strong(t!("inspector.boot_sector"));
                egui::Grid::new("inspect_bpb")
                    .num_columns(2)
                    .show(ui, |ui| {
                        kv_row(
                            ui,
                            t!("inspector.bytes_per_sector"),
                            bpb.bytes_per_sector.to_string(),
                        );
                        kv_row(
                            ui,
                            t!("inspector.sectors_per_cluster"),
                            bpb.sectors_per_cluster.to_string(),
                        );
                        kv_row(
                            ui,
                            t!("inspector.reserved_sectors"),
                            bpb.reserved.to_string(),
                        );
                        kv_row(ui, t!("inspector.fat_copies"), bpb.num_fats.to_string());
                        kv_row(
                            ui,
                            t!("inspector.root_entries"),
                            bpb.root_entries.to_string(),
                        );
                        kv_row(
                            ui,
                            t!("inspector.sectors_per_fat"),
                            bpb.sectors_per_fat.to_string(),
                        );
                    });
            }
            if let Some(fcb) = msx_disk::fileinfo::fcb::parse(at) {
                ui.add_space(6.0);
                ui.strong("FCB");
                egui::Grid::new("inspect_fcb")
                    .num_columns(2)
                    .show(ui, |ui| {
                        let drive = if fcb.drive == 0 {
                            t!("inspector.drive_default").into_owned()
                        } else {
                            format!("{} ({}:)", fcb.drive, (b'A' + fcb.drive - 1) as char)
                        };
                        kv_row(ui, t!("inspector.drive"), drive);
                        kv_row(ui, t!("inspector.name"), fcb.name);
                        kv_row(
                            ui,
                            t!("inspector.current_block"),
                            fcb.current_block.to_string(),
                        );
                        kv_row(ui, t!("inspector.record_size"), fcb.record_size.to_string());
                        kv_row(
                            ui,
                            t!("inspector.file_size"),
                            t!("info.bytes", count => fcb.file_size).into_owned(),
                        );
                    });
            }
        });
}

/// Render a selected byte slice as space-separated hex pairs, or as decoded
/// ASCII/glyph characters under `charset`.
pub(crate) fn format_selected_bytes(slice: &[u8], ascii: bool, charset: MsxCharset) -> String {
    if ascii {
        slice.iter().map(|&b| ascii_char(b, charset)).collect()
    } else {
        slice
            .iter()
            .map(|b| format!("{b:02X}"))
            .collect::<Vec<_>>()
            .join(" ")
    }
}

/// A CRC32/SHA-1 grid with a per-value Copy button; copy requests land in `copy`.
pub(crate) fn checksum_grid(
    ui: &mut egui::Ui,
    id_salt: &str,
    cks: &msx_disk::Checksums,
    copy: &mut Option<String>,
) {
    egui::Grid::new(id_salt).num_columns(3).show(ui, |ui| {
        ui.label(t!("stats.crc32"));
        ui.monospace(cks.crc32_hex());
        if ui.small_button(t!("button.copy")).clicked() {
            *copy = Some(cks.crc32_hex());
        }
        ui.end_row();
        ui.label(t!("stats.sha1"));
        ui.monospace(cks.sha1_hex());
        if ui.small_button(t!("button.copy")).clicked() {
            *copy = Some(cks.sha1_hex());
        }
        ui.end_row();
    });
}

/// The display label for a largest-files row: the size and the file's name
/// decoded for display under `charset` (paths on disk are PUA-encoded, so a raw
/// path would show high bytes as tofu).
pub(crate) fn largest_file_label(size: u64, raw_path: &str, charset: MsxCharset) -> String {
    let display = charset::decode_fs_name(charset, raw_path);
    format!("{:>10}  {}", humanize_bytes(size), display)
}

/// Render one volume's statistics. `salt` keeps widget ids unique across
/// partitions; `path_prefix` (`""` or `P{n}/`) turns a largest-file row into a
/// jump target; a clicked row's full path lands in `jump`.
pub(crate) fn render_disk_stats(
    ui: &mut egui::Ui,
    s: &msx_disk::DiskStats,
    salt: usize,
    path_prefix: &str,
    charset: MsxCharset,
    jump: &mut Option<String>,
) {
    egui::Grid::new(("disk_stats_summary", salt))
        .num_columns(2)
        .show(ui, |ui| {
            kv_row(ui, t!("stats.total"), humanize_bytes(s.total_bytes));
            kv_row(ui, t!("stats.used"), humanize_bytes(s.used_bytes));
            kv_row(ui, t!("stats.free"), humanize_bytes(s.free_bytes));
            kv_row(ui, t!("stats.files"), s.file_count.to_string());
            kv_row(ui, t!("stats.directories"), s.dir_count.to_string());
            kv_row(
                ui,
                t!("stats.fragmentation"),
                format!("{:.1}%", s.fragmentation_pct),
            );
        });

    ui.add_space(8.0);
    ui.label(t!("stats.fat_integrity"));
    let integ = &s.integrity;
    if integ.is_clean() {
        ui.colored_label(egui::Color32::from_rgb(0x4C, 0xAF, 0x50), t!("stats.clean"));
    } else {
        let warn = ui.visuals().warn_fg_color;
        if !integ.lost_clusters.is_empty() {
            ui.colored_label(
                warn,
                t!("stats.lost_clusters", count => integ.lost_clusters.len()),
            );
        }
        if !integ.cross_linked.is_empty() {
            ui.colored_label(
                warn,
                t!("stats.cross_linked", count => integ.cross_linked.len()),
            );
        }
        if !integ.bad_pointers.is_empty() {
            ui.colored_label(
                warn,
                t!("stats.bad_pointers", count => integ.bad_pointers.len()),
            );
        }
    }

    if !s.extensions.is_empty() {
        ui.add_space(8.0);
        ui.label(t!("stats.by_file_type"));
        ui.monospace(format!(
            "{:<7} {:>5} {:>12}  {}",
            "Ext", "Count", "Bytes", "Description"
        ));
        for e in &s.extensions {
            let ext = if e.ext.is_empty() {
                "(none)".to_string()
            } else {
                display_ext(&e.ext)
            };
            ui.monospace(format!(
                "{:<7} {:>5} {:>12}  {}",
                ext,
                e.count,
                e.total_bytes,
                e.description.unwrap_or("")
            ));
        }
    }

    if !s.largest_files.is_empty() {
        ui.add_space(8.0);
        ui.label(t!("stats.largest_files"));
        for f in &s.largest_files {
            let label = largest_file_label(f.size, &f.path, charset);
            let resp = ui.add(
                egui::Label::new(egui::RichText::new(label).monospace())
                    .sense(egui::Sense::click()),
            );
            if resp.clicked() {
                // The jump target keeps the raw (PUA-encoded) path as the key.
                *jump = Some(format!("{path_prefix}{}", f.path));
            }
        }
    }
}
