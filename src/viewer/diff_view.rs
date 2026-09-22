use crate::state::ViewerAppStateRef;
use eframe::egui::{
    Color32, CursorIcon, Image, Mesh, Rect, Response, RichText, Sense, Shape, SizeHint,
    TextureOptions, Ui, Vec2,
    load::{ImagePoll, TexturePoll},
    pos2, remap_clamp,
};

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

        // Bottom-to-top draw order:
        let layers = [
            (old, snapshot.old_uri()),
            (new, snapshot.new_uri()),
            (if diff_failed { None } else { diff }, diff_uri.clone()),
        ];

        // What the magnifier needs to re-create the blend, in the same order:
        let mut magnifier_layers: Vec<(String, Color32)> = Vec::new();
        let mut top_response: Option<Response> = None;

        for (image, uri) in layers {
            let Some(image) = image else {
                continue;
            };
            let tint = image.image_options().tint;
            let response = ui.place(rect, image.sense(Sense::click()));
            copy_image_context_menu(&response, uri.as_deref());
            if let Some(uri) = uri {
                magnifier_layers.push((uri, tint));
            }
            top_response = Some(response);
        }

        // Only the topmost image gets the hover, so it carries the magnifier for the whole stack.
        if let Some(response) = top_response
            && !response.context_menu_opened()
        {
            magnifier_on_hover(&response, &magnifier_layers);
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

/// How many points each texel of the image takes up in the magnifier.
const MAGNIFIER_ZOOM: f32 = 16.0;

/// Width and height of the magnifier window, in points.
const MAGNIFIER_SIZE: f32 = 256.0;

/// Show a zoomed-in, nearest-neighbor view of the images around the pointer.
///
/// `layers` are the image URIs and their tints, in bottom-to-top draw order,
/// so that the magnifier shows the same blend as the main view.
fn magnifier_on_hover(response: &Response, layers: &[(String, Color32)]) {
    let image_rect = response.rect;

    response
        .clone()
        .on_hover_cursor(CursorIcon::ZoomIn)
        .on_hover_ui_at_pointer(|ui| {
            let Some(pointer) = ui.ctx().pointer_latest_pos() else {
                return;
            };

            let (_id, zoom_rect) = ui.allocate_space(Vec2::splat(MAGNIFIER_SIZE));

            for (uri, tint) in layers {
                // Load with nearest-neighbor filtering so the texels stay crisp when blown up.
                let Ok(TexturePoll::Ready { texture }) =
                    ui.ctx()
                        .try_load_texture(uri, TextureOptions::NEAREST, SizeHint::default())
                else {
                    continue;
                };

                let Vec2 {
                    x: tex_w,
                    y: tex_h,
                } = texture.size;
                if tex_w <= 0.0 || tex_h <= 0.0 {
                    continue;
                }

                // Half the size of the magnified area, in texels:
                let radius = Vec2::splat(MAGNIFIER_SIZE / MAGNIFIER_ZOOM / 2.0)
                    .min(Vec2::new(tex_w, tex_h) / 2.0);

                let u = remap_clamp(pointer.x, image_rect.x_range(), 0.0..=tex_w)
                    .clamp(radius.x, tex_w - radius.x);
                let v = remap_clamp(pointer.y, image_rect.y_range(), 0.0..=tex_h)
                    .clamp(radius.y, tex_h - radius.y);

                let uv_rect = Rect::from_min_max(
                    pos2((u - radius.x) / tex_w, (v - radius.y) / tex_h),
                    pos2((u + radius.x) / tex_w, (v + radius.y) / tex_h),
                );

                let mut mesh = Mesh::with_texture(texture.id);
                mesh.add_rect_with_uv(zoom_rect, uv_rect, *tint);
                ui.painter().add(Shape::mesh(mesh));
            }
        });
}
