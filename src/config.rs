//! The `kitdiff.toml` config, which kitdiff reads from the root of a GitHub repository.

use crate::github::octokit::RepoClient;
use anyhow::{Context as _, anyhow, bail};

/// The path of the config file, relative to the repository root.
pub const CONFIG_PATH: &str = "kitdiff.toml";

#[derive(Debug, Clone, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub struct Config {
    #[serde(default)]
    pub github: Github,

    #[serde(default)]
    pub artifact: Artifact,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub struct Github {
    /// File name (or ID) of the workflow that the "Commit the updated snapshots" button triggers.
    ///
    /// If this is not set, kitdiff does not show the button.
    ///
    /// For a GitHub Actions artifact, kitdiff gives the workflow a `run_id` input.
    /// For an artifact from [`Artifact::url_template`], kitdiff gives it a `commit` input, and no platform.
    pub update_snapshot_workflow_name: Option<String>,

    /// Glob pattern for the names of the GitHub Actions artifacts with snapshots, e.g. `test-results-*`.
    /// `*` matches any text, `?` matches one character, `[abc]` matches one of the characters.
    ///
    /// If this is not set, kitdiff lists no GitHub Actions artifacts.
    /// If the repository has no `kitdiff.toml`, kitdiff lists all of them.
    pub artifact_pattern: Option<String>,
}

impl Github {
    /// Is this the name of an artifact that kitdiff should list?
    pub fn lists_artifact(&self, name: &str) -> bool {
        self.artifact_pattern
            .as_deref()
            .and_then(|pattern| glob::Pattern::new(pattern).ok())
            .is_some_and(|pattern| pattern.matches(name))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub struct Artifact {
    /// URL of a `.zip` or `.tar.gz` archive with the snapshots of a commit,
    /// for CI systems other than GitHub Actions.
    ///
    /// See [`TemplateVars`] for the placeholders, e.g.
    /// `https://build.rerun.io/commit/{short_commit}/snapshots/{platform}.tar.gz`.
    pub url_template: Option<String>,

    /// The values for the `{platform}` placeholder.
    ///
    /// kitdiff makes one URL for each platform and lists the archives that exist.
    #[serde(default)]
    pub platforms: Vec<String>,
}

impl Config {
    /// The config for a repository without a `kitdiff.toml`.
    /// It lists all GitHub Actions artifacts, so that kitdiff works with a plain GitHub Actions setup.
    pub fn without_file() -> Self {
        Self {
            github: Github {
                artifact_pattern: Some("*".to_owned()),
                ..Default::default()
            },
            ..Default::default()
        }
    }

    pub fn parse(text: &str) -> anyhow::Result<Self> {
        let config: Self =
            toml::from_str(text).with_context(|| format!("Failed to parse {CONFIG_PATH}"))?;
        if let Some(pattern) = &config.github.artifact_pattern {
            glob::Pattern::new(pattern).with_context(|| {
                format!("Invalid artifact_pattern {pattern:?} in {CONFIG_PATH}")
            })?;
        }
        Ok(config)
    }

    /// Read the config at `git_ref`. Gives [`Self::without_file`] if the file does not exist.
    pub async fn fetch(repo: &RepoClient, git_ref: &str) -> anyhow::Result<Self> {
        let response = repo
            .repos()
            .raw_file(git_ref.to_owned(), CONFIG_PATH)
            .await
            .with_context(|| format!("Failed to fetch {CONFIG_PATH}"))?;

        let status = response.status();
        if status == reqwest::StatusCode::NOT_FOUND {
            return Ok(Self::without_file());
        }
        let text = repo.body_to_string(response).await?;
        if !status.is_success() {
            bail!("Failed to fetch {CONFIG_PATH}: HTTP {status}: {text}");
        }

        Self::parse(&text)
    }
}

/// The values kitdiff puts in place of the `{placeholders}` in a URL template.
pub struct TemplateVars<'a> {
    /// `{owner}`: the owner of the repository.
    pub owner: &'a str,

    /// `{repo}`: the name of the repository.
    pub repo: &'a str,

    /// `{commit}`: the full commit hash. `{short_commit}` gives its first 7 characters.
    pub commit: &'a str,

    /// `{branch}`: the head branch of the PR.
    pub branch: &'a str,

    /// `{pr}`: the PR number.
    pub pr: u64,

    /// `{platform}`: one of [`Artifact::platforms`].
    pub platform: Option<&'a str>,
}

pub fn render_url_template(template: &str, vars: &TemplateVars<'_>) -> anyhow::Result<String> {
    let mut out = String::with_capacity(template.len());
    let mut rest = template;

    while let Some(start) = rest.find('{') {
        out.push_str(&rest[..start]);
        let after = &rest[start + 1..];
        let end = after
            .find('}')
            .ok_or_else(|| anyhow!("Unclosed '{{' in URL template {template:?}"))?;

        let name = &after[..end];
        match name {
            "owner" => out.push_str(vars.owner),
            "repo" => out.push_str(vars.repo),
            "commit" => out.push_str(vars.commit),
            "short_commit" => out.push_str(vars.commit.get(..7).unwrap_or(vars.commit)),
            "branch" => out.push_str(vars.branch),
            "pr" => out.push_str(&vars.pr.to_string()),
            "platform" => out.push_str(vars.platform.ok_or_else(|| {
                anyhow!(
                    "URL template {template:?} uses {{platform}}, but [artifact] platforms is empty"
                )
            })?),
            _ => bail!("Unknown placeholder {{{name}}} in URL template {template:?}"),
        }

        rest = &after[end + 1..];
    }
    out.push_str(rest);

    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    const VARS: TemplateVars<'static> = TemplateVars {
        owner: "rerun-io",
        repo: "reality",
        commit: "1cc57e9c7f0123456789abcdef0123456789abcd",
        branch: "lucas/feature",
        pr: 42,
        platform: Some("macos"),
    };

