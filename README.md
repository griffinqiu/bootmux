# bootmux

English | [简体中文](README.zh-CN.md)

Run one tmuxinator-style YAML project in [tmux](https://github.com/tmux/tmux),
[Herdr](https://herdr.dev/), or [zellij](https://zellij.dev/).
bootmux is a single Rust binary: it does not require Ruby and it does not hide
one multiplexer inside another's pane.

```sh
bootmux start myproject
```

## Why bootmux?

- **One project format, three native backends.** A project becomes a tmux
  session, a Herdr workspace, or a zellij session; windows become tmux windows,
  Herdr tabs, or zellij tabs, and panes stay real panes in the selected
  multiplexer.
- **tmuxinator paths and schema.** Existing projects can remain in
  `~/.config/tmuxinator`, `~/.tmuxinator`, `$TMUXINATOR_CONFIG`, or
  `./.tmuxinator.yml`. See the documented
  [compatibility boundaries](docs/mux-compatibility.md).
- **No executable templates.** MiniJinja provides variables, conditionals, and
  loops without evaluating Ruby. The non-executable settings placeholder used
  by [willfish/mux](https://github.com/willfish/mux) is also supported. Project
  pane commands and lifecycle hooks are still shell commands, so only run
  trusted YAML.
- **Tested compatibility.** Three representative tmux renderings are checked
  byte-for-byte against tmuxinator-derived golden snapshots, and the same three
  projects pin their zellij KDL layouts. A separate 19-file matrix exercises
  the pinned mux fixtures on all three backends.

## Requirements

- A Unix-like operating system
- At least one multiplexer:
  - tmux >= 2.6
  - Herdr >= 0.7.5 using socket protocol 17 or 19
  - zellij >= 0.44
- Rust and Cargo >= 1.89 to install or build bootmux
- `$SHELL` and `$EDITOR` for normal project and editor workflows
- Optional: `fzf` for `bootmux picker`

## Installation

With Cargo:

```sh
cargo --version
cargo install bootmux --locked
bootmux version
```

With [mise's Cargo backend](https://mise.jdx.dev/dev-tools/backends/cargo.html):

```sh
mise use -g rust
mise use -g cargo:bootmux@0.1.3
bootmux version
```

The explicit version also works during mise's default 24-hour safety delay for
newly published releases. After a release has aged past that delay,
`mise use -g cargo:bootmux` selects the latest eligible version.

With Homebrew:

```sh
brew install griffinqiu/tap/bootmux
bootmux --version
```

Or build this checkout instead of the crates.io release:

```sh
cargo install --path . --locked
bootmux version
```

Cargo and mise install the executable only. Static Bash, Zsh, and Fish
completion files are in [`completion/`](completion/); copy the
matching-version file from the source tree into your shell's completion
directory. The Homebrew formula installs all three completion files.

After installation, check each backend you intend to use:

```sh
bootmux --backend tmux doctor
bootmux --backend herdr doctor
```

## Quick start

Create `~/.config/tmuxinator/myapp.yml`:

```yaml
name: myapp
root: ~/code/myapp

windows:
  - editor:
      panes:
        - nvim
        - git status
  - server: npm run dev
  - logs: tail -f logs/development.log
```

Replace `root` with an existing directory and use commands installed on your
machine before starting the example.

Preview the selected backend, then start and stop the project:

```sh
bootmux --backend tmux debug myapp
bootmux --backend tmux start myapp
bootmux --backend tmux stop myapp

bootmux --backend herdr debug myapp
bootmux --backend herdr start myapp
bootmux --backend herdr stop myapp

bootmux --backend zellij debug myapp
bootmux --backend zellij start myapp
bootmux --backend zellij stop myapp
```

`debug` validates and renders the backend plan without creating the project
topology. Herdr and zellij debug do not contact a server; zellij debug prints
the exact KDL layout bootmux would use. tmux debug may start a tmux server to
read `base-index`, `pane-base-index`, and active-session state.

Once the project is working, the explicit backend is optional. bootmux resolves
it in this order:

1. `--backend tmux|herdr|zellij`
2. the active multiplexer environment
3. `default_backend` in bootmux's global settings
4. tmux

```sh
bootmux config set default-backend herdr
bootmux myapp                 # shorthand for: bootmux start myapp
bootmux                       # local project when present, otherwise the fzf picker
```

An active Herdr popup takes precedence over inherited tmux variables. If several
multiplexers are genuinely nested and bootmux cannot identify the foreground
owner, it fails with a request for an explicit backend.

## Existing tmuxinator or mux project?

Keep the file in its current directory and preflight it on every backend:

```sh
bootmux --backend tmux debug PROJECT
bootmux --backend herdr debug PROJECT
bootmux --backend zellij debug PROJECT
```

Then review the [compatibility matrix and migration
steps](docs/mux-compatibility.md) before the first non-tmux start.

## Backend overview

| Capability | tmux | Herdr | zellij |
|---|---|---|---|
| Native project container | Session | Workspace | Session |
| Window mapping | Window | Tab | Tab |
| Pane command transport | `send-keys` | `pane run` | `write-chars` + `send-keys Enter` |
| Pane commands and working directories | Yes | Yes | Yes |
| Named and custom sockets | Yes | Yes | No: `socket_name`/`socket_path` are ignored |
| tmux preset layouts | Native | Translated to a BSP plan | Translated to a KDL layout |
| Serialized tmux layouts | Native | Strictly parsed and translated | Strictly parsed and translated |
| `synchronize` | Yes | Truthy values rejected | Warned and ignored |
| `tmux_options`, `tmux_command`, pane-border fields | Supported | Warned and ignored | Warned and ignored |
| Stop identity | Config-rendered session/socket; no ownership state | Persisted exact endpoint/config/name/root ownership | Session name; no ownership state |

[Herdr session restore](https://herdr.dev/docs/session-state/) restores terminal
topology and working directories, not arbitrary child process state. Reusing a
matching workspace or session runs `on_project_restart`; it does not rerun every
pane command. See [Backends and lifecycle](docs/backends.md) before relying on
reboot recovery or `stop-all`.

A normal Herdr or zellij start maps one project to one workspace or session.
`--append` instead adds tabs to the active one and does not create an
independently stoppable project.

## Documentation

- Start here: [Getting started](docs/getting-started.md)
- End-to-end guide: [Complete user manual](docs/manual.md)
- Reference: [CLI](docs/cli.md), [project configuration](docs/configuration.md),
  [backends and lifecycle](docs/backends.md), and
  [mux compatibility](docs/mux-compatibility.md)
- Contributors: [Development and verification](docs/development.md)
- Maintainers: [Release guide](docs/releasing.md) ·
  [中文发布指南](docs/releasing.zh-CN.md)

## Picker bindings

`fzf` is only required for the picker. Print a safe snippet and paste it into
the matching multiplexer configuration:

```sh
bootmux bindings tmux
bootmux bindings herdr
```

The tmux snippet uses a normal window so it remains compatible with tmux 2.6.
The Herdr snippet opens an 80% popup.

## License and credits

bootmux is MIT-licensed and is an independent reimplementation. tmuxinator is
copyright its contributors and MIT-licensed.

The Herdr backend used
[willfish/mux at `927030b`](https://github.com/willfish/mux/tree/927030bb88e4b16b6671f68610980491ffbd2c81)
as a behavioral reference; its implementation was not copied. The upstream
YAML fixtures are vendored with their
[source and license attribution](tests/fixtures/mux/README.md).
