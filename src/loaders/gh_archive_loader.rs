use crate::github_model::GithubArtifactLink;
use crate::loaders::LoadSnapshots;
use crate::loaders::archive_loader::ArchiveLoader;
use crate::snapshot::Snapshot;
use anyhow::Error;
use bytes::Bytes;
use eframe::egui::Context;
use egui_inbox::UiInbox;
use futures::SinkExt;
use octocrab::Octocrab;
use octocrab::models::ArtifactId;
use octocrab::params::actions::ArchiveFormat;
use std::task::Poll;

#[derive(Debug)]
pub enum GHArtifactLoader {
    LoadingData(UiInbox<anyhow::Result<(Bytes, String)>>),
    LoadingArchive(ArchiveLoader),
    Error(anyhow::Error),
}

impl GHArtifactLoader {
    pub fn new(client: Octocrab, artifact: GithubArtifactLink) -> Self {
        let mut inbox = UiInbox::new();

        inbox.spawn(move |tx| async move {
            tx.send(download_artifact(&client, &artifact).await).ok();
        });

        Self::LoadingData(inbox)
    }
}

pub async fn download_artifact(
    client: &Octocrab,
    artifact: &GithubArtifactLink,
) -> anyhow::Result<(Bytes, String)> {
    let data = client
        .actions()
        .download_artifact(
            &artifact.repo.owner,
            &artifact.repo.repo,
            artifact.artifact_id,
            ArchiveFormat::Zip,
        )
        .await?;
    let name = artifact.name();
    Ok((data, name))
}

impl LoadSnapshots for GHArtifactLoader {
    fn update(&mut self, ctx: &Context) {
        let mut new_self = None;
        match self {
            GHArtifactLoader::LoadingData(inbox) => {
                if let Some(result) = inbox.read(ctx).last() {
                    match result {
                        Ok((data, name)) => {
                            new_self = Some(GHArtifactLoader::LoadingArchive(ArchiveLoader::new(
                                crate::loaders::DataReference::Data(data.clone(), name),
                            )));
                        }
                        Err(e) => {
                            new_self = Some(GHArtifactLoader::Error(e));
                        }
                    }
                }
            }
            GHArtifactLoader::LoadingArchive(loader) => {
                loader.update(ctx);
            }
            GHArtifactLoader::Error(_) => {}
        }
        if let Some(new_self) = new_self {
            *self = new_self;
        }
    }

    fn snapshots(&self) -> &[Snapshot] {
        match self {
            GHArtifactLoader::LoadingArchive(loader) => loader.snapshots(),
            _ => &[],
        }
    }

    fn state(&self) -> Poll<Result<(), &Error>> {
        match self {
            GHArtifactLoader::LoadingData(_) => Poll::Pending,
            GHArtifactLoader::LoadingArchive(loader) => loader.state(),
            GHArtifactLoader::Error(e) => Poll::Ready(Err(e)),
        }
    }

    fn files_header(&self) -> String {
        match self {
            GHArtifactLoader::LoadingData(_) => "Github Artifact".to_string(),
            GHArtifactLoader::LoadingArchive(loader) => loader.files_header(),
            GHArtifactLoader::Error(_) => "Github Artifact".to_string(),
        }
    }
}
