use bytes::Bytes;
use futures::SinkExt;
use http_body_util::BodyExt;
use http_body_util::combinators::BoxBody;
use octocrab::{AuthState, LayerReady, NoAuth, NoConfig, NoSvc, NotLayerReady, OctocrabBuilder};
use std::pin::Pin;
use std::task::Poll;
use tower::Service;

pub fn wasm_builder() -> octocrab::OctocrabBuilder<WasmClient, NoConfig, NoAuth, LayerReady> {
    let builder = octocrab::OctocrabBuilder::new_empty()
        .with_service(WasmClient(tonic_web_wasm_client::Client::new(
            "https://api.github.com".to_owned(),
        )))
        .with_executor(Box::new(wasm_bindgen_futures::spawn_local));

    builder
}

pub struct WasmClient(tonic_web_wasm_client::Client);

impl<Body: http_body::Body + 'static + Send> tower::Service<http::Request<Body>> for WasmClient {
    type Response = http::Response<BoxBody<Bytes, tonic_web_wasm_client::Error>>;
    type Error = tonic_web_wasm_client::Error;
    type Future = std::pin::Pin<
        Box<dyn std::future::Future<Output = Result<Self::Response, Self::Error>> + Send>,
    >;

    fn poll_ready(&mut self, cx: &mut std::task::Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, req: http::Request<Body>) -> Self::Future {
        let mut client = self.0.clone();

        Box::pin(async move {
            let (tx, rx) = futures::channel::oneshot::channel();

            wasm_bindgen_futures::spawn_local(async move {
                let result = call(client, req).await;
                tx.send(result).ok();
            });

            let response = rx.await.map_err(|_| {
                tonic_web_wasm_client::Error::JsError("Failed to receive response".to_string())
            })??;

            Ok(response)
        })
    }
}

pub async fn call<Body>(
    mut client: tonic_web_wasm_client::Client,
    request: http::Request<Body>,
) -> Result<
    http::Response<BoxBody<Bytes, tonic_web_wasm_client::Error>>,
    tonic_web_wasm_client::Error,
>
where
    Body: http_body::Body + 'static + Send,
{
    let (parts, body) = request.into_parts();
    let body = body
        .collect()
        .await
        .map_err(|e| tonic_web_wasm_client::Error::JsError("Failed to get body".to_string()))?
        .to_bytes();

    let body = http_body_util::Full::new(body);

    let req = http::Request::from_parts(parts, tonic::body::Body::new(body));

    let response = client.call(req).await?;

    let response = response.map(|b| BoxBody::new(b));

    Ok(response)
}
