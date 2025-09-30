use crate::snapshot::Snapshot;
use crate::state::AppStateRef;
use eframe::egui;
use std::path::PathBuf;
use std::task::Poll;

pub mod archive_loader;
pub mod gh_archive_loader;
pub mod pr_loader;

pub trait LoadSnapshots {
    fn update(&mut self, ctx: &egui::Context);

    fn snapshots(&self) -> &[Snapshot];

    /// State is separate so that snapshots can be streamed in
    fn state(&self) -> Poll<Result<(), &anyhow::Error>>;

    #[expect(unused_variables)]
    fn extra_ui(&self, ui: &mut egui::Ui, state: &AppStateRef<'_>) {}

    fn files_header(&self) -> String;
}

pub type SnapshotLoader = Box<dyn LoadSnapshots + Send + Sync>;

#[derive(Debug, Clone)]
pub enum DataReference {
    Url(String),
    Data(bytes::Bytes, String),
    Path(PathBuf),
}

impl DataReference {
    pub fn file_name(&self) -> &str {
        match self {
            Self::Url(url) => url.split('/').next_back().unwrap_or(url),
            Self::Data(_, name) => name,
            Self::Path(path) => path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("unknown"),
        }
    }

    pub async fn into_bytes(self) -> anyhow::Result<bytes::Bytes> {
        match self {
            Self::Url(url) => {
                let resp = reqwest::get(&url).await?;
                let bytes = resp.bytes().await?;
                Ok(bytes)
            }
            Self::Data(data, _) => Ok(data),
            Self::Path(_path) => {
                #[cfg(target_arch = "wasm32")]
                anyhow::bail!("FileReference::Path is not supported on wasm32");
                #[cfg(not(target_arch = "wasm32"))]
                {
                    let data = tokio::fs::read(_path).await?;
                    Ok(bytes::Bytes::from(data))
                }
            }
        }
    }
}

pub fn sort_snapshots(snapshots: &mut [Snapshot]) {
    snapshots.sort_by_key(|s| s.path.to_string_lossy().to_lowercase());
}
