use std::sync::mpsc;
use crate::DiffSource;
use eframe::egui::Context;
use std::future::Future;

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

/// Async HTTP request helper
pub async fn fetch_json(url: String) -> Result<serde_json::Value, String> {
    let request = ehttp::Request::get(url);

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
    rx: mpsc::Receiver<GithubPrMessage>,
    tx: mpsc::Sender<GithubPrMessage>,
}

impl GithubPr {
    pub fn new(user: String, repo: String, pr_number: u32, ctx: Context) -> Self {
        let (tx, rx) = mpsc::channel();

        let pr = Self {
            user: user.clone(),
            repo: repo.clone(),
            pr_number,
            pr_details: None,
            commits: Vec::new(),
            loading: true,
            error_message: None,
            rx,
            tx,
        };

        // Start fetching PR details
        let pr_clone = pr.clone_channels();
        spawn_async(fetch_pr_data_async(user.clone(), repo.clone(), pr_number, pr_clone, ctx));

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
) {
    // First fetch PR details
    let pr_url = format!("https://api.github.com/repos/{}/{}/pulls/{}", user, repo, pr_number);

    match fetch_json(pr_url).await {
        Ok(json) => {
            let details = PrDetails {
                title: json["title"].as_str().unwrap_or("Unknown").to_string(),
                head_ref: json["head"]["ref"].as_str().unwrap_or("unknown").to_string(),
                base_ref: json["base"]["ref"].as_str().unwrap_or("unknown").to_string(),
                state: json["state"].as_str().unwrap_or("unknown").to_string(),
                html_url: json["html_url"].as_str().unwrap_or("").to_string(),
            };
            let _ = tx.send(GithubPrMessage::FetchedDetails(details));

            // Now fetch commits
            match fetch_commits_async(&user, &repo, pr_number).await {
                Ok(mut commits) => {
                    println!("Found {} commits for PR {}", commits.len(), pr_number);

                    // Fetch artifacts for each commit
                    for commit in &mut commits {
                        println!("Processing commit: {} by {}", &commit.sha[..8], commit.author);
                        match fetch_artifacts_for_commit(&user, &repo, &commit.sha).await {
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
async fn fetch_commits_async(user: &str, repo: &str, pr_number: u32) -> Result<Vec<CommitInfo>, String> {
    let commits_url = format!("https://api.github.com/repos/{}/{}/pulls/{}/commits", user, repo, pr_number);

    let commits_json = fetch_json(commits_url).await?;
    let mut commits = Vec::new();

    if let Some(commits_array) = commits_json.as_array() {
        // Take the last 10 commits (most recent)
        for commit in commits_array.iter().rev().take(10) {
            let sha = commit["sha"].as_str().unwrap_or("").to_string();
            let message = commit["commit"]["message"]
                .as_str()
                .unwrap_or("No message")
                .lines()
                .next()
                .unwrap_or("No message")
                .to_string();
            let author = commit["commit"]["author"]["name"]
                .as_str()
                .unwrap_or("Unknown")
                .to_string();
            let date = commit["commit"]["author"]["date"]
                .as_str()
                .unwrap_or("")
                .to_string();

            // Format the date to be more readable
            let formatted_date = if let Ok(parsed_date) = chrono::DateTime::parse_from_rfc3339(&date) {
                parsed_date.format("%m/%d/%Y %H:%M").to_string()
            } else {
                date
            };

            commits.push(CommitInfo {
                sha: sha.clone(),
                message,
                author,
                date: formatted_date,
                artifacts: Vec::new(), // Will be populated later
            });
        }
    }

    Ok(commits)
}

/// Fetch artifacts for a specific commit
async fn fetch_artifacts_for_commit(user: &str, repo: &str, commit_sha: &str) -> Result<Vec<GithubArtifact>, String> {
    let runs_url = format!("https://api.github.com/repos/{}/{}/actions/runs?head_sha={}", user, repo, commit_sha);

    println!("Fetching runs for commit {}: {}", &commit_sha[..8], runs_url);

    let runs_json = fetch_json(runs_url).await?;
    let mut all_artifacts = Vec::new();

    if let Some(workflow_runs) = runs_json["workflow_runs"].as_array() {
        println!("Found {} workflow runs for commit {}", workflow_runs.len(), &commit_sha[..8]);

        // Check all workflow runs, not just successful ones
        for run in workflow_runs.iter().take(5) {
            let run_id = run["id"].as_u64().unwrap_or(0);
            let run_status = run["status"].as_str().unwrap_or("unknown");
            let run_conclusion = run["conclusion"].as_str().unwrap_or("unknown");
            let run_name = run["name"].as_str().unwrap_or("unknown");

            println!("  Run: {} (ID: {}) - Status: {}, Conclusion: {}", run_name, run_id, run_status, run_conclusion);

            // Try to fetch artifacts for completed runs (regardless of success/failure)
            if run_status == "completed" {
                match fetch_run_artifacts_async(user, repo, run_id).await {
                    Ok(artifacts) => {
                        println!("    Found {} artifacts for run {}", artifacts.len(), run_id);
                        all_artifacts.extend(artifacts);
                    }
                    Err(e) => {
                        println!("    Failed to fetch artifacts for run {}: {}", run_id, e);
                    }
                }
            }
        }
    } else {
        println!("No workflow_runs array found in response");
    }

    println!("Total artifacts found for commit {}: {}", &commit_sha[..8], all_artifacts.len());
    Ok(all_artifacts)
}

/// Fetch artifacts for a specific workflow run
async fn fetch_run_artifacts_async(user: &str, repo: &str, run_id: u64) -> Result<Vec<GithubArtifact>, String> {
    let artifacts_url = format!("https://api.github.com/repos/{}/{}/actions/runs/{}/artifacts", user, repo, run_id);

    println!("    Fetching artifacts: {}", artifacts_url);

    let json = fetch_json(artifacts_url).await?;
    let mut artifacts = Vec::new();

    if let Some(artifacts_array) = json["artifacts"].as_array() {
        println!("    Raw artifacts array length: {}", artifacts_array.len());
        for artifact in artifacts_array {
            let id = artifact["id"].as_u64().unwrap_or(0);
            let name = artifact["name"].as_str().unwrap_or("Unknown").to_string();
            let size_in_bytes = artifact["size_in_bytes"].as_u64().unwrap_or(0);
            let download_url = artifact["archive_download_url"].as_str().unwrap_or("").to_string();
            let expired = artifact["expired"].as_bool().unwrap_or(false);

            println!("      Artifact: {} (ID: {}, Size: {} bytes, Expired: {})", name, id, size_in_bytes, expired);

            // Include even expired artifacts for now, so user can see what was available
            artifacts.push(GithubArtifact {
                id,
                name,
                size_in_bytes,
                download_url,
            });
        }
    } else {
        println!("    No artifacts array found in response");
    }

    Ok(artifacts)
}
