use bytes::Bytes;
use http::uri::{Authority, Scheme};
use http_body_util::BodyExt;
use http_body_util::combinators::BoxBody;
use std::task::Poll;

#[derive(Clone)]
pub struct ReqwestTowerService {
    pub base_url: Option<(Scheme, Authority)>,
    pub client: reqwest::Client,
}

#[derive(thiserror::Error, Debug)]
pub enum ReqwestTowerError<Body>
where
    Body: http_body::Body + 'static + Send,
    Body::Error: Send,
{
    #[error("Reqwest error: {0}")]
    ReqwestError(#[from] reqwest::Error),
    #[error("HTTP error: {0}")]
    BodyError(Body::Error),
    #[error("HTTP error: {0}")]
    FutureCancelled(#[from] futures::channel::oneshot::Canceled),
    #[error("Invalid URI parts: {0}")]
    InvalidUri(#[from] http::uri::InvalidUriParts),
}

impl<Body: http_body::Body + 'static + Send> tower::Service<http::Request<Body>>
    for ReqwestTowerService
where
    Body::Error: Send + 'static,
{
    type Response = http::Response<BoxBody<Bytes, std::convert::Infallible>>;
    type Error = ReqwestTowerError<Body>;
    type Future = std::pin::Pin<
        Box<dyn std::future::Future<Output = Result<Self::Response, Self::Error>> + Send>,
    >;

    fn poll_ready(&mut self, cx: &mut std::task::Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, req: http::Request<Body>) -> Self::Future {
        let Self {
            base_url: base_url,
            client: client,
        } = self.clone();

        Box::pin(async move {
            let (tx, rx) = futures::channel::oneshot::channel();

            wasm_bindgen_futures::spawn_local(async move {
                let result = call(client, base_url, req).await;
                tx.send(result).ok();
            });

            let response = rx.await??;

            Ok(response)
        })
    }
}

pub async fn call<Body>(
    client: reqwest::Client,
    base_url: Option<(Scheme, Authority)>,
    request: http::Request<Body>,
) -> Result<http::Response<BoxBody<Bytes, std::convert::Infallible>>, ReqwestTowerError<Body>>
where
    Body: http_body::Body + 'static + Send,
    Body::Error: Send + 'static,
{
    let (parts, body) = request.into_parts();
    let body = body
        .collect()
        .await
        .map_err(ReqwestTowerError::BodyError)?
        .to_bytes();

    let mut uri = parts.uri;

    let mut uri_parts = uri.into_parts();
    if uri_parts.authority.is_none() {
        if let Some((scheme, authority)) = base_url {
            uri_parts.scheme = Some(scheme);
            uri_parts.authority = Some(authority);
        }
    }

    let request = client
        .request(
            parts.method,
            http::uri::Uri::from_parts(uri_parts)?.to_string(),
        )
        .body(body)
        .headers(parts.headers)
        .build()?;

    let reqwest_response = client.execute(request).await?;

    let headers = reqwest_response.headers().clone();

    let bytes = reqwest_response.bytes().await?;
    let mut response = http::Response::new(BoxBody::new(http_body_util::Full::new(bytes)));
    *response.headers_mut() = headers;

    Ok(response)
}
