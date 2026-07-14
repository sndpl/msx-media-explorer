use super::*;

/// Format bytes as editable hex: 16 space-separated pairs per line.
pub(crate) fn format_hex_for_edit(bytes: &[u8]) -> String {
    use std::fmt::Write;
    let mut out = String::new();
    for (i, b) in bytes.iter().enumerate() {
        if i > 0 {
            out.push(if i % 16 == 0 { '\n' } else { ' ' });
        }
        let _ = write!(out, "{b:02X}");
    }
    out
}

/// Coerce a host filename into an MSX-DOS 8.3 uppercase name.
/// The final path component (file name) of a slash-separated disk path.
pub(crate) fn base_name(path: &str) -> String {
    path.rsplit(['/', '\\'])
        .next()
        .unwrap_or("file")
        .to_string()
}

/// Placeholder for an extension character that cannot be shown in a column-
/// aligned monospace table. ASCII `?` is guaranteed to be a single cell wide in
/// the monospace font (matching the `.` the text view uses for control bytes),
/// whereas control glyphs and wide/undecoded characters are not.
pub(crate) const EXT_PLACEHOLDER: char = '?';

/// A file extension made safe for a monospace, column-aligned table. Real MSX
/// extensions are ASCII, but "strange" filenames can carry control bytes (e.g.
/// SUB/FF) or undecoded high bytes whose glyphs render at an unpredictable
/// width and break column alignment. Each such character is swapped one-for-one
/// for [`EXT_PLACEHOLDER`], so the visible width matches the char count the
/// padding was computed from.
pub(crate) fn display_ext(ext: &str) -> String {
    ext.chars()
        .map(|c| {
            if c.is_ascii_graphic() {
                c
            } else {
                EXT_PLACEHOLDER
            }
        })
        .collect()
}

/// Truncate `s` to at most `max` characters, marking elision with a trailing
/// ellipsis so the monospace member table stays aligned.
pub(crate) fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let mut out: String = s.chars().take(max.saturating_sub(1)).collect();
        out.push('\u{2026}');
        out
    }
}

/// Turn a stored archive member path into a safe relative host path: split on
/// both separators and drop empty, `.`, and `..` components. This preserves
/// subdirectories while preventing path traversal outside the chosen folder.
pub(crate) fn sanitize_member_path(path: &str) -> PathBuf {
    let mut out = PathBuf::new();
    for part in path.split(['/', '\\']) {
        if part.is_empty() || part == "." || part == ".." {
            continue;
        }
        out.push(part);
    }
    if out.as_os_str().is_empty() {
        out.push("extracted");
    }
    out
}

/// A human-readable description of the physical disk format, e.g.
/// `3.5" Double Sided, Double Density (2DD): 2 sides × 80 tracks × 9
/// sectors/track × 512 bytes/sector = 737280 bytes (720 kB)`.
pub(crate) fn describe_geometry(geo: Geometry) -> String {
    let bytes = geo.total_bytes();
    let name = match (geo.sides, geo.tracks, geo.sectors_per_track) {
        (1, 80, 9) => Some("3.5\" Single Sided, Double Density (1DD)"),
        (2, 80, 9) => Some("3.5\" Double Sided, Double Density (2DD)"),
        (1, 40, 9) => Some("5.25\" Single Sided, Double Density (SS,DD)"),
        (2, 40, 9) => Some("5.25\" Double Sided, Double Density (DS,DD)"),
        _ => None,
    };
    let arithmetic = format!(
        "{} side{} \u{00D7} {} tracks \u{00D7} {} sectors/track \u{00D7} {} bytes/sector = {bytes} bytes ({} kB)",
        geo.sides,
        if geo.sides == 1 { "" } else { "s" },
        geo.tracks,
        geo.sectors_per_track,
        SECTOR_SIZE,
        bytes / 1024,
    );
    match name {
        Some(n) => format!("{n}: {arithmetic}"),
        None => arithmetic,
    }
}

