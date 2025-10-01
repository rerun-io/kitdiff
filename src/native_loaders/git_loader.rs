use crate::loaders::{LoadSnapshots, sort_snapshots};
use crate::snapshot::{FileReference, Snapshot};
use eframe::egui::load::Bytes;
use eframe::egui::{Context, ImageSource};
use egui_inbox::{UiInbox, UiInboxSender};
use git2::{ObjectType, Repository};
use octocrab::Octocrab;
use std::borrow::Cow;
use std::fmt::Display;
use std::path::{Path, PathBuf};
use std::str;
use std::task::Poll;

enum Command {
    Snapshot(Snapshot),
    Error(GitError),
    Done,
    GitInfo(GitInfo),
}

type Sender = UiInboxSender<Command>;

struct GitInfo {
    current_branch: String,
    default_branch: String,
    repo_name: String,
}

pub struct GitLoader {
    base_path: PathBuf,
    inbox: UiInbox<Command>,
    git_info: Option<GitInfo>,
    snapshots: Vec<Snapshot>,
    state: Poll<Result<(), anyhow::Error>>,
}

impl GitLoader {
    pub fn new(base_path: PathBuf) -> Self {
        let (sender, inbox) = UiInbox::channel();

        {
            let base_path = base_path.clone();
            std::thread::Builder::new()
                .name(format!("Git loader {}", base_path.display()))
                .spawn(move || {
                    let result = run_git_discovery(&sender, &base_path);
                    match result {
                        Ok(()) => {
                            // Signal done
                            sender.send(Command::Done).ok();
                        }
                        Err(e) => {
                            // Send error
                            sender.send(Command::Error(e)).ok();
                        }
                    }
                })
                .expect("Failed to spawn git loader thread");
        }

        Self {
            base_path,
            inbox,
            git_info: None,
            snapshots: Vec::new(),
            state: Poll::Pending,
        }
    }
}

impl LoadSnapshots for GitLoader {
    fn update(&mut self, ctx: &Context) {
        if let Some(new_data) = self.inbox.read(ctx).last() {
            match new_data {
                Command::Snapshot(snapshot) => {
                    self.snapshots.push(snapshot);
                    sort_snapshots(&mut self.snapshots);
                }
                Command::Error(e) => {
                    self.state = Poll::Ready(Err(e.into()));
                }
                Command::GitInfo(info) => {
                    self.git_info = Some(info);
                }
                Command::Done => {
                    self.state = Poll::Ready(Ok(()));
                }
            }
        }
    }

    fn refresh(&mut self, _client: Octocrab) {
        *self = Self::new(self.base_path.clone());
    }

    fn snapshots(&self) -> &[Snapshot] {
        &self.snapshots
    }

    fn state(&self) -> Poll<Result<(), &anyhow::Error>> {
        match &self.state {
            Poll::Ready(Ok(())) => Poll::Ready(Ok(())),
            Poll::Ready(Err(e)) => Poll::Ready(Err(e)),
            Poll::Pending => Poll::Pending,
        }
    }

    fn files_header(&self) -> String {
        match &self.git_info {
            Some(info) => format!(
                "Git: {} ({} ➡ {})",
                info.repo_name, info.current_branch, info.default_branch
            ),
            None => format!("Git: {}", self.base_path.display()),
        }
    }
}

#[derive(Debug)]
pub enum GitError {
    RepoNotFound,
    BranchNotFound,
    FileNotFound,
    Git2(git2::Error),
    IoError(std::io::Error),
    PrUrlParseError,
    NetworkError(String),
}

impl Display for GitError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::RepoNotFound => write!(f, "Git repository not found"),
            Self::BranchNotFound => write!(f, "Default branch not found"),
            Self::FileNotFound => write!(f, "File not found in git tree"),
            Self::Git2(err) => write!(f, "Git error: {err}"),
            Self::IoError(err) => write!(f, "IO error: {err}"),
            Self::PrUrlParseError => write!(f, "Failed to parse PR URL"),
            Self::NetworkError(msg) => write!(f, "Network error: {msg}"),
        }
    }
}

impl std::error::Error for GitError {}

impl From<git2::Error> for GitError {
    fn from(err: git2::Error) -> Self {
        Self::Git2(err)
    }
}

impl From<std::io::Error> for GitError {
    fn from(err: std::io::Error) -> Self {
        Self::IoError(err)
    }
}

