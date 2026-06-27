//! macOS-only native menu bar, built with `muda`.
//!
//! `muda` installs an `NSMenu` as the application's main menu (replacing the
//! generic one winit creates). Menu clicks are not egui input events, so a
//! handler pushes the activated item's id onto a queue and requests a repaint;
//! the egui loop drains the queue each frame via [`take_menu_events`] and acts
//! on it. The product name and Dock icon are still set directly through AppKit.

use std::sync::{Mutex, OnceLock};

use std::path::{Path, PathBuf};

use muda::accelerator::{Accelerator, Code, Modifiers};
use muda::{CheckMenuItem, Menu, MenuEvent, MenuItem, PredefinedMenuItem, Submenu};
use objc2::{AnyThread, MainThreadMarker};
use objc2_app_kit::{NSApplication, NSImage};
use objc2_foundation::{NSData, NSProcessInfo, NSString};

use crate::settings::{ByteGrouping, Settings, ROW_SIZES};

/// egui context, stored so the menu handler can wake a repaint when the app is
/// otherwise idle (a menu click is not an egui input event).
static EGUI_CTX: OnceLock<egui::Context> = OnceLock::new();

/// Ids of menu items activated since the last drain, in click order.
static PENDING: Mutex<Vec<String>> = Mutex::new(Vec::new());

/// Set the product name used by the app menu ("About …", "Hide …", "Quit …").
/// Must be called before the event loop builds the menu (before `run_native`).
pub fn set_app_name(name: &str) {
    NSProcessInfo::processInfo().setProcessName(&NSString::from_str(name));
}

/// Set the Dock application icon from PNG bytes. No-op if the bytes fail to
/// decode or this is somehow called off the main thread.
pub fn set_app_icon(icon_png: &[u8]) {
    let Some(mtm) = MainThreadMarker::new() else {
        return;
    };
    let data = NSData::with_bytes(icon_png);
    let Some(image) = NSImage::initWithData(NSImage::alloc(), &data) else {
        return;
    };
    let app = NSApplication::sharedApplication(mtm);
    // Safety: standard AppKit setter; `image` is a valid NSImage and we hold a
    // main-thread marker.
    unsafe { app.setApplicationIconImage(Some(&image)) };
}

/// Drain the menu-item ids activated since the previous call.
pub fn take_menu_events() -> Vec<String> {
    std::mem::take(&mut PENDING.lock().unwrap())
}

/// Cmd-modified accelerator for a key code.
fn cmd(code: Code) -> Accelerator {
    Accelerator::new(Some(Modifiers::META), code)
}

/// Handles for the live native menu, kept alive for the process and used to
/// reflect state changes (checkmarks) and rebuild the Recent submenu.
pub struct MacMenu {
    _menu: Menu,
    recent: Submenu,
    recent_items: Vec<MenuItem>,
    /// Check items keyed by their menu id, for `sync_checks`.
    checks: Vec<(&'static str, CheckMenuItem)>,
}

impl MacMenu {
    /// Re-derive every check item's state from `settings` (covers radio groups).
    pub fn sync_checks(&self, s: &Settings) {
        for (id, item) in &self.checks {
            item.set_checked(check_state(id, s));
        }
    }

