use crate::github::model::GithubArtifactLink;
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
use serde_json::json;
use std::task::Poll;

pub struct GHArtifactLoader {
    state: LoaderState,
    artifact: GithubArtifactLink,
}

#[derive(Debug)]
pub enum LoaderState {
    LoadingData(UiInbox<anyhow::Result<(Bytes, String)>>),
    LoadingArchive(ArchiveLoader),
    Error(anyhow::Error),
}

impl GHArtifactLoader {
    pub fn new(client: Octocrab, artifact: GithubArtifactLink) -> Self {
        let mut inbox = UiInbox::new();

        {
            let artifact = artifact.clone();
            inbox.spawn(move |tx| async move {
                tx.send(download_artifact(&client, &artifact).await).ok();
            });
        }

        Self {
            state: LoaderState::LoadingData(inbox),
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
        let mut new_self = None;
        match &mut self.state {
            LoaderState::LoadingData(inbox) => {
                if let Some(result) = inbox.read(ctx).last() {
                    match result {
                        Ok((data, name)) => {
                            new_self = Some(LoaderState::LoadingArchive(ArchiveLoader::new(
                                crate::loaders::DataReference::Data(data.clone(), name),
                            )));
                        }
                        Err(e) => {
                            new_self = Some(LoaderState::Error(e));
                        }
                    }
                }
            }
            LoaderState::LoadingArchive(loader) => {
                loader.update(ctx);
            }
            LoaderState::Error(_) => {}
        }
        if let Some(new_self) = new_self {
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
            LoaderState::LoadingData(_) => "Github Artifact".to_owned(),
            LoaderState::LoadingArchive(loader) => loader.files_header(),
            LoaderState::Error(_) => "Github Artifact".to_owned(),
        }
    }

    fn extra_ui(&self, ui: &mut Ui, state: &AppStateRef<'_>) {
        if let Some((git_ref, run_id)) = self.artifact.branch_name.clone().zip(self.artifact.run_id)
        {
            let response = ui
                .button("Update snapshots from this archive")
                .on_hover_text(
                    "This will create a commit on the PR branch with the updated snapshots.",
                );
            if response.clicked() {
                let client = state.github_auth.client();
                let artifact = self.artifact.clone();
                hello_egui_utils::spawn(async move {
                    let _ = client
                        .actions()
                        .create_workflow_dispatch(
                            artifact.repo.owner,
                            artifact.repo.repo,
                            "update_kittest_snapshots.yml",
                            git_ref,
                        )
                        .inputs(json!({
                            "artifact_id": artifact.artifact_id.to_string(),
                            "run_id": run_id.to_string(),
                        }))
                        .send()
                        .await;
                });
            }
        }
    }

    fn refresh(&mut self, client: Octocrab) {
        *self = Self::new(client, self.artifact.clone());
    }
}
