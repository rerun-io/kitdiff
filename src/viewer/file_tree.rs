use crate::state::{FilteredSnapshot, ViewerAppStateRef, ViewerSystemCommand};
use eframe::egui;
use eframe::egui::{Id, ScrollArea, TextEdit, Ui};
use re_ui::UiExt;
use re_ui::list_item::{LabelContent, ListItem};
use std::collections::BTreeMap;

pub fn file_tree(ui: &mut Ui, state: &ViewerAppStateRef<'_>) {
    ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Truncate);

    let mut filter = state.filter.clone();
    TextEdit::singleline(&mut filter)
        .hint_text("Filter")
        .show(ui);

    if filter != state.filter {
        state.app.send(ViewerSystemCommand::SetFilter(filter));
    }

    ScrollArea::vertical().show(ui, |ui| {
        ui.list_item_scope("file_tree", |ui| {
            let mut tree = BTreeMap::new();

            for (snapshot_index, snapshot) in &state.filtered_snapshots {
                let prefix = snapshot.path.parent().and_then(|p| p.to_str());
                tree.entry(prefix)
                    .or_insert(vec![])
                    .push((*snapshot_index, *snapshot))
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
            response.scroll_to_me(Some(eframe::egui::Align::Center));
        }
    }
}
