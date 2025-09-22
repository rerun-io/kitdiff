use crate::DiffSource;
use eframe::egui;
use eframe::egui::{Context, Popup};
use egui_inbox::UiInbox;
use octocrab::{AuthState, Error, Octocrab};
use std::collections::HashMap;
use std::sync::mpsc;
use std::task::Poll;
// Import octocrab models
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
    pub user: String,
    pub repo: String,
    pub pr_number: u32,
    pub auth_token: Option<String>,
    inbox: UiInbox<Result<PrWithArtifacts, octocrab::Error>>,
    pub data: Poll<Result<PrWithArtifacts, octocrab::Error>>,
}

impl GithubPr {
    pub fn new(
        user: String,
        repo: String,
        pr_number: u32,
        ctx: Context,
        auth_token: Option<String>,
    ) -> Self {
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
            let client = client.clone();
            let user = user.clone();
            let repo = repo.clone();
            let pr_number = pr_number;
            inbox.spawn(|tx| async move {
                let details = get_all_pr_artifacts(&client, &user, &repo, pr_number as u64).await;
                let _ = tx.send(details);
            });
        }

        Self {
            user: user.clone(),
            repo: repo.clone(),
            pr_number,
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
            ui.heading(format!("GitHub PR #{}", self.pr_number));

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
                                                    owner: self.user.clone(),
                                                    repo: self.repo.clone(),
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
    octocrab: &Octocrab,
    owner: &str,
    repo: &str,
    pr_number: u64,
) -> octocrab::Result<HashMap<String, Vec<Run>>> {
    // First, get the PR to find the head branch
    let pr = octocrab.pulls(owner, repo).get(pr_number).await?;

    // Get the branch name from the PR
    let branch_name = &pr.head.ref_field;

    // List all workflow runs for this branch
    let mut runs_by_commit: HashMap<String, Vec<Run>> = HashMap::new();
    let mut page = 1u32;

    loop {
        let runs = octocrab
            .workflows(owner, repo)
            .list_all_runs()
            .branch(branch_name)
            .page(page)
            .per_page(100)
            .send()
            .await?;

        // Group runs by commit SHA
        for run in runs.items {
            runs_by_commit
                .entry(run.head_sha.clone())
                .or_insert_with(Vec::new)
                .push(run);
        }

        // Check if there are more pages
        if runs.next.is_none() {
            break;
        }
        page += 1;
    }

    Ok(runs_by_commit)
}

struct CommitsWithRuns {
    commits: Vec<(RepoCommit, Vec<Run>)>,
}

// Get all commits for a PR to get a complete picture
async fn get_pr_commits_with_runs(
    octocrab: &Octocrab,
    owner: &str,
    repo: &str,
    pr_number: u64,
) -> octocrab::Result<Vec<(RepoCommit, Vec<Run>)>> {
    // Get all commits in the PR
    let commits = octocrab
        .pulls(owner, repo)
        .pr_commits(pr_number)
        .send()
        .await?;

    // Get all runs grouped by commit
    let runs_by_commit = get_pr_runs_by_commit(octocrab, owner, repo, pr_number).await?;

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
    octocrab: &Octocrab,
    owner: &str,
    repo: &str,
    pr_number: u64,
) -> octocrab::Result<PrWithArtifacts> {
    let commits_with_runs = get_pr_commits_with_runs(octocrab, owner, repo, pr_number).await?;

    let mut artifacts_by_commit = Vec::new();
    for (commit, runs) in commits_with_runs {
        let mut all_artifacts = Vec::new();
        for run in runs {
            let artifacts = octocrab
                .actions()
                .list_workflow_run_artifacts(owner, repo, run.id.into())
                .send()
                .await?;
            if let Some(artifacts) = artifacts.value {
                all_artifacts.extend(artifacts.items);
            }
        }
        artifacts_by_commit.push((commit, all_artifacts));
    }

    let pr = octocrab.pulls(owner, repo).get(pr_number).await?;

    Ok(PrWithArtifacts {
        pr,
        artifacts_by_commit,
    })
}
