# Project configuration

bootmux reads tmuxinator-style YAML after rendering its safe template syntax.
The same portable fields can target tmux or Herdr; backend-specific behavior is
called out below and expanded in [Backends and lifecycle](backends.md).

## Complete example

```yaml
name: myapp
root: ~/code/myapp
attach: true

pre_window:
  - export APP_ENV=development

on_project_start: docker compose up -d
on_project_first_start: bin/setup
on_project_restart: echo "project already exists"
on_project_exit: echo "bootmux start is exiting"
on_project_stop: docker compose down

socket_name: myapp
startup_window: editor
startup_pane: shell

enable_pane_titles: true
pane_title_position: top
pane_title_format: "[ #T ]"

windows:
  - editor:
      root: app
      layout: main-vertical
      pre: source .env
      focused_pane: shell
      panes:
        - editor: nvim
        - shell:
        - tests:
            - cargo test
            - cargo watch -x test
  - server: cargo run
  - logs:
      - tail -f logs/app.log
      - tail -f logs/worker.log
```

## Top-level fields

| Field | Value | Behavior |
|---|---|---|
| `name` | scalar, required | tmux session name or Herdr workspace label |
| `root` | path | Project working directory; defaults to the current directory |
| `windows` | non-empty sequence, required | Windows/tabs to create |
| `attach` | scalar | Attach/focus after start; defaults to true |
| `pre_window` | string or list | Run in every configured pane before window/pane commands |
| `on_project_start` | string or list | Run on every start attempt |
| `on_project_first_start` | string or list | Run when creating topology; both backends also run it for append |
| `on_project_restart` | string or list | Run when a matching project already exists |
| `on_project_exit` | string or list | Run as the start operation exits |
| `on_project_stop` | string or list | Run before a confirmed stop |
| `startup_window` | name or index | Final selected window/tab; defaults to the first |
| `startup_pane` | title or index | Final pane in `startup_window`; otherwise that window's `focused_pane` |
| `socket_name` | scalar | tmux `-L` name or named Herdr session |
| `socket_path` | path | tmux `-S` path or `HERDR_SOCKET_PATH`; wins over `socket_name` |
| `tmux_options` | scalar | Extra tmux CLI options |
| `tmux_command` | scalar | Replace the `tmux` executable, for example `wemux` |
| `enable_pane_titles` | scalar | Enable tmux pane-border titles |
| `pane_title_position` | `top`, `bottom`, or `off` | tmux pane-border position |
| `pane_title_format` | scalar | tmux pane-border format |

Herdr warns and ignores `tmux_options`, `tmux_command`, and pane-border fields.
It still uses pane mapping keys as Herdr pane labels.

A Herdr `socket_name` must contain 1–64 ASCII letters, digits, `.`, `_`, or
`-`.

For a portable `socket_path`, use an absolute unambiguous path. tmux passes the
configured value to `-S`; Herdr expands it relative to the invocation
environment before recording endpoint identity.

Project names have `.` and `:` replaced with `_` when targeting tmux because
those characters conflict with tmux target syntax. Herdr retains the project
name as the workspace label.

### Deprecated aliases

These mux/tmuxinator aliases are accepted:

| Alias | Canonical field |
|---|---|
| `project_name` | `name` |
| `project_root` | `root` |
| `tabs` | `windows` |
| `cli_args` | `tmux_options` |

When aliases conflict, YAML document order applies: the last value of the
correct type wins. An invalid scalar alias or an empty window sequence does not
erase an earlier valid value.

The obsolete top-level fields `rbenv`, `rvm`, `pre_tab`, `pre`, and `post` are
accepted as no-ops for mux compatibility. Window-level `pre` remains active.

## Window forms

Every entry in `windows` must be a one-entry mapping. The mapping key is the
window/tab name.

### One command

```yaml
windows:
  - server: npm run dev
```

### Several commands in one pane

```yaml
windows:
  - setup:
      - git fetch
      - git status
```

### Window options and panes

```yaml
windows:
  - editor:
      root: app
      layout: main-vertical
      pre:
        - source .env
      synchronize: after
      focused_pane: shell
      panes:
        - editor: nvim
        - shell:
```

| Window field | Value | Behavior |
|---|---|---|
| `root` | path | Working directory for the window; relative paths resolve under project `root` |
| `layout` | preset or serialized tmux layout | Arrange the window's panes |
| `pre` | string or list | Run in each pane after top-level `pre_window` |
| `synchronize` | `before`, `after`, true, or false | Enable tmux synchronized input before/after commands |
| `focused_pane` | pane title or zero-based index | Selected pane retained by the window |
| `panes` | scalar, mapping, or sequence | Pane definitions |

`synchronize` is tmux-only. Herdr rejects a truthy value because pretending to
support synchronized interactive input would change the config's meaning.
Use the YAML boolean `false` to disable it; the string `"false"` is truthy.

## Pane forms

### Untitled pane with one command

```yaml
panes:
  - nvim
```

### Empty shell pane

```yaml
panes:
  -
```

### Titled pane

```yaml
panes:
  - editor: nvim
  - shell:
```

### Several commands in one pane

