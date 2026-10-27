use crate::config::Config;
use crate::github::content_loader::GithubContentLoader;
use crate::github::model::{GithubPrLink, GithubRepoLink};
use crate::github::octokit::RepoClient;
use crate::github::pr::{GithubPr, pr_ui};
use crate::loaders::{LoadSnapshots, sort_snapshots};
use crate::snapshot::{FileReference, Snapshot};
use crate::state::AppStateRef;
use eframe::egui::{Context, Ui};
use egui_inbox::{UiInbox, UiInboxSender};
use futures::TryStreamExt as _;
use octocrab::models::repos::DiffEntryStatus;
use octocrab::{Octocrab, Result};
use percent_encoding::{AsciiSet, CONTROLS, utf8_percent_encode};
use std::pin::pin;
use std::task::Poll;

/// Percent-encode path segments for use in a URL path, preserving `/`.
/// Mirrors the `PATH` set from the URL spec but lets `utf8_percent_encode` handle non-ASCII.
const PATH_SEGMENT: &AsciiSet = &CONTROLS
    .add(b' ')
    .add(b'"')
    .add(b'#')
    .add(b'<')
    .add(b'>')
    .add(b'?')
    .add(b'`')
    .add(b'{')
    .add(b'}');

type Sender = UiInboxSender<Option<Result<Snapshot>>>;

pub struct PrLoader {
    snapshots: Vec<Snapshot>,
    inbox: UiInbox<Option<Result<Snapshot>>>,
    state: Poll<anyhow::Result<()>>,
    link: GithubPrLink,
    pr_info: GithubPr,
    logged_in: bool,
    config_override: Option<Config>,
}

impl PrLoader {
    pub fn new(
        link: GithubPrLink,
        client: Octocrab,
        logged_in: bool,
        config_override: Option<Config>,
    ) -> Self {
        let mut inbox = UiInbox::new();
        let repo_client = RepoClient::new(client.clone(), link.repo.clone());

        inbox.spawn(|tx| async move {
            let result = stream_files(repo_client, link.pr_number, tx.clone(), logged_in).await;
            match result {
                Ok(()) => {
                    tx.send(None).ok();
                }
                Err(err) => {
                    tx.send(Some(Err(err))).ok();
                }
            }
        });

        Self {
            snapshots: Vec::new(),
            inbox,
            state: Poll::Pending,
            pr_info: GithubPr::new(link.clone(), client, config_override.clone()),
            link,
            logged_in,
            config_override,
        }
    }
}

async fn stream_files(
    repo_client: RepoClient,
    pr_number: u64,
    sender: Sender,
    logged_in: bool,
) -> octocrab::Result<()> {
    let pr = repo_client.pulls().get(pr_number).await?;

    let file = repo_client.pulls().list_files(pr_number).await?;

    let stream = file.into_stream(&repo_client);

    let repo = repo_client.repo();
    let pr = &pr;
    let results = stream.try_filter_map(|file| async move {
        if !file.filename.ends_with(".png") {
            return Ok(None);
        }
        let old = (file.status != DiffEntryStatus::Added).then(|| {
            let name = file.previous_filename.as_deref().unwrap_or(&*file.filename);
            resolve_url(repo, &pr.base.sha, name, logged_in)
        });
        let new = (file.status != DiffEntryStatus::Removed)
            .then(|| resolve_url(repo, &pr.head.sha, &file.filename, logged_in));
        Ok(Some(Snapshot {
            path: file.filename.clone().into(),
            old: old.map(|url| FileReference::Source(url.into())),
            new: new.map(|url| FileReference::Source(url.into())),
            diff: None,
        }))
    });
    let mut results = pin!(results);

    while let Some(snapshot) = results.try_next().await? {
        sender.send(Some(Ok(snapshot))).ok();
    }

    Ok(())
}

/// When logged in, loads the file through the contents API with
/// [`GithubContentLoader`], which works for private repos. Otherwise, uses the
/// public media.githubusercontent.com URL to avoid burning API rate limit.
fn resolve_url(
    repo: &GithubRepoLink,
    commit_sha: &str,
    file_path: &str,
    logged_in: bool,
) -> String {
    let encoded_path = utf8_percent_encode(file_path, PATH_SEGMENT).to_string();
    if logged_in {
        GithubContentLoader::uri(&repo.owner, &repo.repo, commit_sha, &encoded_path)
    } else {
        create_media_url(repo, commit_sha, &encoded_path)
    }
}

fn create_media_url(repo: &GithubRepoLink, commit_sha: &str, file_path: &str) -> String {
    format!(
        "https://media.githubusercontent.com/media/{}/{}/{}/{}",
        repo.owner, repo.repo, commit_sha, file_path,
    )
}

impl LoadSnapshots for PrLoader {
    fn update(&mut self, ctx: &Context) {
        for snapshot in self.inbox.read(ctx) {
            match snapshot {
                Some(Ok(s)) => {
                    self.snapshots.push(s);
                    sort_snapshots(&mut self.snapshots);
                }
                Some(Err(e)) => {
                    self.state = Poll::Ready(Err(e.into()));
                }
                None => {
                    self.state = Poll::Ready(Ok(()));
                }
            }
        }
        self.pr_info.update(ctx);
    }

    fn refresh(&mut self, client: Octocrab) {
        *self = Self::new(
            self.link.clone(),
            client,
            self.logged_in,
            self.config_override.clone(),
        );
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

    fn extra_ui(&self, ui: &mut Ui, state: &AppStateRef<'_>) {
        pr_ui(ui, state, &self.pr_info);
    }

    fn files_header(&self) -> String {
        format!("{}", self.link)
    }
}