/// A human-readable description of a partitioned hard-disk image, e.g.
/// `Hard disk image: 4 partitions (32.00 MB, 32.00 MB, 32.00 MB, 32.00 MB)`.
/// Used instead of [`describe_geometry`], whose sides/tracks are a meaningless
/// fabrication for a hard disk. `sizes` is each partition's byte size.
pub(crate) fn describe_hard_disk(sizes: &[u64]) -> String {
    let n = sizes.len();
    let unit = if n == 1 { "partition" } else { "partitions" };
    if sizes.is_empty() {
        return "Hard disk image".to_string();
    }
    let list = sizes
        .iter()
        .map(|&b| humanize_bytes(b))
        .collect::<Vec<_>>()
        .join(", ");
    format!("Hard disk image: {n} {unit} ({list})")
}

/// Whether `path`'s lowercased extension is in `exts`.
pub(crate) fn ext_in(path: &Path, exts: &[&str]) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase())
        .is_some_and(|e| exts.contains(&e.as_str()))
}

/// Whether `path`'s extension marks it as an openable disk image.
pub(crate) fn is_disk_image(path: &Path) -> bool {
    ext_in(path, DISK_IMAGE_EXTS)
}

/// Whether `path`'s extension marks it as a tape image.
pub(crate) fn is_tape(path: &Path) -> bool {
    ext_in(path, TAPE_EXTS)
}

/// Whether `path` can be opened as a document (disk or tape).
pub(crate) fn is_openable(path: &Path) -> bool {
    is_disk_image(path) || is_tape(path)
}

/// Punctuation allowed in an MSX (FAT 8.3) filename, besides ASCII
/// alphanumerics. Shared by [`sanitize_msx_name`] and [`normalize_msx_input`].
pub(crate) const MSX_NAME_PUNCT: &str = "_-!#$%&@^{}()~'";

/// True if `c` is allowed in an MSX 8.3 filename (ASCII rule only).
pub(crate) fn is_msx_name_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || MSX_NAME_PUNCT.contains(c)
}

/// True if `c` is allowed in a rename under `charset`. ASCII follows the 8.3
/// rule; a high glyph (kana, accented Latin, ...) is allowed when the charset
/// can encode it, so non-ASCII names survive editing instead of being stripped.
pub(crate) fn is_rename_char(c: char, charset: MsxCharset) -> bool {
    if (c as u32) < 0x80 {
        is_msx_name_char(c)
    } else {
        charset::encode_byte(charset, c).is_some()
    }
}

/// Coerce `name` into a valid 8.3 shape: ASCII-uppercase, keep only characters
/// for which `allowed` holds, cap the stem at 8 and extension at 3 characters
/// (counted as characters, since one MSX byte is one glyph). An empty stem
/// becomes `FILE`.
pub(crate) fn sanitize_8_3(name: &str, allowed: impl Fn(char) -> bool) -> String {
    let upper: String = name.chars().map(|c| c.to_ascii_uppercase()).collect();
    let (stem, ext) = match upper.rsplit_once('.') {
        Some((s, e)) => (s, e),
        None => (upper.as_str(), ""),
    };
    let keep =
        |s: &str, max: usize| -> String { s.chars().filter(|c| allowed(*c)).take(max).collect() };
    let mut stem = keep(stem, 8);
    if stem.is_empty() {
        stem = "FILE".to_string();
    }
    let ext = keep(ext, 3);
    if ext.is_empty() {
        stem
    } else {
        format!("{stem}.{ext}")
    }
}

/// Coerce a dropped host filename into a valid ASCII 8.3 MSX name.
pub(crate) fn sanitize_msx_name(name: &str) -> String {
    sanitize_8_3(name, is_msx_name_char)
}

/// Restrict free-form rename input to a valid 8.3 shape *as it is typed*:
/// ASCII-uppercase, only characters allowed under `charset`, at most one `.`
/// separator, an 8-char stem and a 3-char extension. Unlike [`sanitize_8_3`] it
/// leaves an empty stem empty (so the field can be cleared) and keeps a trailing
/// `.` so the extension can still be typed.
pub(crate) fn normalize_msx_input(raw: &str, charset: MsxCharset) -> String {
    let mut stem = String::new();
    let mut ext = String::new();
    let mut in_ext = false;
    for c in raw.chars() {
        let c = c.to_ascii_uppercase();
        if c == '.' {
            // The first dot starts the extension; later dots are ignored.
            in_ext = true;
        } else if is_rename_char(c, charset) {
            if in_ext {
                if ext.chars().count() < 3 {
                    ext.push(c);
                }
            } else if stem.chars().count() < 8 {
                stem.push(c);
            }
        }
    }
    if in_ext {
        format!("{stem}.{ext}")
    } else {
        stem
    }
}

