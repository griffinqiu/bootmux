# Changelog

All notable changes to bootmux are documented here.

## [Unreleased]

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
