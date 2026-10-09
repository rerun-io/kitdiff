//! Loads the images of a GitHub PR.
//!
//! For public repositories, this costs no GitHub API calls.
//! For private ones, it costs one call per image, made only when the image is needed.

use crate::github::model::GithubRepoLink;
use ahash::HashMap;
use eframe::egui::Context;
use eframe::egui::load::{BytesLoadResult, BytesLoader, BytesPoll, LoadError};
use eframe::egui::mutex::Mutex;
use egui_extras::loaders::http_loader::EhttpLoader;
use octocrab::Octocrab;
use std::sync::Arc;
use std::time::Duration;
use web_time::Instant;

const PUBLIC_SCHEME: &str = "github-file://";
const PRIVATE_SCHEME: &str = "github-private-file://";

/// GitHub makes signed download URLs expire after an hour.
/// After this time, kitdiff gets a new one before it starts a download.
const SIGNED_URL_MAX_AGE: Duration = Duration::from_secs(50 * 60);

/// How many signed URLs [`GithubFileLoader::prefetch`] looks up at the same time.
const MAX_PREFETCH_LOOKUPS: usize = 8;

/// The URI of a file at a commit, e.g. `github-file://rerun-io/kitdiff/<sha>/tests/snapshots/home.png`.
///
/// `encoded_path` must be percent-encoded.
///
/// The URI does not change when the URL kitdiff downloads the file from changes.
/// So egui can cache the image, and the diff of two images, by it.
pub fn github_file_uri(
    repo: &GithubRepoLink,
    sha: &str,
    encoded_path: &str,
    private: bool,
) -> String {
    let scheme = if private {
        PRIVATE_SCHEME
    } else {
        PUBLIC_SCHEME
    };
    format!("{scheme}{}/{}/{sha}/{encoded_path}", repo.owner, repo.repo)
}

/// A parsed [`github_file_uri`].
#[derive(Clone)]
struct GithubFile {
    owner: String,
    repo: String,
    sha: String,
    encoded_path: String,
    private: bool,
}

impl GithubFile {
    fn parse(uri: &str) -> Option<Self> {
        let (rest, private) = if let Some(rest) = uri.strip_prefix(PUBLIC_SCHEME) {
            (rest, false)
        } else {
            (uri.strip_prefix(PRIVATE_SCHEME)?, true)
        };
        let mut parts = rest.splitn(4, '/');
        Some(Self {
            owner: parts.next()?.to_owned(),
            repo: parts.next()?.to_owned(),
            sha: parts.next()?.to_owned(),
            encoded_path: parts.next()?.to_owned(),
            private,
        })
    }

    /// Serves files in git-lfs, and 404 for all other files.
    fn media_url(&self) -> String {
        format!(
            "https://media.githubusercontent.com/media/{}/{}/{}/{}",
            self.owner, self.repo, self.sha, self.encoded_path
        )
    }

    /// Serves all files, but only the pointer of a file in git-lfs.
    fn raw_url(&self) -> String {
        format!(
            "https://raw.githubusercontent.com/{}/{}/{}/{}",
            self.owner, self.repo, self.sha, self.encoded_path
        )
    }

    /// Asks the contents API for a download URL. For a private repository,
    /// the URL holds a token that is valid for this one file, for an hour.
    async fn signed_url(&self, client: &Octocrab) -> anyhow::Result<String> {
        let content = client
            .repos(&self.owner, &self.repo)
            .get_content()
            .path(&self.encoded_path)
            .r#ref(&self.sha)
            .send()
            .await?;
        content
            .items
            .into_iter()
            .next()
            .and_then(|item| item.download_url)
            .ok_or_else(|| anyhow::anyhow!("GitHub gave no download URL for {}", self.encoded_path))
    }
}

enum Lookup {
    Pending,
    Ready {
        url: String,
        time: Instant,

        /// A download from `url` has started, so the URL must not change any more.
        used: bool,
    },
    Failed(String),
}

#[derive(Default)]
struct Lookups {
    by_uri: HashMap<String, Lookup>,

    /// The number of [`Lookup::Pending`] entries.
    pending: usize,
}

/// Loads [`github_file_uri`]s.
pub struct GithubFileLoader {
    /// Our own, because this loader may not call [`Context::try_load_bytes`]:
    /// egui holds a lock on the bytes loaders while one of them runs.
    http: EhttpLoader,

    /// Looks up the download URLs of files in private repositories.
    client: Mutex<Octocrab>,
    lookups: Arc<Mutex<Lookups>>,
}

