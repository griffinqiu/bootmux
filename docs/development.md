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
| Backend-neutral spec | `src/spec.rs`, `src/layout.rs` | portable project model and BSP preflight |
| Child-process boundary | `src/process.rs` | injectable `CommandRunner` shared by the structured backends |
| Herdr transport/state | `src/herdr.rs` | typed JSON CLI, protocol focus, ownership persistence |
| Herdr lifecycle | `src/herdr_backend.rs` | start/reuse/append/stop/rollback |
| zellij transport | `src/zellij.rs` | typed CLI, session listing, pane-targeted input |
| zellij renderer | `src/zellij_layout.rs` | BSP tree to KDL layout documents |
| zellij lifecycle | `src/zellij_backend.rs` | start/reuse/append/stop, geometric pane mapping |
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

The same three projects also pin their zellij KDL layouts in
`tests/snapshots/zellij/`. The KDL document is the artifact bootmux hands to
zellij, so it is the zellij equivalent of the generated tmux script.

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
- zellij: 18 accepted, `nowindows.yml` rejected as expected
- accepted tmux scripts receive `/bin/sh -n`
- selected field semantics receive explicit assertions

This is a parse/render/preflight matrix, not 19 real lifecycle tests.

See [mux compatibility](mux-compatibility.md) for the complete table and
fixture attribution.

## Real backend runtime matrix

Each backend has one ignored smoke suite that must prove every row of the
required runtime matrix against the exact executable under test. A row only
counts once its assertions and cleanup succeeded, at which point the test prints
`BOOTMUX_MATRIX <backend> <row> PASS`. The suites fail instead of skipping when
their backend is missing, so a green result is real evidence.

The shared rows are executable identity, topology creation, roots and command
order, startup focus, lifecycle hooks, reuse, `list --active`, append, explicit
stop, `stop-all`, and a failed creation that leaves nothing behind.

Run each suite sequentially, with the exact executable first on `PATH`:

```sh
tmux -V
cargo test --test smoke -- --ignored --nocapture --test-threads=1

herdr --version
cargo test --test herdr_smoke -- --ignored --nocapture --test-threads=1

zellij --version
cargo test --test zellij_smoke -- --ignored --nocapture --test-threads=1
```

Set `BOOTMUX_MATRIX_EXPECT_TMUX`, `BOOTMUX_MATRIX_EXPECT_HERDR`, or
`BOOTMUX_MATRIX_EXPECT_ZELLIJ` to the expected `--version` output to assert that
the suite really ran against the intended release, for example
`BOOTMUX_MATRIX_EXPECT_TMUX="tmux 3.7b"`.

### tmux

The suite isolates itself with a temporary `TMUX_TMPDIR`, HOME, and project
directory, so it never touches a personal server. On top of the shared rows it
asserts the generated CLI script, the running server's `#{version}`, and that a
project selecting its own `tmux -L` socket stays invisible on the default one.

### Herdr

Set `HERDR_BIN=/absolute/path/to/herdr` to choose another Herdr binary. The
suite uses a temporary socket, Herdr config, HOME, and XDG directories, and
starts and stops its own server. On top of the shared rows it asserts the
client/server protocol pair against `SUPPORTED_PROTOCOLS`, the `status --json`
shape, direct-socket pane focus, concurrent starts converging on one workspace,
and that a stop which cannot prove the managed identity is refused without
touching the workspace or its ownership record.

It does not cover client attachment, named sessions, stale-ID adoption, or
rollback failures.

### zellij

Set `ZELLIJ_BIN=/absolute/path/to/zellij` to choose another zellij binary. The
suite uses a temporary HOME, `ZELLIJ_CONFIG_DIR`, and XDG directories, clears
the inherited `ZELLIJ*` variables, and shortens `serialization_interval` so a
stopped session becomes resurrectable within the test. On top of the shared rows
it asserts that zellij loaded the rendered KDL, the `action list-panes --json`
shape, that a failed append closes the tabs it created, and that a session
zellij still lists as `(EXITED` is not reported as active.

It does not cover client attachment or `switch-session`.

## Documentation checks

After changing Markdown:

1. verify every relative link resolves;
2. compare CLI examples with `cargo run -- COMMAND --help`;
3. keep config tables aligned with both `Project` and `ProjectSpec`;
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
6. run the full quality gates and all three real smoke tests where available.

Do not rewrite an upstream fixture merely to make a backend accept it. Model an
expected safety rejection explicitly or add a separate bootmux-owned fixture.

[Documentation index](../README.md#documentation) ·
[Complete user manual](manual.md) ·
[Release guide](releasing.md) ·
[中文发布指南](releasing.zh-CN.md)
