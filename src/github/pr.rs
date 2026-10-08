use crate::DiffSource;
use crate::config::{Config, TemplateVars, render_url_template};
use crate::github::octokit::RepoClient;
use crate::loaders::DataReference;
use crate::loaders::archive_loader::run_discovery;
use crate::state::{AppStateRef, SystemCommand};
use eframe::egui;
use eframe::egui::{Context, Popup, ScrollArea, Spinner};
use egui_inbox::UiInbox;
use futures::TryStreamExt as _;
use futures::stream::FuturesUnordered;
use graphql_client::GraphQLQuery;
use octocrab::Octocrab;
use octocrab::models::{RunId, workflows::WorkflowListArtifact};
use re_ui::egui_ext::boxed_widget::BoxedWidgetLocalExt as _;
use re_ui::list_item::{LabelContent, ListItemContentButtonsExt as _, list_item_scope};
use re_ui::{SectionCollapsingHeader, UiExt as _, icons};
use std::collections::HashMap;
use std::collections::hash_map::Entry;
use std::task::Poll;
pub type GitObjectID = String;
pub type DateTime = String;
#[expect(clippy::upper_case_acronyms)]
pub type URI = String;

#[derive(GraphQLQuery, Debug)]
#[graphql(
    schema_path = "github.graphql",
    query_path = "src/github/pr.graphql",
    response_derives = "Debug, Clone"
)]
pub struct PrDetailsQuery;
use crate::github::model::{
    CommitArchiveLink, GithubArtifactLink, GithubPrLink, GithubRepoLink, PrNumber,
};
use crate::github::update_snapshots::{SnapshotOrigin, UpdateSnapshotsWorkflow};
use anyhow::{Context as _, Error, Result, anyhow};
use eframe::emath::RectAlign;
use re_ui::menu::menu_style;

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
    FetchedData(Result<Box<PrWithCommits>>),
    FetchedCommitArtifacts {
        sha: String,
        artifacts: Result<Vec<ArtifactData>, Error>,
    },
    FetchCommitArtifacts {
        sha: String,
    },
    FetchedCommitArchives {
        sha: String,
        archives: Vec<ArchiveProbe>,
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

    /// The archives from [`crate::config::Artifact::url_template`], by commit.
    commit_archives: HashMap<String, Poll<Result<Vec<ArchiveProbe>>>>,

    /// The `kitdiff.toml` at the head of the PR.
    config: Result<Config>,
}

impl PrWithCommits {
    /// Gives the workflow that commits the snapshots, if `kitdiff.toml` names one.
    fn update_snapshots_workflow(
        &self,
        repo: &GithubRepoLink,
        origin: SnapshotOrigin,
    ) -> Option<UpdateSnapshotsWorkflow> {
        let config = self.config.as_ref().ok()?;
        let workflow = config.github.update_snapshot_workflow_name.clone()?;
        Some(UpdateSnapshotsWorkflow {
            repo: repo.clone(),
            workflow,
            branch: self.head_branch.clone(),
            origin,
        })
    }

    /// Gives the archives from [`crate::config::Artifact::url_template`] for a commit:
    /// one for each platform, or one if there are no platforms.
    /// Gives `None` if the config has no URL template.
    fn commit_archive_candidates(
        &self,
        link: &GithubPrLink,
        sha: &str,
    ) -> Option<Result<Vec<CommitArchiveLink>>> {
        let config = self.config.as_ref().ok()?;
        let template = config.artifact.url_template.as_deref()?;

        let platforms: Vec<Option<&str>> = if config.artifact.platforms.is_empty() {
            vec![None]
        } else {
            config
                .artifact
                .platforms
                .iter()
                .map(|p| Some(p.as_str()))
                .collect()
        };

        Some(
            platforms
                .into_iter()
                .map(|platform| {
                    let vars = TemplateVars {
                        owner: &link.repo.owner,
                        repo: &link.repo.repo,
                        commit: sha,
                        branch: &self.head_branch,
                        pr: link.pr_number,
                        platform,
                    };
                    let url = render_url_template(template, &vars)?;
                    Ok(CommitArchiveLink {
                        url,
                        commit: sha.to_owned(),
                        platform: platform.map(str::to_owned),
                        update_snapshots: self.update_snapshots_workflow(
                            &link.repo,
                            SnapshotOrigin::Commit(sha.to_owned()),
                        ),
                    })
                })
                .collect(),
        )
    }
}

#[derive(Debug)]
pub struct ArtifactData {
    data: WorkflowListArtifact,
    run_id: RunId,
}

