use crate::loaders::{DataReference, LoadSnapshots};
use crate::snapshot::{FileReference, Snapshot};
use anyhow::{Error, Result};
use bytes::Bytes;
use eframe::egui::{Context, ImageSource};
use egui_inbox::{UiInbox, UiInboxSender};
use flate2::read::GzDecoder;
use std::borrow::Cow;
use std::collections::HashMap;
use std::io::{Cursor, Read as _};
use std::path::{Path, PathBuf};
use std::task::Poll;
use tar::Archive;
use zip::ZipArchive;

#[derive(Debug)]
pub struct ArchiveLoader {
    data: Poll<anyhow::Result<Vec<Snapshot>>>,
    inbox: UiInbox<Result<Vec<Snapshot>>>,
    name: String,
}

fn is_zip(data: &[u8]) -> bool {
    data.starts_with(b"PK")
}

fn is_tar_gz(data: &[u8]) -> bool {
    data.starts_with(&[0x1F, 0x8B, 0x08])
}

impl ArchiveLoader {
    pub fn new(data: DataReference) -> Self {
        let name = data.file_name().to_owned();
        let mut inbox = UiInbox::new();

        inbox.spawn(|tx| async move {
            let result = run_discovery(data).await;
            tx.send(result).ok();
        });

        Self {
            name,
            data: Poll::Pending,
            inbox,
        }
    }
}

impl LoadSnapshots for ArchiveLoader {
    fn files_header(&self) -> String {
        format!("Archive: {}", self.name)
    }

    fn update(&mut self, ctx: &Context) {
        if let Some(mut new_data) = self.inbox.read(ctx).last() {
            if let Ok(data) = &mut new_data {
                data.sort_by_key(|s| s.path.to_string_lossy().to_lowercase());
                for snapshot in data {
                    // We need to register bytes so that the diff loader can find them
                    snapshot.register_bytes(ctx);
                }
            }
            self.data = Poll::Ready(new_data);
        }
    }

    fn snapshots(&self) -> &[Snapshot] {
        match &self.data {
            Poll::Ready(Ok(snapshots)) => snapshots,
            _ => &[],
        }
    }

    fn state(&self) -> Poll<std::result::Result<(), &Error>> {
        match &self.data {
            Poll::Ready(Ok(_)) => Poll::Ready(Ok(())),
            Poll::Ready(Err(e)) => Poll::Ready(Err(e)),
            Poll::Pending => Poll::Pending,
        }
    }
}

pub async fn run_discovery(file: DataReference) -> anyhow::Result<Vec<Snapshot>> {
    let data = file.into_bytes().await?;

    #[cfg(target_arch = "wasm32")]
    {
        sync_discovery(data)
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        tokio::task::spawn_blocking(move || sync_discovery(data)).await?
    }
}

fn sync_discovery(data: Bytes) -> anyhow::Result<Vec<Snapshot>> {
    let files = if is_zip(&data) {
        run_zip_discovery(data)?
    } else if is_tar_gz(&data) {
        run_tar_discovery(data)?
    } else {
        anyhow::bail!("Unsupported archive format");
    };

    Ok(get_snapshots(files))
}

fn run_zip_discovery(zip_data: Bytes) -> Result<HashMap<PathBuf, Vec<u8>>> {
    // Extract all files into memory (similar to tar loader)
    let cursor = Cursor::new(zip_data);
    let mut archive = ZipArchive::new(cursor)?;

    let mut files = HashMap::new();

    for i in 0..archive.len() {
        let mut file = archive.by_index(i)?;
        let file_path = match file.enclosed_name() {
            Some(path) => path.clone(),
            None => continue, // Skip files with invalid names
        };

        // Only process PNG files
        if file_path.extension().and_then(|s| s.to_str()) == Some("png") {
            let mut data = Vec::new();
            file.read_to_end(&mut data)?;
            files.insert(file_path, data);
        }
    }

    Ok(files)
}

fn run_tar_discovery(tar_data: Bytes) -> Result<HashMap<PathBuf, Vec<u8>>> {
    let cursor = Cursor::new(tar_data);
    let gz_decoder = GzDecoder::new(cursor);
    let mut archive = Archive::new(gz_decoder);

    // Extract all files into memory
    let mut files = HashMap::new();

    for entry in archive.entries()? {
        let mut entry = entry?;
        let path = entry.path()?.to_path_buf();

        // Only process PNG files
        if path.extension().and_then(|s| s.to_str()) == Some("png") {
            let mut data = Vec::new();
            entry.read_to_end(&mut data)?;
            files.insert(path, data);
        }
    }

    Ok(files)
}