    /// Replace the Recent submenu's contents with the current list (plus a
    /// "Clear Menu" entry, or a disabled placeholder when empty).
    pub fn rebuild_recent(&mut self, recent: &[PathBuf]) {
        while self.recent.remove_at(0).is_some() {}
        self.recent_items.clear();
        if recent.is_empty() {
            let empty = MenuItem::new("No Recent Files", false, None);
            let _ = self.recent.append(&empty);
            self.recent_items.push(empty);
            return;
        }
        for (i, path) in recent.iter().enumerate() {
            let item = MenuItem::with_id(format!("recent.{i}"), recent_label(path), true, None);
            let _ = self.recent.append(&item);
            self.recent_items.push(item);
        }
        let _ = self.recent.append(&PredefinedMenuItem::separator());
        let clear = MenuItem::with_id("recent.clear", "Clear Menu", true, None);
        let _ = self.recent.append(&clear);
        self.recent_items.push(clear);
    }
}

/// The checked state a given check-item id should show for `settings`.
fn check_state(id: &str, s: &Settings) -> bool {
    match id {
        "view.line_numbers" => s.hex.show_line_numbers,
        "view.hex" => s.hex.show_hex,
        "view.ascii" => s.hex.show_ascii,
        "view.status_bar" => s.show_status_bar,
        "view.columns" => s.hex.show_columns,
        "view.hide_nulls" => s.hex.hide_null_bytes,
        "view.lnf.hex" => s.hex.line_number_hex,
        "view.lnf.dec" => !s.hex.line_number_hex,
        "view.group.none" => s.hex.grouping == ByteGrouping::None,
        _ if id.starts_with("view.bpr.") => s.hex.bytes_per_row == id_suffix(id, "view.bpr."),
        _ => s.hex.grouping == ByteGrouping::Of(id_suffix(id, "view.group.")),
    }
}

/// The numeric suffix of an id like `view.bpr.16` (0 if it does not match).
fn id_suffix(id: &str, prefix: &str) -> usize {
    id.strip_prefix(prefix)
        .and_then(|n| n.parse().ok())
        .unwrap_or(0)
}

/// A short label for a recent-file path: the file name, falling back to the
/// full path.
fn recent_label(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.to_string_lossy().into_owned())
}

