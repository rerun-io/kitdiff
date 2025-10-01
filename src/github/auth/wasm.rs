use crate::github::auth::{AuthSender, GitHubAuth, parse_supabase_fragment};
use eframe::egui;
use eframe::egui::OpenUrl;
use hello_egui_utils::spawn;
use wasm_bindgen::JsValue;

pub fn login_github(ctx: &egui::Context, _tx: AuthSender) {
    if let Some(window) = web_sys::window() {
        if let Ok(origin) = window.location().href() {
            let auth_url = GitHubAuth::auth_url(&origin);
            ctx.open_url(OpenUrl::same_tab(auth_url));
        }
    }
}

pub fn check_for_auth_callback(sender: AuthSender) {
    if let Some(window) = web_sys::window() {
        if let Ok(hash) = window.location().hash() {
            if let Ok(hash) = parse_supabase_fragment(hash.strip_prefix('#').unwrap_or(&hash)) {
                // Remove the hash from the URL to clean it up
                let path = window.location().pathname().unwrap_or_default();
                let search = window.location().search().unwrap_or_default();
                let new_url = format!("{}{}", path, search);
                let _ = window.history().unwrap().replace_state_with_url(
                    &JsValue::NULL,
                    "",
                    Some(&new_url),
                );
                spawn(async move {
                    GitHubAuth::handle_callback_fragment(sender, hash).await;
                });
            }
        }
    }
}
