use crate::settings::ImageMode;
use crate::state::{AppStateRef, PageRef, ViewFilter, ViewerAppStateRef, ViewerStateRef};
use eframe::egui::{Image, RichText, SizeHint, TextureOptions, Ui};

pub fn diff_view(ui: &mut Ui, state: &ViewerAppStateRef<'_>) {
    ui.label("Use 1/2/3 to only show old / new / diff at 100% opacity. Arrow keys to navigate.");

    if let Some(snapshot) = state.active_snapshot {
        let diff_uri = snapshot.diff_uri(
            state.app.settings.use_original_diff,
            state.app.settings.options,
        );

        if let Some(info) = state.app.diff_image_loader.diff_info(&diff_uri) {
            if info.diff == 0 {
                ui.strong("All differences below threshold!");
            } else {
                ui.label(
                    RichText::new(format!("Diff pixels: {}", info.diff))
                        .color(ui.visuals().warn_fg_color),
                );
            }
        } else {
            ui.label("No diff info yet...");
        }

        let rect = ui.available_rect_before_wrap();

        let old = snapshot.old_image(state.app);
        let new = snapshot.new_image(state.app);
        let diff = snapshot.diff_image(state.app);

        let is_loading = |maybe_image: &Option<Image>| {
            maybe_image
                .as_ref()
                .map(|img| {
                    img.load_for_size(ui.ctx(), rect.size())
                        .is_ok_and(|poll| poll.is_pending())
                })
                .unwrap_or(false)
        };

        let any_loading = is_loading(&old) || is_loading(&new) || is_loading(&diff);

        if let Some(old) = old {
            ui.place(rect, old);
        }

        if let Some(new) = new {
            ui.place(rect, new);
        }

        if let Some(diff) = diff {
            ui.place(rect, diff);
        }

        // Preload surrounding snapshots once our image is loaded
        if !any_loading {
            for i in -10..=10 {
                if let Some((_, surrounding_snapshot)) = state
                    .filtered_snapshots
                    .get((state.active_filtered_index as isize + i) as usize)
                {
                    ui.ctx()
                        .try_load_image(&surrounding_snapshot.old_uri(), SizeHint::default())
                        .ok();
                    ui.ctx()
                        .try_load_image(&surrounding_snapshot.new_uri(), SizeHint::default())
                        .ok();
                    ui.ctx()
                        .try_load_image(
                            &surrounding_snapshot.diff_uri(
                                state.app.settings.use_original_diff,
                                state.app.settings.options,
                            ),
                            SizeHint::default(),
                        )
                        .ok();
                }
            }
        }
    } else if state.loader.state().is_pending() {
        // TODO: Display error
        ui.label("Searching for snapshots...");
    } else {
        ui.label("No snapshots found.");
    }
}
