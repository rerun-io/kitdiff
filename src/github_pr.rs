use crate::DiffSource;
use eframe::egui;
use eframe::egui::{Context, Popup};
use egui_inbox::UiInbox;
use futures::stream::FuturesUnordered;
use futures::{StreamExt, TryStreamExt};
use octocrab::{AuthState, Error, Octocrab};
use std::collections::HashMap;
use std::future::ready;
use std::pin::pin;
use std::sync::mpsc;
use std::task::Poll;
// Import octocrab models
use crate::github_model::{GithubPrLink, PrNumber};
use crate::octokit::RepoClient;
use octocrab::models::{
    pulls::PullRequest,
    repos::RepoCommit,
    workflows::{Run, WorkflowListArtifact},
};

pub fn parse_github_pr_url(url: &str) -> Result<(String, String, u32), String> {
    // Parse URLs like: https://github.com/rerun-io/rerun/pull/11253
    if !url.starts_with("https://github.com/") {
        return Err("URL must start with https://github.com/".to_string());
    }

    let path = url
        .strip_prefix("https://github.com/")
        .ok_or("Invalid GitHub URL")?;

    let parts: Vec<&str> = path.split('/').collect();
    if parts.len() != 4 || parts[2] != "pull" {
        return Err("Expected format: https://github.com/owner/repo/pull/123".to_string());
    }

    let user = parts[0].to_string();
    let repo = parts[1].to_string();
    let pr_number = parts[3].parse::<u32>().map_err(|_| "Invalid PR number")?;

    Ok((user, repo, pr_number))
}

#[derive(Debug, Clone)]
pub enum GithubPrMessage {
    FetchedDetails(PrDetails),
    FetchedCommits(Vec<CommitInfo>),
    Error(String),
}

#[derive(Debug, Clone)]
pub struct PrDetails {
    pub title: String,
    pub head_ref: String,
    pub base_ref: String,
    pub state: String,
    pub html_url: String,
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
    pub auth_token: Option<String>,
    inbox: UiInbox<Result<PrWithArtifacts, octocrab::Error>>,
    pub data: Poll<Result<PrWithArtifacts, octocrab::Error>>,
}

impl GithubPr {
    pub fn new(link: GithubPrLink, auth_token: Option<String>) -> Self {
        let mut client = octocrab_wasm::builder()
            .build()
            .expect("Failed to build Octocrab client");

        if let Some(token) = &auth_token {
            client = client
                .user_access_token(token.to_owned())
                .expect("Failed to set token");
        }

        let mut inbox = UiInbox::new();

        {
            let client = RepoClient::new(client, link.repo.clone());
            inbox.spawn(|tx| async move {
                let details = get_all_pr_artifacts(&client, link.pr_number).await;
                let _ = tx.send(details);
            });
        }

        Self {
            link,
            auth_token: auth_token.clone(),
            inbox,
            data: Poll::Pending,
        }
    }

    /// Display details about the PR and allow selecting an artifact to load
    pub fn ui(&mut self, ui: &mut eframe::egui::Ui) -> Option<DiffSource> {
        if let Some(data) = self.inbox.read(ui).last() {
            self.data = Poll::Ready(data);
        }

        let mut selected_source = None;

        ui.group(|ui| {
            ui.heading(format!("GitHub PR #{}", self.link.pr_number));

            match &self.data {
                Poll::Ready(Ok(data)) => {
                    let details = &data.pr;

                    if let Some(title) = &details.title {
                        ui.label(title);
                    }

                    ui.separator();

                    if let Some(html_url) = &details.html_url {
                        if ui.button("Compare PR Branches").clicked() {
                            selected_source = Some(DiffSource::Pr(html_url.to_string()));
                        }
                    }

                    ui.separator();
                    ui.heading("Recent Commits & Artifacts");

                    ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Truncate);

                    for (commit, artifacts) in &data.artifacts_by_commit {
                        egui::Sides::new().shrink_left().show(
                            ui,
                            |ui| {
                                ui.label(&commit.commit.message);
                            },
                            |ui| {
                                if artifacts.len() > 0 {
                                    let response =
                                        ui.link(format!("{} artifacts", artifacts.len()));
                                    Popup::menu(&response).show(|ui| {
                                        for artifact in artifacts {
                                            if ui.button(&artifact.name).clicked() {
                                                selected_source = Some(DiffSource::GHArtifact {
                                                    repo: self.link.repo.clone(),
                                                    artifact_id: artifact.id.to_string(),
                                                });
                                            }
                                        }
                                    });
                                }
                            },
                        );
                    }
                }
                Poll::Ready(Err(error)) => {
                    ui.colored_label(ui.visuals().error_fg_color, format!("Error: {}", error));
                }
                Poll::Pending => {
                    ui.spinner();
                }
            }
        });

        selected_source
    }
}

async fn get_pr_runs_by_commit(
    repo: &RepoClient,
    pr_number: PrNumber,
) -> octocrab::Result<HashMap<String, Vec<Run>>> {
    // First, get the PR to find the head branch
    let pr = repo.pulls().get(pr_number).await?;

    // Get the branch name from the PR
    let branch_name = &pr.head.ref_field;

    // List all workflow runs for this branch
    let mut runs_by_commit: HashMap<String, Vec<Run>> = HashMap::new();

    let page = repo
        .workflows()
        .list_all_runs()
        .branch(branch_name)
        .per_page(100)
        .send()
        .await?
        .into_stream(&repo);
    
    let mut page = pin!(page);

    while let Some(run) = page.next().await.transpose()? {
        runs_by_commit
            .entry(run.head_sha.clone())
            .or_insert_with(Vec::new)
            .push(run);
    }

    Ok(runs_by_commit)
}

// Get all commits for a PR to get a complete picture
async fn get_pr_commits_with_runs(
    repo: &RepoClient,
    pr: PrNumber,
) -> octocrab::Result<Vec<(RepoCommit, Vec<Run>)>> {
    // Get all commits in the PR
    let commits = repo.pulls().pr_commits(pr).send().await?;

    // Get all runs grouped by commit
    let runs_by_commit = get_pr_runs_by_commit(repo, pr).await?;

    Ok(commits
        .items
        .into_iter()
        .map(|commit| {
            let runs = runs_by_commit
                .get(&commit.sha)
                .cloned()
                .unwrap_or_else(Vec::new);
            (commit, runs)
        })
        .collect())
}

struct PrWithArtifacts {
    pr: PullRequest,
    artifacts_by_commit: Vec<(RepoCommit, Vec<WorkflowListArtifact>)>,
}

async fn get_all_pr_artifacts(
    repo: &RepoClient,
    pr: PrNumber,
) -> octocrab::Result<PrWithArtifacts> {
    let commits_with_runs = get_pr_commits_with_runs(repo, pr).await?;

    let mut artifacts_by_commit = Vec::new();
    for (commit, runs) in commits_with_runs {
        let artifacts = FuturesUnordered::from_iter(runs.into_iter().map(|run| async move {
            repo.actions()
                .list_workflow_run_artifacts(&repo.repo().owner, &repo.repo().repo, run.id)
                .send()
                .await
        }))
        .try_filter_map(|item| {
            ready(Ok(item
                .value
                .map(|page| futures::stream::iter(page.items).map(Ok))))
        })
        .try_flatten()
        .try_collect::<Vec<WorkflowListArtifact>>()
        .await?;

        artifacts_by_commit.push((commit, artifacts));
    }

    let pr = repo.pulls().get(pr).await?;

    Ok(PrWithArtifacts {
        pr,
        artifacts_by_commit,
    })
}
