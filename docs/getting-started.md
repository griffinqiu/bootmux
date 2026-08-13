# Getting started

This guide takes a new installation through one validated start/stop cycle.
For a continuous, feature-complete walkthrough, read the
[user manual](manual.md). For exact flags and YAML fields, use the
[CLI reference](cli.md) and [configuration reference](configuration.md).

## 1. Install the prerequisites

bootmux needs at least one supported multiplexer:

- a Unix-like operating system
- tmux 2.6 or newer
- Herdr 0.7.5 or newer, using socket protocol 17 or 19
- zellij 0.44 or newer

The fastest install is Homebrew, which downloads a prebuilt executable and
installs all three completion files:

```sh
brew install griffinqiu/tap/bootmux
bootmux --version
```

mise's `ubi` backend downloads the same prebuilt executable:

```sh
mise use -g ubi:griffinqiu/bootmux
bootmux version
```

Prebuilt archives for macOS and Linux, on both Apple Silicon/ARM64 and x86-64,
are attached to every [GitHub release](https://github.com/griffinqiu/bootmux/releases)
together with a `SHA256SUMS` manifest.

Installing from source instead requires Rust and Cargo 1.89 or newer:

```sh
rustc --version
cargo --version
cargo install bootmux --locked
bootmux version
```

To install the current source checkout instead:

```sh
cargo install --path . --locked
bootmux version
```

To compile through mise instead, install Rust/Cargo and bootmux with the Cargo
backend:

```sh
mise use -g rust
mise use -g cargo:bootmux@0.3.0
bootmux version
```

mise applies a 24-hour minimum release age to fuzzy versions by default. The
explicit version works immediately; after that delay,
`mise use -g cargo:bootmux` selects the latest eligible release.

Set `$SHELL` and `$EDITOR`. Install `fzf` only if you want the interactive
picker.

Run the environment check for each backend you intend to use:

```sh
bootmux --backend tmux doctor
bootmux --backend herdr doctor
bootmux --backend zellij doctor
```

## 2. Create a project

The default project directory is:

```text
${XDG_CONFIG_HOME:-$HOME/.config}/tmuxinator
```

`bootmux new` creates a starter file and opens it in `$EDITOR`:

```sh
bootmux new myapp
```

Use this minimal portable project for a first test:

```yaml
name: myapp
root: ~/code/myapp
attach: false

windows:
  - editor:
      panes:
        - nvim
        - git status
  - server: npm run dev
```

The `root` directory should already exist. `attach: false` makes the first run
easy to inspect without switching the current client. Remove it later or pass
`--attach` when you want bootmux to attach/focus.

## 3. Validate before starting

Choose the backend explicitly while introducing a project:

```sh
bootmux --backend tmux debug myapp
bootmux --backend herdr debug myapp
```

Both commands render and validate the selected plan without creating the
project topology.

- tmux debug prints the generated shell script. It may start a tmux server to
  read tmux indices and session state.
- Herdr debug prints the endpoint, ownership actions, tab/pane layout, command
  counts, and startup selection. It does not contact or start a Herdr server.
- zellij debug prints the resolved session name, the complete KDL layout, and
  command counts. It does not contact zellij.

Warnings in a Herdr or zellij plan normally identify tmux-only fields that will be
ignored. Errors such as `synchronize` or an unrepresentable layout must be
resolved before start.

## 4. Start, inspect, and stop

```sh
bootmux --backend tmux start myapp
bootmux --backend tmux list --active
bootmux --backend tmux stop myapp
```

Or with Herdr:

```sh
bootmux --backend herdr start myapp
bootmux --backend herdr list --active
bootmux --backend herdr stop myapp
```

Or with zellij:

```sh
bootmux --backend zellij start myapp
bootmux --backend zellij list --active
bootmux --backend zellij stop myapp
```

Repeated `start` reuses the matching session/workspace and runs
`on_project_restart`; it does not create a duplicate.

If the project uses templates in its name, root, socket, or stop hook, reproduce
the same template inputs for `stop`: `key=value` settings, positional args, and
any referenced environment variables.

```sh
bootmux --backend herdr start myapp root=/work/myapp
bootmux --backend herdr stop myapp root=/work/myapp
```

This is especially important for Herdr, where a mismatched lifecycle identity
is rejected instead of guessed.

## 5. Let bootmux select the backend

After validating both paths, set a default if desired:

```sh
bootmux config set default-backend herdr
bootmux config get default-backend
bootmux config path
```

Backend resolution order is:

1. an explicit `--backend`
2. the active tmux, Herdr, or zellij environment
3. the global `default_backend`
4. tmux

An active Herdr popup wins over variables inherited from a surrounding
tmux session. A genuinely ambiguous nested environment fails with a request
for `--backend`.

## 6. Use a project-local file

Commit `.tmuxinator.yml` or `.tmuxinator.yaml` in a repository:

```yaml
name: myapp
root: .
windows:
  - editor: nvim
  - tests: cargo test
```

From that directory:

```sh
bootmux local
bootmux .
```

A bare `bootmux` starts the local file when one exists. Otherwise it opens the
`fzf` project picker.

## 7. Add the picker

Install `fzf`, then inspect the generated binding for your multiplexer:

```sh
bootmux bindings tmux
bootmux bindings herdr
bootmux bindings zellij
```

Paste the printed snippet into `tmux.conf`, the Herdr TOML configuration, or
the zellij KDL configuration.
Use `--key` to select another key:

```sh
bootmux bindings tmux --key C-f
bootmux bindings herdr --key prefix+alt+f
```

## Next steps

- Follow the [complete user manual](manual.md) for real project workflows.
- Read [Project configuration](configuration.md) for every supported field.
- Read [Backends and lifecycle](backends.md) before using Herdr sockets,
  `--append`, restart hooks, or `stop-all`.
- Read [mux compatibility](mux-compatibility.md) before migrating a large
  tmuxinator/mux collection.
