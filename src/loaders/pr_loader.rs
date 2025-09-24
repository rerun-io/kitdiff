use crate::github_model::{GithubPrLink, GithubRepoLink};
use crate::loaders::{LoadSnapshots, SnapshotLoader};
use crate::octokit::RepoClient;
use crate::snapshot::{FileReference, Snapshot};
use anyhow::Error;
use eframe::egui::Context;
use egui_inbox::{UiInbox, UiInboxSender};
use futures::StreamExt;
use octocrab::Octocrab;
use octocrab::models::repos::{DiffEntry, DiffEntryStatus};
use std::ops::Deref;
use std::path::Path;
use std::pin::pin;
use std::task::Poll;

pub struct PrLoader {
    snapshots: Vec<Snapshot>,
    inbox: UiInbox<Option<Snapshot>>,
    loading: bool,
    link: GithubPrLink,
}

impl PrLoader {
    pub fn new(link: GithubPrLink, client: Octocrab) -> Self {
        let mut inbox = UiInbox::new();
        let repo_client = RepoClient::new(client, link.repo.clone());

        inbox.spawn(|tx| async move {
            let result = stream_files(repo_client, link.pr_number, tx).await;
            if let Err(e) = result {
                eprintln!("Error loading PR files: {}", e);
            }
        });

        Self {
            snapshots: Vec::new(),
            inbox,
            loading: true,
            link,
        }
    }
}

async fn stream_files(
    repo_client: RepoClient,
    pr_number: u64,
    sender: UiInboxSender<Option<Snapshot>>,
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
            dbg!(&snapshot);
            sender.send(Some(snapshot)).ok();
        }
    }

    sender.send(None).ok();

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
                Some(s) => self.snapshots.push(s),
                None => self.loading = false,
            }
        }
    }

    fn snapshots(&self) -> &[Snapshot] {
        &self.snapshots
    }

    fn state(&self) -> Poll<Result<(), &Error>> {
        if self.loading {
            Poll::Pending
        } else {
            Poll::Ready(Ok(()))
        }
    }
}
