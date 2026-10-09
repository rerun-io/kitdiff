use crate::github::model::CommitArchiveLink;
use crate::github::update_snapshots::UpdateSnapshotsButton;
use crate::loaders::archive_loader::ArchiveLoader;
use crate::loaders::{DataReference, LoadSnapshots};
use crate::snapshot::Snapshot;
use crate::state::AppStateRef;
use eframe::egui::{Context, Ui};
use octocrab::Octocrab;
use std::task::Poll;

/// Loads the snapshots of a commit from [`crate::config::Artifact::url_template`].
pub struct CommitArchiveLoader {
    link: CommitArchiveLink,
    archive: ArchiveLoader,
    update_button: Option<UpdateSnapshotsButton>,
}

impl CommitArchiveLoader {
    pub fn new(link: CommitArchiveLink) -> Self {
        Self {
            archive: ArchiveLoader::new(DataReference::Url(link.url.clone())),
            update_button: link
                .update_snapshots
                .clone()
                .map(UpdateSnapshotsButton::new),
            link,
        }
    }
}

impl LoadSnapshots for CommitArchiveLoader {
    fn update(&mut self, ctx: &Context) {
        self.archive.update(ctx);
        if let Some(button) = &mut self.update_button {
            button.update(ctx);
        }
    }

    fn refresh(&mut self, _client: Octocrab) {
        self.archive = ArchiveLoader::new(DataReference::Url(self.link.url.clone()));
    }

    fn snapshots(&self) -> &[Snapshot] {
        self.archive.snapshots()
    }

    fn state(&self) -> Poll<Result<(), &anyhow::Error>> {
        self.archive.state()
    }

    fn extra_ui(&self, ui: &mut Ui, state: &AppStateRef<'_>) {
        if let Some(button) = &self.update_button {
            button.ui(ui, state);
        }
    }

    fn files_header(&self) -> String {
        let short_commit = self.link.commit.get(..7).unwrap_or(&self.link.commit);
        format!("{} @ {short_commit}", self.archive.files_header())
    }
}
