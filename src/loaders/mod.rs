use crate::snapshot::Snapshot;
use eframe::egui;
use std::task::Poll;

pub mod tar_loader;
pub mod zip_loader;
pub mod pr_loader;

pub trait LoadSnapshots {
    fn update(&mut self, ctx: &egui::Context);

    fn snapshots(&self) -> &[Snapshot];

    /// State is separate so that snapshots can be streamed in
    fn state(&self) -> Poll<Result<(), &anyhow::Error>>;
    
    fn extra_ui(&mut self, ui: &mut egui::Ui) {}
}

pub type SnapshotLoader = Box<dyn LoadSnapshots + Send + Sync>;