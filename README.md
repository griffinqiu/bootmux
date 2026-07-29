# bootmux

Manage complex tmux sessions from simple YAML project files — a Rust reimplementation of [tmuxinator](https://github.com/tmuxinator/tmuxinator).

```sh
bootmux start myproject
```

One command, and your editor, server, logs, and shell windows are laid out exactly the way you left them.

## Why bootmux?

tmuxinator is a great tool with one structural problem: it's a Ruby gem.

- **No Ruby required.** tmuxinator needs a Ruby interpreter and a gem installation, which drags in rbenv/rvm/system-Ruby version juggling — ironically, often the very thing you're trying to manage *with* tmux sessions. bootmux is a single ~3 MB static binary with zero runtime dependencies. Drop it on a server, in a container, or on a fresh laptop and it just runs.
- **Instant startup.** No interpreter boot, no gem loading. Session startup latency is dominated by tmux itself, not the tool.
- **Drop-in compatible.** bootmux reads the **same YAML project files** from the **same config directories** (`~/.config/tmuxinator`, `~/.tmuxinator`, `$TMUXINATOR_CONFIG`, `./.tmuxinator.yml`). Your existing projects work unchanged — switching costs nothing, and switching back costs nothing.
- **Verified, not "inspired by".** The generated tmux script is compared **byte-for-byte** against tmuxinator's own golden test snapshots in CI. This is a port with a proof, not a lookalike.
- **Safer templating.** tmuxinator embeds ERB in YAML, which means every project file can execute arbitrary Ruby. bootmux uses sandboxed [MiniJinja](https://github.com/mitsuhiko/minijinja) templates instead: variables, conditionals, and loops — not code execution.

## Requirements

- tmux >= 2.6 (2017). Older versions trigger a warning (`--suppress-tmux-version-warning` silences it).
- `$EDITOR` and `$SHELL` set — check your setup with `bootmux doctor`.

## Installation

```sh
# From source
cargo install --path .

# From crates.io (once published)
cargo install bootmux
```

Shell completions live in `completion/bootmux.{bash,zsh,fish}` — source the one for your shell.

## Quick start

```sh
bootmux new myproject          # create a project file and open it in $EDITOR
bootmux start myproject        # build the session and attach
bootmux myproject              # shorthand for start
bootmux stop myproject         # kill the session
```

All commands:

```sh
bootmux new myproject SESSION  # generate a project file from a running session
bootmux start myproject -n alt # same project under a different session name
bootmux start myproject --append   # append this project's windows to the current session
bootmux debug myproject        # print the generated shell script instead of running it
bootmux stop-all               # stop every active bootmux-managed session
bootmux local                  # start from ./.tmuxinator.yml (alias: bootmux .)
bootmux list                   # list projects (-a: active only, -n: one per line)
bootmux edit / open / copy / delete / implode / doctor / version
```

Project files are searched in `$TMUXINATOR_CONFIG`, then `$XDG_CONFIG_HOME/tmuxinator` (default `~/.config/tmuxinator`), then `~/.tmuxinator` — recursively, matched by file basename.

## Project files

The full tmuxinator schema is supported:

```yaml
name: myapp
root: ~/code/myapp

# Runs in every window and pane before its commands
pre_window: rbenv shell 3.3.0

# Lifecycle hooks (string or list)
on_project_start: docker compose up -d
on_project_first_start: bin/setup
on_project_restart: echo "welcome back"
on_project_stop: docker compose down
on_project_exit: echo "bye"

tmux_options: -f ~/.tmux.custom.conf
socket_name: myapp          # or socket_path: /path/to/socket
startup_window: editor      # window selected on startup (name or index)
startup_pane: 1
attach: true                # set false to build the session without attaching

enable_pane_titles: true
pane_title_position: top    # top | bottom | off
pane_title_format: "[ #T ]"

windows:
  - editor:
      root: app             # per-window root, relative to the project root
      layout: main-vertical # preset or custom layout string
      pre:                  # runs in each pane of this window
        - echo "hello pane"
      synchronize: after    # before | after | false
      focused_pane: editor  # by pane title or zero-based index
      panes:
        - editor: vim       # titled pane
        - guard             # plain pane
        - [git fetch, git status]  # one pane running several commands
  - shell:                  # window with commands, no panes
      - git pull
  - logs: tail -f log/development.log
```

### Templating

Project files are rendered with MiniJinja (Jinja2 syntax) before YAML parsing:

- `settings` — `key=value` command-line arguments: `bootmux start myapp workspace=/code`
- `args` — remaining positional arguments
- `env` — process environment variables

```yaml
name: {{ settings.session | default('myapp') }}
root: {{ env.HOME }}/code/{{ args[0] }}
```

Undefined variables render as empty strings, matching tmuxinator's ERB behavior.

## Migrating from tmuxinator

Most projects need no changes at all. The differences:

| tmuxinator | bootmux |
|---|---|
| ERB templating (`<%= @settings["x"] %>`, `<%= @args[0] %>`, `<%= ENV["V"] %>`) | MiniJinja: `{{ settings.x }}`, `{{ args[0] }}`, `{{ env.V }}`. Files containing `<%` are rejected with a migration hint. |
| Deprecated options `rbenv`, `rvm`, `pre_tab`, `tabs`, `cli_args`, top-level `pre`/`post` (warnings) | Rejected with an error naming the replacement (`pre_window`, `windows`, `tmux_options`, project hooks). Window-level `pre` is still supported. |
| wemux support | Not supported. |
| tmux 1.5+ | tmux >= 2.6 only. |

Everything else is kept faithfully, including the quirks: project names have `.` and `:` replaced by `_`, the first window is re-created with `new-window -k`, every pane split is followed by a `tiled` re-layout to prevent "no space for new pane", and `base-index`/`pane-base-index` are read from your tmux configuration.

## How it works

Like tmuxinator, bootmux is not a daemon. It renders your project file into a plain shell script of tmux commands (`new-session`, `new-window`, `splitw`, `send-keys`, `select-layout`, ...) and `exec`s it. `bootmux debug` shows you that exact script — there is no hidden state and nothing magic to trust.

## Development

```sh
cargo test                              # unit + golden-snapshot + CLI integration tests
cargo test --test smoke -- --ignored    # end-to-end against a real tmux (isolated socket)
```

The golden tests in `tests/golden.rs` compare generated scripts byte-for-byte against snapshot files copied from tmuxinator's own test suite (`tests/snapshots/2.6/`). If you touch `src/script.rs`, those tests are the contract.

## License

MIT. tmuxinator is © the tmuxinator contributors (MIT); bootmux is an independent reimplementation.
