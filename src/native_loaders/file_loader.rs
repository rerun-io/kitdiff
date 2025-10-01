use crate::loaders::LoadSnapshots;
use crate::snapshot::{FileReference, Snapshot};
use anyhow::Error;
use eframe::egui::Context;
use egui_inbox::UiInbox;
use ignore::WalkBuilder;
use ignore::types::TypesBuilder;
use octocrab::Octocrab;
use std::path::{Path, PathBuf};
use std::task::Poll;

pub struct FileLoader {
    base_path: PathBuf,
    inbox: UiInbox<Option<Snapshot>>,
    loading: bool,
    snapshots: Vec<Snapshot>,
}

impl FileLoader {
    pub fn new(base_path: impl Into<PathBuf>) -> Self {
        let base_path = base_path.into();

        let (sender, inbox) = UiInbox::channel();

        {
            let base_path = base_path.clone();
            std::thread::Builder::new()
                .name(format!("File loader {}", base_path.display()))
                .spawn(move || {
                    let mut types_builder = TypesBuilder::new();
                    types_builder
                        .add("png", "*.png")
                        .expect("Failed to add png type");
                    types_builder.select("png");
                    let types = types_builder.build().expect("Failed to build types");

                    for entry in WalkBuilder::new(&base_path).types(types).build().flatten() {
                        if entry.file_type().is_some_and(|ft| ft.is_file()) {
                            if let Some(snapshot) = try_create_snapshot(entry.path(), &base_path) {
                                if sender.send(Some(snapshot)).is_err() {
                                    break;
                                };
                            }
                        }
                    }

                    // Signal completion
                    sender.send(None).ok();
                })
                .expect("Failed to spawn file loader thread");
        }

        Self {
            base_path,
            inbox,
            snapshots: Vec::new(),
            loading: true,
        }
    }
}

impl LoadSnapshots for FileLoader {
    fn update(&mut self, ctx: &Context) {
        for snapshot in self.inbox.read(ctx) {
            if let Some(snapshot) = snapshot {
                self.snapshots.push(snapshot);
            } else {
                self.loading = false;
            }
        }
    }

    fn refresh(&mut self, _client: Octocrab) {
        *self = Self::new(self.base_path.clone());
    }

    fn snapshots(&self) -> &[Snapshot] {
        &self.snapshots
    }

    fn state(&self) -> Poll<Result<(), &Error>> {
        if self.loading {
            Poll::Pending
        } else {
            Poll::Ready(Ok(()))
        }
    }

    fn files_header(&self) -> String {
        format!("Files in {}", self.base_path.display())
    }
}

fn try_create_snapshot(png_path: &Path, base_path: &Path) -> Option<Snapshot> {
    let file_name = png_path.file_name()?.to_str()?;

    // Skip files that are already variants (.old.png, .new.png, .diff.png)
    if file_name.ends_with(".old.png")
        || file_name.ends_with(".new.png")
        || file_name.ends_with(".diff.png")
    {
        return None;
    }

    // Get base path without .png extension
    let file_base_path = png_path.with_extension("");
    let old_path = file_base_path.with_extension("old.png");
    let new_path = file_base_path.with_extension("new.png");
    let diff_path = file_base_path.with_extension("diff.png");

    // Only create snapshot if diff exists
    if !diff_path.exists() {
        return None;
    }

    // Create relative path from the base directory
    let relative_path = png_path.strip_prefix(base_path).unwrap_or(png_path);

    if old_path.exists() {
        // old.png exists, use original as new and old.png as old
        Some(Snapshot {
            path: relative_path.to_path_buf(),
            old: Some(FileReference::Path(old_path)),
            new: Some(FileReference::Path(png_path.to_path_buf())),
            diff: Some(FileReference::Path(diff_path)),
        })
    } else if new_path.exists() {
        // new.png exists, use original as old and new.png as new
        Some(Snapshot {
            path: relative_path.to_path_buf(),
            old: Some(FileReference::Path(png_path.to_path_buf())),
            new: Some(FileReference::Path(new_path)),
            diff: Some(FileReference::Path(diff_path)),
        })
    } else {
        // No old or new variant, skip this snapshot
        None
    }
}