fn run_git_discovery(sender: &Sender, base_path: &Path) -> Result<(), GitError> {
    // Open git repository in current directory
    let repo = Repository::open(base_path).map_err(|_err| GitError::RepoNotFound)?;

    // Get current branch
    let head = repo.head()?;
    let current_branch = head.shorthand().unwrap_or("HEAD").to_owned();

    // Find default branch (try main, then master, then first branch)
    let default_branch = find_default_branch(&repo)?;

    // Send git info
    let repo_name = repo
        .path()
        .parent()
        .and_then(|p| p.file_name())
        .and_then(|n| n.to_str())
        .unwrap_or("unknown")
        .to_owned();
    sender
        .send(Command::GitInfo(GitInfo {
            current_branch: current_branch.clone(),
            default_branch: default_branch.clone(),
            repo_name,
        }))
        .ok();

    // Don't compare branch with itself
    if current_branch == default_branch {
        eprintln!("Current branch is the same as default branch ({current_branch})");
        return Ok(());
    }

    // Get the merge base between current branch and default branch
    let head_commit = repo.head()?.peel_to_commit()?;
    let default_commit = repo
        .resolve_reference_from_short_name(&default_branch)?
        .peel_to_commit()?;
    let base_commit = repo.merge_base(head_commit.id(), default_commit.id())?;
    let base_commit = repo.find_commit(base_commit)?;

    // Get GitHub repository info for LFS support
    let github_repo_info = get_github_repo_info(&repo);
    let commit_sha = base_commit.id().to_string();

    // Get current HEAD tree for comparison
    let head_tree = head_commit.tree()?;

    // Use git2 diff to find changed PNG files between merge base and current HEAD
    let diff = repo.diff_tree_to_tree(Some(&base_commit.tree()?), Some(&head_tree), None)?;

    // Process each delta (changed file)
    diff.foreach(
        &mut |delta, _progress| {
            // Check both old and new file paths (handles renames/moves)
            let files_to_check = [delta.old_file().path(), delta.new_file().path()];

            for file_path in files_to_check.into_iter().flatten() {
                // Check if this is a PNG file
                if let Some(extension) = file_path.extension() {
                    if extension == "png" {
                        // Create snapshot for this changed PNG file
                        if let Ok(base_tree) = base_commit.tree() {
                            if let Ok(Some(snapshot)) = create_git_snapshot(
                                &repo,
                                &base_tree,
                                file_path,
                                &github_repo_info,
                                &commit_sha,
                            ) {
                                sender.send(Command::Snapshot(snapshot)).ok();
                            }
                        }
                        break; // Only process once per delta
                    }
                }
            }
            true // Continue iteration
        },
        None,
        None,
        None,
    )?;

    Ok(())
}

fn find_default_branch(repo: &Repository) -> Result<String, GitError> {
    // Try common default branch names
    for branch_name in ["main", "master"] {
        if repo.resolve_reference_from_short_name(branch_name).is_ok() {
            return Ok(branch_name.to_owned());
        }
    }

    // Fall back to first branch found
    let branches = repo.branches(Some(git2::BranchType::Local))?;
    for branch in branches {
        let (branch, _) = branch?;
        if let Some(name) = branch.name()? {
            return Ok(name.to_owned());
        }
    }

    Err(GitError::BranchNotFound)
}

