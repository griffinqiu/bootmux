# Backends and lifecycle

bootmux keeps one project format while preserving the behavior and safety
boundaries of two different multiplexers. Portable fields share an intent, not
a promise of pixel-identical or process-identical runtime behavior.

## Backend selection

Resolution order:

1. explicit `--backend tmux|herdr`
2. active multiplexer environment
3. `default_backend` in bootmux's global settings
4. tmux

The active-environment rules are:

- `TMUX` identifies tmux.
- `HERDR_ACTIVE_*` identifies a Herdr popup and wins over inherited `TMUX`.
- `HERDR_ENV`, `HERDR_WORKSPACE_ID`, `HERDR_TAB_ID`, `HERDR_PANE_ID`, or
  `HERDR_SOCKET_PATH` identify a Herdr environment.
- When both multiplexers appear active outside a popup, bootmux asks Herdr for
  foreground process information. A foreground `tmux` selects tmux; another
  process selects Herdr.
- If that nested environment cannot be classified safely, bootmux fails and
  asks for `--backend`.

`HERDR_SESSION` selects an ambient named endpoint but does not, by itself,
prove that the process is inside Herdr. Use `--backend herdr` or a global
default when invoking from outside.

## Comparison

| Behavior | tmux | Herdr |
|---|---|---|
| Normal project container | Session | Workspace |
| Window mapping | Window | Tab |
| Pane command transport | `send-keys` through a generated shell script | `herdr pane run` |
| Repeated start | Reuse session; run restart hook | Reuse/adopt workspace; run restart hook |
| Pane commands on reuse | Not rerun | Not rerun |
| `--append` | Add windows to current session | Add tabs to current same-endpoint workspace |
| Ordinary stop | Render name/socket, then `kill-session` | Require persisted identity, then close workspace |
| `stop-all` | Heuristic config/session-name match | Persisted ownership records |
| Server lifecycle on stop | tmux command semantics | Never stop server; close workspaces only |
| Hook failure behavior | Shell-script dependent | Checked and propagated |
| Synchronized interactive input | Supported | Truthy values rejected |

## Shared lifecycle

For a normal start:

```text
preflight/render
on_project_start
  project missing -> on_project_first_start -> create topology and run pane commands
  project exists  -> on_project_restart
attach/focus policy
on_project_exit
```

`on_project_first_start` means “this start is creating topology,” not “this
config has never run before.” It runs again after a project was stopped and
recreated. Both backends run it before creating appended windows/tabs; append
follows `on_project_start` → `on_project_first_start` → create append topology
→ `on_project_exit`.

Repeated start does not reconcile or rebuild existing topology. If panes still
exist but their ordinary child processes have exited, bootmux does not restart
those processes automatically. Put recovery logic in `on_project_restart`, or
stop and recreate the project.

## tmux backend

### Generated script

bootmux renders a tmux shell script containing `start-server`,
`new-session`/`new-window`, `splitw`, `send-keys`, layout, focus, and attachment
commands.

```sh
bootmux --backend tmux debug myapp
```

The renderer reads `base-index` and `pane-base-index` from tmux. Consequently,
debug may start a tmux server even though it does not create the project
session.

Start/local executes the script with `/bin/sh -e`. A failing setup command
causes a non-zero bootmux result, but creation is not transactional: a failure
after tmux has created resources can leave a partial session. Commands sent
into panes are asynchronous; bootmux validates that `send-keys` succeeded, not
the eventual exit status of the program inside the pane.

### Reuse and append

If the rendered session already exists, start runs `on_project_start`,
`on_project_restart`, and the attachment policy without recreating windows or
pane commands.

`--append` targets the current tmux session and requires that session to exist:

```sh
bootmux --backend tmux start tools --append
```

The configured project name is not used as a new container in append mode.

### Stop behavior

Ordinary stop renders the configured session/socket and runs the stop script
with `/bin/sh -c`, deliberately without `-e`. This allows `kill-session` to be
attempted even if the project root was removed or the stop hook failed.

That compatibility behavior has two consequences:

- if `cd root` fails, the hook may run from the caller's current directory;
- a later successful kill can mask an earlier hook failure in the final shell
  status.

Use idempotent stop hooks that do not assume the directory still exists.

### tmux `stop-all` is heuristic

tmux has no bootmux ownership database. `stop-all`:

1. lists names on the ambient tmux server: current inside tmux, normally
   default outside;
2. enumerates discoverable `.yml` project names;
3. stops exact name matches using each config's rendered tmux/socket command,
   which can select a different endpoint than the discovery server.

It is a tmuxinator-compatible convenience, not proof that bootmux created the
session. It does not discover:

- `.yaml`-only projects;
- projects started only with an external `-p` path;
- sessions renamed with `-n`;
- template-produced names that differ from the config basename;
- sessions visible only on a non-default tmux server.

Review the confirmation list before proceeding. It contains project names, not
verified endpoint identity.

The loop continues when an individual generated stop script exits non-zero and
does not aggregate those statuses. Verify the remaining tmux sessions after
`stop-all`; its top-level success is not proof that every candidate closed.

## Herdr backend

The Herdr backend requires a Unix-like platform, Herdr 0.7.5 or newer, and
socket protocol exactly 17.

