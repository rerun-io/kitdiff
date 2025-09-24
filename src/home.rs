use crate::state::AppStateRef;
use eframe::egui::{CentralPanel, Context, Ui};

pub fn home_view(ctx: &Context, app: &AppStateRef<'_>) {
    CentralPanel::default().show(ctx, |ui| {
        ui.heading("Kitdiff");
        ui.label("Drag in a file to start");
    });
}
