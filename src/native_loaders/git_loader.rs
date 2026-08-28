use crate::loaders::{LoadSnapshots, sort_snapshots};
use crate::snapshot::{FileReference, Snapshot};
use eframe::egui::load::Bytes;
use eframe::egui::{Context, ImageSource};
use egui_inbox::{UiInbox, UiInboxSender};
use gix::Repository;
use gix::bstr::{BStr, BString, ByteSlice as _};
use octocrab::Octocrab;
use std::borrow::Cow;
use std::collections::BTreeSet;
use std::io::Read as _;
use std::path::{Path, PathBuf};
use std::str;
use std::task::Poll;

enum Command {
    Snapshot(Snapshot),
    Error(anyhow::Error),
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
        for new_data in self.inbox.read(ctx) {
            match new_data {
                Command::Snapshot(snapshot) => {
                    self.snapshots.push(snapshot);
                    sort_snapshots(&mut self.snapshots);
                }
                Command::Error(e) => {
                    self.state = Poll::Ready(Err(e));
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
            // On the default branch there is nothing to compare against but the working tree
            Some(info) if info.current_branch == info.default_branch => {
                format!("Git: {} ({})", info.repo_name, info.current_branch)
            }
            Some(info) => format!(
                "Git: {} ({} ➡ {})",
                info.repo_name, info.current_branch, info.default_branch
            ),
            None => format!("Git: {}", self.base_path.display()),
        }
    }
}

fn run_git_discovery(sender: &Sender, base_path: &Path) -> anyhow::Result<()> {
    // Search upwards, so this also works when pointed at a subdirectory of the repo
    let repo =
        gix::discover(base_path).map_err(|e| anyhow::anyhow!("Git repository not found: {e}"))?;

    // All paths git reports are relative to the work dir, not to `base_path`.
    // Canonicalize it, since discovery may return a relative path and we build `file://`
    // uris from it.
    let workdir = repo
        .workdir()
        .ok_or_else(|| anyhow::anyhow!("Repository has no working tree"))?;
    let workdir = workdir
        .canonicalize()
        .unwrap_or_else(|_| workdir.to_path_buf());

    // Get current branch
    let head = repo.head()?;
    let current_branch = head
        .referent_name()
        .and_then(|n| n.shorten().as_bstr().to_str().ok())
        .unwrap_or("HEAD")
        .to_owned();

    // Find default branch (try main, then master, then first branch)
    let default_branch = find_default_branch(&repo)?;

    // Send git info
    let repo_name = workdir
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("unknown")
        .to_owned();
    sender
        .send(Command::GitInfo(GitInfo {
            current_branch,
            default_branch: default_branch.clone(),
            repo_name,
        }))
        .ok();

    let head_commit = repo.head_commit()?;

    let default_commit = repo
        .find_reference(&format!("refs/heads/{default_branch}"))?
        .into_fully_peeled_id()?
        .object()?
        .try_into_commit()
        .map_err(|e| anyhow::anyhow!("Failed to get commit from default branch: {e:?}"))?;

    // Diff against the merge base, so commits that landed on the default branch after we
    // branched off don't show up as changes.
    let base_commit = match repo.merge_base(head_commit.id, default_commit.id) {
        Ok(id) => repo.find_commit(id.detach())?,
        // Unrelated histories, so the tip is the best we can do
        Err(gix::repository::merge_base::Error::NotFound { .. }) => {
            log::warn!("No merge base with {default_branch}, comparing against its tip");
            default_commit
        }
        Err(err) => return Err(err.into()),
    };

    let base_tree = base_commit.tree()?;
    let head_tree = head_commit.tree()?;

    // Get GitHub repository info for LFS support
    let github_repo_info = get_github_repo_info(&repo);
    let commit_sha = base_commit.id.to_string();

    let mut filters = LfsFilters::new(&repo);

    // Collect every image that differs from the base, no matter if the change is committed,
    // staged, or only present in the working tree.
    let mut changed = BTreeSet::new();
    collect_committed_changes(&base_tree, &head_tree, &mut changed)?;
    collect_uncommitted_changes(&repo, &mut changed)?;

    for relative_path in changed {
        match create_git_snapshot(
            &repo,
            &mut filters,
            &base_tree,
            &relative_path,
            &github_repo_info,
            &commit_sha,
            &workdir,
        ) {
            Ok(Some(snapshot)) => {
                sender.send(Command::Snapshot(snapshot)).ok();
            }
            Ok(None) => {
                log::info!("No snapshot created for {}", relative_path.display());
            }
            Err(err) => {
                log::error!(
                    "Failed to create snapshot for {}: {err}",
                    relative_path.display()
                );
            }
        }
    }

    Ok(())
}

/// Is this a `.png` we want to show, i.e. not one of the `.old`/`.new`/`.diff` variants
/// that the file loader pairs up on its own?
fn is_snapshot_candidate(path: &Path) -> bool {
    let Some(file_name) = path.file_name().and_then(|n| n.to_str()) else {
        return false;
    };
    file_name.ends_with(".png")
        && !file_name.ends_with(".old.png")
        && !file_name.ends_with(".new.png")
        && !file_name.ends_with(".diff.png")
}

/// Changes between the base commit and `HEAD`.
fn collect_committed_changes(
    base_tree: &gix::Tree<'_>,
    head_tree: &gix::Tree<'_>,
    changed: &mut BTreeSet<PathBuf>,
) -> anyhow::Result<()> {
    let mut changes = base_tree.changes()?;
    // A rewrite only reports its destination, so we'd lose the renamed-away path. We want the
    // state of every path anyway, and similarity detection on pngs is wasted work.
    changes.options(|options| {
        options.track_rewrites(None);
    });
    changes.for_each_to_obtain_tree(
        head_tree,
        |change: gix::object::tree::diff::Change<'_, '_, '_>| -> Result<
            gix::object::tree::diff::Action,
            Box<dyn std::error::Error + Send + Sync>,
        > {
            if let Ok(path) = gix::path::try_from_bstr(change.location())
                && is_snapshot_candidate(&path)
            {
                changed.insert(path.into_owned());
            }
            Ok(gix::object::tree::diff::Action::Continue(()))
        },
    )?;
    Ok(())
}

/// Changes that aren't committed yet: staged (`HEAD` vs index), unstaged (index vs working
/// tree) and untracked files.
fn collect_uncommitted_changes(
    repo: &Repository,
    changed: &mut BTreeSet<PathBuf>,
) -> anyhow::Result<()> {
    // `status()` drops the dirwalk options when `status.showUntrackedFiles=no` is configured,
    // and `untracked_files` can't bring them back, so put them in place first.
    let dirwalk_options = repo.dirwalk_options()?;

    let platform = repo
        .status(gix::progress::Discard)?
        .index_worktree_options_mut(|options| options.dirwalk_options = Some(dirwalk_options))
        // Show newly created snapshots too
        .untracked_files(gix::status::UntrackedFiles::Files)
        // Submodule status is expensive and never yields paths inside the submodule anyway
        .index_worktree_submodules(gix::status::Submodule::Given {
            ignore: gix::submodule::config::Ignore::All,
            check_dirty: false,
        })
        // Same reason as in `collect_committed_changes`
        .tree_index_track_renames(gix::status::tree_index::TrackRenames::Disabled)
        // Keep the modification checks and the directory walk from visiting the whole repo.
        // `:(top)` anchors the pattern to the work dir, since pathspecs are otherwise relative
        // to the directory the process was started in.
        ;
    let patterns = Some(BString::from(":(top)*.png"));

    let mut insert = |rela_path: &BStr| {
        if let Ok(path) = gix::path::try_from_bstr(rela_path)
            && is_snapshot_candidate(&path)
        {
            changed.insert(path.into_owned());
        }
    };

    if repo.index_or_empty()?.is_sparse() {
        // gix refuses to diff a sparse index against a tree, so we have to make do without
        // the staged changes
        log::warn!("Sparse index, so staged image changes won't be listed");
        for item in platform.into_index_worktree_iter(patterns)? {
            insert(item?.rela_path());
        }
    } else {
        for item in platform.into_iter(patterns)? {
            let item = item?;
            insert(match &item {
                gix::status::Item::IndexWorktree(item) => item.rela_path(),
                gix::status::Item::TreeIndex(change) => change.location(),
            });
        }
    }

    Ok(())
}

fn find_default_branch(repo: &Repository) -> anyhow::Result<String> {
    // Try common default branch names
    for branch_name in ["main", "master"] {
        if repo
            .find_reference(&format!("refs/heads/{branch_name}"))
            .is_ok()
        {
            return Ok(branch_name.to_owned());
        }
    }

    // Fall back to first branch found
    let references = repo.references()?;

    for reference in references.prefixed("refs/heads/")?.flatten() {
        if let Ok(name) = reference.name().shorten().to_str() {
            return Ok(name.to_owned());
        }
    }

    anyhow::bail!("No default branch found")
}

fn create_git_snapshot(
    repo: &Repository,
    filters: &mut LfsFilters<'_>,
    base_tree: &gix::Tree<'_>,
    relative_path: &Path,
    github_repo_info: &Option<(String, String)>,
    commit_sha: &str,
    workdir: &Path,
) -> anyhow::Result<Option<Snapshot>> {
    // Missing from the base tree means the file was added, everything else is a real error
    let base_content = get_file_from_tree(repo, base_tree, relative_path)?;

    // The working tree is the source of truth for the new side, so uncommitted edits show up
    let full_path = workdir.join(relative_path);
    let new_exists = full_path.is_file();

    if !new_exists && is_sparse(repo, relative_path)? {
        // Not checked out rather than deleted, and we have no file to show
        log::warn!(
            "Skipping {}, it is excluded by the sparse checkout",
            relative_path.display()
        );
        return Ok(None);
    }

    if let Some(base_content) = &base_content
        && new_exists
        && worktree_matches_base(base_content, &full_path)
    {
        // The individual changes we found cancel each other out, e.g. a committed change that
        // was reverted in the working tree
        return Ok(None);
    }

    let old = base_content.map(|content| {
        base_file_reference(
            repo,
            filters,
            content,
            relative_path,
            github_repo_info,
            commit_sha,
        )
    });
    let new = new_exists.then_some(FileReference::Path(full_path));

    if old.is_none() && new.is_none() {
        return Ok(None);
    }

    Ok(Some(Snapshot {
        path: relative_path.to_path_buf(),
        old,
        new,
        diff: None, // Always None for git mode
    }))
}

/// Is this path tracked, but left out of the working tree by a sparse checkout?
fn is_sparse(repo: &Repository, relative_path: &Path) -> anyhow::Result<bool> {
    let rela_path =
        gix::path::to_unix_separators_on_windows(gix::path::into_bstr(relative_path)).into_owned();

    let index = repo.index_or_empty()?;

    if let Some(entry) = index.entry_by_path(rela_path.as_ref()) {
        return Ok(entry
            .flags
            .contains(gix::index::entry::Flags::SKIP_WORKTREE));
    }

    if !index.is_sparse() {
        return Ok(false);
    }

    // A sparse index has no entry for the file itself, only a directory entry for the
    // excluded tree it lives in
    Ok(rela_path
        .rfind_iter(b"/")
        .map(|slash| rela_path[..=slash].as_bstr())
        .any(|dir| {
            index
                .entry_by_path(dir)
                .is_some_and(|entry| entry.mode.is_sparse())
        }))
}

/// Does the working tree file have the exact same content as the base version?
///
/// Only ever returns `true` if we could prove it, so a file we can't read counts as changed.
fn worktree_matches_base(base_content: &[u8], full_path: &Path) -> bool {
    let Ok(worktree_content) = std::fs::read(full_path) else {
        return false;
    };

    let Some(base_pointer) = parse_lfs_pointer(base_content) else {
        return base_content == worktree_content;
    };

    // The base is an LFS pointer, so the working tree holds either the smudged file or,
    // if LFS isn't set up, the very same pointer
    match parse_lfs_pointer(&worktree_content) {
        Some(worktree_pointer) => worktree_pointer == base_pointer,
        None => {
            base_pointer.size == worktree_content.len() as u64
                && base_pointer.oid == sha256_hex(&worktree_content)
        }
    }
}

fn sha256_hex(content: &[u8]) -> String {
    use sha2::Digest as _;
    format!("{:x}", sha2::Sha256::digest(content))
}

fn base_file_reference(
    repo: &Repository,
    filters: &mut LfsFilters<'_>,
    content: Vec<u8>,
    relative_path: &Path,
    github_repo_info: &Option<(String, String)>,
    commit_sha: &str,
) -> FileReference {
    // Check if this is an LFS pointer file
    if let Some(pointer) = parse_lfs_pointer(&content) {
        // git-lfs has usually downloaded the object already. Unlike the media url below this
        // needs no network and works for private repos.
        if let Some(object_path) = lfs_object_path(repo, &pointer) {
            return FileReference::Path(object_path);
        }

        // Not downloaded yet, so let git-lfs fetch it. This uses the user's git credentials,
        // so unlike the media url below it also works for private repos.
        log::info!(
            "Fetching {} through git-lfs, it is not in the local store",
            relative_path.display()
        );
        match filters.convert_to_worktree(&content, &pointer, relative_path) {
            Ok(smudged) => {
                return FileReference::Source(ImageSource::Bytes {
                    uri: Cow::Owned(format!("bytes://{}", relative_path.display())),
                    bytes: Bytes::Shared(smudged.into()),
                });
            }
            Err(err) => log::warn!(
                "git-lfs could not provide {}: {err}",
                relative_path.display()
            ),
        }

        // If we have GitHub repo info, create media URL
        if let Some((org, repo_name)) = github_repo_info {
            log::warn!(
                "Falling back to media.githubusercontent.com for {}, which only serves public \
                 repositories",
                relative_path.display()
            );
            let media_url = create_lfs_media_url(org, repo_name, commit_sha, relative_path);
            return FileReference::Source(ImageSource::Uri(Cow::Owned(media_url)));
        }
        log::warn!(
            "{} is a git-lfs pointer, but the object is neither in the local store nor on \
             GitHub. `git lfs fetch` to get it.",
            relative_path.display()
        );
        // Fall through to bytes (will likely fail to load but better than nothing)
    }

    FileReference::Source(ImageSource::Bytes {
        uri: Cow::Owned(format!("bytes://{}", relative_path.display())),
        bytes: Bytes::Shared(content.into()),
    })
}

/// Runs the configured filter chain, which for a git-lfs pointer means asking `git-lfs` for
/// the real content, downloading it if needed.
///
/// The pipeline is only built on first use, since most repositories never need it, and it is
/// kept around because it holds on to the long-running `git-lfs filter-process`. That makes
/// the first conversion cost ~150ms and every one after it well under a millisecond.
struct LfsFilters<'repo> {
    repo: &'repo Repository,
    pipeline: Option<gix::filter::Pipeline<'repo>>,
}

