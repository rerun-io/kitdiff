# kitdiff auth callback

This handles login via the GitHub app and is deployed via Google Cloud run in the gcp kitdiff project.

Two environment variables must be set:
- `GITHUB_CLIENT_ID`: The GitHub apps client id
- `GITHUB_CLIENT_SECRET`: The GitHub apps client secret