Most operations use Herdr's
[structured CLI](https://herdr.dev/docs/cli-reference/) and thread the returned
workspace/tab/pane IDs through the plan. Herdr 0.7.5 does not expose arbitrary
exact-pane focus in its CLI, so that one operation uses the protocol-17 local
Unix socket directly.

### Endpoint selection

Precedence:

1. YAML `socket_path`
2. YAML `socket_name`
3. `HERDR_SOCKET_PATH`
4. `HERDR_SESSION`
5. Herdr's default endpoint

`socket_path` wins over `socket_name`. A Herdr session name must contain 1–64
ASCII letters, digits, `.`, `_`, or `-`.

When needed, bootmux starts the selected Herdr server and waits up to 15
seconds for a compatible client/server pair. Version and protocol mismatches
fail rather than falling back.

### Normal start and adoption

A normal start maps one project to one workspace. Windows become tabs, panes
become real Herdr PTYs, and commands run in those PTYs.

Before contacting a server or running hooks, bootmux fully parses and preflights
the static layout plan. It then:

1. checks persisted ownership by exact endpoint and workspace ID;
2. if the ID is stale, accepts only one exact label + launch-root match;
3. if there is no ownership record, may adopt one existing workspace only when
   exactly one workspace has the same label and root;
4. otherwise creates a new workspace.

Ambiguous matches and identity changes fail closed.

```sh
bootmux --backend herdr debug myapp
```

Herdr debug is offline. It prints a static preflight/layout plan, hook presence,
and command counts. It does not print command bodies or inspect live ownership,
adoption, or server state.

### Attach and focus

With attachment enabled, bootmux selects the configured startup tab/pane and
focuses or attaches the Herdr client. With attachment disabled, it configures
each tab's selected pane and restores the prior global focus when possible.

The real-backend smoke suite does not exercise client attachment or detached
focus restoration. Lower-level focus transport is unit-tested, but lifecycle
orchestration remains outside real-smoke coverage.

### Herdr append

`--append` does not create an independently managed project. It adds the
project's tabs to the active Herdr workspace:

```sh
bootmux --backend herdr start tools --append
```

Requirements:

- the command runs inside a Herdr workspace or popup;
- the selected endpoint is the endpoint containing that workspace.

On failure, bootmux attempts to close tabs created by the partial append.
Successful appended tabs are part of the active workspace; `bootmux stop tools`
cannot later identify and remove them as a separate project.

## Layouts

Both backends accept:

- `tiled`
- `even-horizontal`
- `even-vertical`
- `main-horizontal`
- `main-vertical`
- serialized tmux layout strings
- explicit bootmux pane chains

tmux applies its native layout. Herdr translates the requested topology to a
binary split (BSP) plan.

For serialized tmux layouts, Herdr strictly checks:

- checksum and geometry syntax;
- unique pane IDs and configured pane count;
- split tree and ratios representable from 0.1 through 0.9.

Translation preserves topology and approximate ratios, not tmux's exact
cell/pixel geometry. Pane-chain percentages are rounded to an integer for tmux
while Herdr receives a floating-point ratio, so visual proportions may differ
slightly.

## Herdr ownership state

State is stored at:

```text
${XDG_STATE_HOME:-$HOME/.local/state}/bootmux/herdr-workspaces.json
```

`XDG_STATE_HOME` is used only when it is an absolute path.

Each managed record includes:

- canonical endpoint and launch selector;
- workspace ID;
- workspace label and project name;
- launch root;
- canonical config path;
- root pane ID when available;
- the fully rendered `on_project_stop` snapshot.

On Unix, bootmux creates the state file and locks with mode `0600`, creates a
new state directory with mode `0700`, uses advisory lifecycle locking, and
replaces the JSON atomically.

The rendered stop hook is stored as plain text in this private JSON file. Do
not place secrets directly in hook text; reference a credential at runtime
instead.

## Herdr stop safety

Ordinary stop requires the selected config to match the managed record's
endpoint, config path, project name, label, root, workspace identity, and stop
hook snapshot. A template or socket change that alters endpoint, lifecycle
identity, or the rendered stop hook is not accepted.

If a workspace ID disappeared, recovery succeeds only for one exact
label-and-root match. If the ID now names a different workspace, or several
workspaces match, bootmux refuses the operation.

The stop hook runs from the project root and its exit status is checked. A
failing hook prevents workspace closure.

Newly managed workspaces snapshot the fully rendered stop hook, so `stop-all`
can work after the config is removed or template inputs are unavailable.
Legacy state without a snapshot may still need to read the config.

When a recorded Herdr server is not running, stop/stop-all may start it to
verify and close the managed workspace. Neither command stops the server
afterward; only the confirmed workspace is closed.

Herdr `stop-all` stops at the first endpoint-verification, hook, or close error.
Later ownership records remain untouched; resolve the error and run it again.

Topology-construction, state-persistence, and startup-selection failures
trigger best-effort rollback. Partial append topology is also rolled back when
possible. If a new workspace cannot be rolled back, bootmux attempts to retain
its ownership record so it can be stopped later.

## Backend-specific fields

Herdr warns and ignores truthy values for:

- `tmux_options` and its alias `cli_args`
- `tmux_command`
- `enable_pane_titles`
- `pane_title_position`
- `pane_title_format`

Titled pane mappings remain portable: Herdr uses the mapping key as a pane
label. Only the tmux pane-border controls are ignored.

Herdr accepts `synchronize: false` as disabled. Truthy values, including
`true`, `before`, `after`, and the string `"false"`, are rejected because
Herdr has no equivalent synchronized-input semantics.

[Documentation index](../README.md#documentation) ·
[Complete user manual](manual.md)