    #[test]
    fn renders_all_placeholders() {
        let url = render_url_template(
            "https://example.com/{owner}/{repo}/{pr}/{branch}/{commit}/{short_commit}/{platform}.tar.gz",
            &VARS,
        )
        .expect("valid template");
        assert_eq!(
            url,
            "https://example.com/rerun-io/reality/42/lucas/feature/1cc57e9c7f0123456789abcdef0123456789abcd/1cc57e9/macos.tar.gz"
        );
    }

    #[test]
    fn rejects_bad_templates() {
        assert!(render_url_template("https://example.com/{nope}", &VARS).is_err());
        assert!(render_url_template("https://example.com/{commit", &VARS).is_err());

        let no_platform = TemplateVars {
            platform: None,
            ..VARS
        };
        assert!(render_url_template("https://example.com/{platform}", &no_platform).is_err());
    }

    #[test]
    fn lists_artifacts() {
        assert!(Config::without_file().github.lists_artifact("whatever"));
        assert!(!Config::default().github.lists_artifact("whatever"));

        let config = Config::parse(
            r#"
            [github]
            artifact_pattern = "test-results-*"
            "#,
        )
        .expect("valid config");
        assert!(config.github.lists_artifact("test-results-linux"));
        assert!(config.github.lists_artifact("test-results-macos-all"));
        assert!(!config.github.lists_artifact("wheels"));

        assert!(
            Config::parse(
                r#"
                [github]
                artifact_pattern = "test-results-["
                "#,
            )
            .is_err()
        );
    }

    #[test]
    fn parses_config() {
        let config = Config::parse(
            r#"
            [github]
            update_snapshot_workflow_name = "update_kittest_snapshots.yml"

            [artifact]
            url_template = "https://build.rerun.io/commit/{short_commit}/snapshots/{platform}.tar.gz"
            platforms = ["linux", "macos", "windows"]
            "#,
        )
        .expect("valid config");
        assert_eq!(
            config.github.update_snapshot_workflow_name.as_deref(),
            Some("update_kittest_snapshots.yml")
        );
        assert_eq!(
            config.artifact.url_template.as_deref(),
            Some("https://build.rerun.io/commit/{short_commit}/snapshots/{platform}.tar.gz")
        );
        assert_eq!(config.artifact.platforms, ["linux", "macos", "windows"]);

        assert_eq!(Config::parse("").expect("empty config"), Config::default());
    }
}
