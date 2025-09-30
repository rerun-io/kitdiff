use crate::github::model::{GithubPrLink, GithubRepoLink};
use crate::github::octokit::RepoClient;
use crate::github::pr::{GithubPr, pr_ui};
use crate::loaders::{LoadSnapshots, SnapshotLoader, sort_snapshots};
use crate::snapshot::{FileReference, Snapshot};
use crate::state::AppStateRef;
use eframe::egui::{Context, Ui};
use egui_inbox::{UiInbox, UiInboxSender};
use futures::StreamExt;
use octocrab::models::repos::{DiffEntry, DiffEntryStatus};
use octocrab::{Error, Octocrab, Result};
use std::ops::Deref;
use std::path::Path;
use std::pin::pin;
use std::task::Poll;

type Sender = UiInboxSender<Option<Result<Snapshot>>>;

pub struct PrLoader {
    snapshots: Vec<Snapshot>,
    inbox: UiInbox<Option<Result<Snapshot>>>,
    state: Poll<anyhow::Result<()>>,
    link: GithubPrLink,
    pr_info: GithubPr,
}

impl PrLoader {
    pub fn new(link: GithubPrLink, client: Octocrab) -> Self {
        let mut inbox = UiInbox::new();
        let repo_client = RepoClient::new(client.clone(), link.repo.clone());

        inbox.spawn(|tx| async move {
            let result = stream_files(repo_client, link.pr_number, tx.clone()).await;
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
            pr_info: GithubPr::new(link.clone(), client),
            link,
        }
    }
}

async fn stream_files(
    repo_client: RepoClient,
    pr_number: u64,
    sender: Sender,
) -> octocrab::Result<()> {
    let pr = repo_client.pulls().get(pr_number).await?;

    let file = repo_client.pulls().list_files(pr_number).await?;

    let stream = file.into_stream(&repo_client);

    let mut stream = pin!(stream);

    while let Some(file) = stream.next().await.transpose()? {
        let file: DiffEntry = file;
        if file.filename.ends_with(".png") {
            let old_url = if file.status != DiffEntryStatus::Added {
                let old_file_name = file
                    .previous_filename
                    .as_deref()
                    .unwrap_or(file.filename.deref());
                Some(create_media_url(
                    repo_client.repo(),
                    &pr.base.sha,
                    old_file_name,
                ))
            } else {
                None
            };

            let new_url = if file.status != DiffEntryStatus::Removed {
                Some(create_media_url(
                    repo_client.repo(),
                    &pr.head.sha,
                    &file.filename,
                ))
            } else {
                None
            };

            let snapshot = Snapshot {
                path: file.filename.clone().into(),
                old: old_url.map(|url| FileReference::Source(url.into())),
                new: new_url.map(|url| FileReference::Source(url.into())),
                diff: None,
            };
            sender.send(Some(Ok(snapshot))).ok();
        }
    }

    Ok(())
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
