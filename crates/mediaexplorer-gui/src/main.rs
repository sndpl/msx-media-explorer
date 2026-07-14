//! Entry point for the MSX Media Explorer GUI.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod dnd;
mod hexlayout;
mod i18n;
#[cfg(target_os = "macos")]
mod macos;
mod settings;
mod state;
mod tree_filter;
mod tree_nav;

use app::MediaExplorerApp;

// UI translations: embed every `locales/*.yml` catalog at compile time and fall
// back to English for any key a translation is missing. The `t!` macro looks up
// the active locale set by `rust_i18n::set_locale` (see `MediaExplorerApp::new`).
rust_i18n::i18n!("locales", fallback = "en");

/// Product name shown in the window title and (on macOS) the app menu and
/// About panel.
const APP_NAME: &str = "MSX Media Explorer";

/// Minimal `Info.plist` embedded directly in the executable. macOS reads the
/// `__TEXT,__info_plist` section of an *unbundled* binary as its bundle info, so
/// the dev binary reports the product name to the system. Without it the bold
/// app-menu title and `NSRunningApplication::localizedName` (which drives muda's
/// "Hide …"/"Quit …" labels) fall back to the executable file name,
/// "mediaexplorer". A packaged `.app` ignores this and uses its on-disk
/// `Info.plist` (kept in sync via `[package.metadata.packager]`). Keep the
/// `CFBundleName` string equal to [`APP_NAME`] and the `CFBundleIdentifier`
/// equal to `identifier` in `[package.metadata.packager]` (Cargo.toml).
#[cfg(target_os = "macos")]
const INFO_PLIST_XML: &[u8] = br#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleName</key>
    <string>MSX Media Explorer</string>
    <key>CFBundleDisplayName</key>
    <string>MSX Media Explorer</string>
    <key>CFBundleIdentifier</key>
    <string>io.github.sandy.mediaexplorer</string>
</dict>
</plist>
"#;

#[cfg(target_os = "macos")]
#[used]
#[link_section = "__TEXT,__info_plist"]
static INFO_PLIST: [u8; INFO_PLIST_XML.len()] = {
    let mut bytes = [0u8; INFO_PLIST_XML.len()];
    let mut i = 0;
    while i < INFO_PLIST_XML.len() {
        bytes[i] = INFO_PLIST_XML[i];
        i += 1;
    }
    bytes
};

fn main() -> eframe::Result<()> {
    // macOS: name the app menu / About panel before the event loop builds the
    // menu, so it reads "About MSX Media Explorer" rather than the executable
    // name (winit derives the menu titles from the process name).
    #[cfg(target_os = "macos")]
    macos::set_app_name(APP_NAME);

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1100.0, 720.0])
            .with_min_inner_size([640.0, 400.0])
            .with_drag_and_drop(true)
            .with_title(APP_NAME),
        ..Default::default()
    };

    eframe::run_native(
        APP_NAME,
        options,
        Box::new(|cc| {
            app::install_fonts(&cc.egui_ctx);
            // macOS: replace the generic icon shown in the Dock and the standard
            // About panel (the unbundled dev binary has none).
            #[cfg(target_os = "macos")]
            macos::set_app_icon(app::ICON_PNG);
            let mut app = MediaExplorerApp::new(cc);
            // Open a disk/tape image passed on the command line, so
            // `mediaexplorer game.dsk` and the OS's "Open With" work.
            if let Some(path) = std::env::args_os().nth(1) {
                app.open_path(std::path::Path::new(&path));
            }
            Ok(Box::new(app))
        }),
    )
}
