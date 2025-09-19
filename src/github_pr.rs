use std::sync::mpsc;
use crate::DiffSource;
use eframe::egui::Context;
use std::future::Future;
use serde::Deserialize;

// Import octocrab models
use octocrab::models::{
    pulls::PullRequest,
    repos::RepoCommit,
    workflows::{Run, WorkflowListArtifact},
};

// Response wrappers for paginated GitHub API responses
#[derive(Debug, Clone, Deserialize)]
pub struct WorkflowRunsResponse {
    pub workflow_runs: Vec<Run>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ArtifactsResponse {
    pub artifacts: Vec<WorkflowListArtifact>,
}

/// Cross-platform async spawn helper
#[cfg(not(target_arch = "wasm32"))]
pub fn spawn_async<F>(future: F)
where
    F: Future<Output = ()> + Send + 'static,
{
    tokio::task::spawn(future);
}

#[cfg(target_arch = "wasm32")]
pub fn spawn_async<F>(future: F)
where
    F: Future<Output = ()> + 'static,
{
    wasm_bindgen_futures::spawn_local(future);
}

/// Async HTTP request helper with optional authentication
pub async fn fetch_json<T>(url: String, auth_token: Option<&str>) -> Result<T, String>
where
    T: for<'de> Deserialize<'de>,
{
    let mut request = ehttp::Request::get(url);

    // Add Authorization header if token is provided
    if let Some(token) = auth_token {
        request.headers.insert("Authorization".to_string(), format!("Bearer {}", token));
        request.headers.insert("User-Agent".to_string(), "kitdiff/1.0".to_string());
    }

    match ehttp::fetch_async(request).await {
        Ok(response) if response.ok => {
            serde_json::from_slice(&response.bytes)
                .map_err(|e| format!("JSON parse error: {}", e))
        }
        Ok(response) => {
            Err(format!("HTTP {}: {}", response.status, response.status_text))
        }
        Err(e) => {
            Err(format!("Network error: {}", e))
        }
    }
}

pub fn parse_github_pr_url(url: &str) -> Result<(String, String, u32), String> {
    // Parse URLs like: https://github.com/rerun-io/rerun/pull/11253
    if !url.starts_with("https://github.com/") {
        return Err("URL must start with https://github.com/".to_string());
    }

    let path = url.strip_prefix("https://github.com/")
        .ok_or("Invalid GitHub URL")?;

    let parts: Vec<&str> = path.split('/').collect();
    if parts.len() != 4 || parts[2] != "pull" {
        return Err("Expected format: https://github.com/owner/repo/pull/123".to_string());
    }

    let user = parts[0].to_string();
    let repo = parts[1].to_string();
    let pr_number = parts[3].parse::<u32>()
        .map_err(|_| "Invalid PR number")?;

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
    pub pr_details: Option<PrDetails>,
    pub commits: Vec<CommitInfo>,
    pub loading: bool,
    pub error_message: Option<String>,
    pub auth_token: Option<String>,
    rx: mpsc::Receiver<GithubPrMessage>,
    tx: mpsc::Sender<GithubPrMessage>,
}

impl GithubPr {
    pub fn new(user: String, repo: String, pr_number: u32, ctx: Context, auth_token: Option<String>) -> Self {
        let (tx, rx) = mpsc::channel();

        let pr = Self {
            user: user.clone(),
            repo: repo.clone(),
            pr_number,
            pr_details: None,
            commits: Vec::new(),
            loading: true,
            error_message: None,
            auth_token: auth_token.clone(),
            rx,
            tx,
        };

        // Start fetching PR details
        let pr_clone = pr.clone_channels();
        spawn_async(fetch_pr_data_async(user.clone(), repo.clone(), pr_number, pr_clone, ctx, auth_token));

        pr
    }

    fn clone_channels(&self) -> mpsc::Sender<GithubPrMessage> {
        self.tx.clone()
    }


    fn update(&mut self) {
        // Process messages from background thread
        while let Ok(message) = self.rx.try_recv() {
            match message {
                GithubPrMessage::FetchedDetails(details) => {
                    self.pr_details = Some(details);
                    self.loading = false;
                }
                GithubPrMessage::FetchedCommits(commits) => {
                    self.commits = commits;
                }
                GithubPrMessage::Error(error) => {
                    self.error_message = Some(error);
                    self.loading = false;
                }
            }
        }
    }