/// The stem (pre-extension) part of a normalized 8.3 name; empty means the name
/// has no usable base and cannot be applied.
pub(crate) fn msx_name_stem(name: &str) -> &str {
    name.split('.').next().unwrap_or("")
}

/// Extensions that default to the Text view.
pub(crate) const TEXT_EXTENSIONS: &[&str] = &[
    "txt", "bat", "asc", "doc", "me", "ini", "cfg", "diz", "nfo", "log", "csv", "md", "hlp",
];

/// Pick a sensible default view mode for a file based on its extension.
pub(crate) fn default_view_mode(path: &str) -> ViewMode {
    let ext = path.rsplit('.').next().unwrap_or("").to_ascii_lowercase();
    if msx_disk::archive::is_archive(path) {
        ViewMode::Archive
    } else if recoil::is_supported(path) {
        ViewMode::Screen
    } else if ext == "bas" {
        ViewMode::Basic
    } else if msx_disk::fileinfo::music::is_music_ext(&ext) {
        ViewMode::Info
    } else if matches!(ext.as_str(), "com" | "cpm" | "bin") {
        ViewMode::Disasm
    } else if TEXT_EXTENSIONS.contains(&ext.as_str()) {
        ViewMode::Text
    } else {
        ViewMode::Hex
    }
}

/// Decode the selected file as an MSX image: forcing `forced` when set,
/// otherwise classifying by file extension. The single seam shared by the
/// Screen view, Copy image, and Save PNG so they never diverge.
pub(crate) fn decode_screen(
    forced: Option<recoil::ImageFormat>,
    path: &str,
    bytes: &[u8],
    companions: &dyn recoil::CompanionFiles,
) -> Option<recoil::Image> {
    match forced {
        Some(fmt) => recoil::decode_as(fmt, bytes, companions),
        None => recoil::decode(path, bytes, companions),
    }
}

/// The message shown in the Screen view when decoding fails. It names the
/// forced format so a failed manual attempt reads differently per format (and
/// differently from the initial auto-detect failure), giving explicit feedback
/// that an attempt was made.
pub(crate) fn screen_decode_failed_message(forced: Option<recoil::ImageFormat>) -> String {
    match forced {
        Some(fmt) => format!(
            "Could not decode as {}. Try another format below.",
            fmt.label()
        ),
        None => {
            "Not a recognized MSX graphics file. Pick a format below to try decoding it anyway."
                .to_string()
        }
    }
}

/// Decode the embedded app icon into a GPU texture for the About window. On the
/// (unexpected) decode failure of our own bundled asset, falls back to a 1x1
/// transparent pixel so the window still opens.
pub(crate) fn load_about_icon(ctx: &egui::Context) -> egui::TextureHandle {
    let color_image = image::load_from_memory(ICON_PNG)
        .map(|img| {
            let rgba = img.to_rgba8();
            let (width, height) = rgba.dimensions();
            egui::ColorImage::from_rgba_unmultiplied(
                [width as usize, height as usize],
                rgba.as_raw(),
            )
        })
        .unwrap_or_else(|_| egui::ColorImage::new([1, 1], vec![egui::Color32::TRANSPARENT]));
    ctx.load_texture("about_icon", color_image, egui::TextureOptions::LINEAR)
}

/// The Version / Built / Commit table in the About window. Labels are right-
/// aligned in a muted colour against a monospace value column. Rows with no
/// value (no git checkout) are omitted.
pub(crate) fn about_meta_grid(ui: &mut egui::Ui) {
    egui::Grid::new("about_meta")
        .num_columns(2)
        .spacing([12.0, 6.0])
        .show(ui, |ui| {
            meta_label(ui, "Version");
            ui.label(egui::RichText::new(VERSION).monospace());
            ui.end_row();

            if !BUILD_DATE.is_empty() {
                meta_label(ui, "Built");
                ui.label(egui::RichText::new(BUILD_DATE).monospace());
                ui.end_row();
            }

            if !GIT_HASH.is_empty() {
                meta_label(ui, "Commit");
                ui.label(egui::RichText::new(GIT_HASH).monospace());
                ui.end_row();
            }
        });
}

