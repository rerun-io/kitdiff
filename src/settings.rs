use crate::diff_image_loader::DiffOptions;
use crate::github_auth::{AuthState, LoggedInState};
use eframe::egui::TextureFilter;

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum ImageMode {
    Pixel,
    Fit,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Settings {
    pub new_opacity: f32,
    pub diff_opacity: f32,
    pub mode: ImageMode,
    pub texture_magnification: TextureFilter,
    pub use_original_diff: bool,
    pub options: DiffOptions,
    pub auth: AuthState,
}

impl Settings {
    fn auth(&self) -> Option<&LoggedInState> {
        #[cfg(target_arch = "wasm32")]
        {
            self.auth.logged_in.as_ref()
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            None
        }
    }
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            new_opacity: 0.5,
            diff_opacity: 0.25,
            mode: ImageMode::Fit,
            texture_magnification: TextureFilter::Nearest,
            use_original_diff: true,
            options: DiffOptions::default(),
            auth: Default::default(),
        }
    }
}
