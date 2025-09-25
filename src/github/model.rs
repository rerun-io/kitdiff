use std::fmt::Display;
use std::str::FromStr;
use octocrab::models::{ArtifactId, RunId};

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

        Ok(GithubRepoLink {
            owner: owner.to_string(),
            repo: repo.to_string(),
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

        Ok(GithubPrLink {
            repo: GithubRepoLink {
                owner: owner.to_string(),
                repo: repo.to_string(),
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
    pub branch_name: Option<String>,
    pub run_id: Option<RunId>,
}

impl GithubArtifactLink {
    pub fn name(&self) -> String {
        self.name
            .as_deref()
            .unwrap_or(&self.artifact_id.to_string())
            .to_string()
    }
}