/// A right-aligned, muted label cell for [`about_meta_grid`].
pub(crate) fn meta_label(ui: &mut egui::Ui, text: &str) {
    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
        ui.label(egui::RichText::new(text).weak());
    });
}

/// An action chosen from the Screen view's right-click menu, run by the caller
/// once the borrows the menu was built under have been released.
#[derive(Clone, Copy)]
pub(crate) enum ScreenAction {
    CopyImage,
    SavePng,
}

/// Outcome of [`render_screen`]: whether an image was shown (false on decode
/// failure, so the caller can offer the format picker) and any action the user
/// triggered from the image's right-click menu.
pub(crate) struct ScreenRender {
    pub(crate) shown: bool,
    pub(crate) action: Option<ScreenAction>,
}

/// Display size for a decoded screen image: the native 2x nearest-neighbour
/// zoom, shrunk uniformly so the whole image fits within `available`. Large
/// pages (e.g. 512x1408 Dynamic Publisher pictures, or SCREEN 6 stamps) then
/// fit the panel without the user having to resize the window, while small
/// SCREEN 2 images are never enlarged past 2x.
pub(crate) fn screen_display_size(texture: egui::Vec2, available: egui::Vec2) -> egui::Vec2 {
    let native = texture * 2.0;
    if native.x <= 0.0 || native.y <= 0.0 {
        return native;
    }
    let scale = (available.x / native.x)
        .min(available.y / native.y)
        .clamp(0.0, 1.0);
    native * scale
}

/// Vertical space (separator + picker row) reserved below the image when the
/// format picker is shown, so a fit-to-height image does not crowd it out.
pub(crate) const SCREEN_PICKER_HEADROOM: f32 = 40.0;

/// Render a decoded MSX graphics image, caching the GPU texture by `(path,
/// forced format)`. The image is shrunk to fit the panel and carries a
/// right-click menu for copying or saving it.
pub(crate) fn render_screen(
    ui: &mut egui::Ui,
    cache: &mut Option<(String, Option<recoil::ImageFormat>, egui::TextureHandle)>,
    path: &str,
    bytes: &[u8],
    forced: Option<recoil::ImageFormat>,
    companions: &dyn recoil::CompanionFiles,
) -> ScreenRender {
    let stale = cache
        .as_ref()
        .map(|(p, f, _)| p != path || *f != forced)
        .unwrap_or(true);
    if stale {
        let Some(img) = decode_screen(forced, path, bytes, companions) else {
            *cache = None;
            let msg = screen_decode_failed_message(forced);
            if forced.is_some() {
                // A manual attempt that failed: make it prominent (theme-aware
                // warning colour) so the result of the pick is unmistakable.
                let color = ui.visuals().warn_fg_color;
                ui.colored_label(color, msg);
            } else {
                ui.weak(msg);
            }
            return ScreenRender {
                shown: false,
                action: None,
            };
        };
        let image =
            egui::ColorImage::from_rgba_unmultiplied([img.width, img.height], &img.to_rgba());
        let texture = ui.ctx().load_texture(
            format!("screen:{path}"),
            image,
            // Nearest magnification keeps small pictures crisp when zoomed to
            // 2x; linear minification smooths large pages as they are shrunk to
            // fit the panel.
            egui::TextureOptions {
                minification: egui::TextureFilter::Linear,
                ..egui::TextureOptions::NEAREST
            },
        );
        *cache = Some((path.to_string(), forced, texture));
    }

    let mut action = None;
    if let Some((_, _, texture)) = cache {
        // Confirm which forced format produced the image, so a successful manual
        // attempt is acknowledged (not just the initial extension-based decode).
        if let Some(fmt) = forced {
            ui.colored_label(
                egui::Color32::from_rgb(0x3c, 0xa8, 0x4b),
                format!("Decoded as {}.", fmt.label()),
            );
        }
        let mut available = ui.available_size();
        if forced.is_some() {
            // The caller draws the format picker below the image in this case;
            // leave room so a full-height image does not push it out of view.
            available.y = (available.y - SCREEN_PICKER_HEADROOM).max(0.0);
        }
        egui::ScrollArea::both().show(ui, |ui| {
            let size = screen_display_size(texture.size_vec2(), available);
            let image = egui::Image::new(egui::load::SizedTexture::new(texture.id(), size))
                .sense(egui::Sense::click());
            ui.add(image).context_menu(|ui| {
                if ui.button("Copy image").clicked() {
                    action = Some(ScreenAction::CopyImage);
                    ui.close();
                }
                if ui.button("Save PNG…").clicked() {
                    action = Some(ScreenAction::SavePng);
                    ui.close();
                }
            });
        });
    }
    ScreenRender {
        shown: cache.is_some(),
        action,
    }
}

