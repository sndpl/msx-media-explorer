//! Entry point for the MSX Media Explorer GUI.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod dnd;
mod hexlayout;
#[cfg(target_os = "macos")]
mod macos;
mod settings;
mod state;
mod tree_nav;

use app::MediaExplorerApp;

/// Product name shown in the window title and (on macOS) the app menu and
/// About panel.
const APP_NAME: &str = "MSX Media Explorer";

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
            Ok(Box::new(MediaExplorerApp::new(cc)))
        }),
    )
}
