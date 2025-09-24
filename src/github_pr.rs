use crate::DiffSource;
use eframe::egui;
use eframe::egui::{Button, Context, Popup, Spinner};
use egui_inbox::UiInbox;
use futures::stream::FuturesUnordered;
use futures::{StreamExt, TryStreamExt};
use octocrab::{AuthState, Error, Octocrab, Page};
use re_ui::egui_ext::boxed_widget::BoxedWidgetLocalExt;
use std::collections::HashMap;
use std::future::ready;
use std::pin::pin;
use std::str::FromStr;
use std::sync::mpsc;
use std::task::Poll;
// Import octocrab models
use crate::github_model::{GithubPrLink, PrNumber};
use crate::octokit::RepoClient;
use crate::state::{AppStateRef, SystemCommand};
use octocrab::models::commits::GithubCommitStatus;
use octocrab::models::{
    CombinedStatus, Status, StatusState,
    pulls::PullRequest,
    repos::RepoCommit,
    workflows::{Run, WorkflowListArtifact},
};
use octocrab::params::repos::Reference;
use octocrab::workflows::ListRunsBuilder;
use re_ui::list_item::{LabelContent, ListItemContentButtonsExt, list_item_scope};
use re_ui::{OnResponseExt, UiExt, icons};

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

#[derive(Debug)]
pub enum GithubPrCommand {
    FetchedData(Result<PrWithCommits, octocrab::Error>),
    FetchedCommitArtifacts {
        sha: String,
        artifacts: Result<Vec<ArtifactData>, octocrab::Error>,
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
    pub data: Poll<Result<PrWithCommits, octocrab::Error>>,
    client: Octocrab,
}

#[derive(Debug)]
struct PrWithCommits {
    pr: PullRequest,
    commits: Vec<CommitWithArtifacts>,
}

#[derive(Debug)]
struct CommitWithArtifacts {
    commit: RepoCommit,
    status: CombinedStatus,
    artifacts: Option<Poll<Result<Vec<ArtifactData>, octocrab::Error>>>,
}

#[derive(Debug, Clone)]
struct ArtifactData {
    artifact: WorkflowListArtifact,
    run: Run,
}

impl GithubPr {
    pub fn new(link: GithubPrLink, client: Octocrab) -> Self {
        let mut inbox = UiInbox::new();

        {
            let client = RepoClient::new(client.clone(), link.repo.clone());
            inbox.spawn(|tx| async move {
                let details = get_all_pr_artifacts(&client, link.pr_number).await;
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
                        for commit in &mut pr_data.commits {
                            if commit.commit.sha == sha {
                                commit.artifacts = Some(Poll::Ready(artifacts));
                                break;
                            }
                        }
                    }
                }
                GithubPrCommand::FetchCommitArtifacts { sha } => {
                    match &mut self.data {
                        Poll::Ready(Ok(pr_data)) => {
                            for commit in &mut pr_data.commits {
                                if commit.commit.sha == sha {
                                    commit.artifacts = Some(Poll::Pending);
                                    break;
                                }
                            }
                        }
                        _ => {}
                    }

                    let client = RepoClient::new(self.client.clone(), self.link.repo.clone());
                    self.inbox.spawn(move |tx| async move {
                        let artifacts = fetch_commit_artifacts(&client, &sha).await;
                        let _ = tx.send(GithubPrCommand::FetchedCommitArtifacts { sha, artifacts });
                    });
                }
            }
        }
    }
}

// Get all commits for a PR to get a complete picture
async fn get_pr_commits(
    repo: &RepoClient,
    pr: PrNumber,
) -> octocrab::Result<Vec<CommitWithArtifacts>> {
    let page = repo.pulls().pr_commits(pr).send().await?;
    let commits: Vec<_> = page
        .into_stream(repo)
        .map_ok(|commit| async move {
            let status = repo
                .get(
                    format!(
                        "/repos/{}/{}/commits/{}/status",
                        repo.repo().owner,
                        repo.repo().repo,
                        commit.sha
                    ),
                    None::<&()>,
                )
                .await?;
            Ok(CommitWithArtifacts {
                commit,
                status,
                artifacts: None,
            })
        })
        .try_buffered(10)
        .try_collect()
        .await?;

    Ok(commits)
}

async fn get_all_pr_artifacts(
    repo: &RepoClient,
    pr_number: PrNumber,
) -> octocrab::Result<PrWithCommits> {
    let pr = repo.pulls().get(pr_number).await?;
    let commits = get_pr_commits(repo, pr_number).await?;

    Ok(PrWithCommits { pr, commits })
}

