use crate::github::model::GithubRepoLink;
use std::ops::Deref;

pub struct RepoClient {
    client: octocrab::Octocrab,
    link: GithubRepoLink,
}

impl Deref for RepoClient {
    type Target = octocrab::Octocrab;

    fn deref(&self) -> &Self::Target {
        &self.client
    }
}

impl RepoClient {
    pub fn new(client: octocrab::Octocrab, link: GithubRepoLink) -> Self {
        Self { client, link }
    }

    pub fn repo(&self) -> &GithubRepoLink {
        &self.link
    }

    pub fn issues(&self) -> octocrab::issues::IssueHandler<'_> {
        self.client.issues(&self.link.owner, &self.link.repo)
    }

    pub fn commits(&self) -> octocrab::commits::CommitHandler<'_> {
        self.client.commits(&self.link.owner, &self.link.repo)
    }

    pub fn pulls(&self) -> octocrab::pulls::PullRequestHandler<'_> {
        self.client.pulls(&self.link.owner, &self.link.repo)
    }

    pub fn workflows(&self) -> octocrab::workflows::WorkflowsHandler<'_> {
        self.client.workflows(&self.link.owner, &self.link.repo)
    }

    pub fn repos(&self) -> octocrab::repos::RepoHandler<'_> {
        self.client.repos(&self.link.owner, &self.link.repo)
    }

    pub fn checks(&self) -> octocrab::checks::ChecksHandler<'_> {
        self.client.checks(&self.link.owner, &self.link.repo)
    }
}
