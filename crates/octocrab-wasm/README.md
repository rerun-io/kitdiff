## Wasm support for the Octocrab GitHub API client

This is a thin wrapper around [octocrab](https://crates.io/crates/octocrab) that
adds support for using octocrab in wasm based on reqwest.

### Usage

This crate exports a builder that works natively and in wasm:
```rust
    let mut client = octocrab_wasm::builder()
        .build()
        .expect("Failed to build Octocrab client");

    // Optionally set a GitHub auth token
    if let Some(token) = &auth_token {
        client = client
            .user_access_token(token.to_owned())
            .expect("Failed to set token");
    }

    // Now do some requests!
```