#[derive(Debug, Clone, Copy, PartialEq)]
/// The combined status of all checks of a commit, as GitHub shows it.
enum CommitState {
    NoChecks,
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
    /// `config_override` replaces the `kitdiff.toml` of the repository.
    pub fn new(link: GithubPrLink, client: Octocrab, config_override: Option<Config>) -> Self {
        let mut inbox = UiInbox::new();

        {
            let client = RepoClient::new(client.clone(), link.repo.clone());
            inbox.spawn(|tx| async move {
                let details = get_pr_commits(&client, link.pr_number, config_override)
                    .await
                    .map(Box::new);
                tx.send(GithubPrCommand::FetchedData(details)).ok();
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
                    self.data = Poll::Ready(data.map(|data| *data));
                }
                GithubPrCommand::FetchedCommitArtifacts { sha, artifacts } => {
                    if let Poll::Ready(Ok(pr_data)) = &mut self.data {
                        pr_data.artifacts.insert(sha, Poll::Ready(artifacts));
                    }
                }
                GithubPrCommand::FetchedCommitArchives { sha, archives } => {
                    if let Poll::Ready(Ok(pr_data)) = &mut self.data {
                        pr_data
                            .commit_archives
                            .insert(sha, Poll::Ready(Ok(archives)));
                    }
                }
                GithubPrCommand::FetchCommitArtifacts { sha } => {
                    if let Poll::Ready(Ok(pr_data)) = &mut self.data {
                        match pr_data.commit_archive_candidates(&self.link, &sha) {
                            None => {}
                            Some(Err(err)) => {
                                pr_data
                                    .commit_archives
                                    .insert(sha.clone(), Poll::Ready(Err(err)));
                            }
                            Some(Ok(candidates)) => {
                                pr_data
                                    .commit_archives
                                    .entry(sha.clone())
                                    .or_insert(Poll::Pending);
                                let sha = sha.clone();
                                self.inbox.spawn(move |tx| async move {
                                    let archives = probe_archives(candidates).await;
                                    tx.send(GithubPrCommand::FetchedCommitArchives {
                                        sha,
                                        archives,
                                    })
                                    .ok();
                                });
                            }
                        }

                        // Without a pattern, kitdiff lists no artifacts, so it need not fetch them.
                        let Some(github) = pr_data
                            .config
                            .as_ref()
                            .ok()
                            .map(|config| config.github.clone())
                            .filter(|github| github.artifact_pattern.is_some())
                        else {
                            continue;
                        };

                        match pr_data.artifacts.entry(sha.clone()) {
                            Entry::Occupied(_) => {}
                            Entry::Vacant(entry) => {
                                entry.insert(Poll::Pending);
                            }
                        }

                        let workflow_run_ids = pr_data
                            .commits
                            .iter()
                            .find(|c| c.sha == sha)
                            .map(|c| c.workflow_run_ids.clone())
                            .unwrap_or_default();

                        let client = RepoClient::new(self.client.clone(), self.link.repo.clone());
                        self.inbox.spawn(move |tx| async move {
                            let artifacts = fetch_commit_artifacts(&client, workflow_run_ids)
                                .await
                                .map(|artifacts| {
                                    artifacts
                                        .into_iter()
                                        .filter(|artifact| {
                                            github.lists_artifact(&artifact.data.name)
                                        })
                                        .collect()
                                });
                            tx.send(GithubPrCommand::FetchedCommitArtifacts { sha, artifacts })
                                .ok();
                        });
                    }
                }
            }
        }
    }
}

