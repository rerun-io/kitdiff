use crate::DiffSource;
use eframe::egui;
use eframe::egui::{Button, Context, ScrollArea, Spinner};
use egui_inbox::UiInbox;
use futures::TryStreamExt as _;
use futures::stream::FuturesUnordered;
use graphql_client::GraphQLQuery;
use octocrab::Octocrab;
use re_ui::egui_ext::boxed_widget::BoxedWidgetLocalExt as _;
use std::collections::hash_map::Entry;
use std::collections::{HashMap, HashSet};
use std::task::Poll;
// Import octocrab models
use crate::github::octokit::RepoClient;
use crate::state::{AppStateRef, SystemCommand};
use octocrab::models::{RunId, workflows::WorkflowListArtifact};
use re_ui::list_item::{LabelContent, ListItemContentButtonsExt as _, list_item_scope};
use re_ui::{OnResponseExt as _, SectionCollapsingHeader, UiExt as _, icons};
// use chrono::DateTime;
pub type GitObjectID = String;
pub type DateTime = String;
pub type URI = String;

// The paths are relative to the directory where your `Cargo.toml` is located.
// Both json and the GraphQL schema language are supported as sources for the schema
#[derive(GraphQLQuery, Debug)]
#[graphql(
    schema_path = "github.graphql",
    query_path = "src/github/pr.graphql",
    response_derives = "Debug, Clone"
)]
pub struct PrDetailsQuery;
use crate::github::model::{GithubArtifactLink, GithubPrLink, PrNumber};
use anyhow::{Error, Result, anyhow};

pub fn parse_github_pr_url(url: &str) -> Result<(String, String, u32), String> {
    // Parse URLs like: https://github.com/rerun-io/rerun/pull/11253
    if !url.starts_with("https://github.com/") {
        return Err("URL must start with https://github.com/".to_owned());
    }

    let path = url
        .strip_prefix("https://github.com/")
        .ok_or("Invalid GitHub URL")?;

    let parts: Vec<&str> = path.split('/').collect();
    if parts.len() != 4 || parts[2] != "pull" {
        return Err("Expected format: https://github.com/owner/repo/pull/123".to_owned());
    }

    let user = parts[0].to_owned();
    let repo = parts[1].to_owned();
    let pr_number = parts[3]
        .parse::<u32>()
        .map_err(|_err| "Invalid PR number")?;

    Ok((user, repo, pr_number))
}

#[derive(Debug)]
pub enum GithubPrCommand {
    FetchedData(Result<PrWithCommits>),
    FetchedCommitArtifacts {
        sha: String,
        artifacts: Result<Vec<ArtifactData>, Error>,
    },
    FetchCommitArtifacts {
        sha: String,
    },
}

#[derive(Debug, Clone)]
pub struct CommitInfo {
    pub sha: String,
    pub message: String,
    pub author: String,
    pub date: String,
    pub artifacts: Vec<GithubArtifact>,
}

#[derive(Debug, Clone)]
pub struct GithubArtifact {
    pub id: u64,
    pub name: String,
    pub size_in_bytes: u64,
    pub download_url: String,
}

pub struct GithubPr {
    link: GithubPrLink,
    inbox: UiInbox<GithubPrCommand>,
    pub data: Poll<Result<PrWithCommits, Error>>,
    client: Octocrab,
}

#[derive(Debug)]
pub struct PrWithCommits {
    title: String,
    head_branch: String,
    #[expect(dead_code)]
    base_branch: String,
    commits: Vec<CommitData>,
    artifacts: HashMap<String, Poll<Result<Vec<ArtifactData>>>>,
}

#[derive(Debug)]
pub struct ArtifactData {
    data: WorkflowListArtifact,
    run_id: RunId,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum CommitState {
    Pending,
    Success,
    Failure,
}

#[derive(Debug)]
struct CommitData {
    message: String,
    sha: String,
    status: CommitState,
    workflow_run_ids: Vec<u64>,
}

impl GithubPr {
    pub fn new(link: GithubPrLink, client: Octocrab) -> Self {
        let mut inbox = UiInbox::new();

        {
            let client = RepoClient::new(client.clone(), link.repo.clone());
            inbox.spawn(|tx| async move {
                let details = get_pr_commits(&client, link.pr_number).await;
                let _ = tx.send(GithubPrCommand::FetchedData(details));
            });
        }

        Self {
            link,
            inbox,
            data: Poll::Pending,
            client,
        }
    }

