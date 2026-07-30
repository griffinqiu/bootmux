# bootmux

English | [简体中文](README.zh-CN.md)

Run one tmuxinator-style YAML project in either
[tmux](https://github.com/tmux/tmux) or [Herdr](https://herdr.dev/).
bootmux is a single Rust binary: it does not require Ruby and it does not hide
tmux inside a Herdr pane.

```sh
bootmux start myproject
```

## Why bootmux?

- **One project format, two native backends.** A project becomes a tmux session
  or a Herdr workspace; windows become tmux windows or Herdr tabs, and panes
  stay real panes in the selected multiplexer.
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
  byte-for-byte against tmuxinator-derived golden snapshots. A separate
  19-file matrix exercises the pinned mux fixtures on both backends.

## Requirements

- A Unix-like operating system
- At least one multiplexer:
  - tmux >= 2.6
  - Herdr >= 0.7.5 using socket protocol 17
- Rust >= 1.89 to build from source
- `$SHELL` and `$EDITOR` for normal project and editor workflows
- Optional: `fzf` for `bootmux picker`

Install from this checkout:

```sh
cargo install --path .
bootmux doctor
```

Static Bash, Zsh, and Fish completions are in `completion/`.

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
```

`debug` validates and renders the backend plan without creating the project
topology. Herdr debug does not contact or start a server; tmux debug may start a
tmux server to read `base-index`, `pane-base-index`, and active-session state.

Once the project is working, the explicit backend is optional. bootmux resolves
it in this order:

1. `--backend tmux|herdr`
2. the active multiplexer environment
3. `default_backend` in bootmux's global settings
4. tmux

```sh
bootmux config set default-backend herdr
bootmux myapp                 # shorthand for: bootmux start myapp
bootmux                       # local project when present, otherwise the fzf picker
```

An active Herdr popup takes precedence over inherited tmux variables. If tmux
and Herdr are genuinely nested and bootmux cannot identify the foreground
owner, it fails with a request for an explicit backend.

## Existing tmuxinator or mux project?

Keep the file in its current directory and preflight it on both backends:

```sh
bootmux --backend tmux debug PROJECT
bootmux --backend herdr debug PROJECT
```

Then review the [compatibility matrix and migration
steps](docs/mux-compatibility.md) before the first Herdr start.

## Backend overview

| Capability | tmux | Herdr |
|---|---|---|
| Native project container | Session | Workspace |
| Window mapping | Window | Tab |
| Pane commands and working directories | Yes | Yes |
| Named and custom sockets | Yes | Yes |
| tmux preset layouts | Native | Translated to a BSP plan |
| Serialized tmux layouts | Native | Strictly parsed and translated |
| `synchronize` | Yes | Truthy values rejected: no equivalent input semantics |
| `tmux_options`, `tmux_command`, pane-border fields | Supported | Warned and ignored |
| Stop identity | Config-rendered session/socket; no ownership state | Persisted exact endpoint/config/name/root ownership |

[Herdr session restore](https://herdr.dev/docs/session-state/) restores terminal
topology and working directories, not arbitrary child process state. Reusing a
matching workspace runs `on_project_restart`; it does not rerun every pane
command. See [Backends and lifecycle](docs/backends.md) before relying on reboot
recovery or `stop-all`.

A normal Herdr start maps one project to one workspace. `--append` instead adds
tabs to the active workspace and does not create an independently stoppable
project.

## Documentation

- Start here: [Getting started](docs/getting-started.md)
- End-to-end guide: [Complete user manual](docs/manual.md)
- Reference: [CLI](docs/cli.md), [project configuration](docs/configuration.md),
  [backends and lifecycle](docs/backends.md), and
  [mux compatibility](docs/mux-compatibility.md)
- Contributors: [Development and verification](docs/development.md)

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
