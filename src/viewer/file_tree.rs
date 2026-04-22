use crate::state::{FilteredSnapshot, ViewerAppStateRef, ViewerSystemCommand};
use eframe::egui;
use eframe::egui::{Id, OpenUrl, ScrollArea, TextEdit, Ui};
use re_ui::UiExt as _;
use re_ui::alert::Alert;
use re_ui::list_item::LabelContent;
use std::task::Poll;

fn is_github_permission_error(err: &anyhow::Error) -> bool {
    for cause in err.chain() {
        if let Some(github_err) = cause.downcast_ref::<octocrab::GitHubError>() {
            return matches!(
                github_err.status_code,
                reqwest::StatusCode::FORBIDDEN | reqwest::StatusCode::NOT_FOUND
            );
        }
    }
    // octocrab can fail to parse a 404 error body, producing a serde error instead
    let msg = err.to_string().to_lowercase();
    msg.contains("not found") || msg.contains("missing field")
}

pub fn file_tree(ui: &mut Ui, state: &ViewerAppStateRef<'_>) {
    ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Truncate);

    state.loader.extra_ui(ui, state.app);

    if let Poll::Ready(Err(e)) = state.loader.state() {
        if is_github_permission_error(e) {
            Alert::warning().show(ui, |ui: &mut Ui| {
                ui.vertical(|ui| {
                    ui.label("kitdiff does not have access to this repository.");
                    if ui.link("Grant repository access").clicked() {
                        ui.ctx().open_url(OpenUrl::new_tab(
                            crate::github::auth::GitHubAuth::MANAGE_REPO_ACCESS_URL,
                        ));
                    }
                });
            });
        } else {
            Alert::error().show(ui, |ui: &mut Ui| {
                ui.label(e.to_string());
            });
        }
    }

    ui.panel_title_bar_with_buttons(&state.loader.files_header(), None, |ui| {
        if state.loader.state().is_pending() {
            ui.spinner();
        }
    });

    let mut filter = state.filter.clone();
    TextEdit::singleline(&mut filter)
        .hint_text("Filter")
        .show(ui);

    if filter != state.filter {
        state.app.send(ViewerSystemCommand::SetFilter(filter));
    }

    ScrollArea::vertical().show(ui, |ui| {
        ui.list_item_scope("file_tree", |ui| {
            let mut tree: Vec<(Option<&str>, Vec<FilteredSnapshot<'_>>)> = Vec::new();

            // Snapshots should already be sorted, so we only need to group them
            for filtered_snapshot in state.filtered_snapshots.iter().copied() {
                let prefix = filtered_snapshot.1.path.parent().and_then(|p| p.to_str());
                if let Some((current_prefix, snapshots)) = tree.last_mut()
                    && *current_prefix == prefix
                {
                    snapshots.push(filtered_snapshot);
                    continue;
                }
                tree.push((prefix, vec![filtered_snapshot]));
            }

            for (prefix, snapshots) in tree {
                if let Some(prefix) = prefix {
                    ui.list_item().show_hierarchical_with_children(
                        ui,
                        Id::new(prefix),
                        true,
                        LabelContent::new(prefix),
                        |ui| show_prefix(ui, state, &snapshots),
                    );
                } else {
                    show_prefix(ui, state, &snapshots);
                }
            }

            if state.loader.snapshots().is_empty() {
                if state.loader.state().is_ready() {
                    ui.label("No snapshots were found.");
                }
            } else if state.filtered_snapshots.is_empty() {
                ui.label("No snapshots match the filter.");
            }
        });
    });
}

fn show_prefix(
    ui: &mut Ui,
    state: &ViewerAppStateRef<'_>,
    filtered_snapshots: &[FilteredSnapshot<'_>],
) {
    for (index, snapshot) in filtered_snapshots {
        let selected = *index == state.index;
        let content = LabelContent::new(snapshot.file_name());
        let item = ui.list_item().selected(selected);

        let response = item.show_hierarchical(ui, content);

        if response.clicked() {
            state.app.send(ViewerSystemCommand::SelectSnapshot(*index));
        }

        if selected && state.index_just_selected {
            response.scroll_to_me(None);
        }
    }
}
