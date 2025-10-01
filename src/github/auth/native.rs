use crate::github::auth::{AuthSender, GitHubAuth, parse_supabase_fragment};
use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{Html, Response};
use eframe::egui::{Context, OpenUrl};
use std::net::{Ipv4Addr, SocketAddrV4};
use tokio::spawn;

pub fn login_github(ctx: &Context, tx: AuthSender) {
    let ctx = ctx.clone();
    spawn(async move {
        if let Err(err) = login(ctx, tx).await {
            eprintln!("Error during GitHub login: {err:?}");
        }
    });
}

#[expect(clippy::needless_pass_by_value)]
pub fn check_for_auth_callback(_sender: AuthSender) {
    // Not implemented for native
}

pub async fn login(ctx: Context, tx: AuthSender) -> anyhow::Result<()> {
    let listener = tokio::net::TcpListener::bind(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0)).await?;

    let addr = listener.local_addr()?;

    ctx.open_url(OpenUrl::new_tab(GitHubAuth::auth_url(&format!(
        "http://{addr}"
    ))));

    let router = axum::Router::new()
        .route("/", axum::routing::get(home_route))
        .route("/api/auth", axum::routing::post(auth_route))
        .with_state(tx);

    axum::serve(listener, router).await?;

    Ok(())
}

pub async fn home_route() -> Html<&'static str> {
    Html(include_str!("handle_redirect.html"))
}

#[derive(serde::Deserialize)]
struct AuthBody {
    fragment: String,
}

async fn auth_route(
    State(tx): State<AuthSender>,
    Json(body): Json<AuthBody>,
) -> Result<String, Response<String>> {
    let fragment = body.fragment;

    let data = parse_supabase_fragment(&fragment).map_err(|e| {
        Response::builder()
            .status(StatusCode::INTERNAL_SERVER_ERROR)
            .body(e.to_string())
            .expect("Failed to build error response")
    })?;

    GitHubAuth::handle_callback_fragment(tx, data).await;

    Ok("Success".to_owned())
}