#[derive(serde::Serialize)]
struct ListWorkflowRunsHeadSha {
    head_sha: String,
}

async fn fetch_commit_artifacts(
    repo: &RepoClient,
    sha: &str,
) -> octocrab::Result<Vec<ArtifactData>> {
    
    let workflow_runs: Page<Run> = repo
        .get(
            // Unfortunately octocrab is missing the head_sha filter
            format!(
                "/repos/{}/{}/actions/runs",
                repo.repo().owner,
                repo.repo().repo
            ),
            Some(&ListWorkflowRunsHeadSha {
                head_sha: sha.to_string(),
            }),
        )
        .await?;

    let runs: Vec<Run> = workflow_runs.into_stream(repo).try_collect().await?;

    let artifacts = FuturesUnordered::from_iter(runs.into_iter().map(|run| async move {
        let artifacts_page = repo
            .actions()
            .list_workflow_run_artifacts(&repo.repo().owner, &repo.repo().repo, run.id)
            .send()
            .await?
            .value
            .expect("No etag was provided, so we should have a value");

        let stream = artifacts_page
            .into_stream(repo)
            .map_ok(move |artifact| ArtifactData {
                artifact,
                run: run.clone(),
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

    ui.group(|ui| {
        ui.heading(format!("GitHub PR #{}", pr.link.pr_number));

        match &pr.data {
            Poll::Ready(Ok(data)) => {
                let details = &data.pr;

                if let Some(title) = &details.title {
                    ui.label(title);
                }

                ui.separator();

                if let Some(html_url) = &details.html_url {
                    if ui.button("Compare PR Branches").clicked() {
                        selected_source =
                            Some(DiffSource::Pr(html_url.to_string().parse().unwrap()));
                    }
                }

                ui.separator();
                ui.heading("Recent Commits & Artifacts");

                ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Truncate);

                list_item_scope(ui, "pr_info", |ui| {
                    for commit in data.commits.iter().rev() {
                        let item = ui.list_item();

                        let button = match &commit.status.state {
                            StatusState::Failure | StatusState::Error => Button::image(
                                icons::ERROR.as_image().tint(ui.tokens().alert_error.icon),
                            )
                            .boxed_local(),
                            StatusState::Pending => Spinner::new().boxed_local(),
                            StatusState::Success => Button::image(
                                icons::SUCCESS
                                    .as_image()
                                    .tint(ui.tokens().alert_success.icon),
                            )
                            .boxed_local(),
                            _ => Button::image(icons::HELP.as_image()).boxed_local(),
                        };

                        let button = button.on_menu(|ui| {
                            ui.set_min_width(250.0);
                            match &commit.artifacts {
                                None => {
                                    pr.inbox
                                        .sender()
                                        .send(GithubPrCommand::FetchCommitArtifacts {
                                            sha: commit.commit.sha.clone(),
                                        })
                                        .ok();
                                }
                                Some(Poll::Pending) => {
                                    ui.spinner();
                                }
                                Some(Poll::Ready(Err(error))) => {
                                    ui.colored_label(
                                        ui.visuals().error_fg_color,
                                        format!("Error: {}", error),
                                    );
                                }
                                Some(Poll::Ready(Ok(artifacts))) => {
                                    if artifacts.is_empty() {
                                        ui.label("No artifacts found");
                                    } else {
                                        for artifact in artifacts {
                                            // let label = format!(
                                            //     "{} (from run: {})",
                                            //     artifact.artifact.name, artifact.run.name
                                            // );

                                            if ui.button(&artifact.artifact.name).clicked() {
                                                selected_source = Some(DiffSource::GHArtifact {
                                                    repo: pr.link.repo.clone(),
                                                    artifact_id: artifact.artifact.id.to_string(),
                                                });
                                            }
                                        }
                                    }
                                }
                            }
                        });

                        let content = LabelContent::new(&commit.commit.commit.message)
                            .with_button(button)
                            .with_always_show_buttons(true);

                        let response = item.show_hierarchical(ui, content);
                    }
                });
            }
            Poll::Ready(Err(error)) => {
                ui.colored_label(ui.visuals().error_fg_color, format!("Error: {}", error));
            }
            Poll::Pending => {
                ui.spinner();
            }
        }
    });

    if let Some(source) = selected_source {
        state.send(SystemCommand::Open(source));
    }
}