impl GithubFileLoader {
    pub fn new(client: Octocrab) -> Self {
        Self {
            http: EhttpLoader::default(),
            client: Mutex::new(client),
            lookups: Arc::default(),
        }
    }

    /// Use this client for the next lookups, e.g. after the user logged in.
    pub fn set_client(&self, client: Octocrab) {
        *self.client.lock() = client;
    }

    /// Looks up the download URL of a file in a private repository ahead of time,
    /// so that the image loads fast once it is shown.
    ///
    /// Does nothing for other URIs, and while [`MAX_PREFETCH_LOOKUPS`] lookups run.
    pub fn prefetch(&self, ctx: &Context, uri: &str) {
        let Some(file) = GithubFile::parse(uri).filter(|file| file.private) else {
            return;
        };
        if self.lookups.lock().pending < MAX_PREFETCH_LOOKUPS {
            self.signed_url(ctx, uri, &file, false).ok();
        }
    }

    /// Gives the download URL of a file in a private repository,
    /// or `None` while kitdiff looks it up.
    ///
    /// Set `download` if a download from the URL starts now.
    fn signed_url(
        &self,
        ctx: &Context,
        uri: &str,
        file: &GithubFile,
        download: bool,
    ) -> Result<Option<String>, LoadError> {
        let mut lookups = self.lookups.lock();
        match lookups.by_uri.get_mut(uri) {
            Some(Lookup::Ready { url, time, used })
                if *used || time.elapsed() < SIGNED_URL_MAX_AGE =>
            {
                *used |= download;
                return Ok(Some(url.clone()));
            }
            Some(Lookup::Pending) => return Ok(None),
            Some(Lookup::Failed(err)) => return Err(LoadError::Loading(err.clone())),
            Some(Lookup::Ready { .. }) | None => {}
        }
        lookups.by_uri.insert(uri.to_owned(), Lookup::Pending);
        lookups.pending += 1;
        drop(lookups);

        let client = self.client.lock().clone();
        let lookups = Arc::clone(&self.lookups);
        let ctx = ctx.clone();
        let uri = uri.to_owned();
        let file = file.clone();
        hello_egui_utils::spawn(async move {
            let lookup = match file.signed_url(&client).await {
                Ok(url) => Lookup::Ready {
                    url,
                    time: Instant::now(),
                    used: false,
                },
                Err(err) => Lookup::Failed(format!("{err:#}")),
            };
            {
                let mut lookups = lookups.lock();
                // `forget` may have removed the entry in the meantime.
                if matches!(lookups.by_uri.get(&uri), Some(Lookup::Pending)) {
                    lookups.by_uri.insert(uri, lookup);
                    lookups.pending -= 1;
                }
            }
            // Not while we hold the lock, see `ImageLoader::load`.
            ctx.request_repaint();
        });

        Ok(None)
    }
}

impl BytesLoader for GithubFileLoader {
    fn id(&self) -> &'static str {
        eframe::egui::generate_loader_id!(GithubFileLoader)
    }

    fn load(&self, ctx: &Context, uri: &str) -> BytesLoadResult {
        let Some(file) = GithubFile::parse(uri) else {
            return Err(LoadError::NotSupported);
        };

        if file.private {
            match self.signed_url(ctx, uri, &file, true)? {
                Some(url) => self.http.load(ctx, &url),
                None => Ok(BytesPoll::Pending { size: None }),
            }
        } else {
            // Most snapshots are in git-lfs, so try that first.
            match self.http.load(ctx, &file.media_url()) {
                Err(LoadError::Loading(_)) => self.http.load(ctx, &file.raw_url()),
                result => result,
            }
        }
    }

    fn forget(&self, uri: &str) {
        let Some(file) = GithubFile::parse(uri) else {
            return;
        };
        let mut lookups = self.lookups.lock();
        match lookups.by_uri.remove(uri) {
            Some(Lookup::Ready { url, .. }) => self.http.forget(&url),
            Some(Lookup::Pending) => lookups.pending -= 1,
            Some(Lookup::Failed(_)) | None => {}
        }
        if !file.private {
            self.http.forget(&file.media_url());
            self.http.forget(&file.raw_url());
        }
    }

    fn forget_all(&self) {
        *self.lookups.lock() = Lookups::default();
        self.http.forget_all();
    }

    fn byte_size(&self) -> usize {
        self.http.byte_size()
    }

    fn has_pending(&self) -> bool {
        self.http.has_pending() || 0 < self.lookups.lock().pending
    }
}
