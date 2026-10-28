use ahash::HashMap;
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use eframe::egui::load::{Bytes, BytesLoadResult, BytesLoader, BytesPoll, LoadError};
use eframe::egui::mutex::Mutex;
use eframe::egui::{self, Context};
use std::sync::Arc;
use std::task::Poll;

const PROTOCOL: &str = "github-content://";

type Entry = Poll<Result<Arc<[u8]>, String>>;

/// Loads files from GitHub repos through the contents API, with the user's token.
///
/// We don't store the `download_url` of the contents API, because for private repos it holds
/// a token that expires after a short time. Images that the user opens later would fail to load.
/// Instead, we ask the contents API when the image loads.
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

#[derive(serde::Deserialize)]
struct Content {
    /// Base64 with line breaks. Empty for files larger than 1 MB.
    content: String,
    download_url: Option<String>,
}

async fn fetch(url: String, token: Option<String>) -> Result<Arc<[u8]>, String> {
    let content = get(&url, Some("application/vnd.github+json"), token.as_deref()).await?;
    let content: Content = serde_json::from_slice(&content).map_err(|err| err.to_string())?;

    let encoded: String = content.content.split_whitespace().collect();
    let bytes = BASE64
        .decode(encoded)
        .map_err(|err| format!("Failed to decode {url}: {err}"))?;
    if !bytes.is_empty() && !bytes.starts_with(LFS_POINTER_PREFIX) {
        return Ok(bytes.into());
    }

    // The file is too large, or is a git-lfs pointer. Then we download it through
    // `download_url`, which for git-lfs files points to the object on
    // media.githubusercontent.com. Its token is short-lived, but we only got it now.
    let download_url = content
        .download_url
        .ok_or_else(|| format!("No download_url for {url}"))?;
    // The token is in the URL, so we don't send our own.
    Ok(get(&download_url, None, None).await?.into())
}

async fn get(url: &str, accept: Option<&str>, token: Option<&str>) -> Result<Vec<u8>, String> {
    let mut request = reqwest::Client::new()
        .get(url)
        // The API rejects requests without a User-Agent. Browsers set their own.
        .header(reqwest::header::USER_AGENT, "kitdiff");
    // GitHub sends the same ETag for the raw and the JSON response of the contents API.
    // So the browser may answer a JSON request with a cached raw response (e.g. from an
    // older kitdiff version that requested raw), after revalidating it with a 304.
    // The loader caches the files itself, so we skip the browser cache.
    #[cfg(target_arch = "wasm32")]
    {
        request = request.fetch_cache_no_store();
    }
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
                // No mime: the contents API doesn't tell us the file type, so the
                // image loader must detect the format itself.
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
