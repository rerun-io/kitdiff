use crate::github::model::GithubArtifactLink;
use crate::github::update_snapshots::UpdateSnapshotsButton;
use crate::loaders::LoadSnapshots;
use crate::loaders::archive_loader::ArchiveLoader;
use crate::snapshot::Snapshot;
use crate::state::AppStateRef;
use anyhow::Error;
use bytes::Bytes;
use eframe::egui::{Context, Ui};
use egui_inbox::UiInbox;
use octocrab::Octocrab;
use octocrab::params::actions::ArchiveFormat;
use std::task::Poll;

pub struct GHArtifactLoader {
    state: LoaderState,
    artifact: GithubArtifactLink,
    update_button: Option<UpdateSnapshotsButton>,
}

#[derive(Debug)]
pub enum LoaderState {
    LoadingData(UiInbox<anyhow::Result<(Bytes, String)>>),
    LoadingArchive(ArchiveLoader),
    Error(anyhow::Error),
}

impl GHArtifactLoader {
    pub fn new(client: Octocrab, artifact: GithubArtifactLink) -> Self {
        let mut data_inbox = UiInbox::new();

        {
            let artifact = artifact.clone();
            data_inbox.spawn(move |tx| async move {
                tx.send(download_artifact(&client, &artifact).await).ok();
            });
        }

        Self {
            state: LoaderState::LoadingData(data_inbox),
            update_button: artifact
                .update_snapshots
                .clone()
                .map(UpdateSnapshotsButton::new),
            artifact,
        }
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
        if let Some(button) = &mut self.update_button {
            button.update(ctx);
        }

        let mut new_state = None;
        match &mut self.state {
            LoaderState::LoadingData(inbox) => {
                if let Some(result) = inbox.read(ctx).last() {
                    match result {
                        Ok((data, name)) => {
                            new_state = Some(LoaderState::LoadingArchive(ArchiveLoader::new(
                                crate::loaders::DataReference::Data(data.clone(), name),
                            )));
                        }
                        Err(e) => {
                            new_state = Some(LoaderState::Error(e));
                        }
                    }
                }
            }
            LoaderState::LoadingArchive(loader) => {
                loader.update(ctx);
            }
            LoaderState::Error(_) => {}
        }
        if let Some(new_self) = new_state {
            self.state = new_self;
        }
    }

    fn snapshots(&self) -> &[Snapshot] {
        match &self.state {
            LoaderState::LoadingArchive(loader) => loader.snapshots(),
            _ => &[],
        }
    }

    fn state(&self) -> Poll<Result<(), &Error>> {
        match &self.state {
            LoaderState::LoadingData(_) => Poll::Pending,
            LoaderState::LoadingArchive(loader) => loader.state(),
            LoaderState::Error(e) => Poll::Ready(Err(e)),
        }
    }

    fn files_header(&self) -> String {
        match &self.state {
            LoaderState::LoadingData(_) | LoaderState::Error(_) => "Github Artifact".to_owned(),
            LoaderState::LoadingArchive(loader) => loader.files_header(),
        }
    }

    fn extra_ui(&self, ui: &mut Ui, state: &AppStateRef<'_>) {
        if let Some(button) = &self.update_button {
            button.ui(ui, state);
        }
    }

    fn refresh(&mut self, client: Octocrab) {
        *self = Self::new(client, self.artifact.clone());
    }
}
