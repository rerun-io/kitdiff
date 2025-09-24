use crate::settings::ImageMode;
use crate::state::{SystemCommand, ViewerAppStateRef, ViewerSystemCommand};
use eframe::egui::{Slider, TextureFilter, Ui};

pub fn viewer_options(ui: &mut Ui, state: &ViewerAppStateRef<'_>) {
    let mut settings = state.app.settings.clone();

    ui.add(Slider::new(&mut settings.new_opacity, 0.0..=1.0).text("New Opacity"));
    ui.add(Slider::new(&mut settings.diff_opacity, 0.0..=1.0).text("Diff Opacity"));

    let mut filtered_index = state.active_filtered_index;

    ui.add(
        Slider::new(&mut filtered_index, 0..=state.filtered_snapshots.len()).text("Snapshot Index"),
    );

    if filtered_index != state.active_filtered_index {
        if let Some((index, _)) = state.filtered_snapshots.get(filtered_index) {
            state.app.send(ViewerSystemCommand::SelectSnapshot(*index));
        }
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

        ui.add(
            Slider::new(&mut settings.options.threshold, 0.01..=1000.0)
                .logarithmic(true)
                .text("Diff Threshold"),
        );
        ui.checkbox(&mut settings.options.detect_aa_pixels, "Detect AA Pixels");
    });

    if settings != state.app.settings {
        state
            .app
            .tx
            .send(SystemCommand::UpdateSettings(settings))
            .ok();
    }
}
