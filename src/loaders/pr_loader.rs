use crate::config::Config;
use crate::github::model::GithubPrLink;
use crate::github::octokit::RepoClient;
use crate::github::pr::{GithubPr, pr_ui};
use crate::loaders::github_file_loader::github_file_uri;
use crate::loaders::{LoadSnapshots, sort_snapshots};
use crate::snapshot::{FileReference, Snapshot};
use crate::state::AppStateRef;
use eframe::egui::{Context, Ui};
use egui_inbox::{UiInbox, UiInboxSender};
use futures::TryStreamExt as _;
use octocrab::models::repos::{DiffEntry, DiffEntryStatus};
use octocrab::{Octocrab, Page, Result};
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

    // If GitHub doesn't say, assume a private repository when logged in: that works for both.
    let private = pr
        .base
        .repo
        .as_ref()
        .and_then(|repo| repo.private)
        .unwrap_or(logged_in);

    // `pulls().list_files()` can't set the page size, and the default of 30 costs three times the API calls.
    let route = format!(
        "/repos/{}/{}/pulls/{pr_number}/files",
        repo_client.repo().owner,
        repo_client.repo().repo
    );
    let first_page: Page<DiffEntry> = repo_client.get(route, Some(&[("per_page", 100)])).await?;

    let files = first_page
        .into_stream(&repo_client)
        .try_filter(|file| std::future::ready(file.filename.ends_with(".png")));
    let mut files = pin!(files);

    let uri = |sha: &str, path: &str| {
        let encoded_path = utf8_percent_encode(path, PATH_SEGMENT).to_string();
        FileReference::Source(
            github_file_uri(repo_client.repo(), sha, &encoded_path, private).into(),
        )
    };

    while let Some(file) = files.try_next().await? {
        let old_path = file.previous_filename.as_deref().unwrap_or(&file.filename);
        let snapshot = Snapshot {
            path: file.filename.clone().into(),
            old: (file.status != DiffEntryStatus::Added).then(|| uri(&pr.base.sha, old_path)),
            new: (file.status != DiffEntryStatus::Removed)
                .then(|| uri(&pr.head.sha, &file.filename)),
            diff: None,
        };
        sender.send(Some(Ok(snapshot))).ok();
    }

    Ok(())
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
