use crate::config::Config;
use crate::diff_image_loader::DiffImageLoader;
use crate::github::auth::{GitHubAuth, GithubAuthCommand};
use crate::github::model::GithubPrLink;
use crate::github::pr::GithubPr;
use crate::loaders::SnapshotLoader;
use crate::loaders::github_file_loader::GithubFileLoader;
use crate::settings::Settings;
use crate::snapshot::Snapshot;
use eframe::egui::{self, Context};
use egui_inbox::UiInboxSender;
use octocrab::Octocrab;
use std::ops::Deref;
use std::sync::Arc;

pub struct AppState {
    pub github_auth: GitHubAuth,

    /// Loads the images of PRs. Its client follows `github_auth`.
    pub github_files: Arc<GithubFileLoader>,
    pub github_pr: Option<GithubPr>,
    pub settings: Settings,

    /// From `--config`: used instead of the `kitdiff.toml` of each repository.
    pub config_override: Option<Config>,
    pub page: Page,
}

pub enum Page {
    Home,
    DiffViewer(ViewerState),
}

pub struct ViewerState {
    pub loader: SnapshotLoader,
    pub index: usize,

    /// If true, this item will scroll into view.
    pub index_just_selected: bool,
    pub filter: String,
    pub view: View,

    /// The viewer this one was opened from, e.g. the PR of an artifact.
    /// The back button returns to it as it was.
    pub back: Option<Box<Self>>,
}

impl ViewerState {
    fn filtered_snapshots(&self) -> Vec<FilteredSnapshot<'_>> {
        let filter = self.filter.to_lowercase();
        self.loader
            .snapshots()
            .iter()
            .enumerate()
            .filter(|(_, s)| {
                if filter.is_empty() {
                    true
                } else {
                    s.path.to_string_lossy().to_lowercase().contains(&filter)
                }
            })
            .collect()
    }
}

#[derive(Copy, Clone, Default, PartialEq, Eq)]
pub enum View {
    /// View all stacked on each other, with opacity settings.
    #[default]
    BlendAll,

    /// View old image
    Old,

    /// View new image
    New,

    /// View diff
    Diff,
}

impl std::fmt::Display for View {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::BlendAll => write!(f, "Blend all"),
            Self::Old => write!(f, "Old"),
            Self::New => write!(f, "New"),
            Self::Diff => write!(f, "Diff"),
        }
    }
}

impl View {
    pub const ALL: [Self; 4] = [Self::BlendAll, Self::Old, Self::New, Self::Diff];

    pub fn key(self) -> egui::Key {
        match self {
            Self::BlendAll => egui::Key::Num1,
            Self::Old => egui::Key::Num2,
            Self::New => egui::Key::Num3,
            Self::Diff => egui::Key::Num4,
        }
    }
}

impl AppState {
    pub fn new(
        settings: Settings,
        config_override: Option<Config>,
        sender: UiInboxSender<SystemCommand>,
    ) -> Self {
        let github_auth = GitHubAuth::new(settings.auth.clone(), sender);
        Self {
            github_files: Arc::new(GithubFileLoader::new(github_auth.client())),
            github_auth,
            github_pr: None,
            settings,
            config_override,
            page: Page::Home,
        }
    }

    pub fn persist(&self) -> Settings {
        let mut settings = self.settings.clone();
        settings.auth = self.github_auth.get_auth_state().clone();
        settings
    }

    pub fn reference<'a>(
        &'a self,
        ctx: &'a Context,
        diff_image_loader: &'a DiffImageLoader,
        tx: UiInboxSender<SystemCommand>,
    ) -> AppStateRef<'a> {
        let page = match &self.page {
            Page::Home => PageRef::Home,
            Page::DiffViewer(viewer) => {
                let filtered_snapshots = viewer.filtered_snapshots();

                let active_filtered_index = filtered_snapshots
                    .iter()
                    .position(|(i, _)| *i == viewer.index)
                    .unwrap_or(0);

                let viewer_ref = ViewerStateRef {
                    state: viewer,
                    active_snapshot: filtered_snapshots
                        .get(active_filtered_index)
                        .map(|(_, s)| *s),
                    filtered_snapshots,
                    active_filtered_index,
                };
                PageRef::DiffViewer(viewer_ref)
            }
        };

        AppStateRef {
            state: self,
            page,
            diff_image_loader,
            egui_ctx: ctx,
            tx,
        }
    }
}

pub struct AppStateRef<'a> {
    pub egui_ctx: &'a Context,
    pub state: &'a AppState,
    pub page: PageRef<'a>,
    pub diff_image_loader: &'a DiffImageLoader,
    pub tx: UiInboxSender<SystemCommand>,
}

impl AppStateRef<'_> {
    pub fn send(&self, command: impl Into<SystemCommand>) {
        self.tx.send(command.into()).ok();
    }
}

impl Deref for AppStateRef<'_> {
    type Target = AppState;

    fn deref(&self) -> &Self::Target {
        self.state
    }
}

pub enum PageRef<'a> {
    Home,
    DiffViewer(ViewerStateRef<'a>),
}

pub type FilteredSnapshot<'a> = (usize, &'a Snapshot);

