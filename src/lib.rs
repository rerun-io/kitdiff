use crate::github_auth::{AuthState, LoggedInState};
use crate::loaders::SnapshotLoader;
use crate::snapshot::Snapshot;
use eframe::egui::Context;
use eframe::egui::load::Bytes;
use std::any::Any;
use std::path::PathBuf;
use std::sync::mpsc::Sender;
use crate::github_model::{GithubArtifactLink, GithubPrLink, GithubRepoLink};
use crate::state::AppState;

pub mod app;
pub mod diff_image_loader;
pub mod github_auth;
pub mod github_pr;
pub mod loaders;
#[cfg(not(target_arch = "wasm32"))]
pub mod native_loaders;
mod settings;
pub mod snapshot;
mod state;
pub mod github_model;
mod octokit;
mod viewer;
mod home;
mod bar;

#[derive(Debug, Clone)]
pub enum DiffSource {
    #[cfg(not(target_arch = "wasm32"))]
    Files(PathBuf),
    #[cfg(not(target_arch = "wasm32"))]
    Git,
    Pr(GithubPrLink),
    Zip(PathOrBlob),
    TarGz(PathOrBlob),
    GHArtifact(GithubArtifactLink),
}

impl DiffSource {
    pub fn load(
        self,
        ctx: Context,
        state: &AppState,
    ) -> SnapshotLoader {
        match self {
            #[cfg(not(target_arch = "wasm32"))]
            DiffSource::Files(path) => {
                Box::new(native_loaders::file_loader::FileLoader::new(path))
            }
            // #[cfg(not(target_arch = "wasm32"))]
            // DiffSource::Git => {
            //     native_loaders::git_loader::git_discovery(tx, ctx)
            //         .expect("Failed to run git discovery");
            //     None
            // }
            DiffSource::Pr(url) => {
                Box::new(loaders::pr_loader::PrLoader::new(url, state.github_auth.client()))
            }
            DiffSource::GHArtifact(artifact) => {
                Box::new(loaders::gh_archive_loader::GHArtifactLoader::new(
                    state.github_auth.client(),
                    artifact,
                ))
            }
            // DiffSource::Zip(data) => {
            //     #[cfg(target_arch = "wasm32")]
            //     {
            //         // For URLs in wasm, spawn async task
            //         if matches!(data, PathOrBlob::Url(_, _)) {
            //             let data_clone = data.clone();
            //             let tx_clone = tx.clone();
            //             let ctx_clone = ctx.clone();
            //
            //             wasm_bindgen_futures::spawn_local(async move {
            //                 if let Some(bytes) = data_clone.load_bytes_async().await {
            //                     if let Err(e) = loaders::zip_loader::extract_and_discover_zip(
            //                         bytes.to_vec(),
            //                         tx_clone,
            //                         ctx_clone,
            //                     ) {
            //                         eprintln!("Failed to run zip discovery: {:?}", e);
            //                     }
            //                 }
            //             });
            //             None
            //         } else {
            //             // For blobs, use sync method
            //             loaders::zip_loader::extract_and_discover_zip(
            //                 data.load_bytes()?.to_vec(),
            //                 tx,
            //                 ctx,
            //             )
            //             .expect("Failed to run zip discovery");
            //             None
            //         }
            //     }
            //     #[cfg(not(target_arch = "wasm32"))]
            //     {
            //         loaders::zip_loader::extract_and_discover_zip(
            //             data.load_bytes()?.to_vec(),
            //             tx,
            //             ctx,
            //         )
            //         .expect("Failed to run zip discovery");
            //         None
            //     }
            // }
            // DiffSource::TarGz(data) => {
            //     #[cfg(target_arch = "wasm32")]
            //     {
            //         // For URLs in wasm, spawn async task
            //         if matches!(data, PathOrBlob::Url(_, _)) {
            //             let data_clone = data.clone();
            //             let tx_clone = tx.clone();
            //             let ctx_clone = ctx.clone();
            //
            //             wasm_bindgen_futures::spawn_local(async move {
            //                 if let Some(bytes) = data_clone.load_bytes_async().await {
            //                     if let Err(e) = loaders::tar_loader::extract_and_discover_tar_gz(
            //                         bytes.to_vec(),
            //                         tx_clone,
            //                         ctx_clone,
            //                     ) {
            //                         eprintln!("Failed to run tar.gz discovery: {:?}", e);
            //                     }
            //                 }
            //             });
            //             None
            //         } else {
            //             // For blobs, use sync method
            //             loaders::tar_loader::extract_and_discover_tar_gz(
            //                 data.load_bytes()?.to_vec(),
            //                 tx,
            //                 ctx,
            //             )
            //             .expect("Failed to run tar.gz discovery");
            //             None
            //         }
            //     }
            //     #[cfg(not(target_arch = "wasm32"))]
            //     {
            //         loaders::tar_loader::extract_and_discover_tar_gz(
            //             data.load_bytes()?.to_vec(),
            //             tx,
            //             ctx,
            //         )
            //         .expect("Failed to run tar.gz discovery");
            //         None
            //     }
            // }
            // DiffSource::GHArtifact {
            //     owner,
            //     repo,
            //     artifact_id,
            // } => {
            //     #[cfg(target_arch = "wasm32")]
            //     {
            //         use crate::github_auth::github_artifact_api_url;
            //
            //         // Create GitHub API URL for artifact
            //         let api_url = github_artifact_api_url(&owner, &repo, &artifact_id);
            //
            //         // TODO: Get GitHub token from auth state - we'll need to pass this context
            //         // For now, try without token (works for public repos)
            //         let data = PathOrBlob::Url(api_url, auth.map(|l| l.provider_token.clone()));
            //
            //         // Use async zip loading since it's a URL
            //         let tx_clone = tx.clone();
            //         let ctx_clone = ctx.clone();
            //
            //         wasm_bindgen_futures::spawn_local(async move {
            //             if let Some(bytes) = data.load_bytes_async().await {
            //                 if let Err(e) = loaders::zip_loader::extract_and_discover_zip(
            //                     bytes.to_vec(),
            //                     tx_clone,
            //                     ctx_clone,
            //                 ) {
            //                     eprintln!("Failed to run GitHub artifact zip discovery: {:?}", e);
            //                 }
            //             }
            //         });
            //         None
            //     }
            //     #[cfg(not(target_arch = "wasm32"))]
            //     {
            //         eprintln!(
            //             "GitHub artifact loading not supported on native platforms yet. Please download the artifact manually and use the zip command instead."
            //         );
            //         None
            //     }
            // }
            _ => todo!()
        }
    }
}