impl<'repo> LfsFilters<'repo> {
    fn new(repo: &'repo Repository) -> Self {
        Self {
            repo,
            pipeline: None,
        }
    }

    fn convert_to_worktree(
        &mut self,
        content: &[u8],
        pointer: &LfsPointer,
        relative_path: &Path,
    ) -> anyhow::Result<Vec<u8>> {
        if self.pipeline.is_none() {
            let (pipeline, _index) = self.repo.filter_pipeline(None)?;
            self.pipeline = Some(pipeline);
        }
        let pipeline = self
            .pipeline
            .as_mut()
            .ok_or_else(|| anyhow::anyhow!("Filter pipeline is missing"))?;

        let rela_path =
            gix::path::to_unix_separators_on_windows(gix::path::into_bstr(relative_path))
                .into_owned();

        let mut converted = pipeline.convert_to_worktree(
            content,
            rela_path.as_ref(),
            gix::filter::plumbing::driver::apply::Delay::Forbid,
        )?;

        let mut buffer = Vec::new();
        converted.read_to_end(&mut buffer)?;

        // The pipeline hands the input straight back if no filter applies to this path, e.g.
        // when git-lfs isn't installed or the attributes stopped matching. Checking the
        // content against the pointer catches that, and any other filter misbehaviour.
        if buffer.len() as u64 != pointer.size || sha256_hex(&buffer) != pointer.oid {
            anyhow::bail!("the filter output does not match the pointer");
        }

        Ok(buffer)
    }
}