```yaml
panes:
  - [git fetch, git status]
  - tests:
      - cargo test
      - cargo watch -x test
```

Commands are sent/run in order. For every pane, the order is:

1. top-level `pre_window`
2. window-level `pre`
3. the pane's commands

`--no-pre-window` skips only the first item.

## Sequential pane chains

bootmux also accepts an explicit split chain on both backends:

```yaml
windows:
  - app:
      panes:
        - editor:
            command: nvim
        - server:
            split: right
            ratio: 0.65
            commands:
              - npm run dev
        - logs:
            split: down
            ratio: 0.5
            command: tail -f logs/development.log
```

Each pane after the first splits the pane immediately before it.

- `split` is `right` or `down`; the default is `right`.
- `ratio` is the share retained by the existing pane, from 0.1 through 0.9;
  the default is 0.5.
- `command` and `commands` are mutually exclusive.
- The first pane cannot set `split` or `ratio`.
- A pane chain cannot also set the window's `layout`.

On tmux, the split is emitted directly with `splitw -h/-v -p`. On Herdr, the
same chain becomes its native BSP split plan.

## Layouts

Portable preset names are:

- `tiled`
- `even-horizontal`
- `even-vertical`
- `main-horizontal`
- `main-vertical`

tmux applies these natively. Herdr translates them to a deterministic BSP plan.

A serialized tmux layout string is also accepted. Herdr verifies its checksum,
parses the split tree, checks pane count, and rejects ratios it cannot represent
instead of silently changing the topology.

See [Layout behavior](backends.md#layouts) for backend details.

## Working directories

Paths support `~` expansion.

- A missing top-level `root` uses the process's current directory.
- A relative window `root` resolves beneath the expanded project root.
- An absolute window `root` remains absolute.

Create working directories before start. tmux starts project setup with
`/bin/sh -e`, so a failed `cd` aborts the generated start script. Herdr
preflights the config, then creates panes with the requested working directory.

## Focus and attachment

`focused_pane` controls the selected pane saved inside each window/tab.
`startup_window` and `startup_pane` control the final project selection.
Pane indices in YAML are zero-based; tmux automatically adjusts for
`pane-base-index`.

For a portable `startup_window`, prefer a window name. Herdr treats a number as
a zero-based logical window index; tmux treats it as a tmux target affected by
`base-index`. An invalid `focused_pane` falls back to the first pane on tmux,
while Herdr rejects it during preflight.

Attachment precedence is:

1. `--attach`
2. `--no-attach`
3. YAML `attach`
4. default true

For strict mux compatibility, only the raw scalar spellings `false` and `0`
(including their quoted forms) mean false. Spellings such as `False`, `+0`,
`00`, `0x0`, and `0.0` are treated as true. Prefer an unquoted lowercase
boolean:

```yaml
attach: false
```

## Hooks

Hooks can be a string or a YAML list. Lists are joined with `; `:

```yaml
on_project_stop:
  - docker compose down
  - rm -f tmp/dev.pid
```

The lifecycle is:

```text
start
  on_project_start
  create -> on_project_first_start -> create topology
  reuse  -> on_project_restart
  on_project_exit

stop
  on_project_stop
  close session/workspace
```

On both backends, a successful append follows
`on_project_start` → `on_project_first_start` → create appended windows/tabs →
`on_project_exit`.

Hooks run from the project root. A newly managed Herdr project snapshots the
fully rendered stop hook in its ownership state. Regular stop refuses a changed
rendered hook; `stop-all` uses the trusted snapshot and therefore does not need
missing template inputs. Legacy ownership state without a snapshot may still
need to read the config.

## MiniJinja templates

The YAML source is rendered before parsing with these values:

- `settings`: non-empty `key=value` CLI arguments
- `args`: remaining positional arguments
- `env`: the process environment

```yaml
name: {{ settings.session | default('myapp') }}
root: {{ env.HOME }}/code/{{ args[0] }}
windows:
  - server: npm run {{ settings.task | default('dev') }}
```

Run it with:

```sh
bootmux start myapp session=myapp-dev frontend task=serve
```

Undefined values render as empty strings. Quote or default values when an empty
result would make the YAML ambiguous.

The first duplicate `key=value` wins. A token with an empty key, such as
`=value`, stays in `args`.

## Restricted mux ERB

The non-executable placeholder supported by willfish/mux works unchanged:

```yaml
root: <%= @settings["root"] %>
```

```sh
bootmux start myapp root=/work/myapp
```

Only the exact `@settings["key"]` expression is supported. bootmux never
evaluates Ruby. Convert other ERB forms to MiniJinja:

| tmuxinator ERB | bootmux |
|---|---|
| `<%= @args[0] %>` | `{{ args[0] }}` |
| `<%= ENV["HOME"] %>` | `{{ env.HOME }}` |
| `<%= @settings["root"] %>` | Keep as-is, or use `{{ settings.root }}` |

Reproduce the same template inputs for `stop` if they affect project name,
root, endpoint, or the rendered stop hook: settings, positional args, and
referenced environment variables.

[Documentation index](../README.md#documentation) ·
[Complete user manual](manual.md)
