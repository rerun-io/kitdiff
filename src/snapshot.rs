use crate::diff_image_loader;
use crate::diff_image_loader::DiffOptions;
use crate::state::{AppStateRef, PageRef, ViewerStateRef};
use eframe::egui::{Color32, ImageSource};
use std::path::PathBuf;

#[derive(Debug, Clone)]
pub struct Snapshot {
    pub path: PathBuf,
    /// If only old is set, the file was deleted.
    pub old: Option<FileReference>,
    /// If only new is set, the file was added.
    pub new: Option<FileReference>,
    pub diff: Option<PathBuf>,
}

#[derive(Debug, Clone)]
pub enum FileReference {
    Path(PathBuf),
    Source(ImageSource<'static>),
}

impl FileReference {
    pub fn to_uri(&self) -> String {
        match self {
            FileReference::Path(path) => format!("file://{}", path.display()),
            FileReference::Source(source) => match source {
                ImageSource::Uri(uri) => uri.to_string(),
                ImageSource::Bytes { uri, .. } => uri.to_string(),
                _ => "unknown://unknown".to_string(),
            },
        }
    }
}

impl Snapshot {
    pub fn file_name(&self) -> std::borrow::Cow<'_, str> {
        self.path
            .file_name()
            .map(|n| n.to_string_lossy())
            .unwrap_or_else(|| self.path.as_os_str().to_string_lossy())
    }

    pub fn added(&self) -> bool {
        self.old.is_none() && self.new.is_some()
    }

    pub fn deleted(&self) -> bool {
        self.old.is_some() && self.new.is_none()
    }

    pub fn old_uri(&self) -> Option<String> {
        self.old.as_ref().map(|p| p.to_uri())
    }

    pub fn new_uri(&self) -> Option<String> {
        self.new.as_ref().map(|p| p.to_uri())
    }

    pub fn file_diff_uri(&self) -> Option<String> {
        self.diff
            .as_ref()
            .map(|p| format!("file://{}", p.display()))
    }

    pub fn diff_uri(&self, use_file_if_available: bool, options: DiffOptions) -> Option<String> {
        use_file_if_available
            .then(|| self.file_diff_uri())
            .flatten()
            .or_else(|| {
                self.old_uri()
                    .zip(self.new_uri())
                    .map(|(old, new)| diff_image_loader::DiffUri { old, new, options }.to_uri())
            })
    }

    fn make_image(
        &self,
        state: &AppStateRef<'_>,
        uri: String,
        opacity: f32,
        show_all: bool,
    ) -> eframe::egui::Image {
        let mut image = eframe::egui::Image::new(uri)
            .texture_options(eframe::egui::TextureOptions {
                magnification: state.settings.texture_magnification,
                ..eframe::egui::TextureOptions::default()
            })
            .tint(Color32::from_white_alpha(if show_all {
                (255.0 * opacity) as u8
            } else {
                u8::MAX
            }));

        match state.settings.mode {
            crate::settings::ImageMode::Pixel => {
                image = image.fit_to_original_size(1.0 / state.egui_ctx.pixels_per_point());
            }
            crate::settings::ImageMode::Fit => {}
        }
        image
    }

    pub fn old_image(&self, state: &AppStateRef<'_>) -> Option<eframe::egui::Image> {
        let PageRef::DiffViewer(vs) = &state.page else {
            return None;
        };
        let show_all = vs.view_filter.all();
        let show_old = vs.view_filter.show_old;
        (show_all || show_old)
            .then(|| self.old_uri())
            .flatten()
            .map(|uri| self.make_image(state, uri, 1.0, show_all))
    }

    pub fn new_image(&self, state: &AppStateRef<'_>) -> Option<eframe::egui::Image> {
        let PageRef::DiffViewer(vs) = &state.page else {
            return None;
        };
        let show_all = vs.view_filter.all();
        let show_new = vs.view_filter.show_new;
        (show_all || show_new)
            .then(|| self.new_uri())
            .flatten()
            .map(|new_uri| self.make_image(state, new_uri, state.settings.new_opacity, show_all))
    }

    pub fn diff_image(&self, state: &AppStateRef<'_>) -> Option<eframe::egui::Image> {
        let PageRef::DiffViewer(vs) = &state.page else {
            return None;
        };
        let show_all = vs.view_filter.all();
        let show_diff = vs.view_filter.show_diff;
        (show_all || show_diff)
            .then(|| self.diff_uri(state.settings.use_original_diff, state.settings.options))
            .flatten()
            .map(|diff_uri| self.make_image(state, diff_uri, state.settings.diff_opacity, show_all))
    }
}
