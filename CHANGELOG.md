# Changelog

All notable changes to bootmux are documented here.

## [Unreleased]

## [0.1.4] - 2026-08-10

- Publish prebuilt executables with every tagged release, covering macOS and
  Linux on both ARM64 and x86-64. Each archive carries the `bootmux` executable
  and the Bash, Zsh, and Fish completion files, and a `SHA256SUMS` manifest is
  attached alongside them.
- The Homebrew formula now installs the prebuilt executable instead of
  compiling the tagged release. Installing it previously pulled Homebrew's
  `rust` and its `llvm` runtime dependency — roughly 1 GB of downloads to
  produce a 4.5 MB executable. The stable formula now declares no dependencies
  at all; `brew install --HEAD` still builds from source.
- Document `mise use -g ubi:griffinqiu/bootmux`, which installs the same
  prebuilt executable without a Rust toolchain.

## [0.1.3] - 2026-08-06

- Accept Herdr socket protocol 19 (Herdr 0.8.0) alongside protocol 17. Herdr
  0.8.0 kept every CLI shape and JSON field bootmux reads, so upgrading Herdr
  no longer fails with `client Herdr protocol 19 is unsupported`.

## [0.1.2] - 2026-07-31

- Add a native zellij backend requiring zellij 0.44 or newer. A project becomes
  a zellij session built from a single generated KDL layout, windows become
  tabs, and pane commands are typed into each pane's shell so the shell survives
  them, matching tmux `send-keys` semantics.
- Support `--backend zellij` across `start`, `stop`, `stop-all`, `local`,
  `debug`, `list --active`, `doctor`, `picker`, `bindings`, `--append`, and
  `config set default-backend`.
- Detect an active zellij environment from `ZELLIJ`, `ZELLIJ_SESSION_NAME`, or
  `ZELLIJ_PANE_ID`. When several multiplexers are nested and the foreground one
  cannot be identified, the error now names every candidate it saw.
- `bootmux --backend zellij debug` prints the exact KDL layout, offline.
- Warn and ignore `socket_name` and `socket_path` on zellij, which derives its
  socket from the session name; reject a session name zellij cannot hold rather
  than mangling it.

## [0.1.1] - 2026-07-30

- Document mise's 24-hour minimum release age and use an immediately
  installable exact Cargo version.
- Add guarded stable and prerelease automation for crates.io, GitHub Releases,
  and the Homebrew tap.
- Add Linux, macOS, and Rust 1.89 CI coverage.

## 0.1.0 - 2026-07-30

First stable release.

- Run tmuxinator-style YAML projects natively in tmux or Herdr.
- Translate windows, panes, commands, working directories, hooks, focus, and
  tmux layouts into backend-specific operations.
- Support the non-executable settings template syntax and pinned fixture
  behavior from willfish/mux.
- Select backends explicitly, from the active multiplexer, or from global
  settings.
- Track exact Herdr workspace ownership for safe stop and stop-all behavior.
- Provide an fzf project picker, tmux/Herdr binding generators, and static
  Bash, Zsh, and Fish completions.
- Include English and Simplified Chinese README files and complete user
  manuals.
