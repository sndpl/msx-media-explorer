use super::*;

impl MediaExplorerApp {
    /// "View disk by sector" — a hex view of one sector, optionally editable.
    pub(crate) fn sector_panel(&mut self, ui: &mut egui::Ui) {
        let Some(sector_count) = self.disk.as_ref().map(LoadedDisk::sector_count) else {
            ui.weak(t!("status.no_disk_open"));
            return;
        };
        if sector_count == 0 {
            ui.weak(t!("disk.empty"));
            return;
        }
        if self.current_sector >= sector_count {
            self.current_sector = sector_count - 1;
        }
        let writable = self.disk_writable();
        ui.horizontal(|ui| {
            ui.label(t!("disk.sector"));
            let mut s = self.current_sector;
            if ui
                .add(egui::DragValue::new(&mut s).range(0..=sector_count - 1))
                .changed()
            {
                self.current_sector = s;
                self.sector_edit = None;
                self.sector_hex.clear_selection();
            }
            if ui.button("\u{25C0}").clicked() && self.current_sector > 0 {
                self.current_sector -= 1;
                self.sector_edit = None;
                self.sector_hex.clear_selection();
            }
            if ui.button("\u{25B6}").clicked() && self.current_sector + 1 < sector_count {
                self.current_sector += 1;
                self.sector_edit = None;
                self.sector_hex.clear_selection();
            }
            ui.label(format!(
                "/ {sector_count}    offset {:#08X}",
                self.current_sector * 512
            ));
            if self.sector_edit.is_some() {
                ui.separator();
                if ui.button(t!("disk.save_sector")).clicked() {
                    self.save_sector_edit();
                }
                if ui.button(t!("button.cancel")).clicked() {
                    self.sector_edit = None;
                }
            } else if writable && ui.button(t!("disk.edit_sector")).clicked() {
                if let Some(b) = self
                    .disk
                    .as_ref()
                    .and_then(|d| d.sector_bytes(self.current_sector))
                {
                    self.sector_edit = Some(format_hex_for_edit(&b));
                }
            }
            if self.sector_edit.is_none() {
                ui.separator();
                ui.checkbox(&mut self.show_inspector, t!("disk.inspector"));
            }
        });
        ui.horizontal(|ui| {
            ui.label(t!("disk.find_on_disk"));
            let resp = ui
                .add(egui::TextEdit::singleline(&mut self.disk_search_query).desired_width(160.0));
            ui.checkbox(&mut self.disk_search_is_hex, t!("disk.hex"));
            let submit = resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            if ui.button(t!("disk.find")).clicked() || submit {
                self.run_disk_search();
            }
            if !self.disk_search_matches.is_empty() {
                if ui.button("\u{25C0}").clicked() {
                    self.step_disk_search(false);
                }
                if ui.button("\u{25B6}").clicked() {
                    self.step_disk_search(true);
                }
                ui.label(format!(
                    "{}/{}",
                    self.disk_search_pos + 1,
                    self.disk_search_matches.len()
                ));
            }
        });
        ui.separator();

        if self.sector_edit.is_some() {
            egui::ScrollArea::vertical()
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    if let Some(text) = self.sector_edit.as_mut() {
                        ui.add(
                            egui::TextEdit::multiline(text)
                                .code_editor()
                                .desired_width(f32::INFINITY),
                        );
                    }
                });
            return;
        }
        let bytes = self
            .disk
            .as_ref()
            .and_then(|d| d.sector_slice(self.current_sector));
        if let Some(bytes) = bytes {
            let scroll = self.sector_scroll_row.take();
            let highlight = self
                .sector_highlight
                .filter(|(s, _)| *s == self.current_sector)
                .map(|(_, r)| r);
            if self.show_inspector {
                let origin = self
                    .sector_hex
                    .cursor
                    .or(self.sector_hex.selection.map(|(s, _)| s))
                    .unwrap_or(0);
                let charset = self.charset;
                egui::Panel::bottom("sector_inspector")
                    .resizable(true)
                    .default_size(170.0)
                    .show_inside(ui, |ui| render_inspector(ui, bytes, origin, charset));
            }
            let sel = HexSelection {
                cursor: self.sector_hex.cursor,
                selection: self.sector_hex.selection,
            };
            let opts = self.settings.hex;
            let gesture = render_hex(ui, bytes, 16, scroll, highlight, self.charset, sel, &opts);
            if let Some(g) = gesture {
                apply_hex_gesture(&mut self.sector_hex, g);
            }
            if self.sector_hex.selection.is_some()
                && !ui.ctx().egui_wants_keyboard_input()
                && copy_event_pending(ui.ctx())
            {
                let ascii = self.sector_hex.region == HexRegion::Ascii;
                if let Some(text) = self.sector_selection_text(ascii) {
                    self.copy_text_to_clipboard(text);
                }
            }
        }
    }

    pub(crate) fn run_disk_search(&mut self) {
        let matches = if self.disk_search_is_hex {
            match search::parse_hex(&self.disk_search_query) {
                Some(needle) => self
                    .disk
                    .as_ref()
                    .map(|d| search::find_bytes(d.data(), &needle))
                    .unwrap_or_default(),
                None => {
                    self.status = t!("disk.invalid_hex_pattern").to_string();
                    return;
                }
            }
        } else {
            self.disk
                .as_ref()
                .map(|d| search::find_text(d.data(), &self.disk_search_query, true))
                .unwrap_or_default()
        };
        if matches.is_empty() {
            self.status =
                t!("status.no_matches", query => self.disk_search_query.clone()).to_string();
            self.disk_search_matches.clear();
            return;
        }
        self.status = tn!("disk.matches_on_disk", matches.len()).to_string();
        self.disk_search_matches = matches;
        self.disk_search_pos = 0;
        self.jump_to_disk_match();
    }

    pub(crate) fn step_disk_search(&mut self, forward: bool) {
        let n = self.disk_search_matches.len();
        if n == 0 {
            return;
        }
        self.disk_search_pos = if forward {
            (self.disk_search_pos + 1) % n
        } else {
            (self.disk_search_pos + n - 1) % n
        };
        self.jump_to_disk_match();
    }

    pub(crate) fn jump_to_disk_match(&mut self) {
        if let Some(&off) = self.disk_search_matches.get(self.disk_search_pos) {
            let sector = off / 512;
            let row = (off % 512) / 16;
            self.current_sector = sector;
            self.sector_edit = None;
            self.sector_hex.clear_selection();
            self.sector_scroll_row = Some(row);
            self.sector_highlight = Some((sector, row));
        }
    }

    pub(crate) fn save_sector_edit(&mut self) {
        let Some(edited) = self.sector_edit.clone() else {
            return;
        };
        let Some(bytes) = search::parse_hex(&edited) else {
            self.status = t!("disk.invalid_hex_pairs").to_string();
            return;
        };
        if bytes.len() != 512 {
            self.status = t!("disk.sector_size_got", count => bytes.len()).to_string();
            return;
        }
        let idx = self.current_sector;
        let result = self.disk.as_mut().unwrap().write_sector(idx, &bytes);
        match result {
            Ok(()) => {
                self.status = t!("disk.saved_sector", sector => idx).to_string();
                self.sector_edit = None;
                self.content = None;
                self.selected = None;
                self.disk_map = self.disk.as_ref().and_then(LoadedDisk::disk_map);
            }
            Err(e) => self.status = t!("status.write_failed", error => e).to_string(),
        }
    }

    /// Graphical disk-usage map. Either a flat sector grid or a circular disk
    /// platter, per [`MapStyle`]; the selected file's sectors are outlined.
    pub(crate) fn map_panel(&mut self, ui: &mut egui::Ui) {
        if self.disk_map.is_none() {
            ui.weak(t!("disk.no_map"));
            return;
        }
        let file_set: std::collections::HashSet<usize> = self
            .selected
            .as_deref()
            .map(|p| {
                self.disk
                    .as_ref()
                    .map(|d| d.file_sectors(p))
                    .unwrap_or_default()
            })
            .unwrap_or_default()
            .into_iter()
            .collect();
        let geometry = self.disk.as_ref().map(|d| d.geometry);

        ui.horizontal_wrapped(|ui| {
            for (label, color) in [
                (t!("disk.legend_reserved"), kind_color(SectorKind::Reserved)),
                (t!("disk.legend_fat"), kind_color(SectorKind::Fat)),
                (t!("disk.legend_root"), kind_color(SectorKind::RootDir)),
                (t!("disk.legend_used"), kind_color(SectorKind::DataUsed)),
                (t!("disk.legend_free"), kind_color(SectorKind::DataFree)),
            ] {
                ui.colored_label(color, "\u{25A0}");
                ui.label(label);
                ui.add_space(6.0);
            }
            if !file_set.is_empty() {
                ui.separator();
                ui.label(t!("disk.white_outline"));
            }
        });
        ui.horizontal(|ui| {
            ui.selectable_value(&mut self.map_style, MapStyle::Grid, t!("disk.map_grid"));
            ui.selectable_value(&mut self.map_style, MapStyle::Disk, t!("disk.map_disk"));
        });
        ui.separator();

        let style = self.map_style;
        let Some(map) = self.disk_map.as_ref() else {
            return;
        };
        let clicked = match style {
            MapStyle::Grid => Self::map_grid(ui, map, &file_set),
            MapStyle::Disk => match geometry {
                Some(geo) => Self::map_disk(ui, map, geo, &file_set),
                None => {
                    ui.weak(t!("disk.no_geometry"));
                    None
                }
            },
        };
        if let Some(idx) = clicked {
            self.current_sector = idx;
            self.sector_edit = None;
            self.app_view = AppView::Sectors;
        }
    }

    /// Flat sector grid: each cell is one sector, colored by kind. Returns the
    /// clicked sector index, if any.
    pub(crate) fn map_grid(
        ui: &mut egui::Ui,
        map: &msx_disk::fs::map::DiskMap,
        file_set: &std::collections::HashSet<usize>,
    ) -> Option<usize> {
        const COLS: usize = 64;
        const CELL: f32 = 9.0;
        let count = map.sector_count;
        let rows = count.div_ceil(COLS);
        let mut clicked = None;
        egui::ScrollArea::both().show(ui, |ui| {
            let (resp, painter) = ui.allocate_painter(
                egui::vec2(COLS as f32 * CELL, rows as f32 * CELL),
                egui::Sense::click(),
            );
            let origin = resp.rect.min;
            for i in 0..count {
                let pos = origin + egui::vec2((i % COLS) as f32 * CELL, (i / COLS) as f32 * CELL);
                let rect = egui::Rect::from_min_size(pos, egui::vec2(CELL - 1.0, CELL - 1.0));
                painter.rect_filled(rect, 0.0, kind_color(map.kinds[i]));
                if file_set.contains(&i) {
                    painter.rect_stroke(
                        rect,
                        0.0,
                        egui::Stroke::new(1.5_f32, egui::Color32::WHITE),
                        egui::StrokeKind::Inside,
                    );
                }
            }
            if resp.clicked() {
                if let Some(p) = resp.interact_pointer_pos() {
                    let rel = p - origin;
                    if rel.x >= 0.0 && rel.y >= 0.0 {
                        let idx = (rel.y / CELL) as usize * COLS + (rel.x / CELL) as usize;
                        if idx < count {
                            clicked = Some(idx);
                        }
                    }
                }
            }
        });
        clicked
    }

    /// Circular disk-platter map: concentric rings are tracks (track 0 is the
    /// outermost ring), angular wedges are sectors, with one platter per side
    /// drawn side-by-side. Uses the same usage palette as the grid. Returns the
    /// clicked sector index, if any.
    pub(crate) fn map_disk(
        ui: &mut egui::Ui,
        map: &msx_disk::fs::map::DiskMap,
        geo: Geometry,
        file_set: &std::collections::HashSet<usize>,
    ) -> Option<usize> {
        use std::f32::consts::{PI, TAU};

        let sides = (geo.sides as usize).max(1);
        let tracks = (geo.tracks as usize).max(1);
        let spt = (geo.sectors_per_track as usize).max(1);
        let count = map.sector_count;

        const GAP: f32 = 16.0; // space between the two platters
        const MARGIN: f32 = 8.0; // padding around each platter
        let avail = ui.available_width().max(120.0);
        let d = ((avail - GAP * (sides as f32 - 1.0)) / sides as f32).clamp(160.0, 520.0);
        let r_out = d / 2.0 - MARGIN;
        let r_in = (r_out * 0.18).max(18.0);
        let bw = (r_out - r_in) / tracks as f32;
        let total_w = d * sides as f32 + GAP * (sides as f32 - 1.0);
        let total_h = d;
        let steps = 5usize; // arc subdivisions per wedge, for round-looking rings

        let mut clicked = None;
        egui::ScrollArea::both().show(ui, |ui| {
            let (resp, painter) =
                ui.allocate_painter(egui::vec2(total_w, total_h), egui::Sense::click());
            let origin = resp.rect.min;

            for h in 0..sides {
                let center = origin + egui::vec2(d / 2.0 + h as f32 * (d + GAP), d / 2.0);
                for track in 0..tracks {
                    let ro = r_out - track as f32 * bw;
                    let ri = (ro - bw + 0.5).max(r_in);
                    for sector in 0..spt {
                        let lba = (track * sides + h) * spt + sector;
                        if lba >= count {
                            continue;
                        }
                        let a0 = -PI / 2.0 + sector as f32 * (TAU / spt as f32);
                        let a1 = a0 + TAU / spt as f32;
                        let color = kind_color(map.kinds[lba]);

                        let mut mesh = egui::epaint::Mesh::default();
                        for s in 0..=steps {
                            let a = a0 + (a1 - a0) * s as f32 / steps as f32;
                            let (sin, cos) = a.sin_cos();
                            let dir = egui::vec2(cos, sin);
                            mesh.colored_vertex(center + dir * ri, color);
                            mesh.colored_vertex(center + dir * ro, color);
                        }
                        for s in 0..steps {
                            let base = (s * 2) as u32;
                            mesh.add_triangle(base, base + 1, base + 2);
                            mesh.add_triangle(base + 1, base + 3, base + 2);
                        }
                        painter.add(egui::Shape::mesh(mesh));

                        if file_set.contains(&lba) {
                            let stroke = egui::Stroke::new(1.0_f32, egui::Color32::WHITE);
                            let mut outer = Vec::with_capacity(steps + 1);
                            let mut inner = Vec::with_capacity(steps + 1);
                            for s in 0..=steps {
                                let a = a0 + (a1 - a0) * s as f32 / steps as f32;
                                let (sin, cos) = a.sin_cos();
                                let dir = egui::vec2(cos, sin);
                                outer.push(center + dir * ro);
                                inner.push(center + dir * ri);
                            }
                            painter.line_segment([outer[0], inner[0]], stroke);
                            painter.line_segment([outer[steps], inner[steps]], stroke);
                            painter.add(egui::Shape::line(outer, stroke));
                            painter.add(egui::Shape::line(inner, stroke));
                        }
                    }
                }

                painter.circle_filled(center, r_in, egui::Color32::from_gray(20));
                painter.text(
                    center,
                    egui::Align2::CENTER_CENTER,
                    t!("disk.side", n => h),
                    egui::FontId::proportional(13.0),
                    egui::Color32::from_gray(160),
                );
            }

            if resp.clicked() {
                if let Some(p) = resp.interact_pointer_pos() {
                    for h in 0..sides {
                        let center = origin + egui::vec2(d / 2.0 + h as f32 * (d + GAP), d / 2.0);
                        let rel = p - center;
                        let radius = rel.length();
                        if radius < r_in || radius > r_out {
                            continue;
                        }
                        let track = (((r_out - radius) / bw) as usize).min(tracks - 1);
                        let angle = (rel.y.atan2(rel.x) + PI / 2.0).rem_euclid(TAU);
                        let sector = ((angle / (TAU / spt as f32)) as usize).min(spt - 1);
                        let lba = (track * sides + h) * spt + sector;
                        if lba < count {
                            clicked = Some(lba);
                        }
                        break;
                    }
                }
            }
        });
        clicked
    }

    /// Bottom status bar. With a disk open it shows the physical disk type plus
    /// BPB-derived filesystem facts; with a tape open, tape facts; otherwise
    /// just the transient status text.
    pub(crate) fn status_bar(&self, ui: &mut egui::Ui) {
        if let Some(disk) = &self.disk {
            let geo = disk.geometry;
            // A hard disk's sides/tracks are a fabrication (they only carry the
            // byte total); describe it by its partitions instead.
            let description = if disk.is_partitioned() {
                describe_hard_disk(disk.partition_sizes())
            } else {
                describe_geometry(geo)
            };
            ui.label(egui::RichText::new(description).weak());
            ui.horizontal(|ui| {
                ui.label(t!("disk.size", value => humanize_bytes(geo.total_bytes() as u64)));
                if disk.is_partitioned() {
                    ui.separator();
                    ui.label(t!("disk.sectors", count => geo.total_sectors()));
                } else if let Some(fs) = self.disk_fs_geometry {
                    ui.separator();
                    ui.label(t!("disk.free", value => humanize_bytes(fs.free_bytes())));
                    ui.separator();
                    ui.label(t!("disk.clusters", count => fs.cluster_count));
                    ui.separator();
                    ui.label(t!("disk.sectors_per_cluster", count => fs.sectors_per_cluster));
                    ui.separator();
                    ui.label(t!("disk.bytes_per_sector", count => fs.bytes_per_sector));
                    ui.separator();
                    ui.label(t!("disk.sectors", count => fs.total_sectors));
                } else {
                    ui.separator();
                    ui.label(t!("disk.sectors", count => geo.total_sectors()));
                }
                if let Some(label) = &disk.label {
                    ui.separator();
                    ui.label(t!("disk.vol", label => label));
                }
                if !disk.writable() {
                    ui.separator();
                    ui.weak(t!("disk.read_only"));
                }
                ui.separator();
                ui.label(&self.status);
            });
        } else if let Some(tape) = &self.tape {
            ui.horizontal(|ui| {
                ui.label(t!("disk.tape", format => tape.format.label()));
                ui.separator();
                ui.label(t!("disk.tape_files", count => tape.file_count()));
                ui.separator();
                ui.label(t!("disk.tape_data", count => tape.total_bytes()));
                ui.separator();
                ui.weak(t!("disk.read_only"));
                ui.separator();
                ui.label(&self.status);
            });
        } else {
            ui.label(&self.status);
        }
    }

    /// The DMK per-track analysis table (only shown for `.dmk` disks).
    pub(crate) fn analyze_panel(&mut self, ui: &mut egui::Ui) {
        let Some(analysis) = &self.dmk_analysis else {
            ui.weak(t!("disk.no_dmk"));
            return;
        };
        let standard = analysis
            .track_infos
            .iter()
            .filter(|t| t.is_standard())
            .count();
        ui.label(format!(
            "{} tracks x {} side(s){} — {}/{} tracks standard (9x512 MFM)",
            analysis.tracks,
            analysis.sides,
            if analysis.write_protected {
                ", write-protected"
            } else {
                ""
            },
            standard,
            analysis.track_infos.len(),
        ));
        if analysis.single_sided_in_two_sided_container() {
            ui.colored_label(
                egui::Color32::from_rgb(0xE0, 0xA0, 0x30),
                "Single-sided disk in a two-sided container: the header declares 2 \
                 sides but only side 0 is formatted. Read as a 360KB single-sided image.",
            );
        }
        ui.separator();
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                ui.monospace("Track Side Sectors Size Dens CRC   Notes");
                for t in &analysis.track_infos {
                    ui.monospace(format_dmk_track_row(t));
                }
            });
    }

    /// The tape block overview (only shown for tapes).
    pub(crate) fn blocks_panel(&mut self, ui: &mut egui::Ui) {
        let Some(tape) = &self.tape else {
            ui.weak(t!("disk.no_tape_open"));
            return;
        };
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                for (i, block) in tape.tape.blocks.iter().enumerate() {
                    render_tape_block(ui, i + 1, block);
                    ui.separator();
                }
            });
    }
}
