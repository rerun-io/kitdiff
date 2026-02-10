use crate::github::model::{GithubPrLink, GithubRepoLink};
use crate::github::octokit::RepoClient;
use crate::github::pr::{GithubPr, pr_ui};
use crate::loaders::{LoadSnapshots, sort_snapshots};
use crate::snapshot::{FileReference, Snapshot};
use crate::state::AppStateRef;
use eframe::egui::{Context, Ui};
use egui_inbox::{UiInbox, UiInboxSender};
use futures::{StreamExt as _, TryStreamExt as _};
use octocrab::models::repos::DiffEntryStatus;
use octocrab::{Octocrab, Result};
use std::pin::pin;
use std::task::Poll;

type Sender = UiInboxSender<Option<Result<Snapshot>>>;

pub struct PrLoader {
    snapshots: Vec<Snapshot>,
    inbox: UiInbox<Option<Result<Snapshot>>>,
    state: Poll<anyhow::Result<()>>,
    link: GithubPrLink,
    pr_info: GithubPr,
    logged_in: bool,
}

impl PrLoader {
    pub fn new(link: GithubPrLink, client: Octocrab, logged_in: bool) -> Self {
        let mut inbox = UiInbox::new();
        let repo_client = RepoClient::new(client.clone(), link.repo.clone());

        inbox.spawn(|tx| async move {
            let result =
                stream_files(repo_client, link.pr_number, tx.clone(), logged_in).await;
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
            logged_in,
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

    let results = stream
        .try_filter_map(|file| async move {
            Ok(file.filename.ends_with(".png").then_some(file))
        })
        .map_ok(|file| {
            let repo_client = &repo_client;
            let pr = &pr;
            async move {
                let (old_url, new_url) = futures::join!(
                    async {
                        if file.status != DiffEntryStatus::Added {
                            let name =
                                file.previous_filename.as_deref().unwrap_or(&*file.filename);
                            resolve_url(repo_client, &pr.base.sha, name, logged_in).await
                        } else {
                            None
                        }
                    },
                    async {
                        if file.status != DiffEntryStatus::Removed {
                            resolve_url(repo_client, &pr.head.sha, &file.filename, logged_in)
                                .await
                        } else {
                            None
                        }
                    },
                );

                Ok::<_, octocrab::Error>(Snapshot {
                    path: file.filename.clone().into(),
                    old: old_url.map(|url| FileReference::Source(url.into())),
                    new: new_url.map(|url| FileReference::Source(url.into())),
                    diff: None,
                })
            }
        })
        .try_buffer_unordered(4);
    let mut results = pin!(results);

    while let Some(snapshot) = results.next().await.transpose()? {
        sender.send(Some(Ok(snapshot))).ok();
    }

    Ok(())
}

/// When logged in, uses the GitHub contents API to get a signed download URL
/// that works for private repos. Otherwise, falls back to the public
/// media.githubusercontent.com URL to avoid burning API rate limit.
async fn resolve_url(
    repo_client: &RepoClient,
    commit_sha: &str,
    file_path: &str,
    logged_in: bool,
) -> Option<String> {
    if logged_in {
        let content = repo_client
            .repos()
            .get_content()
            .path(file_path)
            .r#ref(commit_sha)
            .send()
            .await
            .ok()?;
        content.items.first()?.download_url.clone()
    } else {
        Some(create_media_url(repo_client.repo(), commit_sha, file_path))
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
        *self = Self::new(self.link.clone(), client, self.logged_in);
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