pub struct ViewerStateRef<'a> {
    pub state: &'a ViewerState,
    pub filtered_snapshots: Vec<FilteredSnapshot<'a>>,
    pub active_filtered_index: usize,
    pub active_snapshot: Option<&'a Snapshot>,
}

impl Deref for ViewerStateRef<'_> {
    type Target = ViewerState;

    fn deref(&self) -> &Self::Target {
        self.state
    }
}

impl<'a> ViewerStateRef<'a> {
    pub fn with_app(&'a self, app: &'a AppStateRef<'a>) -> ViewerAppStateRef<'a> {
        ViewerAppStateRef { app, viewer: self }
    }
}

pub struct ViewerAppStateRef<'a> {
    pub app: &'a AppStateRef<'a>,
    pub viewer: &'a ViewerStateRef<'a>,
}

impl<'a> Deref for ViewerAppStateRef<'a> {
    type Target = ViewerStateRef<'a>;

    fn deref(&self) -> &Self::Target {
        self.viewer
    }
}

pub enum SystemCommand {
    Open(crate::DiffSource),

    /// Open a source from the current viewer, e.g. an artifact from a PR.
    /// The new viewer gets a back button to the current one.
    OpenFromViewer(crate::DiffSource),

    /// Return to [`ViewerState::back`].
    Back,

    /// Close the viewer and go to the home page.
    Close,
    GithubAuth(GithubAuthCommand),
    LoadPrDetails(GithubPrLink),
    UpdateSettings(Settings),
    ViewerCommand(ViewerSystemCommand),
    Refresh,
}

pub enum ViewerSystemCommand {
    SetFilter(String),
    SelectSnapshot(usize),
    SetView(View),
}

impl From<ViewerSystemCommand> for SystemCommand {
    fn from(value: ViewerSystemCommand) -> Self {
        Self::ViewerCommand(value)
    }
}

impl AppState {
    pub fn handle(&mut self, ctx: &Context, command: SystemCommand) {
        match command {
            SystemCommand::Open(source) => {
                let loader = source.load(ctx, self);
                self.page = Page::DiffViewer(ViewerState::new(loader, None));
            }
            SystemCommand::OpenFromViewer(source) => {
                let loader = source.load(ctx, self);
                let back = match std::mem::replace(&mut self.page, Page::Home) {
                    Page::DiffViewer(viewer) => Some(Box::new(viewer)),
                    Page::Home => None,
                };
                self.page = Page::DiffViewer(ViewerState::new(loader, back));
            }
            SystemCommand::Close => {
                self.page = Page::Home;
            }
            SystemCommand::Back => {
                if let Page::DiffViewer(viewer) = &mut self.page
                    && let Some(back) = viewer.back.take()
                {
                    self.page = Page::DiffViewer(*back);
                }
            }
            SystemCommand::GithubAuth(auth) => {
                self.github_auth.handle(ctx, auth);
                self.github_files.set_client(self.github_auth.client());
            }
            SystemCommand::LoadPrDetails(url) => {
                self.github_pr = Some(GithubPr::new(
                    url,
                    self.github_auth.client(),
                    self.config_override.clone(),
                ));
            }
            SystemCommand::UpdateSettings(settings) => {
                self.settings = settings;
            }

            SystemCommand::ViewerCommand(command) => {
                if let Page::DiffViewer(viewer) = &mut self.page {
                    viewer.handle(ctx, command);
                } else {
                    log::warn!("Received ViewerCommand but not in DiffViewer page");
                }
            }
            SystemCommand::Refresh => {
                // A login also refreshes.
                self.github_files.set_client(self.github_auth.client());

                // The URIs of PR images don't change, so forget the images that failed to load,
                // e.g. before the user gave kitdiff access to the repository.
                ctx.forget_all_images();

                match &mut self.page {
                    Page::Home => {}
                    Page::DiffViewer(viewer) => {
                        let client = self.github_auth.client();
                        viewer.refresh(client);
                    }
                }
            }
        }
    }

    pub fn update(&mut self, ctx: &Context) {
        if let Page::DiffViewer(viewer) = &mut self.page {
            viewer.loader.update(ctx);
            viewer.index_just_selected = false;
        }

        self.github_auth.update(ctx);
    }
}

impl ViewerState {
    fn new(loader: SnapshotLoader, back: Option<Box<Self>>) -> Self {
        Self {
            filter: String::new(),
            index: 0,
            index_just_selected: true,
            loader,
            view: View::default(),
            back,
        }
    }

    pub fn handle(&mut self, _ctx: &Context, command: ViewerSystemCommand) {
        match command {
            ViewerSystemCommand::SetFilter(filter) => {
                self.filter = filter;
                self.index_just_selected = true;
            }
            ViewerSystemCommand::SelectSnapshot(index) => {
                if index < self.loader.snapshots().len() {
                    self.index = index;
                    self.index_just_selected = true;
                }
            }
            ViewerSystemCommand::SetView(view_filter) => {
                self.view = view_filter;
            }
        }
    }

    pub fn refresh(&mut self, client: Octocrab) {
        self.loader.refresh(client);
        self.index = 0;
    }
}
