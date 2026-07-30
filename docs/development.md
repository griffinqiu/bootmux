# Development and verification

This page describes the repository's implementation boundaries and what each
test layer actually proves.

## Module map

| Area | Files | Responsibility |
|---|---|---|
| CLI and dispatch | `src/cli.rs`, `src/commands/` | Clap syntax, shorthand routing, backend dispatch |
| Project discovery | `src/config.rs` | tmuxinator paths, recursive lookup, local config |
| Global settings | `src/settings.rs` | default backend, environment detection, atomic TOML update |
| Templates/YAML | `src/template.rs`, `src/yaml_ext.rs` | MiniJinja, restricted mux ERB, alias/scalar behavior |
| tmux model | `src/project.rs`, `src/window.rs`, `src/pane.rs` | tmuxinator-compatible parsed behavior |
| tmux renderer | `src/script.rs`, `src/tmux.rs` | generated shell and tmux introspection |
| Neutral Herdr spec | `src/spec.rs`, `src/layout.rs` | portable project model and BSP preflight |
| Herdr transport/state | `src/herdr.rs` | typed JSON CLI, protocol focus, ownership persistence |
| Herdr lifecycle | `src/herdr_backend.rs` | start/reuse/append/stop/rollback |
| Picker integration | `src/picker.rs`, `src/bindings.rs` | `fzf` selection and safe config snippets |

## Fast local checks

```sh
cargo fmt --all -- --check
cargo clippy --all-targets -- -D warnings
cargo test --all-targets
git diff --check
```

Completion syntax:

```sh
bash -n completion/bootmux.bash
zsh -n completion/bootmux.zsh
fish -n completion/bootmux.fish
```

The completion files currently provide project/subcommand-oriented completion,
not exhaustive completion for every option and template argument.

## Test layers

### Unit and behavior tests

`cargo test --all-targets` covers parsing, shell escaping, templates, aliases,
backend selection, layout translation, state matching, CLI integration, and
many failure branches with mocks/fakes.

Unit tests are the primary coverage for ownership matching, layout, state
persistence, endpoint selection, and transport encoding. Lifecycle rollback,
client attachment, and detached-focus restoration are not exercised by the
real smoke tests.

### Golden tmux snapshots

```sh
cargo test --test golden
```

Three representative renderings (`basic`, `pane_titles`, and `session_name`)
are compared byte-for-byte with tmuxinator-derived tmux 2.6 snapshots in
`tests/snapshots/2.6/`.

This is an exact renderer contract for those cases, not a claim that every YAML
project is byte-identical to tmuxinator.

### mux fixture matrix

```sh
cargo test --test mux_compat
```

The suite covers all 19 YAML files vendored from willfish/mux commit
`927030bb88e4b16b6671f68610980491ffbd2c81`.

- tmux: 18 accepted, `nowindows.yml` rejected as expected
- Herdr: 17 accepted, `nowindows.yml` and `synchronize.yml` rejected as expected
- accepted tmux scripts receive `/bin/sh -n`
- selected field semantics receive explicit assertions

This is a parse/render/preflight matrix, not 19 real lifecycle tests.

See [mux compatibility](mux-compatibility.md) for the complete table and
fixture attribution.

## Real tmux smoke test

```sh
tmux -V
cargo test --test smoke -- --ignored
```

The test uses the isolated tmux socket name `bootmux-smoke` and kills that
server before and after the test. Do not reuse that socket for personal work
while the test is running.

It verifies:

- repeated start reuses one session;
- a startup window containing a quote is selected correctly;
- a two-pane window is created;
- stop still closes the session after its root directory is deleted;
- a missing initial root fails start without leaving a session.

It does not cover every hook, append, custom command, socket, or stop-all path.

## Real Herdr smoke test

```sh
herdr --version
cargo test --test herdr_smoke -- --ignored
```

Set `HERDR_BIN=/absolute/path/to/herdr` to choose another Herdr binary.

The test uses a temporary socket, Herdr config, HOME, and XDG directories. It
starts and stops its own temporary Herdr server and verifies:

- concurrent and repeated start reuse one workspace;
- two tabs/four panes exist and real pane command output appears;
- append produces four tabs/eight panes;
- wrong socket and wrong rendered identity are rejected on stop;
- correct stop runs the rendered hook;
- after a new managed record is written, deleting the config still allows
  `stop-all` to use the snapshotted hook.

If Herdr is not installed, this test prints a skip message and returns success.
A green test result alone is therefore not evidence that the real Herdr path
ran; confirm `herdr --version` and inspect the test output.

The smoke does not cover client attachment, detached focus restoration, named
sessions, stale-ID adoption, custom serialized layouts, or rollback failures.

## Documentation checks

After changing Markdown:

1. verify every relative link resolves;
2. compare CLI examples with `cargo run -- COMMAND --help`;
3. keep config tables aligned with both `Project` and `HerdrProjectSpec`;
4. distinguish static debug/preflight coverage from real lifecycle coverage;
5. do not turn expected Herdr safety rejections into generic incompatibility
   claims.

## Updating compatibility fixtures

The vendored files are third-party test data. When updating them:

1. choose and record one immutable upstream commit;
2. preserve fixture bytes unless a local derivative is intentionally added;
3. update `tests/fixtures/mux/README.md` attribution;
4. update the exhaustive `CASES` list and expected backend errors;
5. update the compatibility matrix documentation;
6. run the full quality gates and both real smoke tests where available.

Do not rewrite an upstream fixture merely to make a backend accept it. Model an
expected safety rejection explicitly or add a separate bootmux-owned fixture.

[Documentation index](../README.md#documentation) ·
[Complete user manual](manual.md)
