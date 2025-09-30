use crate::github::auth::GithubAuthCommand;
use crate::state::AppStateRef;
use eframe::egui;
use eframe::egui::{Context, Popup, Ui};

pub fn bar(ctx: &Context, state: &AppStateRef<'_>) {
    egui::TopBottomPanel::top("top bar")
        .resizable(false)
        .show(ctx, |ui| {
            egui::Sides::new().show(
                ui,
                |ui| {},
                |ui| {
                    auth_ui(ui, state);
                },
            )
        });
}

pub fn auth_ui(ui: &mut Ui, state: &AppStateRef<'_>) {
    match &state.github_auth.get_auth_state().logged_in {
        Some(logged_in) => {
            if let Some(image) = &logged_in.user_image {
                ui.image(image);
            }
            let response = ui.button(&logged_in.username);

            Popup::menu(&response).show(|ui| {
                if ui.button("Log out").clicked() {
                    state.send(GithubAuthCommand::Logout);
                }
            });
        }
        None => {
            if ui.button("Log in with GitHub").clicked() {
                state.send(GithubAuthCommand::Login);
            }
        }
    }
}
