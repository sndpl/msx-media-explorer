//! The hex views' graphics preview: a side panel that reinterprets the bytes at
//! the hex cursor as pixels under a layout the user picks.
//!
//! The decoding itself is [`msx_disk::view::bitmap`]; everything here is
//! controls, texture caching and the right-click menu.

use msx_disk::view::bitmap::{self, PaletteId, RawFormat, RawView};

use super::*;
use crate::settings::{PreviewOptions, PREVIEW_WIDTH_RANGE, PREVIEW_ZOOMS};

/// Pixel rows rendered when the panel's height is not yet known or is tiny, so
/// the preview always has something to show.
const MIN_ROWS: usize = 64;

/// Starting width of the preview panel: a 256-pixel-wide MSX screen at 1x plus
/// room for the controls, so the common case needs no horizontal scrolling.
pub(crate) const PREVIEW_PANEL_WIDTH: f32 = 320.0;

/// What a cached preview texture was rendered from. Any change re-renders; a
/// buffer edit is signalled by the caller bumping `revision`, so the bytes
/// themselves are never re-hashed per frame.
#[derive(Clone, PartialEq, Eq)]
struct PreviewKey {
    source: String,
    revision: u64,
    view: RawView,
}

/// The preview's per-frame render cache. The decoded image is kept alongside
/// the texture so Copy/Save can reuse it instead of decoding a second time.
#[derive(Default)]
pub(crate) struct PreviewState {
    cached: Option<(PreviewKey, egui::TextureHandle, recoil::Image)>,
}

impl PreviewState {
    /// A copy of the image currently on screen, with the byte offset it starts
    /// at. Used by the Copy/Save actions, which need it after the cache borrow
    /// has been released.
    pub(crate) fn snapshot(&self) -> Option<(recoil::Image, usize)> {
        let (key, _, img) = self.cached.as_ref()?;
        Some((img.clone(), key.view.offset))
    }
}

/// Draw the preview panel's contents: the controls, then the bitmap.
///
/// `bytes` is the buffer being viewed, `anchor` the byte the top-left pixel
/// comes from, and `source`/`revision` identify that buffer for cache
/// invalidation. Returns any action chosen from the image's right-click menu.
pub(crate) fn render_preview(
    ui: &mut egui::Ui,
    opts: &mut PreviewOptions,
    state: &mut PreviewState,
    bytes: &[u8],
    anchor: usize,
    source: &str,
    revision: u64,
) -> Option<ScreenAction> {
    ui.horizontal(|ui| {
        ui.strong(t!("preview.title"));
        ui.weak(format!("@ 0x{anchor:06X}"));
    });
    ui.separator();
    preview_controls(ui, opts);
    ui.separator();

    let zoom = opts.zoom.max(1);
    // Render only the rows the panel can show; the rest would be decoded and
    // uploaded every time the cursor moves for nothing.
    let max_rows = ((ui.available_height() / zoom as f32).ceil() as usize).max(MIN_ROWS);
    let view = opts.view(anchor, max_rows);
    let key = PreviewKey {
        source: source.to_string(),
        revision,
        view,
    };

    if state.cached.as_ref().map(|(k, _, _)| k) != Some(&key) {
        let img = bitmap::render(bytes, &view);
        if img.pixels.is_empty() {
            state.cached = None;
        } else {
            let color =
                egui::ColorImage::from_rgba_unmultiplied([img.width, img.height], &img.to_rgba());
            // Nearest in both directions: a magnified preview must show the
            // pixel grid exactly, never a smoothed guess at it.
            let texture =
                ui.ctx()
                    .load_texture("hex_preview", color, egui::TextureOptions::NEAREST);
            state.cached = Some((key, texture, img));
        }
    }

    let Some((_, texture, _)) = state.cached.as_ref() else {
        ui.weak(t!("preview.nothing_here"));
        return None;
    };

    let mut action = None;
    egui::ScrollArea::both()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            let size = texture.size_vec2() * zoom as f32;
            let image = egui::Image::new(egui::load::SizedTexture::new(texture.id(), size))
                .sense(egui::Sense::click());
            ui.add(image).context_menu(|ui| {
                if ui.button(t!("button.copy_image")).clicked() {
                    action = Some(ScreenAction::CopyImage);
                    ui.close();
                }
                if ui.button(t!("button.save_png")).clicked() {
                    action = Some(ScreenAction::SavePng);
                    ui.close();
                }
            });
        });
    action
}

/// The layout / palette / width / zoom controls above the image.
fn preview_controls(ui: &mut egui::Ui, opts: &mut PreviewOptions) {
    egui::Grid::new("preview_controls")
        .num_columns(2)
        .show(ui, |ui| {
            ui.label(t!("preview.layout"));
            let mut format = opts.format();
            egui::ComboBox::from_id_salt("preview_format")
                .selected_text(format.label())
                .show_ui(ui, |ui| {
                    for &f in RawFormat::all() {
                        ui.selectable_value(&mut format, f, f.label());
                    }
                });
            // Changing layout also moves to that layout's natural palette and
            // width, so the picker never lands on a combination that renders
            // as noise.
            if format != opts.format() {
                opts.set_format(format);
            }
            ui.end_row();

            ui.label(t!("preview.palette"));
            let current = opts.palette();
            egui::ComboBox::from_id_salt("preview_palette")
                .selected_text(current.label())
                .show_ui(ui, |ui| {
                    for &p in PaletteId::all() {
                        // Matched by id, not by value: `FromFile` in the list
                        // carries a zero offset, so comparing values would show
                        // the active entry as unselected and re-picking it would
                        // reset the offset the user chose.
                        let selected = p.id() == current.id();
                        if ui.selectable_label(selected, p.label()).clicked() && !selected {
                            opts.set_palette(p);
                        }
                    }
                });
            ui.end_row();

            if opts.palette().is_from_file() {
                ui.label(t!("preview.palette_offset"));
                ui.add(
                    egui::DragValue::new(&mut opts.palette_offset)
                        .hexadecimal(4, false, true)
                        .prefix("0x"),
                );
                ui.end_row();
            }

            ui.label(t!("preview.width"));
            ui.add(
                egui::DragValue::new(&mut opts.width)
                    .range(PREVIEW_WIDTH_RANGE)
                    .suffix(" px"),
            );
            ui.end_row();

            ui.label(t!("preview.zoom"));
            egui::ComboBox::from_id_salt("preview_zoom")
                .selected_text(format!("{}x", opts.zoom))
                .show_ui(ui, |ui| {
                    for z in PREVIEW_ZOOMS {
                        ui.selectable_value(&mut opts.zoom, z, format!("{z}x"));
                    }
                });
            ui.end_row();
        });
}
