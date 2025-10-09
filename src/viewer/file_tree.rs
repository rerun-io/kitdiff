use crate::state::{FilteredSnapshot, ViewerAppStateRef, ViewerSystemCommand};
use eframe::egui;
use eframe::egui::{Id, ScrollArea, TextEdit, Ui, Widget as _};
use re_ui::list_item::LabelContent;
use re_ui::{UiExt as _, icons};
use std::task::Poll;

pub fn file_tree(ui: &mut Ui, state: &ViewerAppStateRef<'_>) {
    ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Truncate);

    state.loader.extra_ui(ui, state.app);

    ui.panel_title_bar_with_buttons(&state.loader.files_header(), None, |ui| {
        match state.loader.state() {
            Poll::Ready(Ok(())) => {}
            Poll::Ready(Err(e)) => {
                icons::ERROR
                    .as_image()
                    .tint(ui.tokens().alert_error.icon)
                    .ui(ui)
                    .on_hover_text(e.to_string());
            }
            Poll::Pending => {
                ui.spinner();
            }
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
                if let Some((current_prefix, snapshots)) = tree.last_mut() {
                    if *current_prefix == prefix {
                        snapshots.push(filtered_snapshot);
                        continue;
                    }
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