    /// Display details about the PR and allow selecting an artifact to load
    pub fn ui(&mut self, ui: &mut eframe::egui::Ui) -> Option<DiffSource> {
        self.update();

        let mut selected_source = None;

        ui.group(|ui| {
            ui.heading(format!("GitHub PR #{}", self.pr_number));

            if self.loading {
                ui.label("Loading PR details...");
                ui.spinner();
                return;
            }

            if let Some(error) = &self.error_message {
                ui.colored_label(ui.visuals().error_fg_color, format!("Error: {}", error));
                return;
            }

            if let Some(details) = &self.pr_details {
                ui.label(format!("Title: {}", details.title));
                ui.label(format!("State: {}", details.state));
                ui.label(format!("Base: {} → Head: {}", details.base_ref, details.head_ref));

                ui.separator();

                if ui.button("Compare PR Branches").clicked() {
                    selected_source = Some(DiffSource::Pr(details.html_url.clone()));
                }

                ui.separator();
                ui.heading("Recent Commits & Artifacts");

                if self.commits.is_empty() {
                    ui.label("No commits found for this PR.");
                } else {
                    for commit in &self.commits {
                        ui.group(|ui| {
                            ui.horizontal(|ui| {
                                ui.label("📝");
                                ui.vertical(|ui| {
                                    ui.strong(&commit.message);
                                    ui.label(format!("by {} on {}", commit.author, commit.date));
                                    ui.monospace(format!("SHA: {}", &commit.sha[..8]));
                                });
                            });

                            if !commit.artifacts.is_empty() {
                                ui.separator();
                                ui.label(format!("Artifacts ({}):", commit.artifacts.len()));
                                for artifact in &commit.artifacts {
                                    ui.horizontal(|ui| {
                                        ui.label("📦");
                                        ui.label(&artifact.name);
                                        ui.label(format!("({} KB)", artifact.size_in_bytes / 1024));
                                        if ui.button("Load").clicked() {
                                            selected_source = Some(DiffSource::GHArtifact {
                                                owner: self.user.clone(),
                                                repo: self.repo.clone(),
                                                artifact_id: artifact.id.to_string(),
                                            });
                                        }
                                    });
                                }
                            } else {
                                ui.label("No artifacts available for this commit");
                            }
                        });
                        ui.add_space(5.0);
                    }
                }
            }
        });

        selected_source
    }
}

/// Main async function to fetch all PR data
async fn fetch_pr_data_async(
    user: String,
    repo: String,
    pr_number: u32,
    tx: mpsc::Sender<GithubPrMessage>,
    ctx: Context,
    auth_token: Option<String>,
) {
    // First fetch PR details
    let pr_url = format!("https://api.github.com/repos/{}/{}/pulls/{}", user, repo, pr_number);

    match fetch_json::<PullRequest>(pr_url, auth_token.as_deref()).await {
        Ok(pr_response) => {
            let details = PrDetails {
                title: pr_response.title.unwrap_or_else(|| "Unknown".to_string()),
                head_ref: pr_response.head.ref_field,
                base_ref: pr_response.base.ref_field,
                state: pr_response.state.map(|s| format!("{:?}", s)).unwrap_or_else(|| "Unknown".to_string()),
                html_url: pr_response.html_url.map(|u| u.to_string()).unwrap_or_default(),
            };
            let _ = tx.send(GithubPrMessage::FetchedDetails(details));

            // Now fetch commits
            match fetch_commits_async(&user, &repo, pr_number, auth_token.as_deref()).await {
                Ok(mut commits) => {
                    println!("Found {} commits for PR {}", commits.len(), pr_number);

                    // Fetch artifacts for each commit
                    for commit in &mut commits {
                        println!("Processing commit: {} by {}", &commit.sha[..8], commit.author);
                        match fetch_artifacts_for_commit(&user, &repo, &commit.sha, auth_token.as_deref()).await {
                            Ok(artifacts) => {
                                commit.artifacts = artifacts;
                                println!("  Assigned {} artifacts to commit {}", commit.artifacts.len(), &commit.sha[..8]);
                            }
                            Err(e) => {
                                println!("  Error fetching artifacts for commit {}: {}", &commit.sha[..8], e);
                            }
                        }
                    }
                    let _ = tx.send(GithubPrMessage::FetchedCommits(commits));
                }
                Err(e) => {
                    let _ = tx.send(GithubPrMessage::Error(format!("Failed to fetch commits: {}", e)));
                }
            }
        }
        Err(e) => {
            let _ = tx.send(GithubPrMessage::Error(format!("Failed to fetch PR details: {}", e)));
        }
    }

    ctx.request_repaint();
}

/// Fetch commits for a PR
async fn fetch_commits_async(user: &str, repo: &str, pr_number: u32, auth_token: Option<&str>) -> Result<Vec<CommitInfo>, String> {
    let commits_url = format!("https://api.github.com/repos/{}/{}/pulls/{}/commits", user, repo, pr_number);

    let commits_response = fetch_json::<Vec<RepoCommit>>(commits_url, auth_token).await?;
    let mut commits = Vec::new();

    // Take the last 10 commits (most recent)
    for commit in commits_response.iter().rev().take(10) {
        let message = commit.commit.message
            .lines()
            .next()
            .unwrap_or("No message")
            .to_string();

        // Format the date to be more readable
        let formatted_date = if let Some(author) = &commit.commit.author {
            if let Some(date) = &author.date {
                if let Ok(parsed_date) = chrono::DateTime::parse_from_rfc3339(&date.to_rfc3339()) {
                    parsed_date.format("%m/%d/%Y %H:%M").to_string()
                } else {
                    date.to_rfc3339()
                }
            } else {
                "Unknown date".to_string()
            }
        } else {
            "Unknown date".to_string()
        };

        let author_name = commit.commit.author
            .as_ref()
            .map(|a| a.name.clone())
            .unwrap_or_else(|| "Unknown".to_string());

        commits.push(CommitInfo {
            sha: commit.sha.clone(),
            message,
            author: author_name,
            date: formatted_date,
            artifacts: Vec::new(), // Will be populated later
        });
    }

    Ok(commits)
}

/// Fetch artifacts for a specific commit
async fn fetch_artifacts_for_commit(user: &str, repo: &str, commit_sha: &str, auth_token: Option<&str>) -> Result<Vec<GithubArtifact>, String> {
    let runs_url = format!("https://api.github.com/repos/{}/{}/actions/runs?head_sha={}", user, repo, commit_sha);

    println!("Fetching runs for commit {}: {}", &commit_sha[..8], runs_url);

    let runs_response = fetch_json::<WorkflowRunsResponse>(runs_url, auth_token).await?;
    let mut all_artifacts = Vec::new();

    println!("Found {} workflow runs for commit {}", runs_response.workflow_runs.len(), &commit_sha[..8]);

    // Check all workflow runs, not just successful ones
    for run in &runs_response.workflow_runs {
        println!("  Run: {} (ID: {}) - Status: {}, Conclusion: {}",
                run.name,
                run.id.into_inner(),
                run.status,
                run.conclusion.as_deref().unwrap_or("unknown"));

        // Try to fetch artifacts for completed runs (regardless of success/failure)
        if run.status == "completed" {
            match fetch_run_artifacts_async(user, repo, run.id.into_inner(), auth_token).await {
                Ok(artifacts) => {
                    println!("    Found {} artifacts for run {}", artifacts.len(), run.id.into_inner());
                    all_artifacts.extend(artifacts);
                }
                Err(e) => {
                    println!("    Failed to fetch artifacts for run {}: {}", run.id.into_inner(), e);
                }
            }
        }
    }

    println!("Total artifacts found for commit {}: {}", &commit_sha[..8], all_artifacts.len());
    Ok(all_artifacts)
}

/// Fetch artifacts for a specific workflow run
async fn fetch_run_artifacts_async(user: &str, repo: &str, run_id: u64, auth_token: Option<&str>) -> Result<Vec<GithubArtifact>, String> {
    let artifacts_url = format!("https://api.github.com/repos/{}/{}/actions/runs/{}/artifacts", user, repo, run_id);

    println!("    Fetching artifacts: {}", artifacts_url);

    let artifacts_response = fetch_json::<ArtifactsResponse>(artifacts_url, auth_token).await?;
    let mut artifacts = Vec::new();

    println!("    Raw artifacts array length: {}", artifacts_response.artifacts.len());

    for artifact_data in &artifacts_response.artifacts {
        println!("      Artifact: {} (ID: {}, Size: {} bytes, Expired: {})",
                artifact_data.name,
                artifact_data.id,
                artifact_data.size_in_bytes,
                artifact_data.expired);

        // Include even expired artifacts for now, so user can see what was available
        artifacts.push(GithubArtifact {
            id: artifact_data.id.into_inner(),
            name: artifact_data.name.clone(),
            size_in_bytes: artifact_data.size_in_bytes as u64,
            download_url: artifact_data.archive_download_url.to_string(),
        });
    }

    Ok(artifacts)
}
