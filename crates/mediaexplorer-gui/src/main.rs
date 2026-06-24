//! Entry point for the MSX Media Explorer GUI.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod dnd;
mod state;

use app::MediaExplorerApp;

fn main() -> eframe::Result<()> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1100.0, 720.0])
            .with_min_inner_size([640.0, 400.0])
            .with_drag_and_drop(true)
            .with_title("MSX Media Explorer"),
        ..Default::default()
    };

    eframe::run_native(
        "MSX Media Explorer",
        options,
        Box::new(|cc| {
            app::install_fonts(&cc.egui_ctx);
            Ok(Box::<MediaExplorerApp>::default())
        }),
    )
}
