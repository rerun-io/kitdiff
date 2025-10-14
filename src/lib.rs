use crate::github::auth::parse_github_artifact_url;
use crate::github::model::{GithubArtifactLink, GithubPrLink};
pub use crate::loaders::{DataReference, SnapshotLoader};
use crate::state::AppState;
use eframe::egui::Context;

pub mod app;
mod bar;
pub mod config;
pub mod diff_image_loader;
pub mod github;
mod home;
pub mod loaders;
#[cfg(not(target_arch = "wasm32"))]
pub mod native_loaders;
mod settings;
pub mod snapshot;
mod state;
mod viewer;

#[derive(Debug, Clone)]
pub enum DiffSource {
    #[cfg(not(target_arch = "wasm32"))]
    Files(std::path::PathBuf),
    #[cfg(not(target_arch = "wasm32"))]
    Git(std::path::PathBuf),
    Pr(GithubPrLink),
    GHArtifact(GithubArtifactLink),
    Archive(DataReference),
}

impl DiffSource {
    pub fn from_url(url: &str) -> Self {
        if let Ok(link) = url.parse() {
            Self::Pr(link)
        } else if let Some(link) = parse_github_artifact_url(url) {
            Self::GHArtifact(link)
        } else {
            // Try to load it as direct zip/tar.gz URL
            Self::Archive(DataReference::Url(url.to_owned()))
        }
    }

    pub fn load(self, _ctx: &Context, state: &AppState) -> SnapshotLoader {
        match self {
            #[cfg(not(target_arch = "wasm32"))]
            Self::Files(path) => Box::new(native_loaders::file_loader::FileLoader::new(path)),
            #[cfg(not(target_arch = "wasm32"))]
            Self::Git(path) => Box::new(native_loaders::git_loader::GitLoader::new(path)),
            Self::Pr(url) => Box::new(loaders::pr_loader::PrLoader::new(
                url,
                state.github_auth.client(),
            )),
            Self::GHArtifact(artifact) => {
                Box::new(loaders::gh_archive_loader::GHArtifactLoader::new(
                    state.github_auth.client(),
                    artifact,
                ))
            }
            Self::Archive(file_ref) => {
                Box::new(loaders::archive_loader::ArchiveLoader::new(file_ref))
            }
        }
    }
}