/// Build the native menu bar, install it, and return the handles to keep alive.
pub fn build_menu(ctx: &egui::Context, settings: &Settings) -> MacMenu {
    let _ = EGUI_CTX.set(ctx.clone());
    MenuEvent::set_event_handler(Some(|event: MenuEvent| {
        PENDING
            .lock()
            .unwrap()
            .push(event.id().as_ref().to_string());
        if let Some(ctx) = EGUI_CTX.get() {
            ctx.request_repaint();
        }
    }));

    let menu = Menu::new();
    let mut checks: Vec<(&'static str, CheckMenuItem)> = Vec::new();

    // App menu (first submenu). About opens the in-app window, so it is a plain
    // item rather than the system About panel.
    let app_menu = Submenu::new(crate::APP_NAME, true);
    let about = MenuItem::with_id("app.about", "About MSX Media Explorer", true, None);
    let _ = app_menu.append_items(&[
        &about,
        &PredefinedMenuItem::separator(),
        &PredefinedMenuItem::services(None),
        &PredefinedMenuItem::separator(),
        &PredefinedMenuItem::hide(None),
        &PredefinedMenuItem::hide_others(None),
        &PredefinedMenuItem::show_all(None),
        &PredefinedMenuItem::separator(),
        &PredefinedMenuItem::quit(None),
    ]);

    // File menu.
    let file = Submenu::new("File", true);
    let recent = Submenu::new("Open Recent", true);
    let _ = file.append_items(&[
        &MenuItem::with_id("file.new", "New Disk…", true, Some(cmd(Code::KeyN))),
        &MenuItem::with_id(
            "file.open",
            "Open Disk/Tape Image…",
            true,
            Some(cmd(Code::KeyO)),
        ),
        &recent,
        &PredefinedMenuItem::separator(),
        &MenuItem::with_id("file.close", "Close", true, Some(cmd(Code::KeyW))),
    ]);

    // View menu.
    let view = Submenu::new("View", true);
    let line_numbers = check("view.line_numbers", "Line Numbers", settings, &mut checks);
    let hex = check("view.hex", "Hexadecimal", settings, &mut checks);
    let ascii = check("view.ascii", "Plain Text", settings, &mut checks);
    let status = check("view.status_bar", "Status Bar", settings, &mut checks);
    let columns = check("view.columns", "Columns", settings, &mut checks);
    let hide_nulls = check("view.hide_nulls", "Hide Null Bytes", settings, &mut checks);

    let lnf = Submenu::new("Line Number Format", true);
    let lnf_dec = check("view.lnf.dec", "Decimal", settings, &mut checks);
    let lnf_hex = check("view.lnf.hex", "Hexadecimal", settings, &mut checks);
    let _ = lnf.append_items(&[&lnf_dec, &lnf_hex]);

    let bpr = Submenu::new("Bytes per Row", true);
    for n in ROW_SIZES {
        let item = check(
            Box::leak(format!("view.bpr.{n}").into_boxed_str()),
            &n.to_string(),
            settings,
            &mut checks,
        );
        let _ = bpr.append(&item);
    }

    let group = Submenu::new("Byte Grouping", true);
    let group_none = check("view.group.none", "None", settings, &mut checks);
    let _ = group.append(&group_none);
    for n in ByteGrouping::SIZES {
        let item = check(
            // Leak a 'static id string for each fixed grouping size.
            Box::leak(format!("view.group.{n}").into_boxed_str()),
            &n.to_string(),
            settings,
            &mut checks,
        );
        let _ = group.append(&item);
    }

    let _ = view.append_items(&[
        &line_numbers,
        &hex,
        &ascii,
        &status,
        &columns,
        &PredefinedMenuItem::separator(),
        &bpr,
        &lnf,
        &group,
        &PredefinedMenuItem::separator(),
        &hide_nulls,
    ]);

    // Window menu (macOS injects Minimize/Zoom/Bring All to Front).
    let window = Submenu::new("Window", true);

    let _ = menu.append_items(&[&app_menu, &file, &view, &window]);
    menu.init_for_nsapp();
    window.set_as_windows_menu_for_nsapp();

    let mut mac = MacMenu {
        _menu: menu,
        recent,
        recent_items: Vec::new(),
        checks,
    };
    mac.rebuild_recent(&settings.recent);
    mac
}

/// Build a check item initialised from `settings`, record it for later state
/// syncing, and return it for placement in the menu.
fn check(
    id: &'static str,
    label: &str,
    settings: &Settings,
    checks: &mut Vec<(&'static str, CheckMenuItem)>,
) -> CheckMenuItem {
    let item = CheckMenuItem::with_id(id, label, true, check_state(id, settings), None);
    checks.push((id, item.clone()));
    item
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::HexViewOptions;

    #[test]
    fn set_app_name_updates_process_name() {
        let original = NSProcessInfo::processInfo().processName();
        set_app_name("MSX Media Explorer");
        assert_eq!(
            NSProcessInfo::processInfo().processName().to_string(),
            "MSX Media Explorer"
        );
        NSProcessInfo::processInfo().setProcessName(&original);
    }

    #[test]
    fn embedded_icon_decodes_as_nsimage() {
        let data = NSData::with_bytes(crate::app::ICON_PNG);
        assert!(
            NSImage::initWithData(NSImage::alloc(), &data).is_some(),
            "embedded icon should decode as an NSImage"
        );
    }

    #[test]
    fn check_state_reflects_settings() {
        let s = Settings {
            hex: HexViewOptions {
                show_line_numbers: false,
                line_number_hex: false,
                grouping: ByteGrouping::Of(4),
                ..HexViewOptions::default()
            },
            ..Settings::default()
        };
        assert!(!check_state("view.line_numbers", &s));
        assert!(check_state("view.lnf.dec", &s));
        assert!(!check_state("view.lnf.hex", &s));
        assert!(check_state("view.group.4", &s));
        assert!(!check_state("view.group.none", &s));
        assert!(!check_state("view.group.8", &s));
    }

    #[test]
    fn recent_label_uses_file_name() {
        assert_eq!(recent_label(Path::new("/a/b/Game.dsk")), "Game.dsk");
    }
}
