//! Native drag-out of files to the OS file manager.
//!
//! egui exposes no OS drag-source API, so we hand off to the `drag` crate using
//! the window handle that `eframe::Frame` provides. Only macOS and Windows are
//! supported; on Linux this module is empty and the UI falls back to the
//! Extract dialog.
#![cfg(any(target_os = "macos", target_os = "windows"))]

use std::path::PathBuf;

use drag::{DragItem, Image, Options};

/// Start an OS drag carrying `files` (which must be absolute paths). The drag
/// preview is a small generated icon. Returns an error string on failure.
pub fn start_file_drag(frame: &eframe::Frame, files: Vec<PathBuf>) -> Result<(), String> {
    if files.is_empty() {
        return Ok(());
    }
    let preview = preview_png()?;
    drag::start_drag(
        frame,
        DragItem::Files(files),
        Image::Raw(preview),
        |_result, _cursor| {},
        Options::default(),
    )
    .map_err(|e| e.to_string())
}

/// A small opaque PNG used as the drag preview. macOS panics if the image is
/// missing or invalid, so this must always produce a decodable image.
fn preview_png() -> Result<Vec<u8>, String> {
    use image::codecs::png::PngEncoder;
    use image::{ExtendedColorType, ImageEncoder};

    const SIZE: u32 = 24;
    let mut rgba = vec![0u8; (SIZE * SIZE * 4) as usize];
    for px in rgba.chunks_exact_mut(4) {
        px.copy_from_slice(&[0x33, 0x66, 0x99, 0xCC]);
    }
    let mut out = Vec::new();
    PngEncoder::new(&mut out)
        .write_image(&rgba, SIZE, SIZE, ExtendedColorType::Rgba8)
        .map_err(|e| format!("drag preview: {e}"))?;
    Ok(out)
}
