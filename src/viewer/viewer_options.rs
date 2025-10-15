use crate::state::{SystemCommand, ViewerAppStateRef, ViewerSystemCommand};
use crate::{settings::ImageMode, state::View};
use eframe::egui::{self, Slider, TextureFilter, Ui};

pub fn viewer_options(ui: &mut Ui, state: &ViewerAppStateRef<'_>) {
    let mut settings = state.app.settings.clone();

    ui.group(|ui| {
        ui.strong("View");
        let mut new_view = state.view;

        for view in View::ALL {
            ui.radio_value(
                &mut new_view,
                view,
                format!("{view} ({})", view.key().name()),
            );
        }

        ui.label("Toggle old/new with SPACE");
        if ui.input_mut(|i| i.consume_key(egui::Modifiers::default(), egui::Key::Space)) {
            if new_view == View::Old {
                new_view = View::New;
            } else {
                new_view = View::Old;
            }
        }

        if new_view != state.view {
            state.app.send(ViewerSystemCommand::SetView(new_view));
        }
    });

    ui.add_enabled_ui(state.view == View::BlendAll, |ui| {
        ui.add(Slider::new(&mut settings.new_opacity, 0.0..=1.0).text("New Opacity"));
        ui.add(Slider::new(&mut settings.diff_opacity, 0.0..=1.0).text("Diff Opacity"));
    });

    let mut filtered_index = state.active_filtered_index;

    ui.add(
        Slider::new(&mut filtered_index, 0..=state.filtered_snapshots.len()).text("Snapshot Index"),
    );

    if filtered_index != state.active_filtered_index
        && let Some((index, _)) = state.filtered_snapshots.get(filtered_index)
    {
        state.app.send(ViewerSystemCommand::SelectSnapshot(*index));
    }

    ui.horizontal_wrapped(|ui| {
        ui.label("Size:");
        ui.selectable_value(&mut settings.mode, ImageMode::Pixel, "1:1");
        ui.selectable_value(&mut settings.mode, ImageMode::Fit, "Fit");
    });

    ui.horizontal_wrapped(|ui| {
        ui.label("Filtering:");
        ui.selectable_value(
            &mut settings.texture_magnification,
            TextureFilter::Nearest,
            "Nearest",
        );
        ui.selectable_value(
            &mut settings.texture_magnification,
            TextureFilter::Linear,
            "Linear",
        );
    });

    ui.group(|ui| {
        ui.heading("Diff Options");
        ui.checkbox(
            &mut settings.use_original_diff,
            "Use original diff if available",
        );

        ui.add_enabled_ui(!settings.use_original_diff, |ui| {
            ui.add(
                Slider::new(&mut settings.options.threshold, 0.01..=1000.0)
                    .logarithmic(true)
                    .text("Diff Threshold"),
            );
            ui.checkbox(&mut settings.options.detect_aa_pixels, "Detect AA Pixels");
        });
    });

    if settings != state.app.settings {
        state
            .app
            .tx
            .send(SystemCommand::UpdateSettings(settings))
            .ok();
    }
}
