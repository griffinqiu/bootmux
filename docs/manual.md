# bootmux user manual

English | [简体中文](manual.zh-CN.md)

This is the complete usage manual for bootmux. It starts with installation,
then follows the normal project lifecycle, common tmux/Herdr workflows, safe
stopping, migration, and troubleshooting.

Use the narrower reference pages when you need an exact lookup:

- [CLI reference](cli.md)
- [Project configuration](configuration.md)
- [Backends and lifecycle](backends.md)
- [mux compatibility](mux-compatibility.md)

## Contents

- Foundations: [model](#1-the-bootmux-model),
  [installation](#2-install-and-verify), [file locations](#3-know-where-files-live),
  [first project](#4-create-your-first-project), and
  [preflight](#5-preflight-before-start)
- Daily use: [start/stop](#6-start-inspect-and-stop),
  [backend selection](#7-choose-a-backend),
  [attachment/focus](#8-control-attachment-and-focus), and
  [windows/panes](#9-write-windows-and-panes)
- Configuration: [layouts](#10-use-layouts), [hooks](#11-add-lifecycle-hooks),
  [templates](#12-template-projects-safely), and
  [sockets](#13-use-sockets-and-named-sessions)
- Workflows: [append](#14-append-another-project),
  [picker](#15-use-the-picker), [tmux import](#16-import-an-existing-tmux-session),
  [file management](#17-manage-project-files), and
  [safe stopping](#18-stop-safely)
- Operations: [reboot/recovery](#19-plan-for-reboot-and-process-recovery),
  [migration](#20-migrate-tmuxinator-or-mux-projects),
  [troubleshooting](#21-troubleshooting),
  [environment reference](#22-environment-quick-reference), and
  [command sheet](#23-command-quick-sheet)

For a first run, read sections 1–7. For project authoring, continue through
sections 8–13. Sections 14–21 cover daily operations, migration, recovery, and
troubleshooting.

## 1. The bootmux model

One YAML project describes:

- a project name and working directory;
- one or more windows;
- optional panes inside each window;
- the command sequence for each pane;
- optional hooks, focus, layout, attachment, and socket settings.

bootmux translates that intent natively:

| Project concept | tmux | Herdr | zellij |
|---|---|---|---|
| Project | Session | Workspace | Session |
| Window | Window | Tab | Tab |
| Pane | Pane | PTY pane | Pane |
| Pane command | `send-keys` | `pane run` | `write-chars` + `send-keys Enter` |
| Layout | tmux layout | Herdr BSP splits | KDL layout |

A normal start creates or reuses one container. `--append` is the exception: it
adds windows/tabs to the current container instead of creating an independent
project.

Portable YAML does not imply identical multiplexer behavior. tmux-specific
options remain tmux-specific, Herdr has stronger ownership checks, zellij
identifies a project only by its session name, and layout translation can
preserve topology without preserving exact cell geometry.

## 2. Install and verify

Requirements:

- a Unix-like operating system
- Rust and Cargo 1.89 or newer
- tmux 2.6 or newer, Herdr 0.7.5/protocol 17, or both
- `$SHELL` and `$EDITOR`
- optional `fzf`

### Install from crates.io

```sh
rustc --version
cargo --version
cargo install bootmux --locked
bootmux version
```

Cargo normally installs the binary into
`${CARGO_HOME:-$HOME/.cargo}/bin`. Add that directory to `PATH` if the shell
cannot find `bootmux`.

### Install with mise

[mise's Cargo backend](https://mise.jdx.dev/dev-tools/backends/cargo.html)
builds the crate with Cargo and adds the selected version to mise's global
configuration:

```sh
mise use -g rust
mise use -g cargo:bootmux@0.1.1
bootmux version
```

Use `mise use cargo:bootmux@0.1.1` without `-g` when a project should pin
bootmux locally.

mise filters fuzzy requests such as `latest` through a 24-hour minimum release
age by default. A just-published crate may therefore produce “no versions
found” with `cargo:bootmux@latest`. Use the explicit version shown above
instead of disabling the safety delay. Once the release is old enough,
`mise use -g cargo:bootmux` selects the latest eligible version.

### Install with Homebrew

The project tap builds the tagged release from source and installs Bash, Zsh,
and Fish completions:

```sh
brew install griffinqiu/tap/bootmux
bootmux --version
```

Upgrade or uninstall it with:

```sh
brew upgrade griffinqiu/tap/bootmux
brew uninstall griffinqiu/tap/bootmux
```

### Install from a source checkout

From the repository root:

```sh
cargo install --path . --locked
bootmux version
```

### Upgrade or uninstall

```sh
cargo install bootmux --locked --force
cargo uninstall bootmux
```

`cargo uninstall` removes the Cargo-installed executable only. It does not
delete project YAML, bootmux settings, Herdr ownership state, or completion
files copied by hand.

### Check each backend

```sh
bootmux --backend tmux doctor
bootmux --backend herdr doctor
bootmux --backend zellij doctor
```

`doctor` checks the selected multiplexer, optional `fzf`, `$EDITOR`, and
`$SHELL`.

For Herdr, verify the binary directly too:

```sh
herdr --version
```

bootmux requires a compatible Herdr client and server. It will not silently
downgrade from another protocol.

For zellij, confirm the version directly too:

```sh
zellij --version
```

bootmux requires zellij 0.44 or newer, the first release whose CLI can build and
drive a session from outside it.

### Shell completion

Cargo and mise install the executable only. Homebrew installs all three
completion files. The repository/source archive provides the same
project/subcommand-oriented static files:

```text
completion/bootmux.bash
completion/bootmux.zsh
completion/bootmux.fish
```

Bash can source its file directly:

```sh
source /path/to/bootmux/completion/bootmux.bash
```

Fish can source its file or place it in the normal Fish completions directory:

```fish
source /path/to/bootmux/completion/bootmux.fish
```

Zsh needs explicit registration after `compinit`:

```zsh
autoload -Uz compinit && compinit
source /path/to/bootmux/completion/bootmux.zsh
compdef _bootmux bootmux
```

The files do not attempt exhaustive completion of every flag or free-form
template argument.

## 3. Know where files live

### Project files

Named projects are searched in this order:

1. `$TMUXINATOR_CONFIG`
2. `${XDG_CONFIG_HOME:-$HOME/.config}/tmuxinator`
3. `$HOME/.tmuxinator`

Both `.yml` and `.yaml` are accepted for direct lookup. Each directory is
checked for a direct relative path first, then recursively by basename.

Examples:

```text
~/.config/tmuxinator/api.yml              -> bootmux start api
~/.config/tmuxinator/team/api.yml         -> bootmux start team/api
~/.tmuxinator/legacy.yaml                 -> bootmux start legacy
```

When recursive basenames collide, the sorted first match wins. Prefer a
relative name such as `team/api` to remove ambiguity.

Setting `$TMUXINATOR_CONFIG` makes it the highest-priority project directory.
bootmux creates that directory when it is set but missing.

For tmuxinator compatibility, project enumeration (`list`, picker, completion,
and tmux `stop-all`) includes `.yml` files only. Directly starting a known
`.yaml` project still works.

### Repository-local project

bootmux recognizes:

```text
./.tmuxinator.yml
./.tmuxinator.yaml
```

The `.yml` form wins if both exist.

### Global bootmux settings

```text
${XDG_CONFIG_HOME:-$HOME/.config}/bootmux/config.toml
```

Currently supported:

```toml
default_backend = "herdr"
```

Use the CLI rather than editing it by hand:

```sh
bootmux config set default-backend herdr
bootmux config get default-backend
bootmux config path
```

### Herdr ownership state

```text
${XDG_STATE_HOME:-$HOME/.local/state}/bootmux/herdr-workspaces.json
```

This file is not a user project config. It records exact managed workspace
identity and rendered stop-hook snapshots. Do not edit it casually.

## 4. Create your first project

```sh
bootmux new myapp
```

This creates `myapp.yml` in the selected project directory and opens it in
`$EDITOR`.

Start with a project that is valid on every backend:

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
  - logs: tail -f logs/development.log
```

The root should exist before start. `attach: false` is useful for the first
run because it avoids switching the current client while you inspect the
result.

## 5. Preflight before start

Always debug a new or migrated project on each intended backend:

```sh
bootmux --backend tmux debug myapp
bootmux --backend herdr debug myapp
```

tmux debug prints the generated shell script. It can start/query a tmux server
to read:

- `base-index`;
- `pane-base-index`;
- whether the target session already exists.

It does not execute the project creation script.

Herdr debug is offline. It prints:

- the selected endpoint;
- project and config identity;
- attach/append policy;
- static ownership actions;
- hook presence;
- layout splits, pane counts, and command counts;
- startup selection.

It does not print command bodies or inspect live ownership/server state.

Treat warnings and errors differently:

- a Herdr warning means a truthy tmux-only cosmetic/CLI field will be ignored;
- an error means the config cannot preserve its declared behavior, such as
  enabled `synchronize` or an invalid serialized layout.

## 6. Start, inspect, and stop

### Explicit first run

tmux:

```sh
bootmux --backend tmux start myapp
bootmux --backend tmux list --active
bootmux --backend tmux stop myapp
```

Herdr:

```sh
bootmux --backend herdr start myapp
bootmux --backend herdr list --active
bootmux --backend herdr stop myapp
```

### Project shorthand

If a project name is discoverable, this:

```sh
bootmux myapp
```

is shorthand for:

```sh
bootmux start myapp
```

Command names and aliases win over shorthand. For a project named `l`, `st`,
or another alias, use `bootmux start NAME` explicitly.

### Local project

From a repository containing `.tmuxinator.yml`:

```sh
bootmux local
bootmux .
```

A bare `bootmux` starts the local project when present; otherwise it launches
the `fzf` picker.

`local` has no free-form template arguments. For a templated local file, use:

```sh
bootmux start -p ./.tmuxinator.yml root=/work/myapp
```

### Repeated start

Repeated start reuses a matching session/workspace:

```text
on_project_start
on_project_restart
attachment/focus behavior
on_project_exit
```

It does not recreate windows/tabs or rerun pane commands. This makes repeated
invocation idempotent with respect to topology, but it is not a process
supervisor.

If a process should be restored when the container still exists, use an
idempotent `on_project_restart` hook or stop and recreate the project.

### Run alternate instances

`start -n NAME` is convenient for a temporary launch, but `stop` has no
corresponding `--name` option. For instances that must be started and stopped
declaratively, template the project name:

```yaml
name: {{ settings.instance | default('myapp') }}
root: ~/code/myapp
windows:
  - shell:
```

```sh
bootmux start myapp instance=myapp-review
bootmux stop myapp instance=myapp-review
```

Without that reproducible identity, a tmux `-n` session may require direct
tmux cleanup. A Herdr alternate workspace is still recorded and can be handled
by ownership-based `stop-all`, but ordinary stop cannot reconstruct its
identity from the unchanged config.

## 7. Choose a backend

Resolution order:

1. `--backend tmux|herdr|zellij`
2. active multiplexer environment
3. global `default_backend`
4. tmux

Examples:

```sh
bootmux --backend tmux start myapp
bootmux --backend herdr start myapp
bootmux --backend zellij start myapp
```

A zellij environment is recognized from `ZELLIJ`, `ZELLIJ_SESSION_NAME`, or
`ZELLIJ_PANE_ID`. zellij sets `ZELLIJ` to the string `0`, so bootmux tests
whether the variable is present rather than whether it is truthy.

An active Herdr popup uses `HERDR_ACTIVE_*` and wins over an inherited `TMUX`
value. For a genuine tmux-inside-Herdr process, bootmux asks Herdr which
foreground process owns the pane. If it cannot classify a nested situation, it
names every candidate it saw and fails rather than guessing; there is no
classifier for a tmux/zellij nesting, so that combination always needs an
explicit `--backend`.

Only setting `HERDR_SESSION` outside Herdr selects an endpoint; it does not
select the backend. Add `--backend herdr` or configure the default.

## 8. Control attachment and focus

YAML defaults to attachment:

```yaml
attach: true
```

Override per invocation:

```sh
bootmux start myapp --attach
bootmux start myapp --no-attach
```

Precedence:

1. `--attach`
2. `--no-attach`
3. YAML `attach`
4. default true

For mux compatibility, use lowercase YAML `false`. Alternate scalar spellings
can be surprising: only exact `false` and `0`, quoted or unquoted, disable
attachment.

Final selection:

```yaml
startup_window: editor
startup_pane: shell

windows:
  - editor:
      focused_pane: shell
      panes:
        - editor: nvim
        - shell:
```

- `focused_pane` stores a preferred pane inside a window/tab.
- `startup_window` chooses the final window/tab.
- `startup_pane` overrides that startup window's preferred pane.

Prefer names for `startup_window` in a cross-backend config. Numeric window
selection interacts with tmux `base-index`, while Herdr uses a zero-based
logical index.

Pane indices are zero-based logical values. tmux adjusts them using
`pane-base-index`.

## 9. Write windows and panes

### One command in one pane

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

### Several panes

```yaml
windows:
  - editor:
      panes:
        - editor: nvim
        - shell:
        - tests:
            - cargo test
            - cargo watch -x test
```

An empty value creates a shell pane.

### Per-window root and setup

```yaml
root: ~/code/myapp
pre_window: export APP_ENV=development

windows:
  - api:
      root: services/api
      pre: source .env
      panes:
        - server: cargo run
        - tests: cargo test
```

`services/api` resolves beneath the project root. Commands in each pane run in
this order:

1. top-level `pre_window`
2. window `pre`
3. pane commands

`--no-pre-window` skips only the first step.

## 10. Use layouts

Portable presets:

```yaml
layout: tiled
layout: even-horizontal
layout: even-vertical
layout: main-horizontal
layout: main-vertical
```

tmux applies the preset natively. Herdr builds a deterministic binary split
plan.

Serialized tmux layout strings are also supported. Herdr checks checksum,
tree geometry, pane count, and representable ratios. A valid tmux string can
still be rejected if it cannot be translated safely.

### Explicit split chain

For a layout that reads directly in YAML:

```yaml
windows:
  - app:
      panes:
        - editor:
            command: nvim
        - server:
            split: right
            ratio: 0.65
            command: npm run dev
        - logs:
            split: down
            ratio: 0.5
            command: tail -f logs/development.log
```

Each later pane splits the immediately preceding pane.

Rules:

- `split`: `right` or `down`
- `ratio`: existing pane's share, 0.1 through 0.9
- `command` and `commands`: choose one
- no `split`/`ratio` on the first pane
- no window `layout` together with a pane chain

This syntax works on every backend. tmux and zellij round the percentage to an
integer;
Herdr receives a floating-point ratio, so proportions can differ slightly.

## 11. Add lifecycle hooks

```yaml
on_project_start: docker compose up -d
on_project_first_start: bin/setup
on_project_restart: bin/recover-dev-processes
on_project_exit: echo "start operation exited"
on_project_stop: docker compose down
```

A hook can also be a list:

```yaml
on_project_stop:
  - docker compose down
  - rm -f tmp/dev.pid
```

Lifecycle:

```text
every start:
  on_project_start

missing container:
  on_project_first_start
  create topology and dispatch pane commands

existing container:
  on_project_restart

start operation exits:
  on_project_exit

stop:
  on_project_stop
  close container
```

`on_project_first_start` runs whenever missing topology is created. It is not a
permanent installation marker. Every backend also runs it before creating
appended windows/tabs.

Hook failure differs by backend:

- tmux start is a `/bin/sh -e` script, but commands dispatched into panes are
  asynchronous;
- tmux stop is intentionally not fail-fast, so it still attempts session
  closure after a failed `cd` or hook;
- Herdr runs hooks from the project root, checks their status, and prevents a
  stop when the stop hook fails.

Make hooks idempotent. Do not depend on tmux stop hooks running in the intended
directory after that directory has been deleted.

Hooks and pane commands are shell commands. Safe template rendering does not
make an untrusted project file safe to execute; review YAML before running it.

## 12. Template projects safely

### MiniJinja variables

```yaml
name: {{ settings.session | default('myapp') }}
root: "{{ env.HOME }}/code/{{ args[0] }}"
windows:
  - server: npm run {{ settings.task | default('dev') }}
```

```sh
bootmux start myapp frontend session=myapp-dev task=serve
```

Available values:

- `settings`: CLI tokens with a non-empty `key=`
- `args`: all other positional tokens
- `env`: environment variables

Parsing rules:

- `a=b=c` becomes key `a`, value `b=c`;
- the first duplicate setting wins;
- `=value` remains a positional argument;
- undefined values render as empty text.

The rendered result must still be valid YAML. Quote template expressions whose
values can contain `:`, `#`, braces, or other YAML-significant characters.

### Restricted mux placeholder

This willfish/mux form is supported without evaluating Ruby:

```erb
root: <%= @settings["root"] %>
```

Only that exact double-quoted `@settings["key"]` expression is accepted. Use
MiniJinja for args, environment variables, conditions, and loops.

### Stop with the same values

If a template controls project name, root, socket, or stop hook, reproduce all
of its inputs for stop: `key=value` settings, positional args, and referenced
environment variables.

```sh
bootmux start myapp root=/work/myapp env=staging
bootmux stop myapp root=/work/myapp env=staging
```

Herdr compares the rendered identity and stop-hook snapshot. Mismatches are
rejected instead of risking closure of the wrong workspace or execution of an
unexpected hook.

## 13. Use sockets and named sessions

### tmux

```yaml
socket_name: myapp
# or:
socket_path: /tmp/myapp-tmux.sock
```

This becomes tmux `-L` or `-S`. `socket_path` wins.
Use an absolute `socket_path` in a project shared across backends. zellij has no
endpoint selector of its own and warns that both fields are ignored.

Extra tmux CLI settings:

```yaml
tmux_options: -f ~/.tmux.custom.conf
tmux_command: tmux
```

The legacy alias for `tmux_options` is `cli_args`. A custom command such as
`wemux` is passed through on the tmux backend.

### Herdr

The same YAML keys select a Herdr endpoint:

```yaml
socket_name: myapp
# or:
socket_path: /tmp/myapp-herdr.sock
```

Endpoint precedence:

1. YAML `socket_path`
2. YAML `socket_name`
3. `HERDR_SOCKET_PATH`
4. `HERDR_SESSION`
5. default Herdr endpoint

Named sessions permit 1–64 ASCII letters, numbers, `.`, `_`, and `-`.

`tmux_options`, `tmux_command`, and pane-border settings are warned and ignored
by Herdr. Keep separate backend-specific project files if those values are
essential rather than cosmetic.

## 14. Append another project

### tmux append

From inside the target tmux session:

```sh
bootmux --backend tmux start tools --append
```

The project's windows are added after the current session's last window. No
new project session is created. No append record is persisted;
`bootmux stop tools` does not remove the appended windows. Manage them as part
of the receiving session.

### Herdr append

From inside the target Herdr workspace or popup:

```sh
bootmux --backend herdr start tools --append
```

The selected endpoint must be the one containing the active workspace. The
project's tabs are added to that workspace.

Herdr append does not create a separate managed workspace record. A later
`bootmux stop tools` cannot remove only those appended tabs. Manage them as part
of the receiving workspace.

On a partial failure, bootmux attempts to roll back newly appended tabs.

## 15. Use the picker

Install `fzf`, then:

```sh
bootmux picker
```

Cancellation is normal and does not produce a project.

Print a safe multiplexer configuration snippet:

```sh
bootmux bindings tmux
bootmux bindings herdr
```

Defaults:

- tmux prefix + `F`, opening a normal window for tmux 2.6 compatibility;
- Herdr `prefix+shift+f`, opening an 80% popup.

Custom keys:

```sh
bootmux bindings tmux --key C-f
bootmux bindings herdr --key prefix+alt+f
```

Paste the generated output into the corresponding multiplexer config and
reload that config.

## 16. Import an existing tmux session

```sh
bootmux --backend tmux new imported existing-session
```

This introspects the existing tmux session and writes a starter project with:

- window names;
- serialized layouts;
- pane working directories;
- a derived project root.

It does not capture the full live process state or command history. Review the
generated YAML and replace `cd` placeholders with declarative commands.

This form is tmux-only. It does not import a Herdr workspace.

## 17. Manage project files

```sh
bootmux open myapp
bootmux edit myapp
bootmux copy myapp myapp-alt
bootmux delete myapp-alt
bootmux list
bootmux list --newline
```

`open` creates the project if it does not exist. `edit` only opens an existing
project.

These editor commands target `NAME.yml` in bootmux's current write directory
instead of recursively locating lower-priority copies. The directory is
`$TMUXINATOR_CONFIG`, otherwise an existing XDG directory, otherwise an
existing `~/.tmuxinator`, otherwise a newly created XDG directory. As a result,
`open NAME` can create a higher-priority duplicate and `edit NAME` can report
missing while a lower-priority copy exists. Set `TMUXINATOR_CONFIG`
deliberately or edit the intended file directly.

Local forms:

```sh
bootmux new myapp --local
bootmux open myapp --local
bootmux edit --local
```

Destructive commands:

```sh
bootmux delete project-a project-b
bootmux implode
```

`delete` asks before each file. `implode` asks once, then recursively deletes
the selected project configuration directory/directories, not merely known
YAML files. Check `TMUXINATOR_CONFIG` and your XDG/home tmuxinator directories
before confirming.

## 18. Stop safely

### One tmux project

```sh
bootmux --backend tmux stop myapp
```

tmux stop renders the session/socket from the current config. There is no
ownership proof. Ensure the rendered name points to the intended session.

The stop script is deliberately non-fail-fast so a vanished root does not
prevent `kill-session`. Use stop hooks that remain safe if `cd` fails.

### One Herdr project

```sh
bootmux --backend herdr stop myapp
```

Herdr requires a persisted match across endpoint, config path, name, label,
root, workspace identity, and rendered stop hook. The operation fails closed on
ambiguity or drift.

If the recorded server is down, bootmux can start it to verify and close the
workspace. It does not stop the server afterward.

### Stop all

Preview the confirmation list:

```sh
bootmux --backend tmux stop-all
bootmux --backend herdr stop-all
```

Skip confirmation only in a controlled context:

```sh
bootmux --backend herdr stop-all --noconfirm
```

tmux `stop-all` is heuristic: it matches discoverable `.yml` project basenames
to names from the ambient tmux server (the current server when inside tmux,
normally the default server outside). It then renders each candidate config's
own tmux/socket command, which can target a different server than the one used
for discovery. It can miss custom names, template names, `.yaml`, external
`-p` configs, and custom socket-only sessions. The confirmation list shows
names, not endpoint identity, and cannot prove that a same-named session was
created by bootmux.

Herdr `stop-all` iterates bootmux ownership records, verifies each workspace,
runs the persisted stop-hook snapshot (or the legacy config fallback when no
snapshot exists), and closes only the workspace. It never stops the Herdr
server. The operation stops at the first verification, hook, or close error;
later records stay untouched until the error is resolved and `stop-all` is run
again.

New ownership records contain the rendered stop hook, allowing `stop-all` to
work after a config is removed. The snapshot is plain text in a private
mode-0600 state file; keep secrets out of hook source.

tmux `stop-all` continues after a child stop script returns non-zero and does
not aggregate those statuses. A successful top-level return is therefore not
proof that every candidate session closed; verify the remaining session list.

## 19. Plan for reboot and process recovery

tmux and [Herdr session restore](https://herdr.dev/docs/session-state/) can
preserve or restore terminal topology in different ways, but an ordinary
process inside a pane is not equivalent to a supervised service.

bootmux only runs pane commands while creating/appending topology. If a
matching session/workspace survives or is restored, repeated start chooses the
restart path and does not replay pane commands.

Recommended patterns:

- use `on_project_restart` to probe and restart missing development processes;
- make the restart hook idempotent;
- use a real service manager for long-lived critical services;
- stop and recreate the project when a clean full replay is desired;
- run `debug` after changing layouts before recreating.

Example:

```yaml
on_project_restart: |
  pgrep -f 'npm run dev' >/dev/null || npm run dev >tmp/dev.log 2>&1 &
```

Choose a probe specific enough that it cannot match another project.

## 20. Migrate tmuxinator or mux projects

Recommended process:

1. leave the YAML in the existing tmuxinator directory;
2. run tmux debug;
3. run Herdr debug;
4. migrate unsupported ERB to MiniJinja;
5. handle Herdr warnings and enabled `synchronize`;
6. start with `--no-attach`;
7. inspect real panes and working directories;
8. stop with identical template inputs;
9. set a default backend only after both paths are understood.

Compatible aliases:

```text
project_name -> name
project_root -> root
tabs         -> windows
cli_args     -> tmux_options
```

For conflicting aliases, the last type-valid field in YAML document order
wins.

Only the non-executable mux form below is retained from ERB:

```erb
<%= @settings["key"] %>
```

See the exact [19-file compatibility matrix](mux-compatibility.md#fixture-matrix)
and its expected safety rejections.

## 21. Troubleshooting

### “both tmux and Herdr environments are active”

The nesting could not be classified safely:

```sh
bootmux --backend tmux start myapp
# or
bootmux --backend herdr start myapp
```

### “Project … doesn't exist”

Check:

```sh
bootmux list --newline
printf '%s\n' "$TMUXINATOR_CONFIG"
bootmux config path
```

Remember that `list` enumerates `.yml` only. Use `-p` for an explicit file:

```sh
bootmux start -p /absolute/path/project.yaml
```

### “Your project file should include some windows”

`windows`/`tabs` must resolve to a non-empty sequence:

```yaml
windows:
  - shell:
```

### Start fails on the project root

Create the directory or correct the rendered template:

```sh
bootmux debug myapp root=/expected/path
```

tmux start uses fail-fast shell execution. Herdr needs the intended working
directory to build panes.

### Herdr rejects `synchronize`

Herdr has no equivalent synchronized-input behavior. Remove it, set a real
YAML boolean `false`, or maintain a tmux-only project variant:

```yaml
synchronize: false
```

The string `"false"` is truthy and is rejected.

### Herdr version/protocol mismatch

```sh
herdr --version
bootmux --backend herdr doctor
```

Upgrade the Herdr client/server pair so both use protocol 17 and meet the
minimum version.

### Herdr stop reports endpoint or identity mismatch

Use the same template and socket settings as start:

```sh
bootmux --backend herdr stop myapp root=/same/root socket=/same/socket
```

Do not delete state or guess a workspace label to work around the check.
Inspect the config and `debug` output first.

### Herdr stop reports a changed stop hook

The current rendered `on_project_stop` differs from the managed snapshot.
Restore the prior config/settings, or start the correctly identified project
once to refresh its managed record before stopping.

Do not execute a newly changed destructive hook merely to bypass the refusal.

### Herdr append reports the wrong endpoint

Run from the receiving Herdr workspace/popup and remove conflicting YAML/env
socket selectors, or select the exact same endpoint:

```sh
bootmux --backend herdr debug tools --append
```

### Serialized layout is rejected by Herdr

Check the checksum and pane count. If exact tmux geometry does not translate,
replace the serialized string with a portable preset or explicit pane chain.

### Existing workspace does not rerun commands

This is reuse behavior. Add recovery to `on_project_restart`, or stop and start
to create fresh topology.

### Picker says `fzf` is missing

Install `fzf`, or start projects directly. All non-picker workflows work
without it.

### ERB is rejected

Keep only:

```erb
<%= @settings["key"] %>
```

Migrate other forms:

```text
<%= @args[0] %>  -> {{ args[0] }}
<%= ENV["V"] %>  -> {{ env.V }}
```

### tmux `stop-all` missed a project

Stop it explicitly with its actual config, name, and socket. `stop-all` does
not own or enumerate every possible tmux session.

## 22. Environment quick reference

| Variable | Purpose |
|---|---|
| `HOME` | Default config, project, and state roots |
| `XDG_CONFIG_HOME` | bootmux settings and XDG tmuxinator projects |
| `XDG_STATE_HOME` | Herdr ownership state when absolute |
| `TMUXINATOR_CONFIG` | Highest-priority project directory |
| `EDITOR` | `new`, `open`, and `edit` |
| `SHELL` | Generated shell/hook environment |
| `TMUX` | Active tmux detection |
| `HERDR_ACTIVE_*` | Herdr popup/current context |
| `HERDR_CONFIG_PATH` | Override the Herdr config used by invoked Herdr commands |
| `HERDR_ENV` | Active Herdr detection |
| `HERDR_WORKSPACE_ID` | Current Herdr workspace |
| `HERDR_TAB_ID` | Current Herdr tab |
| `HERDR_PANE_ID` | Current Herdr pane/foreground classification |
| `HERDR_SOCKET_PATH` | Herdr endpoint and active-environment signal |
| `HERDR_CLIENT_SOCKET_PATH` | Attached-client endpoint comparison |
| `HERDR_SESSION` | Ambient named Herdr endpoint |

## 23. Command quick sheet

```sh
# Validate
bootmux --backend tmux doctor
bootmux --backend herdr doctor
bootmux --backend tmux debug PROJECT
bootmux --backend herdr debug PROJECT

# Lifecycle
bootmux start PROJECT [key=value] [args...]
bootmux stop PROJECT [key=value] [args...]
bootmux stop-all
bootmux local

# Overrides
bootmux start PROJECT --attach
bootmux start PROJECT --no-attach
bootmux start PROJECT --name ALT
bootmux start -p PATH
bootmux start PROJECT --append
bootmux start PROJECT --no-pre-window

# Projects
bootmux new NAME
bootmux --backend tmux new NAME SESSION
bootmux open NAME
bootmux edit NAME
bootmux copy OLD NEW
bootmux delete NAME
bootmux implode
bootmux list --newline

# Picker/settings
bootmux picker
bootmux bindings tmux
bootmux bindings herdr
bootmux config set default-backend herdr
bootmux config get default-backend
bootmux config path

# Diagnostics/shell integration
bootmux version
bootmux commands [SHELL]
bootmux completions ARG
```

For every option and alias, see the [CLI reference](cli.md). For every YAML
field and pane shape, see [Project configuration](configuration.md).
