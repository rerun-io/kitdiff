use crate::diff_image_loader::DiffImageLoader;
use crate::loaders::SnapshotLoader;
use crate::settings::Settings;
use crate::snapshot::Snapshot;
use eframe::egui;
use eframe::egui::Context;
use egui_inbox::UiInboxSender;
use std::ops::Deref;
use crate::github::auth::{GitHubAuth, GithubAuthCommand};
use crate::github::model::GithubPrLink;
use crate::github::pr::GithubPr;

pub struct AppState {
    pub github_auth: GitHubAuth,
    pub github_pr: Option<GithubPr>,
    pub settings: Settings,
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
    pub view_filter: ViewFilter,
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

/// If any is true, only show those, but at full opacity
///
/// If all are false, show all at their set opacities
#[derive(Default, Copy, Clone, PartialEq)]
pub struct ViewFilter {
    pub show_old: bool,
    pub show_new: bool,
    pub show_diff: bool,
}

impl ViewFilter {
    pub fn all(&self) -> bool {
        !self.show_old && !self.show_new && !self.show_diff
    }
}

impl AppState {
    pub fn new(settings: Settings) -> AppState {
        Self {
            github_auth: GitHubAuth::new(settings.auth.clone()),
            github_pr: None,
            settings,
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
    GithubAuth(GithubAuthCommand),
    LoadPrDetails(GithubPrLink),
    UpdateSettings(Settings),
    ViewerCommand(ViewerSystemCommand),
}

pub enum ViewerSystemCommand {
    SetFilter(String),
    SelectSnapshot(usize),
    SetViewFilter(ViewFilter),
}

impl From<ViewerSystemCommand> for SystemCommand {
    fn from(value: ViewerSystemCommand) -> Self {
        SystemCommand::ViewerCommand(value)
    }
}

impl AppState {
    pub fn handle(&mut self, ctx: &Context, command: SystemCommand) {
        match command {
            SystemCommand::Open(source) => {
                let loader = source.load(ctx.clone(), &self);
                self.page = Page::DiffViewer(ViewerState {
                    filter: String::new(),
                    index: 0,
                    index_just_selected: true,
                    loader,
                    view_filter: ViewFilter::default(),
                });
            }
            SystemCommand::GithubAuth(auth) => {
                self.github_auth.handle(auth);
            }
            SystemCommand::LoadPrDetails(url) => {
                self.github_pr = Some(GithubPr::new(
                    url,
                    self.github_auth.client(),
                ));
            }
            SystemCommand::UpdateSettings(settings) => {
                self.settings = settings;
            }

            SystemCommand::ViewerCommand(command) => {
                if let Page::DiffViewer(viewer) = &mut self.page {
                    viewer.handle(ctx, command);
                } else {
                    eprintln!("Received ViewerCommand but not in DiffViewer page"); // TODO: Better logging
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
    pub fn handle(&mut self, ctx: &Context, command: ViewerSystemCommand) {
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
            ViewerSystemCommand::SetViewFilter(view_filter) => {
                self.view_filter = view_filter;
            }
        }
    }
}
