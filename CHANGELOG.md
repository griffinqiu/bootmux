# Changelog

All notable changes to bootmux are documented here.

## [Unreleased]

- zellij 0.45.0: preset and serialized layouts now reach the terminal with the
  proportions they ask for. bootmux used to nest every extra split, and zellij
  merges a container into its parent whenever both split the same way while
  keeping the merged children's percentages as written, so any run of three or
  more panes in one direction came out skewed — a five-pane `even-horizontal`
  window opened at 13/13/12/6/6 columns instead of five equal ones, and the top
  row of a five-pane `tiled` window at 17/24/9 instead of 17/17/16. bootmux now
  emits each such run as one flat container and sizes every child from its
  cumulative boundary, so the shares always sum to one hundred. Affects
  `tiled`, `even-horizontal`, `even-vertical`, `main-horizontal`,
  `main-vertical`, serialized tmux layouts, and pane chains on the zellij
  backend only; the tmux and Herdr backends were already correct. No
  configuration change and no migration are required — restart a zellij session
  to pick up the corrected layout.

## [0.3.1] - 2026-08-21

- Herdr 0.8.2: accept socket protocol 20 after verifying that the structured
  CLI, JSON responses, direct pane-focus request, and complete lifecycle matrix
  used by bootmux remain compatible. Existing Herdr 0.7.5 and 0.8.0 users stay
  supported, and upgrading requires no bootmux configuration or state migration.
- zellij 0.45.0: keep a real client attached during the required append and
  rollback matrix rows, matching zellij's new per-client tab sizing. The
  complete runtime contract remains compatible; users need no configuration or
  workflow changes.

## [0.3.0] - 2026-08-14

- Herdr: one templated config can again run several alternate instances at the
  same time. Ownership is now keyed by endpoint, config path, and the rendered
  project name, so starting a second instance from the same config no longer
  fails with `refusing an identity-mismatched lifecycle operation`, and stopping
  one instance leaves its siblings running — matching the tmux backend and the
  manual's "Run alternate instances". Reported against Herdr 0.8.0 in
  [#2](https://github.com/griffinqiu/bootmux/issues/2). A stop whose rendered
  name owns no workspace now reports that instead of failing; the label, root,
  workspace-identity, and stop-hook checks within a matched name are unchanged.
  No user action and no state migration are required, because every ownership
  record already stores its project name. The required Herdr runtime matrix
  gained an `alternate_instances` row covering this against Herdr 0.8.0.

## [0.2.0] - 2026-08-13

- Herdr and zellij: a successful `start`, `local`, or `stop` now prints one line
  on stdout naming the outcome and the workspace or session it applied to, such
  as `bootmux: created herdr workspace "myapp" (socket:…)` or
  `bootmux: stopped zellij session "myapp"`. Both backends place the result
  outside the terminal that ran the command, so exiting 0 in silence was
  indistinguishable from doing nothing — reported against Herdr 0.8.0 and
  zellij 0.44.3 in
  [#1](https://github.com/griffinqiu/bootmux/issues/1). This covers the create,
  reuse, append, and stop outcomes. The tmux backend is unchanged, because a
  tmux start hands the terminal over; `stop-all` keeps its existing output on
  every backend. No user action is required, but scripts that assumed these
  commands write nothing to stdout on success will now see one line.

## [0.1.5] - 2026-08-11

- zellij: hand rendered layouts to zellij as a file instead of an inline
  `--layout-string`. zellij only gained inline layouts in 0.44.1, so starting or
  appending a project failed outright on zellij 0.44.0 even though bootmux
  advertises it as the supported minimum. Users on zellij 0.44.0 no longer need
  to upgrade; users on 0.44.1 or newer see no change.
- tmux 3.7b, Herdr 0.8.0, and zellij 0.44.3 are each covered by a required
  real-runtime matrix: the ignored smoke suites now prove topology creation,
  roots and command order, startup focus, the documented hook order, reuse,
  `list --active`, append, explicit stop, `stop-all`, and failure rollback,
  plus backend-specific rows, and print a machine-readable marker per row. No
  user action is required.

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
