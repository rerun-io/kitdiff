use crate::snapshot::Snapshot;
use crate::state::AppStateRef;
use eframe::egui;
use std::path::PathBuf;
use std::task::Poll;

pub mod archive_loader;
pub mod pr_loader;
pub mod gh_archive_loader;

pub trait LoadSnapshots {
    fn update(&mut self, ctx: &egui::Context);

    fn snapshots(&self) -> &[Snapshot];

    /// State is separate so that snapshots can be streamed in
    fn state(&self) -> Poll<Result<(), &anyhow::Error>>;

    fn extra_ui(&self, ui: &mut egui::Ui, state: &AppStateRef<'_>) {}

    fn files_header(&self) -> String;
}

pub type SnapshotLoader = Box<dyn LoadSnapshots + Send + Sync>;

pub enum DataReference {
    Url(String),
    Data(bytes::Bytes, String),
    Path(PathBuf),
}

impl DataReference {
    pub fn file_name(&self) -> &str {
        match self {
            DataReference::Url(url) => url.split('/').last().unwrap_or(url),
            DataReference::Data(_, name) => name,
            DataReference::Path(path) => path.file_name().and_then(|n| n.to_str()).unwrap_or("unknown"),
        }
    }

    pub async fn into_bytes(self) -> anyhow::Result<bytes::Bytes> {
        match self {
            DataReference::Url(url) => {
                let resp = reqwest::get(&url).await?;
                let bytes = resp.bytes().await?;
                Ok(bytes)
            }
            DataReference::Data(data, _) => Ok(data),
            DataReference::Path(_path) => {
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