    pub fn update(&mut self, _ctx: &Context) {
        for command in self.inbox.read(_ctx) {
            match command {
                GithubPrCommand::FetchedData(data) => {
                    self.data = Poll::Ready(data);
                }
                GithubPrCommand::FetchedCommitArtifacts { sha, artifacts } => {
                    if let Poll::Ready(Ok(pr_data)) = &mut self.data {
                        pr_data.artifacts.insert(sha, Poll::Ready(artifacts));
                    }
                }
                GithubPrCommand::FetchCommitArtifacts { sha } => {
                    if let Poll::Ready(Ok(pr_data)) = &mut self.data {
                        match pr_data.artifacts.entry(sha.clone()) {
                            Entry::Occupied(_) => {}
                            Entry::Vacant(entry) => {
                                entry.insert(Poll::Pending);

                                let workflow_run_ids = pr_data
                                    .commits
                                    .iter()
                                    .find(|c| c.sha == sha)
                                    .map(|c| c.workflow_run_ids.clone())
                                    .unwrap_or_default();

                                let client =
                                    RepoClient::new(self.client.clone(), self.link.repo.clone());
                                self.inbox.spawn(move |tx| async move {
                                    let artifacts =
                                        fetch_commit_artifacts(&client, workflow_run_ids).await;
                                    let _ = tx.send(GithubPrCommand::FetchedCommitArtifacts {
                                        sha,
                                        artifacts,
                                    });
                                });
                            }
                        }
                    }
                }
            }
        }
    }
}

async fn get_pr_commits(repo: &RepoClient, pr: PrNumber) -> Result<PrWithCommits> {
    let response: graphql_client::Response<pr_details_query::ResponseData> = repo
        .graphql(&PrDetailsQuery::build_query(pr_details_query::Variables {
            owner: repo.repo().owner.clone(),
            repo: repo.repo().repo.clone(),
            oid: pr as _,
        }))
        .await?;

    let response = response
        .data
        .ok_or_else(|| anyhow!("No data in response"))?
        .repository
        .ok_or_else(|| anyhow!("Repository not found"))?
        .pull_request
        .ok_or_else(|| anyhow!("Pull request not found"))?;

    let mut data = PrWithCommits {
        title: response.title,
        head_branch: response.head_ref_name,
        base_branch: response.base_ref_name,
        commits: Vec::new(),
        artifacts: HashMap::new(),
    };

    for commit in response
        .commits
        .nodes
        .ok_or_else(|| anyhow!("No commits found"))?
    {
        if let Some(commit) = commit {
            let commit = commit.commit;
            let sha = commit.oid;
            let message = commit.message_headline;

            let mut status = CommitState::Success;
            let mut workflow_run_ids = HashSet::new();

            // Unfortunately github has no easy way to get the status for a commit, best thing seems to be
            // to query all check suites and group them by workflow.
            let mut last_suite_per_workflow = HashMap::new();

            if let Some(suites) = commit.check_suites {
                if let Some(nodes) = suites.nodes {
                    for node in nodes {
                        if let Some(node) = node {
                            if let Some(workflow_run) = node.workflow_run.clone() {
                                last_suite_per_workflow.insert(workflow_run.workflow.id, node);
                            }
                        }
                    }
                }
            }

            for (_workflow_id, suite) in last_suite_per_workflow {
                let pending = match suite.status {
                    pr_details_query::CheckStatusState::COMPLETED => false,
                    pr_details_query::CheckStatusState::IN_PROGRESS => true,
                    pr_details_query::CheckStatusState::PENDING => true,
                    pr_details_query::CheckStatusState::QUEUED => true,
                    pr_details_query::CheckStatusState::REQUESTED => true,
                    pr_details_query::CheckStatusState::WAITING => true,
                    pr_details_query::CheckStatusState::Other(_) => false,
                };
                let error = if let Some(conclusion) = suite.conclusion {
                    match conclusion {
                        pr_details_query::CheckConclusionState::ACTION_REQUIRED => true,
                        pr_details_query::CheckConclusionState::CANCELLED => true,
                        pr_details_query::CheckConclusionState::FAILURE => true,
                        pr_details_query::CheckConclusionState::NEUTRAL => false,
                        pr_details_query::CheckConclusionState::SKIPPED => false,
                        pr_details_query::CheckConclusionState::STALE => false,
                        pr_details_query::CheckConclusionState::STARTUP_FAILURE => true,
                        pr_details_query::CheckConclusionState::SUCCESS => false,
                        pr_details_query::CheckConclusionState::TIMED_OUT => true,
                        pr_details_query::CheckConclusionState::Other(_) => true,
                    }
                } else {
                    false
                };
                if error {
                    status = CommitState::Failure;
                } else if pending && status != CommitState::Failure {
                    status = CommitState::Pending;
                }

                if let Some(run) = suite.workflow_run {
                    if let Some(db_id) = run.database_id {
                        workflow_run_ids.insert(db_id as u64);
                    }
                }
            }

            data.commits.push(CommitData {
                message,
                sha,
                status,
                workflow_run_ids: workflow_run_ids.into_iter().collect(),
            });
        }
    }

    Ok(data)
}

async fn fetch_commit_artifacts(repo: &RepoClient, run_ids: Vec<u64>) -> Result<Vec<ArtifactData>> {
    let artifacts = FuturesUnordered::from_iter(run_ids.into_iter().map(|run| async move {
        let artifacts_page = repo
            .actions()
            .list_workflow_run_artifacts(&repo.repo().owner, &repo.repo().repo, RunId(run))
            .send()
            .await?
            .value
            .expect("No etag was provided, so we should have a value");

        let stream = artifacts_page
            .into_stream(repo)
            .map_ok(move |artifact| ArtifactData {
                data: artifact,
                run_id: RunId(run),
            });

        Ok(stream)
    }))
    .try_flatten()
    .try_collect::<Vec<ArtifactData>>()
    .await?;

    Ok(artifacts)
}

pub fn pr_ui(ui: &mut egui::Ui, state: &AppStateRef<'_>, pr: &GithubPr) {
    let mut selected_source = None;

    list_item_scope(ui, "pr_info", |ui| match &pr.data {
        Poll::Ready(Ok(data)) => {
            SectionCollapsingHeader::new(format!("PR: {}", data.title)).show(ui, |ui| {
                ui.set_max_height(100.0);
                ScrollArea::vertical().show(ui, |ui| {
                    for commit in data.commits.iter().rev() {
                        let item = ui.list_item();

                        let button = match &commit.status {
                            CommitState::Failure => Button::image(
                                icons::ERROR.as_image().tint(ui.tokens().alert_error.icon),
                            )
                            .boxed_local(),
                            CommitState::Pending => Spinner::new().boxed_local(),
                            CommitState::Success => Button::image(
                                icons::SUCCESS
                                    .as_image()
                                    .tint(ui.tokens().alert_success.icon),
                            )
                            .boxed_local(),
                        };

                        let button = button.on_menu(|ui| {
                            ui.set_min_width(250.0);
                            match data.artifacts.get(&commit.sha) {
                                None => {
                                    pr.inbox
                                        .sender()
                                        .send(GithubPrCommand::FetchCommitArtifacts {
                                            sha: commit.sha.clone(),
                                        })
                                        .ok();
                                }
                                Some(Poll::Pending) => {
                                    ui.spinner();
                                }
                                Some(Poll::Ready(Err(error))) => {
                                    ui.colored_label(
                                        ui.visuals().error_fg_color,
                                        format!("Error: {error}"),
                                    );
                                }
                                #[expect(clippy::excessive_nesting)]
                                Some(Poll::Ready(Ok(artifacts))) => {
                                    if artifacts.is_empty() {
                                        ui.label("No artifacts found");
                                    } else {
                                        for artifact in artifacts {
                                            if ui.button(&artifact.data.name).clicked() {
                                                selected_source = Some(DiffSource::GHArtifact(
                                                    GithubArtifactLink {
                                                        repo: pr.link.repo.clone(),
                                                        artifact_id: artifact.data.id,
                                                        name: Some(artifact.data.name.clone()),
                                                        branch_name: Some(data.head_branch.clone()),
                                                        run_id: Some(artifact.run_id),
                                                    },
                                                ));
                                            }
                                        }
                                    }
                                }
                            }
                        });

                        let content = LabelContent::new(&commit.message)
                            .with_button(button)
                            .with_always_show_buttons(true);

                        item.show_hierarchical(ui, content);
                    }
                });
            });
        }
        Poll::Ready(Err(error)) => {
            ui.colored_label(ui.visuals().error_fg_color, format!("Error: {error}"));
        }
        Poll::Pending => {
            SectionCollapsingHeader::new(format!("PR: {}", pr.link))
                .with_button(Spinner::new())
                .show(ui, |_ui| {});
            ui.spinner();
        }
    });

    if let Some(source) = selected_source {
        state.send(SystemCommand::Open(source));
    }
}