fn create_git_snapshot(
    repo: &Repository,
    default_tree: &git2::Tree<'_>,
    relative_path: &Path,
    github_repo_info: &Option<(String, String)>,
    commit_sha: &str,
) -> Result<Option<Snapshot>, GitError> {
    // Skip files that are variants
    let file_name = relative_path
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or(GitError::FileNotFound)?;

    if file_name.ends_with(".old.png")
        || file_name.ends_with(".new.png")
        || file_name.ends_with(".diff.png")
    {
        return Ok(None);
    }

    let Ok(default_file_content) = get_file_from_tree(repo, default_tree, relative_path) else {
        // File doesn't exist in default branch, skip
        return Ok(None);
    };

    // Get the current file from the current branch's tree to compare git objects properly
    let head_commit = repo.head()?.peel_to_commit()?;
    let head_tree = head_commit.tree()?;

    // Compare git object content (both should be LFS pointers if using LFS)
    if let Ok(current_content) = get_file_from_tree(repo, &head_tree, relative_path) {
        if default_file_content == current_content {
            return Ok(None);
        }
    }

    // Check if this is an LFS pointer file
    let default_image_source = if is_lfs_pointer(&default_file_content) {
        // If we have GitHub repo info, create media URL
        if let Some((org, repo_name)) = github_repo_info {
            let media_url = create_lfs_media_url(org, repo_name, commit_sha, relative_path);
            ImageSource::Uri(Cow::Owned(media_url))
        } else {
            // Fallback to bytes (will likely fail to load but better than nothing)
            ImageSource::Bytes {
                uri: Cow::Owned(format!("bytes://{}", relative_path.display())),
                bytes: Bytes::Shared(default_file_content.into()),
            }
        }
    } else {
        // Regular file content
        ImageSource::Bytes {
            uri: Cow::Owned(format!("bytes://{}", relative_path.display())),
            bytes: Bytes::Shared(default_file_content.into()),
        }
    };

    Ok(Some(Snapshot {
        path: relative_path.to_path_buf(),
        old: Some(FileReference::Source(default_image_source)), // Default branch version as ImageSource
        new: Some(FileReference::Path(relative_path.to_path_buf())), // Current working tree version
        diff: None,                                             // Always None for git mode
    }))
}

fn get_file_from_tree(
    repo: &Repository,
    tree: &git2::Tree<'_>,
    path: &Path,
) -> Result<Vec<u8>, GitError> {
    let entry = tree.get_path(path)?;
    let object = entry.to_object(repo)?;

    match object.kind() {
        Some(ObjectType::Blob) => {
            let blob = object.as_blob().ok_or(GitError::FileNotFound)?;
            Ok(blob.content().to_vec())
        }
        _ => Err(GitError::FileNotFound),
    }
}

fn is_lfs_pointer(content: &[u8]) -> bool {
    // LFS pointer files must be < 1024 bytes and UTF-8
    if content.len() >= 1024 {
        return false;
    }

    // Try to parse as UTF-8
    let Ok(text) = str::from_utf8(content) else {
        return false;
    };

    // Check for LFS pointer format
    // Must start with "version https://git-lfs.github.com/spec/v1"
    let lines: Vec<&str> = text.lines().collect();
    if lines.is_empty() {
        return false;
    }

    // First line must be version
    if !lines[0].starts_with("version https://git-lfs.github.com/spec/v1") {
        return false;
    }

    // Look for required oid and size lines
    let mut has_oid = false;
    let mut has_size = false;

    for line in &lines[1..] {
        if line.starts_with("oid sha256:") {
            has_oid = true;
        } else if line.starts_with("size ") {
            has_size = true;
        }
    }

    has_oid && has_size
}

fn get_github_repo_info(repo: &Repository) -> Option<(String, String)> {
    // Try to get the origin remote
    let remote = repo.find_remote("origin").ok()?;
    let url = remote.url()?;

    // Parse GitHub URLs (both HTTPS and SSH)
    if let Some(caps) = parse_github_https_url(url) {
        return Some(caps);
    }

    if let Some(caps) = parse_github_ssh_url(url) {
        return Some(caps);
    }

    None
}

fn parse_github_https_url(url: &str) -> Option<(String, String)> {
    // Match: https://github.com/org/repo.git or https://github.com/org/repo
    if url.starts_with("https://github.com/") {
        let path = url.strip_prefix("https://github.com/")?;
        let path = path.strip_suffix(".git").unwrap_or(path);

        let parts: Vec<&str> = path.split('/').collect();
        if parts.len() >= 2 {
            return Some((parts[0].to_owned(), parts[1].to_owned()));
        }
    }
    None
}

fn parse_github_ssh_url(url: &str) -> Option<(String, String)> {
    // Match: git@github.com:org/repo.git
    if url.starts_with("git@github.com:") {
        let path = url.strip_prefix("git@github.com:")?;
        let path = path.strip_suffix(".git").unwrap_or(path);

        let parts: Vec<&str> = path.split('/').collect();
        if parts.len() >= 2 {
            return Some((parts[0].to_owned(), parts[1].to_owned()));
        }
    }
    None
}

fn create_lfs_media_url(org: &str, repo: &str, commit_sha: &str, file_path: &Path) -> String {
    format!(
        "https://media.githubusercontent.com/media/{}/{}/{}/{}",
        org,
        repo,
        commit_sha,
        file_path.display()
    )
}
