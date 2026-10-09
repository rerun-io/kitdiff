//! Snapshot tests of the kitdiff UI.
//!
//! The viewer tests show the snapshot test results in `tests/fixtures`.

use eframe::egui::accesskit::Role;
use eframe::egui::{Key, Vec2, pos2};
use egui_kittest::kittest::Queryable as _;
use egui_kittest::{Harness, SnapshotResults};
use kitdiff::DiffSource;
use kitdiff::app::App;

fn harness(source: Option<DiffSource>) -> Harness<'static, App> {
    let mut harness = Harness::builder()
        .with_size(Vec2::new(1000.0, 600.0))
        .wgpu()
        .build_eframe(|cc| App::new(cc, source, None));
    harness.run();
    harness
}

/// A viewer of the results in `tests/fixtures`.
///
/// Relative, so that the path in the header is the same on all machines.
fn viewer() -> Harness<'static, App> {
    harness(Some(DiffSource::Files("tests/fixtures".into())))
}

// kitdiff needs a Tokio runtime, e.g. for its GitHub client.
#[tokio::test(flavor = "multi_thread")]
async fn home() {
    let mut harness = harness(None);
    harness.snapshot("home");
}

#[tokio::test(flavor = "multi_thread")]
async fn views() {
    let mut harness = viewer();
    let mut results = SnapshotResults::new();

    results.add(harness.try_snapshot("viewer_blend_all"));

    for (key, name) in [
        (Key::Num2, "viewer_old"),
        (Key::Num3, "viewer_new"),
        (Key::Num4, "viewer_diff"),
    ] {
        harness.key_press(key);
        harness.run();
        results.add(harness.try_snapshot(name));
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn select_snapshot() {
    let mut harness = viewer();
    let mut results = SnapshotResults::new();

    harness.key_press(Key::ArrowDown);
    harness.run();
    results.add(harness.try_snapshot("viewer_next_snapshot"));

    harness.get_by_label("checkbox.png").click();
    harness.run();
    results.add(harness.try_snapshot("viewer_click_in_file_tree"));
}

#[tokio::test(flavor = "multi_thread")]
async fn filter() {
    let mut harness = viewer();

    // The only text field. "Filter" is its hint text, not its label.
    harness.get_by_role(Role::TextInput).focus();
    harness.run();
    harness.get_by_role(Role::TextInput).type_text("check");
    harness.run();
    harness.snapshot("viewer_filter");
}

#[tokio::test(flavor = "multi_thread")]
async fn pixel_size() {
    let mut harness = viewer();

    harness.get_by_label("1:1").click();
    harness.run();
    harness.snapshot("viewer_pixel_size");
}

#[tokio::test(flavor = "multi_thread")]
async fn magnifier() {
    let mut harness = viewer();

    harness.hover_at(pos2(500.0, 330.0));
    harness.run();
    harness.snapshot("viewer_magnifier");
}
