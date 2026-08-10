# Release guide

This guide is for bootmux maintainers. The Makefile provides separate stable
and prerelease entrypoints and distributes the committed source to every
channel used by that track.

## Prerequisites

Run releases from a clean `main` checkout whose history contains
`origin/main`. Install and authenticate:

- Rust/Cargo, including the `rustfmt` and `clippy` components;
- Git and GitHub CLI (`gh auth status`);
- Cargo credentials with permission to publish `bootmux`;
- `curl`, `perl`, Ruby, `make`, Bash, Zsh, and Fish (for completion
  validation);
- Homebrew for stable releases only.

The GitHub and tap repositories can be overridden with
`BOOTMUX_GITHUB_REPO` and `BOOTMUX_HOMEBREW_TAP_REPO`. The tap is always
updated through a disposable clean clone.

Before publishing, also run the real tmux and Herdr smoke tests described in
[Development and verification](development.md). The automated gate runs the
standard, non-ignored test suite; tests that start real interactive backends
remain an explicit maintainer check.

## Stable releases

The recommended flow captures the target once, checks that exact version, and
then publishes the same version:

```sh
VERSION="$(make --no-print-directory release-version)"
make release-check VERSION="$VERSION"
make release VERSION="$VERSION" PUBLISH=1
```

For the common case, the publishing entrypoint can calculate the target
itself:

```sh
make release PUBLISH=1
```

When `VERSION` is omitted, a stable release increments the smallest SemVer
component: `0.1.0` becomes `0.1.1`. A current prerelease such as
`0.1.1-rc.2` is promoted to `0.1.1`. Use an explicit version for a minor or
major release:

```sh
make release-check VERSION=0.2.0
make release VERSION=0.2.0 PUBLISH=1
```

The stable state machine is:

1. generate the target `Cargo.toml` and `Cargo.lock`, roll `[Unreleased]` into
   a dated changelog section, and update exact mise pins in the installation
   docs;
2. run formatting, Clippy, standard tests, completion syntax checks, Ruby and
   Homebrew Formula checks, and an allow-dirty Cargo package dry run;
3. create the local release-preparation commit, then verify the clean committed
   package with a second Cargo dry run;
4. validate the release commit and tag, then push `main` and `v<VERSION>`;
5. create a **draft** GitHub Release;
6. wait for the `Release binaries` workflow to attach the prebuilt archives and
   `SHA256SUMS` to that Release, download every archive, verify each one
   against the manifest, then clone the tap and generate and validate the
   target Formula before publishing the crate;
7. publish to crates.io and verify that the downloaded crate's VCS metadata
   points to the release commit;
8. update and push `griffinqiu/homebrew-tap`, then update the Formula mirror
   in the main repository;
9. publish the GitHub Release only after every preceding step succeeds.

Pushing the tag in step 4 triggers `.github/workflows/release.yml`, which
builds `aarch64-apple-darwin`, `x86_64-apple-darwin`,
`aarch64-unknown-linux-musl`, and `x86_64-unknown-linux-musl`, then uploads the
archives to the draft Release created in step 5. Step 6 therefore blocks on CI
and can take several minutes; it gives up after 30 minutes.

mise is not a separate upload destination. Its Cargo backend discovers and
installs the version published to crates.io, and its `ubi` backend reads the
archives attached to the GitHub Release.

## Prereleases

The default prerelease channel is `rc`. Capture and validate the exact target
the same way:

```sh
VERSION="$(make --no-print-directory prerelease-version CHANNEL=rc)"
make prerelease-check VERSION="$VERSION" CHANNEL=rc
make prerelease VERSION="$VERSION" CHANNEL=rc PUBLISH=1
```

The one-command publishing form is:

```sh
make prerelease PUBLISH=1
```

From `0.1.0`, the default is `0.1.1-rc.1`. After that release completes, the
next default is `0.1.1-rc.2`. To use another channel, run the check and publish
with the same channel:

```sh
VERSION="$(make --no-print-directory prerelease-version CHANNEL=beta)"
make prerelease-check VERSION="$VERSION" CHANNEL=beta
make prerelease VERSION="$VERSION" CHANNEL=beta PUBLISH=1
```

An exact prerelease also needs a matching check:

```sh
make prerelease-check VERSION=0.2.0-alpha.1
make prerelease VERSION=0.2.0-alpha.1 PUBLISH=1
```

Prereleases go to crates.io and a GitHub Release marked as a prerelease. They
do not replace the stable Homebrew Formula. Install one explicitly:

```sh
cargo install bootmux --version 0.2.0-alpha.1 --locked
mise use -g cargo:bootmux@0.2.0-alpha.1
```

## Failure and recovery

`make release-check` and `make prerelease-check` run in an isolated temporary
Git worktree. They leave the caller's checkout untouched and publish no
external state.

The publish flow uses the draft GitHub Release as its transaction boundary.
The main preflight runs before the release-preparation commit or any external
write; the clean package is checked before the first push; the tag-derived
Formula and clean tap are checked before crates.io publication; and the public
GitHub Release is always the final step.

Keep stable release notes under the exact `## [Unreleased]` heading in
`CHANGELOG.md`. Prereleases leave that section open; the next stable release
turns it into a dated version section.

crates.io versions cannot be overwritten. If publishing stops after a
release-preparation commit, tag, draft Release, crate, or Formula update,
rerun with the target printed in the original release plan:

```sh
make release VERSION=0.2.0 PUBLISH=1
# or
make prerelease VERSION=0.2.0-rc.1 PUBLISH=1
```

If `VERSION` was omitted, rerunning the same publish target also recognizes an
unfinished current version and resumes it instead of incrementing again.
Exact `VERSION` remains the clearest cross-machine recovery method.

The script verifies matching existing metadata and skips completed steps. A
stable failure can leave the GitHub Release in draft while the tap or its
main-repository Formula mirror is being completed; the retry finishes those
steps before making the Release public. Conflicting tags, crates, or Release
metadata stop the process instead of being replaced.

[Development and verification](development.md) ·
[中文发布指南](releasing.zh-CN.md)