struct DropMeLater(Box<dyn Any>);

#[derive(Debug, Clone)]
pub enum PathOrBlob {
    Path(std::path::PathBuf),
    Blob(Bytes),
    #[cfg(target_arch = "wasm32")]
    Url(String, Option<String>), // URL and optional auth token
}

impl PathOrBlob {
    pub fn load_bytes(&self) -> Option<Bytes> {
        match self {
            #[cfg(not(target_arch = "wasm32"))]
            PathOrBlob::Path(path) => std::fs::read(path).ok().map(Bytes::from),
            PathOrBlob::Blob(bytes) => Some(bytes.clone()),
            #[cfg(target_arch = "wasm32")]
            PathOrBlob::Path(_) => None, // Paths not supported in wasm
            #[cfg(target_arch = "wasm32")]
            PathOrBlob::Url(_, _) => None, // URLs require async, handled separately
        }
    }

    #[cfg(target_arch = "wasm32")]
    pub async fn load_bytes_async(&self) -> Option<Bytes> {
        match self {
            PathOrBlob::Blob(bytes) => Some(bytes.clone()),
            PathOrBlob::Url(url, token) => {
                let auth_header = token.as_ref().map(|t| format!("Bearer {}", t));
                let mut headers = vec![("User-Agent", "kitdiff")];
                if let Some(ref auth) = auth_header {
                    headers.push(("Authorization", auth.as_str()));
                }

                let request = ehttp::Request {
                    method: "GET".to_string(),
                    url: url.clone(),
                    body: vec![],
                    headers: ehttp::Headers::new(&headers),
                    mode: ehttp::Mode::Cors,
                };

                match ehttp::fetch_async(request).await {
                    Ok(response) if response.ok => Some(Bytes::from(response.bytes)),
                    Ok(response) => {
                        eprintln!("Failed to download {}: HTTP {}", url, response.status);
                        None
                    }
                    Err(e) => {
                        eprintln!("Failed to download {}: {}", url, e);
                        None
                    }
                }
            }
            PathOrBlob::Path(_) => None, // Paths not supported in wasm
        }
    }
}