fn get_snapshots(files: HashMap<PathBuf, Vec<u8>>) -> Vec<Snapshot> {
    let mut snapshots = Vec::new();
    let mut processed_files = std::collections::HashSet::new();

    for png_path in files.keys() {
        if processed_files.contains(png_path) {
            continue;
        }

        if let Some(snapshot) = try_create_snapshot(png_path, &files) {
            // Mark related files as processed
            processed_files.insert(png_path.clone());
            if let Some(old_path) = get_variant_path(png_path, "old") {
                processed_files.insert(old_path);
            }
            if let Some(new_path) = get_variant_path(png_path, "new") {
                processed_files.insert(new_path);
            }
            if let Some(diff_path) = get_variant_path(png_path, "diff") {
                processed_files.insert(diff_path);
            }

            snapshots.push(snapshot);
        }
    }

    snapshots
}

fn try_create_snapshot(png_path: &Path, files: &HashMap<PathBuf, Vec<u8>>) -> Option<Snapshot> {
    let file_name = png_path.file_name()?.to_str()?;

    // Skip files that are already variants (.old.png, .new.png, .diff.png)
    if file_name.ends_with(".old.png")
        || file_name.ends_with(".new.png")
        || file_name.ends_with(".diff.png")
    {
        return None;
    }

    // Get variant paths
    let old_path = get_variant_path(png_path, "old")?;
    let new_path = get_variant_path(png_path, "new")?;
    let diff_path = get_variant_path(png_path, "diff")?;

    // // Check if diff exists (required for a valid snapshot)
    // if !files.contains_key(&diff_path) {
    //     return None;
    // }

    let base_data = files.get(png_path)?;

    let diff_data = files.get(&diff_path);
    let diff_reference = diff_data.map(|data| {
        FileReference::Source(ImageSource::Bytes {
            uri: Cow::Owned(format!("bytes://{}", diff_path.display())),
            bytes: eframe::egui::load::Bytes::Shared(data.clone().into()),
        })
    });

    if files.contains_key(&old_path) {
        // old.png exists, use original as new and old.png as old
        let old_data = files.get(&old_path)?;
        if old_data == base_data {
            // If old and new are identical, skip this snapshot
            return None;
        }
        Some(Snapshot {
            path: png_path.to_path_buf(),
            old: Some(FileReference::Source(ImageSource::Bytes {
                uri: Cow::Owned(format!("bytes://{}", old_path.display())),
                bytes: eframe::egui::load::Bytes::Shared(old_data.clone().into()),
            })),
            new: Some(FileReference::Source(ImageSource::Bytes {
                uri: Cow::Owned(format!("bytes://{}", png_path.display())),
                bytes: eframe::egui::load::Bytes::Shared(base_data.clone().into()),
            })),
            diff: diff_reference, // We'll handle diff separately if needed
        })
    } else if files.contains_key(&new_path) {
        // new.png exists, use original as old and new.png as new
        let new_data = files.get(&new_path)?;
        if new_data == base_data {
            // If old and new are identical, skip this snapshot
            return None;
        }
        Some(Snapshot {
            path: png_path.to_path_buf(),
            old: Some(FileReference::Source(ImageSource::Bytes {
                uri: Cow::Owned(format!("bytes://{}", png_path.display())),
                bytes: eframe::egui::load::Bytes::Shared(base_data.clone().into()),
            })),
            new: Some(FileReference::Source(ImageSource::Bytes {
                uri: Cow::Owned(format!("bytes://{}", new_path.display())),
                bytes: eframe::egui::load::Bytes::Shared(new_data.clone().into()),
            })),
            diff: diff_reference, // We'll handle diff separately if needed
        })
    } else {
        // No old or new variant, skip this snapshot
        None
    }
}

fn get_variant_path(base_path: &Path, variant: &str) -> Option<PathBuf> {
    let stem = base_path.file_stem()?.to_str()?;
    let parent = base_path.parent().unwrap_or(Path::new(""));
    Some(parent.join(format!("{stem}.{variant}.png")))
}
