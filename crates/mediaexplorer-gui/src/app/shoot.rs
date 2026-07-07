//! Environment-driven self-screenshot automation.
//!
//! egui renders the frame and hands the pixels back
//! (`ViewportCommand::Screenshot`), so this needs no OS screen-recording
//! permission and works headlessly on any platform. It exists to produce the
//! website/README screenshots reproducibly; it is inert unless the activation
//! variable is set.
//!
//! Variables (all read once at startup):
//! - `MEDIAEXPLORER_SCREENSHOT=<out.png>` — activate: write the window to
//!   this file after a short warm-up, then quit.
//! - `MEDIAEXPLORER_SCREENSHOT_SELECT=<disk path>` — optionally select this
//!   file first (slash-separated, as shown in the tree).
//! - `MEDIAEXPLORER_SCREENSHOT_VIEW=info|hex|text|basic|disasm|screen` —
//!   optionally switch the file viewer tab.
//! - `MEDIAEXPLORER_SCREENSHOT_TAB=files|sectors|map|stats` — optionally
//!   switch the top-level view.
//! - `MEDIAEXPLORER_SCREENSHOT_FIND=<query>` — optionally run the file-viewer
//!   Find with this query after the selection/view are applied.
//!
//! Combine with the disk-path CLI argument, e.g.:
//! `MEDIAEXPLORER_SCREENSHOT=shot.png mediaexplorer game.dsk`

use std::path::{Path, PathBuf};

use super::{AppView, MediaExplorerApp, ViewMode};

/// Frame on which the selection/view overrides are applied (fonts and layout
/// have settled by then).
const APPLY_FRAME: u32 = 12;
/// Frame on which the screenshot is requested.
const SNAP_FRAME: u32 = 30;

/// Parsed screenshot request, held on the app while active.
pub(crate) struct Shoot {
    out: PathBuf,
    select: Option<String>,
    view: Option<ViewMode>,
    tab: Option<AppView>,
    find: Option<String>,
    frame: u32,
}

impl Shoot {
    /// Read the request from the environment; `None` (the normal case)
    /// disables the whole feature.
    pub(crate) fn from_env() -> Option<Shoot> {
        let out = PathBuf::from(std::env::var_os("MEDIAEXPLORER_SCREENSHOT")?);
        let select = std::env::var("MEDIAEXPLORER_SCREENSHOT_SELECT").ok();
        let view = std::env::var("MEDIAEXPLORER_SCREENSHOT_VIEW")
            .ok()
            .and_then(|v| parse_view(&v));
        let tab = std::env::var("MEDIAEXPLORER_SCREENSHOT_TAB")
            .ok()
            .and_then(|v| parse_tab(&v));
        let find = std::env::var("MEDIAEXPLORER_SCREENSHOT_FIND").ok();
        Some(Shoot {
            out,
            select,
            view,
            tab,
            find,
            frame: 0,
        })
    }
}

fn parse_view(s: &str) -> Option<ViewMode> {
    Some(match s.to_ascii_lowercase().as_str() {
        "info" => ViewMode::Info,
        "hex" => ViewMode::Hex,
        "text" => ViewMode::Text,
        "basic" => ViewMode::Basic,
        "disasm" => ViewMode::Disasm,
        "screen" => ViewMode::Screen,
        _ => return None,
    })
}

fn parse_tab(s: &str) -> Option<AppView> {
    Some(match s.to_ascii_lowercase().as_str() {
        "files" => AppView::Files,
        "sectors" => AppView::Sectors,
        "map" => AppView::Map,
        "stats" => AppView::Stats,
        _ => return None,
    })
}

impl MediaExplorerApp {
    /// Drive one frame of the screenshot automation; no-op when inactive.
    pub(crate) fn process_screenshot_request(&mut self, ctx: &egui::Context) {
        let Some(shoot) = self.shoot.as_mut() else {
            return;
        };
        shoot.frame += 1;
        let frame = shoot.frame;
        // Keep frames flowing — nothing else animates while automated.
        ctx.request_repaint();

        if frame == APPLY_FRAME {
            let (select, view, tab, find) = (
                shoot.select.clone(),
                shoot.view,
                shoot.tab,
                shoot.find.clone(),
            );
            if let Some(path) = select {
                self.select_file(path, false);
            }
            // After select_file so its automatic view pick doesn't override.
            if let Some(view) = view {
                self.view_mode = view;
            }
            if let Some(tab) = tab {
                self.app_view = tab;
            }
            if let Some(query) = find {
                self.search_query = query;
                self.run_search();
            }
        } else if frame == SNAP_FRAME {
            ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::UserData::default()));
        }

        // The pixels come back as an input event a frame or two later.
        let image = ctx.input(|i| {
            i.events.iter().find_map(|e| match e {
                egui::Event::Screenshot { image, .. } => Some(image.clone()),
                _ => None,
            })
        });
        if let Some(image) = image {
            let out = self.shoot.as_ref().expect("checked above").out.clone();
            save_png(&image, &out);
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
    }
}

/// Write the captured frame as a PNG (stderr on failure — the app is about to
/// exit, so the status bar is pointless).
fn save_png(img: &egui::ColorImage, out: &Path) {
    let [w, h] = img.size;
    let rgba: Vec<u8> = img.pixels.iter().flat_map(|c| c.to_array()).collect();
    match image::RgbaImage::from_raw(w as u32, h as u32, rgba) {
        Some(buffer) => {
            if let Err(e) = buffer.save(out) {
                eprintln!("screenshot: failed to save {}: {e}", out.display());
            } else {
                eprintln!("screenshot: wrote {}", out.display());
            }
        }
        None => eprintln!("screenshot: bad buffer size {w}x{h}"),
    }
}
