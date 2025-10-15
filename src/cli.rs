use clap::{Parser, Subcommand};
use kitdiff::DiffSource;
use kitdiff::github::auth::parse_github_artifact_url;

#[derive(Parser)]
#[command(name = "kitdiff")]
#[command(about = "A viewer for egui kittest snapshot test files")]
pub struct Cli {
    #[command(subcommand)]
    pub command: Option<Commands>,
}

#[derive(Subcommand)]
pub enum Commands {
    /// Just show the kitdiff start page
    Ui,
    /// Compare snapshot test files (.png with .old/.new/.diff variants) (default)
    Files { directory: Option<String> },
    /// Compare images between current branch and default branch
    Git { repo_path: Option<String> },
    /// Compare images between PR branches from GitHub PR URL
    Pr { url: String },
    /// Load and compare snapshot files from a zip archive (URL or local file)
    Archive { source: String },
    /// Load and compare snapshot files from a GitHub artifact
    GhArtifact { url: String },
}

impl Commands {
    pub fn to_source(&self) -> Option<DiffSource> {
        Some(match self {
            Self::Ui => return None,
            Self::Files { directory } => {
                DiffSource::Files(directory.clone().unwrap_or_else(|| ".".into()).into())
            }
            Self::Git { repo_path } => {
                DiffSource::Git(repo_path.clone().unwrap_or_else(|| ".".into()).into())
            }
            Self::Pr { url } => {
                // Check if the PR URL is actually a GitHub artifact URL
                if let Some(link) = parse_github_artifact_url(url) {
                    DiffSource::GHArtifact(link)
                } else if let Ok(parsed_url) = url.parse() {
                    DiffSource::Pr(parsed_url)
                } else {
                    panic!("Invalid GitHub PR URL: {url}");
                }
            }
            Self::Archive { source } => {
                if source.starts_with("http://") || source.starts_with("https://") {
                    DiffSource::Archive(kitdiff::DataReference::Url(source.clone()))
                } else {
                    DiffSource::Archive(kitdiff::DataReference::Path(source.clone().into()))
                }
            }
            Self::GhArtifact { url } => {
                if let Some(link) = parse_github_artifact_url(url) {
                    DiffSource::GHArtifact(link)
                } else {
                    panic!("Invalid GitHub artifact URL: {url}");
                }
            }
        })
    }
}
