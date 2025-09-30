use crate::config::Config;
use crate::diff_image_loader::DiffImageLoader;
use crate::settings::Settings;
use crate::state::{AppState, AppStateRef, PageRef, SystemCommand, ViewerSystemCommand};
use crate::{DiffSource, bar, home, viewer};
use eframe::egui::{Context, Key, Modifiers};
use eframe::{Frame, Storage, egui};
use egui_extras::install_image_loaders;
use egui_inbox::UiInbox;
use std::sync::Arc;

pub struct App {
    diff_loader: Arc<DiffImageLoader>,
    state: AppState,
    inbox: UiInbox<SystemCommand>,
}

impl App {
    pub fn new(cc: &eframe::CreationContext, source: Option<DiffSource>, config: Config) -> Self {
        re_ui::apply_style_and_install_loaders(&cc.egui_ctx);

        let settings: Settings = cc
            .storage
            .and_then(|s| eframe::get_value(s, eframe::APP_KEY))
            .unwrap_or_default();

        let state = AppState::new(settings, config);

        install_image_loaders(&cc.egui_ctx);
        let diff_loader = Arc::new(DiffImageLoader::default());
        cc.egui_ctx.add_image_loader(diff_loader.clone());

        let ctx = cc.egui_ctx.clone();

        // if let Some(source) = source {
        //     match source {
        //         // TODO: This kinda sucks, maybe sources should just have an UI?
        //         DiffSource::Pr(pr) => {
        //             if let Ok((user, repo, pr_number)) = parse_github_pr_url(&pr) {
        //                 let auth_token = settings.auth().map(|auth| auth.provider_token.clone());
        //                 github_pr = Some(GithubPr::new(
        //                     user,
        //                     repo,
        //                     pr_number,
        //                     ctx.clone(),
        //                     auth_token,
        //                 ));
        //             } else {
        //                 eprintln!("Invalid GitHub PR URL");
        //             }
        //         }
        //         source => {
        //             source.load(sender.clone(), ctx, settings.auth());
        //         }
        //     }
        // }

        let inbox = UiInbox::new();

        if let Some(source) = source {
            inbox.sender().send(SystemCommand::Open(source)).ok();
        }

        Self {
            diff_loader,
            state,
            inbox,
        }
    }
}

impl eframe::App for App {
    fn save(&mut self, storage: &mut dyn Storage) {
        eframe::set_value(storage, eframe::APP_KEY, &self.state.persist());
    }

    fn update(&mut self, ctx: &Context, _frame: &mut Frame) {
        self.state.update(ctx);
        self.inbox.read(ctx).for_each(|cmd| {
            self.state.handle(ctx, cmd);
        });

        {
            let state_ref = self
                .state
                .reference(ctx, &self.diff_loader, self.inbox.sender());

            bar::bar(ctx, &state_ref);

            match &state_ref.page {
                PageRef::Home => {
                    home::home_view(ctx, &state_ref);
                }
                PageRef::DiffViewer(diff) => {
                    viewer::viewer_ui(ctx, &diff.with_app(&state_ref));
                }
            }

            Self::end_frame(ctx, &state_ref);
        }

        // for file in &ctx.input(|i| i.raw.dropped_files.clone()) {
        //     let data = file
        //         .bytes
        //         .clone()
        //         .map(|b| PathOrBlob::Blob(b.into()))
        //         .or(file.path.as_ref().map(|p| PathOrBlob::Path(p.clone())));
        //
        //     if let Some(data) = data {
        //         let source = if file.name.ends_with(".tar.gz") || file.name.ends_with(".tgz") {
        //             Some(DiffSource::TarGz(data))
        //         } else if file.name.ends_with(".zip") {
        //             Some(DiffSource::Zip(data))
        //         } else {
        //             None
        //         };
        //
        //         if let Some(source) = source {
        //             // Clear existing snapshots for new file
        //             self.snapshots.clear();
        //             self.index = 0;
        //             self.is_loading = true;
        //
        //             source.load(self.sender.clone(), ctx.clone(), self.settings.auth());
        //         }
        //     }
        //
        //     // if let Some(path) = &file.path {
        //     //     if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
        //     //         if name.ends_with(".tar.gz") || name.ends_with(".tgz") {
        //     //             // For native, read from file system
        //     //             #[cfg(not(target_arch = "wasm32"))]
        //     //             if let Ok(data) = std::fs::read(path) {
        //     //                 if let Some(sender) = &self.sender {
        //     //                     // Clear existing snapshots for new file
        //     //                     self.snapshots.clear();
        //     //                     self.index = 0;
        //     //                     self.is_loading = true;
        //     //
        //     //                     if let Err(e) =
        //     //                         extract_and_discover_tar_gz(data, sender.clone(), ctx.clone())
        //     //                     {
        //     //                         eprintln!("Failed to extract tar.gz: {:?}", e);
        //     //                     }
        //     //                 }
        //     //             }
        //     //         }
        //     //     }
        //     // }
        //     //
        //     // // For wasm, use the bytes directly if available
        //     // #[cfg(target_arch = "wasm32")]
        //     // if let Some(bytes) = &file.bytes {
        //     //     let name = &file.name;
        //     //     if name.ends_with(".tar.gz") || name.ends_with(".tgz") {
        //     //         if let Some(sender) = &self.sender {
        //     //             // Clear existing snapshots for new file
        //     //             self.snapshots.clear();
        //     //             self.index = 0;
        //     //             self.is_loading = true;
        //     //
        //     //             if let Err(e) =
        //     //                 extract_and_discover_tar_gz(bytes.to_vec(), sender.clone(), ctx.clone())
        //     //             {
        //     //                 eprintln!("Failed to extract tar.gz: {:?}", e);
        //     //                 panic!("{e:?}")
        //     //             }
        //     //         }
        //     //     }
        //     // }
        // }
    }
}

impl App {
    fn end_frame(ctx: &Context, state: &AppStateRef<'_>) {
        match &state.page {
            PageRef::Home => {}
            PageRef::DiffViewer(vs) => {
                let mut new_index = None;
                if ctx.input_mut(|i| i.consume_key(Modifiers::NONE, egui::Key::ArrowDown)) {
                    // Find next snapshot that matches filter
                    if vs.active_filtered_index + 1 < vs.filtered_snapshots.len() {
                        new_index = Some(vs.filtered_snapshots[vs.active_filtered_index + 1].0);
                    }
                }
                if ctx.input_mut(|i| i.consume_key(Modifiers::NONE, egui::Key::ArrowUp)) {
                    // Find previous snapshot that matches filter
                    if vs.active_filtered_index > 0 {
                        new_index = Some(vs.filtered_snapshots[vs.active_filtered_index - 1].0);
                    }
                }
                if let Some(new_index) = new_index {
                    state.send(ViewerSystemCommand::SelectSnapshot(new_index));
                }

                let handle_key = |key: Key, toggle: &mut bool| {
                    if ctx.input_mut(|i| i.key_pressed(key)) {
                        *toggle = true;
                    }
                    if ctx.input_mut(|i| i.key_released(key)) {
                        *toggle = false;
                    }
                };

                let mut view_filter = vs.state.view_filter;
                handle_key(Key::Num1, &mut view_filter.show_old);
                handle_key(Key::Num2, &mut view_filter.show_new);
                handle_key(Key::Num3, &mut view_filter.show_diff);
                if view_filter != vs.state.view_filter {
                    state.send(ViewerSystemCommand::SetViewFilter(view_filter));
                }
            }
        }
    }
}
