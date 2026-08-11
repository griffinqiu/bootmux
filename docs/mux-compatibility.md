# willfish/mux and tmuxinator compatibility

bootmux is designed for tmuxinator-style projects and uses
[willfish/mux](https://github.com/willfish/mux) as an additional compatibility
reference. Compatibility means the supported project is parsed and translated
with documented backend semantics; it does not mean tmux, Herdr, and zellij
produce identical runtime behavior.

The machine-readable latest tmux, Herdr, and zellij releases proven compatible
with a published bootmux version are stored in
[`mux-support.json`](../mux-support.json). That project-level file advances only
after the exact upstream release passes the required real-runtime matrix and
any necessary bootmux change has been released.

## Reference revision

The mux fixtures are vendored verbatim from commit
[`927030bb88e4b16b6671f68610980491ffbd2c81`](https://github.com/willfish/mux/tree/927030bb88e4b16b6671f68610980491ffbd2c81).

- 18 files come from upstream `tests/fixtures`.
- `demo.yml` comes from upstream `demo/demo.yml`.
- Source and license details are in
  [`tests/fixtures/mux/README.md`](../tests/fixtures/mux/README.md).

## What is compatible

Portable projects can use:

- `name`/`project_name`
- `root`/`project_root`
- `windows`/`tabs`
- `tmux_options`/`cli_args`
- `attach`
- sockets
- lifecycle hooks
- `pre_window`
- window roots and `pre`
- layouts and pane definitions
- pane titles and focus
- startup window/pane selection
- the restricted mux settings placeholder

Deprecated alias conflicts follow mux document-order behavior: the last
type-valid value wins. Obsolete no-op keys such as top-level `rbenv`, `rvm`,
`pre_tab`, `pre`, and `post` do not prevent loading.

## Fixture matrix

The matrix is a debug/render/preflight test. “Accept” does not mean the fixture
was executed end-to-end against a real multiplexer.

| Fixture | tmux debug/render | Herdr preflight | zellij preflight | Note |
|---|---|---|---|---|
| `detach.yml` | Accept | Accept | Accept | `attach: false` |
| `focused_pane.yml` | Accept | Accept | Accept | Pane focus |
| `hooks.yml` | Accept | Accept | Accept | Lifecycle hooks |
| `nameless_window.yml` | Accept | Accept | Accept | Nameless entry |
| `noroot.yml` | Accept | Accept | Accept | Current-directory root |
| `nowindows.yml` | Expected reject | Expected reject | Expected reject | Projects require windows |
| `pane_titles.yml` | Accept | Accept | Accept | Border settings ignored by Herdr and zellij; pane labels remain |
| `sample.yml` | Accept | Accept | Accept | Broad project shape |
| `sample_deprecations.yml` | Accept | Accept | Accept | Legacy aliases/no-op fields |
| `sample_emoji_as_name.yml` | Accept | Accept | Accept | Scalar name |
| `sample_literals_as_window_name.yml` | Accept | Accept | Accept | YAML literal names |
| `sample_number_as_name.yml` | Accept | Accept | Accept | Numeric project name |
| `sample_wemux.yml` | Accept | Accept | Accept | `wemux` used only by tmux |
| `socket.yml` | Accept | Accept | Accept | Named socket/session; ignored by zellij |
| `startup.yml` | Accept | Accept | Accept | Startup window/pane |
| `synchronize.yml` | Accept | Expected reject | Accept | Herdr has no synchronized input; zellij ignores it |
| `template.yml` | Accept | Accept | Accept | Restricted settings placeholder |
| `window_root.yml` | Accept | Accept | Accept | Per-window root |
| `demo.yml` | Accept | Accept | Accept | Upstream demo |

Totals:

- tmux: 18 accepted, 1 expected rejection
- Herdr: 17 accepted, 2 expected rejections
- zellij: 18 accepted, 1 expected rejection

The 18 accepted tmux renderings also receive `/bin/sh -n` syntax checks. A
smaller set of assertions verifies important alias, template, focus, and
socket semantics.

## Template migration

willfish/mux's non-executable placeholder works unchanged:

```erb
<%= @settings["root"] %>
```

```sh
bootmux start myapp root=/work/myapp
```

Other Ruby ERB is deliberately not evaluated:

| Existing tmuxinator form | bootmux form |
|---|---|
| `<%= @args[0] %>` | `{{ args[0] }}` |
| `<%= ENV["VAR"] %>` | `{{ env.VAR }}` |
| ERB condition/loop | MiniJinja condition/loop |

Unsupported `<% ... %>` receives a migration hint rather than executing Ruby.

## Important portability boundaries

### tmux-only behavior

- `tmux_options`/`cli_args`
- `tmux_command`, including `wemux`
- pane-border position and format
- synchronized pane input
- exact native tmux layout geometry

Herdr warns and ignores the first three categories and rejects enabled
`synchronize`. zellij warns and ignores all of them, plus `socket_name` and
`socket_path`.

### Names and indices

tmux replaces `.` and `:` in project names with `_`; Herdr and zellij retain the
original name as the workspace label or session name. zellij additionally caps
the name at 36 characters and rejects `/`.

For cross-backend projects, prefer a window name in `startup_window`. A number
is a zero-based logical index for Herdr and zellij, while tmux interprets it as
a tmux window target affected by `base-index`.

Pane numbers are written as zero-based logical indices. tmux adjusts them for
`pane-base-index`.

An invalid `focused_pane` falls back to the first pane under tmuxinator/tmux
semantics. Herdr and zellij reject an invalid title or out-of-range index while
parsing the project.

### Attachment

For mux lexical compatibility, only exact raw scalar spellings `false` and `0`
(quoted or unquoted) disable attachment. `False`, `FALSE`, `+0`, `00`, `0x0`,
and `0.0` remain truthy. Prefer:

```yaml
attach: false
```

### Repeated start

No backend reruns pane commands when reusing an existing project.
`on_project_restart` is the place for recovery logic.

### Append

tmux appends windows to the current session. Herdr appends tabs to the active
workspace on the same endpoint, and zellij appends tabs to the active session.
Neither Herdr nor zellij append creates an independently stoppable project.

### Project discovery

Direct lookup accepts `.yml` and `.yaml`. List/picker/completion enumeration and
tmux `stop-all` enumerate `.yml` only for tmuxinator parity.

### Known intentional runtime differences

- When `focused_pane` is omitted, bootmux follows tmuxinator and explicitly
  selects the first pane; the pinned mux runtime leaves the last-created pane
  focused.
- A nameless YAML window remains nameless under bootmux/tmuxinator semantics;
  bootmux does not reproduce mux's `~` window-name behavior.

## What the tests prove

The compatibility suite proves:

- every pinned fixture is represented in the matrix;
- expected configs parse and render/preflight on each backend;
- expected safety rejections remain explicit;
- accepted tmux scripts are syntactically valid `/bin/sh`;
- selected key semantics match the reference behavior.

It does not execute all 19 fixtures against real tmux, Herdr, or zellij.

Separate ignored smoke tests exercise representative real lifecycles:

- tmux: repeated start/reuse, a startup window containing a quote, two-pane
  creation, stopping after the root disappears, and fail-fast start on a
  missing root;
- Herdr: concurrent/repeated start reuse, two tabs/four panes with real command
  output, append, wrong-endpoint and wrong-identity stop rejection, a rendered
  stop hook, and config-independent `stop-all` for newly written state;
- zellij: start/reuse, two tabs/three panes with real command output and
  ordering, `list --active`, a rendered stop hook, and a rejected project
  leaving no session behind.

These smoke tests do not claim end-to-end coverage of every fixture, custom
layout, every socket selector mode or named Herdr sessions, attach paths, hook
failures, or rollback branches. See
[Development and verification](development.md) for exact commands.

## Recommended migration process

1. Keep the existing config in its tmuxinator directory.
2. Run `bootmux --backend tmux debug PROJECT`.
3. Run `bootmux --backend herdr debug PROJECT` and
   `bootmux --backend zellij debug PROJECT`.
4. Resolve backend warnings or explicit rejections.
5. Start detached first with `--no-attach`.
6. Inspect topology and commands on each backend.
7. Test stop with the same template inputs used at start.
8. Only then configure `default-backend`.

[Documentation index](../README.md#documentation) ·
[Complete user manual](manual.md)
