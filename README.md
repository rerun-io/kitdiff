# kitdiff 📸🐱, a visual diffing tool

I got frustrated with the experience of reviewing image snapshot changes (from egui_kittest, thus the name) in my ide and on github, so I made something really cool:

Just run `kitdiff` in your terminal and it'll visualize all the diffs! You can use the `1, 2, 3` keys to switch between `original, new, diff` images. You can also generate diffs on the fly to play with different threshold options. Also, it's wicked quick!

https://github.com/user-attachments/assets/c9324ef3-eb24-481f-83b8-42a37b6b075d

## but wait, there's more

You can do `kitdiff pr https://github.com/rerun-io/rerun/pull/11253` to view a diff of that PR, you don't even need to check out the branch!


https://github.com/user-attachments/assets/d5c0b15a-0a75-4506-8dae-51b8bb83836f


## Getting started

Just do a `cargo install --git https://github.com/rerun-io/kitdiff ` to install the binary


## Repository config

When you open a PR, kitdiff reads `kitdiff.toml` from the root of the repository, at the head commit of the PR:

```toml
[github]
# The "Commit the updated snapshots" button triggers this workflow on the PR branch.
# kitdiff hides the button when this is not set.
# For a GitHub Actions artifact, the workflow gets a `run_id` input.
# For an artifact from `url_template` (below), it gets a `commit` input, and no platform:
# the workflow picks the platform that is the source of truth.
update_snapshot_workflow_name = "update_kittest_snapshots.yml"
# Glob pattern for the GitHub Actions artifacts to list: `*` matches any text, `?` one character.
# kitdiff lists no GitHub Actions artifacts when this is not set.
artifact_pattern = "test-results-*"

[artifact]
# Use this when a CI system other than GitHub Actions publishes the snapshots.
# It is the URL of a .zip or .tar.gz archive with the snapshots of a commit.
# Placeholders: {owner}, {repo}, {pr}, {branch}, {commit}, {short_commit} (first 7 characters),
# and {platform}.
url_template = "https://build.rerun.io/commit/{short_commit}/snapshots/{platform}.tar.gz"
# The values for {platform}. kitdiff makes one URL for each and lists the archives that exist.
platforms = ["linux", "macos", "windows"]
```

Without a `kitdiff.toml`, kitdiff lists all GitHub Actions artifacts, and shows no "Commit the updated snapshots" button.

Click a commit in the PR panel to see its artifacts.

To try a config before you commit it, pass it with `--config`. It replaces the repository's `kitdiff.toml`:

```sh
kitdiff --config my-kitdiff.toml pr https://github.com/owner/repo/pull/123
```
