use ahash::HashMap;
use eframe::egui::load::{Bytes, BytesLoadResult, BytesLoader, BytesPoll, LoadError};
use eframe::egui::mutex::Mutex;
use eframe::egui::{self, Context};
use std::sync::Arc;
use std::task::Poll;

const PROTOCOL: &str = "github-content://";

type Entry = Poll<Result<Arc<[u8]>, String>>;

/// Loads files from GitHub repos through the contents API, with the user's token.
///
/// We don't use the `download_url` of the contents API, because for private repos it holds
/// a token that expires after a short time. Images that the user opens later would fail to load.
///
/// URIs look like `github-content://{owner}/{repo}/{sha}/{percent-encoded path}`.
#[derive(Default)]
pub struct GithubContentLoader {
    token: Mutex<Option<String>>,
    cache: Arc<Mutex<HashMap<String, Entry>>>,
}

impl GithubContentLoader {
    pub const ID: &'static str = egui::generate_loader_id!(GithubContentLoader);

    pub fn uri(owner: &str, repo: &str, sha: &str, encoded_path: &str) -> String {
        format!("{PROTOCOL}{owner}/{repo}/{sha}/{encoded_path}")
    }

    /// Sets the token to load with. If it changed, forget all files, so failed loads retry.
    pub fn set_token(&self, token: Option<&str>) {
        let mut current = self.token.lock();
        if current.as_deref() != token {
            *current = token.map(str::to_owned);
            drop(current);
            self.forget_all();
        }
    }
}

fn api_url(uri: &str) -> Option<String> {
    let rest = uri.strip_prefix(PROTOCOL)?;
    let mut parts = rest.splitn(4, '/');
    let (owner, repo, sha, path) = (parts.next()?, parts.next()?, parts.next()?, parts.next()?);
    Some(format!(
        "https://api.github.com/repos/{owner}/{repo}/contents/{path}?ref={sha}"
    ))
}

const LFS_POINTER_PREFIX: &[u8] = b"version https://git-lfs.github.com/spec/v1";

async fn fetch(url: String, token: Option<String>) -> Result<Arc<[u8]>, String> {
    let bytes = get(&url, token.as_deref(), Some("application/vnd.github.raw")).await?;
    if !bytes.starts_with(LFS_POINTER_PREFIX) {
        return Ok(bytes.into());
    }

    // The raw contents of a git-lfs file are its pointer. The `download_url` of the
    // contents metadata points to the object on media.githubusercontent.com instead.
    // We ask for it only now, so its short-lived token is fresh.
    let metadata = get(&url, token.as_deref(), None).await?;
    let metadata: serde_json::Value =
        serde_json::from_slice(&metadata).map_err(|err| err.to_string())?;
    let download_url = metadata["download_url"]
        .as_str()
        .ok_or_else(|| format!("No download_url for git-lfs file {url}"))?;
    // The token is in the URL, so we don't send our own.
    Ok(get(download_url, None, None).await?.into())
}

async fn get(url: &str, token: Option<&str>, accept: Option<&str>) -> Result<Vec<u8>, String> {
    let mut request = reqwest::Client::new()
        .get(url)
        // The API rejects requests without a User-Agent. Browsers set their own.
        .header(reqwest::header::USER_AGENT, "kitdiff");
    if let Some(accept) = accept {
        request = request.header(reqwest::header::ACCEPT, accept);
    }
    if let Some(token) = token {
        request = request.bearer_auth(token);
    }
    let response = request.send().await.map_err(|err| err.to_string())?;
    let status = response.status();
    if !status.is_success() {
        let text = response.text().await.unwrap_or_default();
        return Err(format!("Failed to load {url}: {status} {text}"));
    }
    let bytes = response.bytes().await.map_err(|err| err.to_string())?;
    Ok(bytes.to_vec())
}

impl BytesLoader for GithubContentLoader {
    fn id(&self) -> &str {
        Self::ID
    }

    fn load(&self, ctx: &Context, uri: &str) -> BytesLoadResult {
        if !uri.starts_with(PROTOCOL) {
            return Err(LoadError::NotSupported);
        }

        let mut cache = self.cache.lock();
        if let Some(entry) = cache.get(uri) {
            return match entry {
                // No mime: the contents API answers with `application/vnd.github.raw`,
                // so the image loader must detect the format from the URI's extension.
                Poll::Ready(Ok(bytes)) => Ok(BytesPoll::Ready {
                    size: None,
                    bytes: Bytes::Shared(bytes.clone()),
                    mime: None,
                }),
                Poll::Ready(Err(err)) => Err(LoadError::Loading(err.clone())),
                Poll::Pending => Ok(BytesPoll::Pending { size: None }),
            };
        }

        let Some(url) = api_url(uri) else {
            return Err(LoadError::Loading(format!("Invalid URI: {uri}")));
        };

        let uri = uri.to_owned();
        cache.insert(uri.clone(), Poll::Pending);
        drop(cache);

        let token = self.token.lock().clone();
        let cache = Arc::clone(&self.cache);
        let ctx = ctx.clone();
        hello_egui_utils::spawn(async move {
            let result = fetch(url, token).await;
            let repaint = {
                let mut cache = cache.lock();
                // The entry is gone if the loader forgot it while loading.
                if let Some(entry) = cache.get_mut(&uri) {
                    *entry = Poll::Ready(result);
                    true
                } else {
                    false
                }
            };
            // We may not lock the Context while we hold the cache lock.
            if repaint {
                ctx.request_repaint();
            }
        });

        Ok(BytesPoll::Pending { size: None })
    }

    fn forget(&self, uri: &str) {
        self.cache.lock().remove(uri);
    }

    fn forget_all(&self) {
        self.cache.lock().clear();
    }

    fn byte_size(&self) -> usize {
        self.cache
            .lock()
            .values()
            .map(|entry| match entry {
                Poll::Ready(Ok(bytes)) => bytes.len(),
                Poll::Ready(Err(err)) => err.len(),
                Poll::Pending => 0,
            })
            .sum()
    }

    fn has_pending(&self) -> bool {
        self.cache.lock().values().any(Poll::is_pending)
    }
}