impl Drop for LfsFilters<'_> {
    fn drop(&mut self) {
        let Some(pipeline) = self.pipeline.take() else {
            return;
        };

        // Dropping the pipeline closes the pipes, which makes `git-lfs` exit, but nobody would
        // reap it. Shut down explicitly so we don't leave a zombie behind.
        let (mut pipeline, _cache) = pipeline.into_parts();
        let state = std::mem::take(pipeline.driver_state_mut());
        if let Err(err) =
            state.shutdown(gix::filter::plumbing::driver::shutdown::Mode::WaitForProcesses)
        {
            log::warn!("Failed to shut down the git-lfs filter process: {err}");
        }
    }
}

/// Where git-lfs stores the object of `pointer`, if it has been downloaded.
fn lfs_object_path(repo: &Repository, pointer: &LfsPointer) -> Option<PathBuf> {
    // The oid ends up in a path, so don't trust it blindly
    if pointer.oid.len() < 4 || !pointer.oid.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }

    // In a worktree the common dir is `<repo>/.git/worktrees/<name>/../..`, so tidy it up to
    // keep the uris we build from it readable
    let common_dir = repo.common_dir();
    let common_dir = common_dir
        .canonicalize()
        .unwrap_or_else(|_| common_dir.to_path_buf());

    // `lfs.storage` is the storage root, and git-lfs appends `objects` to it itself. A
    // relative value is relative to the common dir, an absolute one wins the join.
    let storage_root = repo
        .config_snapshot()
        .trusted_path("lfs.storage")
        .transpose()
        .ok()
        .flatten()
        .map_or_else(
            || common_dir.join("lfs"),
            |storage| common_dir.join(storage),
        );
    let storage = storage_root.join("objects");

    let object_path = storage
        .join(&pointer.oid[..2])
        .join(&pointer.oid[2..4])
        .join(&pointer.oid);

    object_path.is_file().then_some(object_path)
}

