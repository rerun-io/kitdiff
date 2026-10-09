//! The "Commit the updated snapshots" button, which triggers a GitHub Actions workflow
//! that commits the snapshots of an artifact to the PR branch.

use crate::github::model::GithubRepoLink;
use crate::state::AppStateRef;
use eframe::egui::Ui;
use egui_inbox::UiInbox;
use octocrab::models::RunId;
use serde_json::json;

/// Tells the workflow where to get the snapshots from.
#[derive(Debug, Clone)]
pub enum SnapshotOrigin {
    /// A GitHub Actions run that uploaded the snapshots as an artifact.
    /// The workflow gets a `run_id` input.
    Run(RunId),

    /// A commit whose snapshots some other CI published, see [`crate::config::Artifact`].
    /// The workflow gets a `commit` input.
    ///
    /// There is no platform: the workflow picks the platform that is the source of truth,
    /// even when the user views the snapshots of another platform.
    Commit(String),
}

#[derive(Debug, Clone)]
pub struct UpdateSnapshotsWorkflow {
    pub repo: GithubRepoLink,

    /// File name or ID of the workflow, from [`crate::config::Github::update_snapshot_workflow_name`].
    pub workflow: String,

    /// The branch to run the workflow on, and so the branch to commit to.
    pub branch: String,

    pub origin: SnapshotOrigin,
}

impl UpdateSnapshotsWorkflow {
    fn inputs(&self) -> serde_json::Value {
        match &self.origin {
            SnapshotOrigin::Run(run_id) => json!({ "run_id": run_id.to_string() }),
            SnapshotOrigin::Commit(commit) => json!({ "commit": commit }),
        }
    }

    fn workflow_link(&self) -> String {
        format!(
            "https://github.com/{}/{}/actions/workflows/{}",
            self.repo.owner, self.repo.repo, self.workflow
        )
    }
}

enum PipelineState {
    Loading,
    Triggered { workflow_link: String },
    Error(anyhow::Error),
}

pub struct UpdateSnapshotsButton {
    workflow: UpdateSnapshotsWorkflow,
    inbox: UiInbox<PipelineState>,
    pipeline_state: Option<PipelineState>,
}

impl UpdateSnapshotsButton {
    pub fn new(workflow: UpdateSnapshotsWorkflow) -> Self {
        Self {
            workflow,
            inbox: UiInbox::new(),
            pipeline_state: None,
        }
    }

    pub fn workflow(&self) -> &UpdateSnapshotsWorkflow {
        &self.workflow
    }

    pub fn update(&mut self, ctx: &eframe::egui::Context) {
        if let Some(state) = self.inbox.read(ctx).last() {
            self.pipeline_state = Some(state);
        }
    }

    pub fn ui(&self, ui: &mut Ui, state: &AppStateRef<'_>) {
        let response = ui
            .button("Commit the updated snapshots")
            .on_hover_text(format!(
                "This will run the {} workflow, which creates a commit on the PR branch with the updated snapshots.",
                self.workflow.workflow
            ));
        if response.clicked() {
            let client = state.github_auth.client();
            let workflow = self.workflow.clone();
            let sender = self.inbox.sender();
            sender.send(PipelineState::Loading).ok();
            hello_egui_utils::spawn(async move {
                let result = client
                    .actions()
                    .create_workflow_dispatch(
                        workflow.repo.owner.clone(),
                        workflow.repo.repo.clone(),
                        workflow.workflow.clone(),
                        workflow.branch.clone(),
                    )
                    .inputs(workflow.inputs())
                    .send()
                    .await;

                let state = match result {
                    Ok(()) => PipelineState::Triggered {
                        workflow_link: workflow.workflow_link(),
                    },
                    Err(err) => PipelineState::Error(err.into()),
                };
                sender.send(state).ok();
            });
        }

        match &self.pipeline_state {
            Some(PipelineState::Loading) => {
                ui.label("Triggering pipeline...");
            }
            Some(PipelineState::Triggered { workflow_link }) => {
                ui.horizontal(|ui| {
                    ui.label("Pipeline triggered!");
                    ui.hyperlink_to("View workflows", workflow_link);
                });
            }
            Some(PipelineState::Error(err)) => {
                ui.colored_label(ui.visuals().error_fg_color, format!("Error: {err}"));
            }
            None => {}
        }
    }
}
