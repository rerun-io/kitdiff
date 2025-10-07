use crate::DiffSource;
use crate::state::{AppStateRef, SystemCommand};
use eframe::egui;
use eframe::egui::{CentralPanel, Context, Id, TextEdit};

pub fn home_view(ctx: &Context, app: &AppStateRef<'_>) {
    CentralPanel::default().show(ctx, |ui| {
        ui.heading("Kitdiff");

        ui.horizontal(|ui| {
            let url_text_id = Id::new("url_text");
            let mut url_text =
                ui.memory_mut(|mem| mem.data.get_temp::<String>(url_text_id).unwrap_or_default());
            let text_resp = ui.add(TextEdit::singleline(&mut url_text).hint_text("Enter url..."));

            let button = ui.add_enabled(!url_text.is_empty(), egui::Button::new("Load"));

            let enter = text_resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));

            if (button.clicked() || enter) && !url_text.is_empty() {
                app.send(SystemCommand::Open(DiffSource::from_url(&url_text)));
            }
            ui.memory_mut(|mem| mem.data.insert_temp(url_text_id, url_text.clone()));
        });
        ui.label("Valid urls are link to github PRs, links to github artifacts, or direct links to zip/tar.gz files.");

        ui.label("You need to sign in to load artifacts. You can see PR diffs without signing in but will quickly run into github rate limits.");

        ui.hyperlink_to("View kitdiff on github", "https://github.com/rerun-io/kitdiff");
    });
}