async fn get_pr_commits(
    repo: &RepoClient,
    pr: PrNumber,
    config_override: Option<Config>,
) -> Result<PrWithCommits> {
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

    let config = match config_override {
        Some(config) => Ok(config),
        None => Config::fetch(repo, &response.head_ref_oid).await,
    };
    if let Err(err) = &config {
        log::warn!("{err:#}");
    }

    let mut data = PrWithCommits {
        title: response.title,
        head_branch: response.head_ref_name,
        base_branch: response.base_ref_name,
        commits: Vec::new(),
        artifacts: HashMap::new(),
        commit_archives: HashMap::new(),
        config,
    };

    for commit in response
        .commits
        .nodes
        .ok_or_else(|| anyhow!("No commits found"))?
        .into_iter()
        .flatten()
    {
        let commit = commit.commit;
        let sha = commit.oid;
        let message = commit.message_headline;

        let status = match commit.status_check_rollup.map(|rollup| rollup.state) {
            None => CommitState::NoChecks,
            Some(pr_details_query::StatusState::SUCCESS) => CommitState::Success,
            Some(
                pr_details_query::StatusState::PENDING | pr_details_query::StatusState::EXPECTED,
            ) => CommitState::Pending,
            Some(
                pr_details_query::StatusState::FAILURE
                | pr_details_query::StatusState::ERROR
                | pr_details_query::StatusState::Other(_),
            ) => CommitState::Failure,
        };

        // The run of a workflow that ran again replaces the earlier run, so keep the last suite of each workflow.
        let mut last_run_per_workflow = HashMap::new();
        if let Some(suites) = commit.check_suites
            && let Some(nodes) = suites.nodes
        {
            for run in nodes
                .into_iter()
                .flatten()
                .filter_map(|node| node.workflow_run)
            {
                last_run_per_workflow.insert(run.workflow.id, run.database_id);
            }
        }
        let workflow_run_ids = last_run_per_workflow
            .into_values()
            .flatten()
            .map(|id| id as u64)
            .collect();

        data.commits.push(CommitData {
            message,
            sha,
            status,
            workflow_run_ids,
        });
    }

    Ok(data)
}

#[derive(Debug)]
pub struct ArchiveProbe {
    archive: CommitArchiveLink,
    status: ArchiveStatus,
}

#[derive(Debug)]
enum ArchiveStatus {
    /// The archive has snapshots to show.
    Available,

    /// CI has not published the archive (yet).
    NotPublished,

    /// The archive exists, but has no changed snapshots.
    NoChanges,

    Error(anyhow::Error),
}

/// kitdiff downloads archives smaller than this to see if they have any snapshots.
/// An empty `.tar.gz` is about 50 bytes, and one snapshot makes it much bigger.
const SMALL_ARCHIVE_BYTES: u64 = 4096;

/// Checks each archive on its own, so that one failure does not hide the others.
async fn probe_archives(candidates: Vec<CommitArchiveLink>) -> Vec<ArchiveProbe> {
    // Not the octocrab client: that would send the user's GitHub token to another host.
    let client = reqwest::Client::new();
    let probes = candidates.into_iter().map(|archive| {
        let client = &client;
        async move {
            let status = probe_archive(client, &archive.url)
                .await
                .unwrap_or_else(ArchiveStatus::Error);
            ArchiveProbe { archive, status }
        }
    });
    futures::future::join_all(probes).await
}

async fn probe_archive(client: &reqwest::Client, url: &str) -> Result<ArchiveStatus> {
    let response = client
        .head(url)
        .send()
        .await
        .with_context(|| format!("Failed to check {url}"))?;

    let status = response.status();
    if status == reqwest::StatusCode::NOT_FOUND || status == reqwest::StatusCode::FORBIDDEN {
        // Cloud storage often gives 403 instead of 404 for a missing file.
        return Ok(ArchiveStatus::NotPublished);
    }
    if !status.is_success() {
        return Err(anyhow!("Failed to check {url}: HTTP {status}"));
    }

    // Not `response.content_length()`: for a HEAD request, that is the size of the empty body.
    let length = response
        .headers()
        .get(reqwest::header::CONTENT_LENGTH)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<u64>().ok());
    if length.is_some_and(|length| length < SMALL_ARCHIVE_BYTES) {
        let snapshots = run_discovery(DataReference::Url(url.to_owned()))
            .await
            .with_context(|| format!("Failed to read {url}"))?;
        if snapshots.is_empty() {
            return Ok(ArchiveStatus::NoChanges);
        }
    }

    Ok(ArchiveStatus::Available)
}

