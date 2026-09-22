use crate::state::ViewerAppStateRef;
use eframe::egui::{
    Align2, Color32, CursorIcon, Image, Mesh, Pos2, Rect, Response, RichText, Sense, Shape,
    SizeHint, TextureOptions, Ui, Vec2,
    emath::GuiRounding as _,
    load::{ImagePoll, TexturePoll},
    pos2, remap, vec2,
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
        let mut magnifier_layers: Vec<MagnifierLayer> = Vec::new();
        let mut top_response: Option<Response> = None;

        for (image, uri) in layers {
            let Some(image) = image else {
                continue;
            };
            let tint = image.image_options().tint;
            let image_size = image.load_and_calc_size(ui, rect.size());
            let response = ui.place(rect, image.sense(Sense::click()));
            copy_image_context_menu(&response, uri.as_deref());

            if let Some(uri) = uri
                && let Some(image_size) = image_size
            {
                magnifier_layers.push(MagnifierLayer {
                    uri,
                    tint,
                    // `Ui::place` centers the image within the allocated rect:
                    image_rect: Align2::CENTER_CENTER
                        .align_size_within_rect(image_size, response.rect)
                        .round_ui(),
                });
            }
            top_response = Some(response);
        }

        // Only the topmost image gets the hover, so it carries the magnifier for the whole stack.
        // The images are centered in the available space, so don't magnify the empty margins.
        if let Some(response) = top_response
            && !response.context_menu_opened()
            && let Some(pointer) = response.hover_pos()
            && magnifier_layers
                .iter()
                .any(|layer| layer.image_rect.contains(pointer))
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

/// One image of the stack, as the magnifier needs it.
struct MagnifierLayer {
    uri: String,

    /// Opacity of this layer in the blend.
    tint: Color32,

    /// Where the image is painted on screen, in points.
    image_rect: Rect,
}

/// Show a zoomed-in, nearest-neighbor view of the images around the pointer.
///
/// `layers` are given in bottom-to-top draw order, so that the magnifier shows
/// the same blend as the main view.
fn magnifier_on_hover(response: &Response, layers: &[MagnifierLayer]) {
    response
        .clone()
        .on_hover_cursor(CursorIcon::ZoomIn)
        .on_hover_ui_at_pointer(|ui| {
            let Some(pointer) = ui.ctx().pointer_latest_pos() else {
                return;
            };

            let (_id, zoom_rect) = ui.allocate_space(Vec2::splat(MAGNIFIER_SIZE));

            for MagnifierLayer {
                uri,
                tint,
                image_rect,
            } in layers
            {
                if !image_rect.contains(pointer) {
                    continue;
                }

                // Load with nearest-neighbor filtering so the texels stay crisp when blown up.
                let Ok(TexturePoll::Ready { texture }) =
                    ui.ctx()
                        .try_load_texture(uri, TextureOptions::NEAREST, SizeHint::default())
                else {
                    continue;
                };

                let tex_size = texture.size;
                if !(tex_size.x > 0.0 && tex_size.y > 0.0) {
                    continue;
                }

                // The hovered texel, and half the size of the magnified area, in texels:
                let texel = vec2(
                    remap(pointer.x, image_rect.x_range(), 0.0..=tex_size.x),
                    remap(pointer.y, image_rect.y_range(), 0.0..=tex_size.y),
                );
                let radius = Vec2::splat(MAGNIFIER_SIZE / MAGNIFIER_ZOOM / 2.0);

                // What we'd like to show, which near an edge reaches outside the image:
                let min = (texel - radius) / tex_size;
                let max = (texel + radius) / tex_size;
                let uv = Rect::from_min_max(pos2(min.x, min.y), pos2(max.x, max.y));

                // Paint only the part that is actually inside the image, so that we
                // show the edge of the image instead of smearing its outermost texels.
                let visible_uv = uv.intersect(Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)));
                if !visible_uv.is_positive() {
                    continue;
                }
                let to_zoom_rect = |pos: Pos2| {
                    zoom_rect.lerp_inside(vec2(
                        remap(pos.x, uv.x_range(), 0.0..=1.0),
                        remap(pos.y, uv.y_range(), 0.0..=1.0),
                    ))
                };
                let visible_rect =
                    Rect::from_min_max(to_zoom_rect(visible_uv.min), to_zoom_rect(visible_uv.max));

                let mut mesh = Mesh::with_texture(texture.id);
                mesh.add_rect_with_uv(visible_rect, visible_uv, *tint);
                ui.painter().add(Shape::mesh(mesh));
            }
        });
}
