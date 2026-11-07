# Releasing

The first crates.io release must be published manually by a crate owner. Afterward, configure a trusted publisher on crates.io for organization `NicDev-Studios`, repository `mdcontent`, workflow `publish.yml`, and environment `crates-io`. Verify ownership and consider enabling trusted-publishing-only mode after testing the configuration.

For later releases, update the Cargo version and lockfile, pass CI on `main`, create a matching tag such as `v1.0.0`, and publish a GitHub release for that tag. The workflow checks the version and ancestry, runs tests and packages the crate, and uses a short-lived OIDC token. Prereleases do not publish. No release or crates.io publication is performed merely by merging to `main`.