/// The file's content in `tree`, or `None` if the tree has no such file.
fn get_file_from_tree(
    repo: &Repository,
    tree: &gix::Tree<'_>,
    path: &Path,
) -> anyhow::Result<Option<Vec<u8>>> {
    let mut tree_clone = tree.clone();
    let Some(entry) = tree_clone.peel_to_entry_by_path(path)? else {
        return Ok(None);
    };

    if entry.mode().is_blob() {
        let object = repo.find_object(entry.oid())?;
        let blob = object
            .try_into_blob()
            .map_err(|e| anyhow::anyhow!("Entry is not a blob: {e:?}"))?;
        Ok(Some(blob.data.clone()))
    } else {
        anyhow::bail!("Path is not a file")
    }
}

#[derive(PartialEq, Eq)]
struct LfsPointer {
    oid: String,
    size: u64,
}

fn parse_lfs_pointer(content: &[u8]) -> Option<LfsPointer> {
    // LFS pointer files must be < 1024 bytes and UTF-8
    if content.len() >= 1024 {
        return None;
    }

    // Try to parse as UTF-8
    let text = str::from_utf8(content).ok()?;

    // Check for LFS pointer format
    // Must start with "version https://git-lfs.github.com/spec/v1"
    let mut lines = text.lines();
    if !lines
        .next()?
        .starts_with("version https://git-lfs.github.com/spec/v1")
    {
        return None;
    }

    // Look for required oid and size lines
    let mut oid = None;
    let mut size = None;

    for line in lines {
        if let Some(rest) = line.strip_prefix("oid sha256:") {
            oid = Some(rest.to_owned());
        } else if let Some(rest) = line.strip_prefix("size ") {
            size = rest.parse().ok();
        }
    }

    Some(LfsPointer {
        oid: oid?,
        size: size?,
    })
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snapshot_candidates_are_pngs_but_not_variants() {
        assert!(is_snapshot_candidate(Path::new("tests/snapshots/foo.png")));
        assert!(!is_snapshot_candidate(Path::new("foo.old.png")));
        assert!(!is_snapshot_candidate(Path::new("foo.new.png")));
        assert!(!is_snapshot_candidate(Path::new("foo.diff.png")));
        assert!(!is_snapshot_candidate(Path::new("foo.jpg")));
        assert!(!is_snapshot_candidate(Path::new("png")));
    }

    #[test]
    fn parses_lfs_pointers() {
        let pointer = parse_lfs_pointer(
            b"version https://git-lfs.github.com/spec/v1\noid sha256:c02ed510\nsize 186438\n",
        )
        .expect("should parse");
        assert_eq!(pointer.oid, "c02ed510");
        assert_eq!(pointer.size, 186438);

        // A png is not a pointer, and neither is a pointer that's missing a field
        assert!(parse_lfs_pointer(b"\x89PNG\r\n\x1a\n").is_none());
        assert!(
            parse_lfs_pointer(b"version https://git-lfs.github.com/spec/v1\nsize 12\n").is_none()
        );
    }

    #[test]
    fn hashes_like_git_lfs_does() {
        // `printf '' | sha256sum`
        assert_eq!(
            sha256_hex(b""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
    }
}
