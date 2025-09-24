use crate::github_auth::{AuthState, GithubAuthCommand, LoggedInState};
use crate::state::{AppStateRef, SystemCommand};
use eframe::egui;
use eframe::egui::panel::TopBottomSide;
use eframe::egui::{Context, Ui};

pub fn bar(ctx: &Context, state: &AppStateRef<'_>) {
    egui::TopBottomPanel::top("top bar")
        .resizable(false)
        .show(ctx, |ui| egui::Sides::new().show(ui, |ui| {}, |ui| {
            auth_ui(ui, state);
        }));
}

pub fn auth_ui(ui: &mut Ui, state: &AppStateRef<'_>) {
    match &state.github_auth.get_auth_state().logged_in {
        Some(logged_in) => {
            if let Some(image) = &logged_in.user_image {
                ui.image(image);
            }
            ui.label(&logged_in.username);
        }
        None => {
            if ui.button("Log in with GitHub").clicked() {
                state.send(GithubAuthCommand::Login);
            }
        }
    }
}
