use std::fmt::Display;
use std::str::FromStr;

pub type PrNumber = u64;

#[derive(Debug, Clone)]
pub struct GithubRepoLink {
    pub owner: String,
    pub repo: String,
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
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        todo!()
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
