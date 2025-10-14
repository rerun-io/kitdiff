use crate::loaders::{LoadSnapshots, sort_snapshots};
use crate::snapshot::{FileReference, Snapshot};
use eframe::egui::load::Bytes;
use eframe::egui::{Context, ImageSource};
use egui_inbox::{UiInbox, UiInboxSender};
use gix::Repository;
use gix::bstr::ByteSlice;
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
    Gix(Box<dyn std::error::Error + Send + Sync>),
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
            Self::Gix(err) => write!(f, "Git error: {err}"),
            Self::IoError(err) => write!(f, "IO error: {err}"),
            Self::PrUrlParseError => write!(f, "Failed to parse PR URL"),
            Self::NetworkError(msg) => write!(f, "Network error: {msg}"),
        }
    }
}

impl std::error::Error for GitError {}

impl From<std::io::Error> for GitError {
    fn from(err: std::io::Error) -> Self {
        Self::IoError(err)
    }
}

fn run_git_discovery(sender: &Sender, base_path: &Path) -> Result<(), GitError> {
    // Open git repository in current directory
    let repo = gix::open(base_path).map_err(|_err| GitError::RepoNotFound)?;

    // Get current branch
    let head = repo.head().map_err(|e| GitError::Gix(Box::new(e)))?;
    let current_branch = head
        .referent_name()
        .and_then(|n| n.shorten().as_bstr().to_str().ok())
        .unwrap_or("HEAD")
        .to_owned();

    // Find default branch (try main, then master, then first branch)
    let default_branch = find_default_branch(&repo)?;

    // Send git info
    let repo_name = repo
        .git_dir()
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
        log::warn!("Current branch is the same as default branch ({current_branch})");
        return Ok(());
    }

    // Get the merge base between current branch and default branch
    let head_ref = repo.head().map_err(|e| GitError::Gix(Box::new(e)))?;
    let head_commit_id = head_ref
        .into_peeled_id()
        .map_err(|e| GitError::Gix(Box::new(e)))?;
    let head_commit_obj = repo
        .find_object(head_commit_id.detach())
        .map_err(|e| GitError::Gix(Box::new(e)))?;
    let head_commit = head_commit_obj
        .try_into_commit()
        .map_err(|_| GitError::BranchNotFound)?;

    let default_ref = repo
        .find_reference(&format!("refs/heads/{}", default_branch))
        .map_err(|e| GitError::Gix(Box::new(e)))?;
    let default_commit_id = default_ref
        .into_fully_peeled_id()
        .map_err(|e| GitError::Gix(Box::new(e)))?;
    let default_commit_obj = repo
        .find_object(default_commit_id.detach())
        .map_err(|e| GitError::Gix(Box::new(e)))?;
    let default_commit = default_commit_obj
        .try_into_commit()
        .map_err(|_| GitError::BranchNotFound)?;

    // Find merge base - for now, just use the default branch commit as the base
    // This is a simplification but will work for the common case
    let base_commit = default_commit;

    // Get GitHub repository info for LFS support
    let github_repo_info = get_github_repo_info(&repo);
    let commit_sha = base_commit.id.to_string();

    // Get current HEAD tree for comparison
    let head_tree = head_commit.tree().map_err(|e| GitError::Gix(Box::new(e)))?;

    let base_tree = base_commit.tree().map_err(|e| GitError::Gix(Box::new(e)))?;

    // Use gix diff to find changed PNG files between merge base and current HEAD
    base_tree
        .changes()
        .map_err(|e| GitError::Gix(Box::new(e)))?
        .for_each_to_obtain_tree(
            &head_tree,
            |change: gix::object::tree::diff::Change<'_, '_, '_>| -> Result<
                gix::object::tree::diff::Action,
                Box<dyn std::error::Error + Send + Sync>,
            > {
                // Check both old and new file paths (handles renames/moves)
                let path = change.location();
                let path_str = path.to_str().unwrap_or("");
                let path_obj = Path::new(path_str);

                // Check if this is a PNG file
                if let Some(extension) = path_obj.extension() {
                    if extension == "png" {
                        // Create snapshot for this changed PNG file
                        if let Ok(Some(snapshot)) = create_git_snapshot(
                            &repo,
                            &mut base_tree.clone(),
                            path_obj,
                            &github_repo_info,
                            &commit_sha,
                        ) {
                            sender.send(Command::Snapshot(snapshot)).ok();
                        }
                    }
                }
                Ok(gix::object::tree::diff::Action::Continue)
            },
        )
        .map_err(|e| GitError::Gix(Box::new(e)))?;

    Ok(())
}

fn find_default_branch(repo: &Repository) -> Result<String, GitError> {
    // Try common default branch names
    for branch_name in ["main", "master"] {
        if repo
            .find_reference(&format!("refs/heads/{}", branch_name))
            .is_ok()
        {
            return Ok(branch_name.to_owned());
        }
    }

    // Fall back to first branch found
    let references = repo.references().map_err(|e| GitError::Gix(Box::new(e)))?;

    for reference in (references
        .prefixed("refs/heads/")
        .map_err(|e| GitError::Gix(Box::new(e)))?)
    .flatten()
    {
        if let Ok(name) = reference.name().shorten().to_str() {
            return Ok(name.to_owned());
        }
    }

    Err(GitError::BranchNotFound)
}

fn create_git_snapshot(
    repo: &Repository,
    default_tree: &mut gix::Tree<'_>,
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
    let head_ref = repo.head().map_err(|e| GitError::Gix(Box::new(e)))?;
    let head_commit_id = head_ref
        .into_peeled_id()
        .map_err(|e| GitError::Gix(Box::new(e)))?;
    let head_commit_obj = repo
        .find_object(head_commit_id.detach())
        .map_err(|e| GitError::Gix(Box::new(e)))?;
    let head_commit = head_commit_obj
        .try_into_commit()
        .map_err(|_| GitError::BranchNotFound)?;
    let mut head_tree = head_commit.tree().map_err(|e| GitError::Gix(Box::new(e)))?;

    // Compare git object content (both should be LFS pointers if using LFS)
    if let Ok(current_content) = get_file_from_tree(repo, &mut head_tree, relative_path)
        && default_file_content == current_content
    {
        return Ok(None);
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
    tree: &mut gix::Tree<'_>,
    path: &Path,
) -> Result<Vec<u8>, GitError> {
    let entry = tree
        .peel_to_entry_by_path(path)
        .map_err(|e| GitError::Gix(Box::new(e)))?
        .ok_or(GitError::FileNotFound)?;

    if entry.mode().is_blob() {
        let object = repo
            .find_object(entry.oid())
            .map_err(|e| GitError::Gix(Box::new(e)))?;
        let blob = object.try_into_blob().map_err(|_| GitError::FileNotFound)?;
        Ok(blob.data.to_vec())
    } else {
        Err(GitError::FileNotFound)
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
    let url = remote.url(gix::remote::Direction::Fetch)?;
    let url_str = url.to_bstring();
    let url = url_str.to_str().ok()?;

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
