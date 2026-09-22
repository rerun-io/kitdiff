use crate::state::ViewerAppStateRef;
use eframe::egui::{Image, Response, RichText, Sense, SizeHint, Ui, load::ImagePoll};

pub fn diff_view(ui: &mut Ui, state: &ViewerAppStateRef<'_>) {
    ui.label("Use 1/2/3 to only show old / new / diff at 100% opacity. Arrow keys to navigate.");

    if let Some(snapshot) = state.active_snapshot {
        let diff_uri = snapshot.diff_uri(
            state.app.settings.use_original_diff,
            state.app.settings.options,
        );

        let diff_info = diff_uri
            .as_deref()
            .and_then(|uri| state.app.diff_image_loader.diff_info(uri));

        match &diff_info {
            Some(Ok(info)) => {
                if info.diff == 0 {
                    ui.strong("All differences below threshold!");
                } else {
                    ui.label(
                        RichText::new(format!("Diff pixels: {}", info.diff))
                            .color(ui.visuals().warn_fg_color),
                    );
                }
            }
            Some(Err(err)) => {
                ui.label(
                    RichText::new(format!("Diff failed: {err}")).color(ui.visuals().error_fg_color),
                );
            }
            None => {
                ui.label("No diff info yet...");
            }
        }

        let diff_failed = matches!(diff_info, Some(Err(_)));

        let rect = ui.available_rect_before_wrap();

        let old = snapshot.old_image(state.app);
        let new = snapshot.new_image(state.app);
        let diff = snapshot.diff_image(state.app);

        let is_loading = |maybe_image: &Option<Image<'_>>| {
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
            let response = ui.place(rect, old.sense(Sense::click()));
            copy_image_context_menu(&response, snapshot.old_uri().as_deref());
        }

        if let Some(new) = new {
            let response = ui.place(rect, new.sense(Sense::click()));
            copy_image_context_menu(&response, snapshot.new_uri().as_deref());
        }

        if let Some(diff) = diff
            && !diff_failed
        {
            let response = ui.place(rect, diff.sense(Sense::click()));
            copy_image_context_menu(&response, diff_uri.as_deref());
        }

        // Preload surrounding snapshots once our image is loaded
        if !any_loading {
            for i in -10..=10 {
                if let Some((_, surrounding_snapshot)) = state
                    .filtered_snapshots
                    .get((state.active_filtered_index as isize + i) as usize)
                {
                    if let Some(old_uri) = surrounding_snapshot.old_uri() {
                        ui.ctx().try_load_image(&old_uri, SizeHint::default()).ok();
                    }
                    if let Some(new_uri) = surrounding_snapshot.new_uri() {
                        ui.ctx().try_load_image(&new_uri, SizeHint::default()).ok();
                    }
                    if let Some(diff_uri) = surrounding_snapshot.diff_uri(
                        state.app.settings.use_original_diff,
                        state.app.settings.options,
                    ) {
                        ui.ctx().try_load_image(&diff_uri, SizeHint::default()).ok();
                    }
                }
            }
        }
    }
}

/// Right-click menu for copying the image at `uri` to the clipboard.
fn copy_image_context_menu(response: &Response, uri: Option<&str>) {
    response.context_menu(|ui| {
        if ui.button("Copy image").clicked() {
            if let Some(uri) = uri
                && let Ok(ImagePoll::Ready { image }) =
                    ui.ctx().try_load_image(uri, SizeHint::default())
            {
                ui.ctx().copy_image((*image).clone());
            }
            ui.close();
        }
    });
}
