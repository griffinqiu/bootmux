# CLI reference

```text
bootmux [--backend tmux|herdr] [COMMAND]
```

`--backend` is global. Put it before the subcommand for one syntax that works
uniformly across all commands. When it is omitted, bootmux resolves the active
backend as described in
[Backend selection](backends.md#backend-selection).

## Invocation shortcuts

| Invocation | Behavior |
|---|---|
| `bootmux PROJECT` | Equivalent to `bootmux start PROJECT` when that project exists |
| `bootmux` | Starts a local `.tmuxinator.y[a]ml`, otherwise opens the picker |
| `bootmux .` | Alias for `bootmux local` |
| `bootmux -v` | Equivalent to `bootmux version` |

Command names and aliases take precedence over project shorthand. To start a
project named like an alias, make the command explicit:

```sh
bootmux start l
bootmux debug st
```

## Project lifecycle

### `start`

```text
bootmux start [OPTIONS] [PROJECT] [ARGS]...
```

Alias: `s`.

| Option | Meaning |
|---|---|
| `-a`, `--attach` | Override YAML and attach/focus after creation |
| `--no-attach` | Override YAML and do not attach/keep focus |
| `-n`, `--name NAME` | Run the project under another session/workspace name |
| `-p`, `--project-config PATH` | Load an explicit YAML file |
| `--append` | Add the configured windows/tabs to the current project container |
| `--no-pre-window` | Skip top-level `pre_window` commands |
| `--suppress-tmux-version-warning` | Suppress the tmux minimum-version warning |

Arguments containing a non-empty `key=value` prefix become template
`settings`. Other arguments remain in the `args` list:

```sh
bootmux start myapp root=/work/myapp production
```

In this example, `settings.root` is `/work/myapp` and `args[0]` is
`production`. If the same key is supplied more than once, the first value wins.

With `-p`, a positional token in the `PROJECT` slot is normalized into the
template argument list because the explicit file already identifies the
project:

```sh
bootmux start -p ./project.yml production root=/work/myapp
```

`-n/--name` is a start/debug override; `stop` has no matching name option. For
repeatable alternate instances, template the YAML `name` and pass the same
setting to both start and stop. A tmux session created only with `-n` may need
direct tmux cleanup, while a Herdr workspace remains eligible for ownership-
based `stop-all`.

### `debug`

```text
bootmux debug [OPTIONS] [PROJECT] [ARGS]...
```

`debug` accepts the start options except the tmux version-warning flag. It
validates the config and prints the generated tmux script or Herdr operation
plan without creating the project topology.

Herdr debug is offline. tmux debug may query/start a tmux server to determine
indices and existing-session state.

### `stop`

```text
bootmux stop [OPTIONS] [PROJECT] [ARGS]...
```

Alias: `st`.

Options:

- `-p`, `--project-config PATH`
- `--suppress-tmux-version-warning`

Reproduce the same template inputs used at start whenever they influence
project identity, socket selection, or `on_project_stop`: `key=value`
settings, positional args, and referenced environment variables.

```sh
bootmux stop myapp root=/work/myapp
bootmux stop -p ./project.yml root=/work/myapp
```

### `stop-all`

```text
bootmux stop-all [-y|--noconfirm]
```

Alias: `stop_all`.

Without `-y`, bootmux lists the selected backend's candidate active projects
and asks for confirmation.

- tmux compares names from the ambient tmux server (the current server when
  inside tmux, normally the default server outside) with discoverable `.yml`
  project basenames. It then renders each candidate config's own tmux/socket
  command for stop, which may target a different server. This is a heuristic,
  not ownership tracking; it can miss `.yaml`, `-p`, `-n`, template-name, and
  custom-socket cases.
- Herdr stops only workspaces proven by bootmux's ownership state. It never
  stops the Herdr server.

See [Stop safety](backends.md#herdr-stop-safety).

### `local`

```text
bootmux local [--suppress-tmux-version-warning]
```

Alias: `.`. Starts `./.tmuxinator.yml` or `./.tmuxinator.yaml`.

## Project files

| Command | Aliases | Behavior |
|---|---|---|
| `new NAME` | `n` | Create a project if missing and open it in `$EDITOR` |
| `new NAME SESSION` | `n` | Generate YAML by introspecting an existing tmux session; tmux only |
| `open NAME` | `o` | Create or open a project |
| `edit [NAME]` | `e` | Edit an existing project |
| `copy EXISTING NEW` | `c`, `cp` | Copy a project and open the destination |
| `delete PROJECT...` | `d`, `rm` | Confirm and delete one or more project files |
| `implode` | `i` | Confirm and delete all project directories in the active config scope |

`new`, `open`, and `edit` accept `-l`/`--local` to target
`./.tmuxinator.yml`. `edit` requires either a name or `--local`.

Without `--local`, `new`, `open`, and `edit` target `NAME.yml` in bootmux's
current write directory: `$TMUXINATOR_CONFIG`, otherwise an existing XDG
directory, otherwise an existing `~/.tmuxinator`, otherwise a newly created
XDG directory. They do not recursively locate a same-named file in a lower-
priority directory. `open NAME` can therefore create a new higher-priority
file, while `edit NAME` can report missing even when a lower-priority copy
exists. Set `TMUXINATOR_CONFIG` deliberately or edit that file directly.

`new NAME SESSION` reads tmux windows, layouts, pane working directories, and
session options. It is not a Herdr workspace importer; selecting Herdr for that
form is an error.

`delete` and `implode` remove files/directories after confirmation. Review the
selected config path before using them.

## Discovery and diagnostics

| Command | Aliases | Behavior |
|---|---|---|
| `list` | `l`, `ls` | List discovered project names |
| `list -n` | | Print one project per line |
| `list -a` | | Filter to active projects on the selected backend |
| `picker` | | Select and start a project with `fzf` |
| `doctor` | | Check the selected backend, optional `fzf`, `$EDITOR`, and `$SHELL` |
| `version` | | Print the bootmux version |
| `commands [SHELL]` | | Print command names for shell integration |
| `completions ARG` | | Internal project-name completion helper |

`picker` treats `fzf` exit codes 1 and 130 as normal cancellation.

Static completion scripts are provided in:

```text
completion/bootmux.bash
completion/bootmux.zsh
completion/bootmux.fish
```

Load Bash directly:

```sh
source /path/to/bootmux/completion/bootmux.bash
```

For Zsh, initialize its completion system, source the function, and register
it explicitly:

```zsh
autoload -Uz compinit && compinit
source /path/to/bootmux/completion/bootmux.zsh
compdef _bootmux bootmux
```

Fish can source the file for the current shell, or install it in the normal
Fish completions directory:

```fish
source /path/to/bootmux/completion/bootmux.fish
```

## Picker bindings

```text
bootmux bindings tmux [--key KEY]
bootmux bindings herdr [--key KEY]
```

The default tmux key is `F` under the tmux prefix. The snippet uses
`new-window`, which is available in tmux 2.6.

The default Herdr key is `prefix+shift+f`. The generated TOML opens
`bootmux picker` in an 80% by 80% popup.

## Global settings

```sh
bootmux config get default-backend
bootmux config set default-backend tmux
bootmux config set default-backend herdr
bootmux config path
```

The settings file is:

```text
${XDG_CONFIG_HOME:-$HOME/.config}/bootmux/config.toml
```

Its corresponding TOML is:

```toml
default_backend = "herdr"
```

The CLI updates this key atomically and preserves unrelated content.

## Project lookup

Named projects are searched recursively by basename, accepting `.yml` and
`.yaml`, in this order:

1. `$TMUXINATOR_CONFIG`, when set
2. `$XDG_CONFIG_HOME/tmuxinator`, defaulting to `~/.config/tmuxinator`
3. `~/.tmuxinator`

An explicit `-p PATH` always wins. When no project name is supplied, bootmux
uses `./.tmuxinator.yml` and then `./.tmuxinator.yaml`.

Within each global directory, a direct `NAME.yml`/`NAME.yaml` or direct
relative path is preferred; recursive basename matches are sorted. Use a
relative project name such as `team/api` when duplicate basenames exist.

Project creation chooses `$TMUXINATOR_CONFIG` when set, otherwise an existing
XDG directory, otherwise an existing `~/.tmuxinator`, and finally creates the
XDG directory.

For tmuxinator parity, `list` discovers `.yml` files; direct lookup also
accepts `.yaml`. When `$TMUXINATOR_CONFIG` is set, enumeration commands use
that directory alone; otherwise they combine the existing XDG and
`~/.tmuxinator` directories.

[Documentation index](../README.md#documentation) ·
[Complete user manual](manual.md)
