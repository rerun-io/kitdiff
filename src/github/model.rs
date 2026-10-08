use crate::github::update_snapshots::UpdateSnapshotsWorkflow;
use octocrab::models::ArtifactId;
use std::fmt::Display;
use std::str::FromStr;

pub type PrNumber = u64;

#[derive(Debug)]
pub enum GithubParseErr {
    MissingOwner,
    MissingRepo,
    MissingPullSegment,
    MissingPrNumber,
    InvalidPrNumber(std::num::ParseIntError),
}

#[derive(Debug, Clone)]
pub struct GithubRepoLink {
    pub owner: String,
    pub repo: String,
}

impl FromStr for GithubRepoLink {
    type Err = GithubParseErr;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let s = s.strip_prefix("https://github.com/").unwrap_or(s);

        // Parse strings like "owner/repo"
        let mut parts = s.split('/');

        let owner = parts.next().ok_or(GithubParseErr::MissingOwner)?;
        let repo = parts.next().ok_or(GithubParseErr::MissingRepo)?;

        Ok(Self {
            owner: owner.to_owned(),
            repo: repo.to_owned(),
        })
    }
}

#[derive(Debug, Clone)]
pub struct GithubPrLink {
    pub repo: GithubRepoLink,
    pub pr_number: PrNumber,
}

impl GithubPrLink {
    pub fn short_name(&self) -> String {
        format!("{}/{}#{}", self.repo.owner, self.repo.repo, self.pr_number)
    }
}

impl FromStr for GithubPrLink {
    type Err = GithubParseErr;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let s = s.strip_prefix("https://github.com/").unwrap_or(s);

        let mut parts = s.split('/');
        let owner = parts.next().ok_or(GithubParseErr::MissingOwner)?;
        let repo = parts.next().ok_or(GithubParseErr::MissingRepo)?;
        _ = parts.next().ok_or(GithubParseErr::MissingPullSegment)?;
        let number: PrNumber = parts
            .next()
            .ok_or(GithubParseErr::MissingPrNumber)?
            .parse()
            .map_err(GithubParseErr::InvalidPrNumber)?;

        Ok(Self {
            repo: GithubRepoLink {
                owner: owner.to_owned(),
                repo: repo.to_owned(),
            },
            pr_number: number,
        })
    }
}

impl Display for GithubPrLink {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{}/{}/pull/{}",
            self.repo.owner, self.repo.repo, self.pr_number
        )
    }
}

#[derive(Debug, Clone)]
pub struct GithubArtifactLink {
    pub repo: GithubRepoLink,
    pub artifact_id: ArtifactId,
    pub name: Option<String>,

    /// Set when the repo's `kitdiff.toml` names a workflow to commit the snapshots.
    pub update_snapshots: Option<UpdateSnapshotsWorkflow>,
}

impl GithubArtifactLink {
    pub fn name(&self) -> String {
        self.name
            .as_deref()
            .unwrap_or(&self.artifact_id.to_string())
            .to_owned()
    }
}

/// A `.zip` or `.tar.gz` archive with the snapshots of one commit,
/// from [`crate::config::Artifact::url_template`].
#[derive(Debug, Clone)]
pub struct CommitArchiveLink {
    pub url: String,
    pub commit: String,

    /// One of [`crate::config::Artifact::platforms`], if the config sets any.
    pub platform: Option<String>,
    pub update_snapshots: Option<UpdateSnapshotsWorkflow>,
}
