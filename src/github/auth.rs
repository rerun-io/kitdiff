use crate::github::model::{GithubArtifactLink, GithubRepoLink};
use crate::state::SystemCommand;
use eframe::egui;
use eframe::egui::{Context, ViewportCommand};
use egui_inbox::{UiInbox, UiInboxSender};
use octocrab::models::{ArtifactId, Author};

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
    pub github_token: String,
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
pub(crate) struct AuthFragment {
    token: String,
}

pub(crate) fn parse_auth_fragment(fragment: &str) -> anyhow::Result<AuthFragment> {
    Ok(serde_urlencoded::from_str(fragment)?)
}

impl GitHubAuth {
    const GITHUB_CLIENT_ID: &'static str = "Iv23li8RPCGatBkgBNaE";
    const CALLBACK_URL: &'static str =
        "https://kitdiff-auth-720893688618.europe-west1.run.app/callback";
    pub const MANAGE_REPO_ACCESS_URL: &'static str =
        "https://github.com/apps/kitdiff/installations/new";

    pub fn new(state: AuthState, sender: UiInboxSender<SystemCommand>) -> Self {
        let this = Self {
            state,
            inbox: UiInbox::new(),
            sender,
        };

        auth_impl::check_for_auth_callback(this.inbox.sender());

        this
    }

    #[expect(clippy::needless_pass_by_value)]
    pub fn handle(&mut self, ctx: &Context, cmd: GithubAuthCommand) {
        match cmd {
            GithubAuthCommand::Login => auth_impl::login_github(ctx, self.inbox.sender()),
            GithubAuthCommand::Logout => {
                self.logout();
            }
        }
    }

    pub fn auth_url(origin: &str) -> String {
        #[derive(serde::Serialize)]
        struct AuthParams<'a> {
            client_id: &'a str,
            redirect_uri: &'a str,
            state: &'a str,
        }

        let query = serde_urlencoded::to_string(&AuthParams {
            client_id: Self::GITHUB_CLIENT_ID,
            redirect_uri: Self::CALLBACK_URL,
            state: origin,
        })
        .unwrap_or_default();

        format!("https://github.com/login/oauth/authorize?{query}")
    }

    async fn handle_callback_fragment(tx: AuthSender, data: AuthFragment) {
        let username = Self::fetch_user_info(&data.token).await;

        match username {
            Ok(username) => {
                tx.send(AuthEvent::LoginSuccessful(AuthState {
                    logged_in: Some(LoggedInState {
                        github_token: data.token,
                        username: username.login,
                        user_image: Some(username.avatar_url.to_string()),
                    }),
                }))
                .ok();
            }
            Err(err) => {
                tx.send(AuthEvent::Error(format!(
                    "Failed to fetch user info: {err}"
                )))
                .ok();
            }
        }
    }

    async fn fetch_user_info(token: &str) -> anyhow::Result<Author> {
        let client = Self::make_client(Some(token));
        let user = client.current().user().await?;

        Ok(user)
    }

    pub fn get_username(&self) -> Option<&str> {
        self.state.logged_in.as_ref().map(|s| s.username.as_str())
    }

    pub fn get_token(&self) -> Option<&str> {
        self.state
            .logged_in
            .as_ref()
            .map(|s| s.github_token.as_str())
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
                    log::error!("Auth error: {error}");
                }
            }
        }
    }
}