async fn fetch_commit_artifacts(repo: &RepoClient, run_ids: Vec<u64>) -> Result<Vec<ArtifactData>> {
    let artifacts = run_ids
        .into_iter()
        .map(|run| async move {
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
        })
        .collect::<FuturesUnordered<_>>()
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
                if state.config_override.is_some() {
                    ui.weak("Using the --config file, not the repository's kitdiff.toml");
                }
                if let Err(err) = &data.config {
                    ui.colored_label(ui.visuals().error_fg_color, format!("{err:#}"));
                }
                // Not `ui.set_max_height`: after the labels above, it moves the cursor back on top of them.
                ScrollArea::vertical().max_height(100.0).show(ui, |ui| {
                    for commit in data.commits.iter().rev() {
                        let item = ui.list_item();

                        let button = match &commit.status {
                            CommitState::NoChecks => None,
                            CommitState::Failure => Some(
                                icons::ERROR
                                    .as_image()
                                    .tint(ui.tokens().alert_error.icon)
                                    .boxed_local(),
                            ),
                            CommitState::Pending => Some(Spinner::new().boxed_local()),
                            CommitState::Success => Some(
                                icons::SUCCESS
                                    .as_image()
                                    .tint(ui.tokens().alert_success.icon)
                                    .boxed_local(),
                            ),
                        };

                        let mut content = LabelContent::new(&commit.message);
                        if let Some(button) = button {
                            content = content.with_button(button).with_always_show_buttons(true);
                        }

                        let response = item.show_hierarchical(ui, content);
                        if response.clicked() {
                            pr.inbox
                                .sender()
                                .send(GithubPrCommand::FetchCommitArtifacts {
                                    sha: commit.sha.clone(),
                                })
                                .ok();
                        }
                        Popup::menu(&response)
                            .align(RectAlign::BOTTOM_END)
                            .style(menu_style())
                            .show(|ui| {
                                ui.set_min_width(250.0);
                                if let Some(source) =
                                    commit_artifacts_ui(ui, &pr.link, data, &commit.sha)
                                {
                                    selected_source = Some(source);
                                }
                            });
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
        state.send(SystemCommand::OpenFromViewer(source));
    }
}

/// The artifacts of one commit. Gives the artifact the user clicked, if any.
fn commit_artifacts_ui(
    ui: &mut egui::Ui,
    link: &GithubPrLink,
    data: &PrWithCommits,
    sha: &str,
) -> Option<DiffSource> {
    let mut selected_source = None;

    let commit_archives = data.commit_archives.get(sha);
    match commit_archives {
        None => {}
        Some(Poll::Pending) => {
            ui.spinner();
        }
        Some(Poll::Ready(Err(err))) => {
            ui.colored_label(ui.visuals().error_fg_color, format!("Error: {err:#}"));
        }
        Some(Poll::Ready(Ok(probes))) => {
            for probe in probes {
                if archive_probe_ui(ui, probe) {
                    selected_source = Some(DiffSource::CommitArchive(probe.archive.clone()));
                }
            }
        }
    }

    match data.artifacts.get(sha) {
        None => {
            // The click handler in `pr_ui` starts the loading.
        }
        Some(Poll::Pending) => {
            ui.spinner();
        }
        Some(Poll::Ready(Err(error))) => {
            ui.colored_label(ui.visuals().error_fg_color, format!("Error: {error}"));
        }
        Some(Poll::Ready(Ok(artifacts))) => {
            for artifact in artifacts {
                if ui.button(&artifact.data.name).clicked() {
                    selected_source = Some(DiffSource::GHArtifact(GithubArtifactLink {
                        repo: link.repo.clone(),
                        artifact_id: artifact.data.id,
                        name: Some(artifact.data.name.clone()),
                        update_snapshots: data.update_snapshots_workflow(
                            &link.repo,
                            SnapshotOrigin::Run(artifact.run_id),
                        ),
                    }));
                }
            }
        }
    }

    if !shows_something(commit_archives) && !shows_something(data.artifacts.get(sha)) {
        ui.label("No artifacts found");
    }

    selected_source
}

/// Shows one archive. Gives true if the user clicked it.
fn archive_probe_ui(ui: &mut egui::Ui, probe: &ArchiveProbe) -> bool {
    let url = &probe.archive.url;
    let file_name = url.rsplit('/').next().unwrap_or(url);
    let label = probe.archive.platform.as_deref().unwrap_or(file_name);

    match &probe.status {
        ArchiveStatus::Available => ui.button(label).on_hover_text(url).clicked(),
        ArchiveStatus::NotPublished => {
            ui.add_enabled(
                false,
                egui::Button::new(format!("{label}: not published yet")),
            )
            .on_disabled_hover_text(format!(
                "Nothing at {url}. CI publishes it when the tests finish."
            ));
            false
        }
        ArchiveStatus::NoChanges => {
            ui.add_enabled(
                false,
                egui::Button::new(format!("{label}: no changed snapshots")),
            )
            .on_disabled_hover_text(url);
            false
        }
        ArchiveStatus::Error(err) => {
            ui.colored_label(ui.visuals().error_fg_color, format!("{label}: {err:#}"))
                .on_hover_text(url);
            false
        }
    }
}

/// Does the popup show anything for this list: a spinner, an error, or items?
fn shows_something<T>(list: Option<&Poll<Result<Vec<T>>>>) -> bool {
    match list {
        None => false,
        Some(Poll::Ready(Ok(items))) => !items.is_empty(),
        Some(Poll::Pending | Poll::Ready(Err(_))) => true,
    }
}