/// Detokenized MSX-BASIC listing.
/// The "showing first N kB of M kB" notice for a truncated listing.
pub(crate) fn size_notice(cap: usize, total: usize) -> String {
    format!("Showing first {} kB of {} kB.", cap / 1024, total / 1024)
}

/// Draw a cached listing: the optional truncation notice, then the selectable
/// monospace text with any Find matches highlighted. Shared by the
/// Text/Basic/Disasm views.
pub(crate) fn draw_listing(ui: &mut egui::Ui, view: &RenderedView, search: &ListingSearch<'_>) {
    // Lines are not wrapped (long ones scroll horizontally), so a listing line
    // number maps directly to a vertical offset for scroll-to-match.
    let row_height = ui.text_style_height(&egui::TextStyle::Monospace);
    let mut area = egui::ScrollArea::both().auto_shrink([false, false]);
    if let Some(line) = search.scroll_to_line {
        // Center the target line in the viewport where possible.
        area = area.vertical_scroll_offset((line as f32 * row_height - 80.0).max(0.0));
    }
    area.show(ui, |ui| {
        if let Some(notice) = &view.notice {
            ui.weak(notice);
        }
        if search.matches.is_empty() || search.len == 0 {
            ui.add(
                egui::Label::new(egui::RichText::new(view.text.as_str()).monospace())
                    .selectable(true)
                    .wrap_mode(egui::TextWrapMode::Extend),
            );
            return;
        }
        // Highlight every match: the active one in the full selection color,
        // the others in a weaker tint of it (readable in both themes).
        let font_id = egui::TextStyle::Monospace.resolve(ui.style());
        let color = ui.visuals().text_color();
        let strong = ui.visuals().selection.bg_fill;
        let weak = strong.gamma_multiply(0.4);
        let fmt = |background: egui::Color32| egui::TextFormat {
            font_id: font_id.clone(),
            color,
            background,
            ..Default::default()
        };
        let mut job = egui::text::LayoutJob::default();
        let mut pos = 0;
        for (i, &start) in search.matches.iter().enumerate() {
            let end = (start + search.len).min(view.text.len());
            if start < pos || start >= view.text.len() {
                continue;
            }
            job.append(&view.text[pos..start], 0.0, fmt(egui::Color32::TRANSPARENT));
            let bg = if i == search.current { strong } else { weak };
            job.append(&view.text[start..end], 0.0, fmt(bg));
            pos = end;
        }
        job.append(&view.text[pos..], 0.0, fmt(egui::Color32::TRANSPARENT));
        ui.add(
            egui::Label::new(job)
                .selectable(true)
                .wrap_mode(egui::TextWrapMode::Extend),
        );
    });
}

/// Detokenized BASIC listing for the Basic view.
pub(crate) fn produce_basic(bytes: &[u8], charset: MsxCharset) -> RenderedView {
    RenderedView {
        text: basic::detokenize(bytes, charset),
        notice: None,
    }
}

/// The most code we disassemble at once: the full Z80 16-bit address space.
/// Beyond this, displayed addresses would wrap and banked ROMs need mapping we
/// don't have, so the listing is capped with a notice.
pub(crate) const MAX_DISASM_BYTES: usize = 64 * 1024;

/// Z80/R800 disassembly listing. The load address and code window come from the
/// file's type and any BSAVE header (see [`disasm::locate`]).
pub(crate) fn produce_disasm(path: &str, bytes: &[u8]) -> RenderedView {
    let img = disasm::locate(path, bytes);
    let shown = &img.code[..img.code.len().min(MAX_DISASM_BYTES)];
    RenderedView {
        text: disasm::disassemble(shown, img.origin, img.exec),
        notice: (img.code.len() > MAX_DISASM_BYTES)
            .then(|| size_notice(MAX_DISASM_BYTES, img.code.len())),
    }
}
