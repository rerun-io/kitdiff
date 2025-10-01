use crate::github::model::{GithubArtifactLink, GithubRepoLink};
use crate::state::SystemCommand;
use eframe::egui;
use eframe::egui::{Context, ViewportCommand};
use egui_inbox::{UiInbox, UiInboxSender};
use ehttp;
use octocrab::models::{ArtifactId, Author};
use serde_json;
use std::fmt;
use std::sync::mpsc;

#[cfg(target_arch = "wasm32")]
#[path = "auth/wasm.rs"]
mod auth_impl;
#[cfg(not(target_arch = "wasm32"))]
#[path = "auth/native.rs"]
mod auth_impl;

pub enum GithubAuthCommand {
    Login,
    Logout,
}

impl From<GithubAuthCommand> for SystemCommand {
    fn from(cmd: GithubAuthCommand) -> Self {
        Self::GithubAuth(cmd)
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct AuthState {
    pub logged_in: Option<LoggedInState>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct LoggedInState {
    pub supabase_token: String,
    pub github_token: String, // GitHub OAuth token
    expires_at: u64,
    pub username: String,
    pub user_image: Option<String>,
}

#[derive(Debug)]
pub struct GitHubAuth {
    state: AuthState,
    inbox: UiInbox<AuthEvent>,
    sender: UiInboxSender<SystemCommand>,
}

impl GitHubAuth {
    fn make_client(token: Option<&str>) -> octocrab::Octocrab {
        let builder = octocrab_wasm::builder();

        let mut client = builder.build().expect("Failed to build Octocrab client");

        if let Some(token) = token {
            client = client
                .user_access_token(token.to_owned())
                .expect("Invalid token");
        }

        client
    }

    pub fn client(&self) -> octocrab::Octocrab {
        Self::make_client(self.get_token())
    }
}

#[derive(Debug, Clone)]
pub enum AuthEvent {
    LoginSuccessful(AuthState),
    Error(String),
}

pub type AuthSender = UiInboxSender<AuthEvent>;

// Helper function to get current timestamp in seconds
fn get_current_timestamp() -> u64 {
    #[cfg(target_arch = "wasm32")]
    {
        // Use JavaScript Date.now() for WASM
        (js_sys::Date::now() / 1000.0) as u64
    }

    #[cfg(not(target_arch = "wasm32"))]
    {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs()
    }
}

// URL parsing utilities
pub fn parse_github_artifact_url(url: &str) -> Option<GithubArtifactLink> {
    // Expected format: github.com/owner/repo/actions/runs/12345/artifacts/67890
    let url = url
        .trim_start_matches("https://")
        .trim_start_matches("http://");

    let parts: Vec<&str> = url.split('/').collect();
    if parts.len() >= 7
        && parts[0] == "github.com"
        && parts[3] == "actions"
        && parts[4] == "runs"
        && parts[6] == "artifacts"
        && parts.len() >= 8
    {
        let owner = parts[1].to_owned();
        let repo = parts[2].to_owned();
        Some(GithubArtifactLink {
            repo: GithubRepoLink { owner, repo },
            artifact_id: ArtifactId(parts[7].parse().ok()?),
            name: None,
            branch_name: None,
            run_id: None,
        })
    } else {
        None
    }
}

pub fn github_artifact_api_url(owner: &str, repo: &str, artifact_id: &str) -> String {
    format!("https://api.github.com/repos/{owner}/{repo}/actions/artifacts/{artifact_id}/zip")
}

#[derive(serde::Deserialize)]
struct AuthFragment {
    access_token: String,
    provider_token: String, // The github token
    expires_at: u64,
}

fn parse_supabase_fragment(fragment: &str) -> anyhow::Result<AuthFragment> {
    Ok(serde_urlencoded::from_str(fragment)?)
}

impl GitHubAuth {
    pub const SUPABASE_URL: &'static str = "https://fqhsaeyjqrjmlkqflvho.supabase.co";
    pub const SUPABASE_ANON_KEY: &'static str = "eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.eyJpc3MiOiJzdXBhYmFzZSIsInJlZiI6ImZxaHNhZXlqcXJqbWxrcWZsdmhvIiwicm9sZSI6ImFub24iLCJpYXQiOjE3NTgyMTk4MzIsImV4cCI6MjA3Mzc5NTgzMn0.TuhMjHhBCNyKquyVWq3djOfpBVDhcpSmNRWSErpseuw";

    pub fn new(state: AuthState, sender: UiInboxSender<SystemCommand>) -> Self {
        let this = Self {
            state,
            inbox: UiInbox::new(),
            sender,
        };

        auth_impl::check_for_auth_callback(this.inbox.sender());

        this
    }

    pub fn handle(&mut self, ctx: &Context, cmd: GithubAuthCommand) {
        match cmd {
            GithubAuthCommand::Login => auth_impl::login_github(&ctx, self.inbox.sender()),
            GithubAuthCommand::Logout => {
                self.logout();
            }
        }
    }

    pub fn auth_url(origin: &str) -> String {
        #[derive(serde::Serialize)]
        struct AuthParams {
            redirect_to: String,
            provider: String,
            scopes: String,
        }

        let query = serde_urlencoded::to_string(&AuthParams {
            redirect_to: origin.to_string(),
            provider: "github".to_string(),
            scopes: "repo".to_string(),
        }).unwrap_or_default();

        format!(
            "{}/auth/v1/authorize?{}",
            Self::SUPABASE_URL,
            query
        )
    }

    async fn handle_callback_fragment(tx: AuthSender, data: AuthFragment) {
        let username = GitHubAuth::fetch_user_info(&data.provider_token).await;

        match username {
            Ok(username) => {
                tx.send(AuthEvent::LoginSuccessful(AuthState {
                    logged_in: Some(LoggedInState {
                        github_token: data.provider_token,
                        supabase_token: data.access_token,
                        expires_at: data.expires_at,
                        username: username.login,
                        user_image: Some(username.avatar_url.to_string()),
                    }),
                }))
                .ok();
            }
            Err(err) => {
                tx.send(AuthEvent::Error(format!(
                    "Failed to fetch user info: {}",
                    err
                )))
                .ok();
            }
        }
    }

    async fn fetch_user_info(token: &str) -> anyhow::Result<Author> {
        let client = GitHubAuth::make_client(Some(token));
        let user = client.current().user().await?;

        Ok(user)
    }

    pub fn is_authenticated(&self) -> bool {
        if let Some(state) = &self.state.logged_in {
            let now = get_current_timestamp();
            return now < state.expires_at;
        }
        false
    }

    pub fn get_username(&self) -> Option<&str> {
        self.state.logged_in.as_ref().map(|s| s.username.as_str())
    }

    pub fn get_token(&self) -> Option<&str> {
        if self.is_authenticated() {
            self.state
                .logged_in
                .as_ref()
                .map(|s| s.github_token.as_str())
        } else {
            None
        }
    }

    pub fn logout(&mut self) {
        self.state.logged_in = None;
    }

    pub fn get_auth_state(&self) -> &AuthState {
        &self.state
    }

    pub fn update(&mut self, _ctx: &egui::Context) {
        // Check for messages from auth flow
        for event in self.inbox.read(_ctx) {
            match event {
                AuthEvent::LoginSuccessful(state) => {
                    self.state = state;
                    _ctx.send_viewport_cmd(ViewportCommand::Focus);
                    self.sender.send(SystemCommand::Refresh).ok();
                }
                AuthEvent::Error(error) => {
                    eprintln!("Auth error: {error}");
                }
            }
        }
    }
}
