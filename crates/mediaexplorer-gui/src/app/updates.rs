use super::*;

use crate::update::OutcomeKind;

impl MediaExplorerApp {
    /// Start a manual update check (the "Check for Updates…" menu item). Its
    /// result opens the update dialog for every outcome; overlapping clicks are
    /// ignored by the in-flight guard in [`crate::update`].
    pub(crate) fn check_for_updates_now(&mut self, ctx: &egui::Context) {
        crate::update::spawn_check(ctx.clone(), true);
    }

    /// The dismissible "an update is available" banner, drawn under the toolbar
    /// while [`update_available`](Self::update_available) is set. Formatted with
    /// `t!` every frame so a language change re-localizes it.
    pub(crate) fn update_banner(&mut self, ui: &mut egui::Ui) {
        let Some(banner) = self.update_available.as_ref() else {
            return;
        };
        let version = banner.version.clone();
        let url = banner.url.clone();
        let mut dismiss = false;
        egui::Panel::top("update_banner").show_inside(ui, |ui| {
            ui.add_space(2.0);
            ui.horizontal(|ui| {
                ui.label(t!("update.banner", version => version));
                ui.hyperlink_to(t!("update.view_release"), &url);
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button("\u{2715}").clicked() {
                        dismiss = true;
                    }
                });
            });
            ui.add_space(2.0);
        });
        if dismiss {
            self.update_available = None;
        }
    }

    /// The manual-check result window: up-to-date, update-available, or failed.
    /// Never shown for automatic checks (those only raise the banner).
    pub(crate) fn update_dialog(&mut self, ctx: &egui::Context) {
        let Some(kind) = self.update_result.as_ref() else {
            return;
        };
        let mut open = true;
        egui::Window::new(t!("dialog.update_title"))
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .open(&mut open)
            .show(ctx, |ui| {
                ui.add_space(4.0);
                match kind {
                    OutcomeKind::Available { version, url } => {
                        ui.label(t!("update.banner", version => version));
                        ui.add_space(4.0);
                        ui.hyperlink_to(t!("update.view_release"), url);
                    }
                    OutcomeKind::UpToDate(version) => {
                        ui.label(t!("update.up_to_date", version => version));
                    }
                    OutcomeKind::Failed(error) => {
                        ui.label(t!("update.failed"));
                        ui.add_space(4.0);
                        ui.small(egui::RichText::new(error).weak());
                    }
                }
                ui.add_space(4.0);
            });
        if !open || ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            self.update_result = None;
        }
    }
}
