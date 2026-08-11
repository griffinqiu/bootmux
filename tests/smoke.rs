#![cfg(unix)]

//! Required real-runtime matrix for the tmux backend.
//!
//! Run explicitly against an exact stable tmux:
//!   cargo test --test smoke -- --ignored --nocapture --test-threads=1
//!
//! Every row prints `BOOTMUX_MATRIX tmux <row> PASS` only after its assertions
//! and cleanup succeeded, so the coverage is machine-verifiable.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::thread::sleep;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use tempfile::TempDir;

const WAIT_TIMEOUT: Duration = Duration::from_secs(30);

fn matrix(row: &str) {
    println!("BOOTMUX_MATRIX tmux {row} PASS");
}

/// Owns both isolated tmux servers and tears them down even on panic.
struct Harness {
    _temp: TempDir,
    bootmux: PathBuf,
    tmux: PathBuf,
    tmux_tmpdir: PathBuf,
    projects: PathBuf,
    home: PathBuf,
    root: PathBuf,
    session: String,
    socket_session: String,
    socket_name: String,
    second_session: String,
}

impl Harness {
    fn env(&self, command: &mut Command) {
        command
            .env("HOME", &self.home)
            .env("SHELL", "/bin/sh")
            .env("TMUX_TMPDIR", &self.tmux_tmpdir)
            .env("TMUXINATOR_CONFIG", &self.projects)
            .env_remove("TMUX");
    }

    fn bootmux(&self, args: &[&str]) -> Output {
        let mut command = Command::new(&self.bootmux);
        command.args(["--backend", "tmux"]).args(args);
        self.env(&mut command);
        command.output().unwrap()
    }

    /// `--append` is only legal from inside a live session, so the caller
    /// supplies the `TMUX` value tmux itself would export.
    fn bootmux_inside(&self, args: &[&str], tmux_value: &str) -> Output {
        let mut command = Command::new(&self.bootmux);
        command.args(["--backend", "tmux"]).args(args);
        self.env(&mut command);
        command.env("TMUX", tmux_value);
        command.output().unwrap()
    }

    fn tmux(&self, args: &[&str]) -> Output {
        let mut command = Command::new(&self.tmux);
        command.args(args);
        self.env(&mut command);
        command.output().unwrap()
    }

    fn tmux_on_socket(&self, args: &[&str]) -> Output {
        let mut full = vec!["-L", self.socket_name.as_str()];
        full.extend_from_slice(args);
        self.tmux(&full)
    }

    fn capture(&self, args: &[&str]) -> String {
        String::from_utf8_lossy(&self.tmux(args).stdout)
            .trim_end()
            .to_string()
    }

    fn sessions(&self) -> String {
        String::from_utf8_lossy(&self.tmux(&["list-sessions", "-F", "#S"]).stdout).into_owned()
    }

    fn has_session(&self, name: &str) -> bool {
        self.sessions().lines().any(|line| line == name)
    }

    fn window_names(&self, session: &str) -> Vec<String> {
        self.capture(&["list-windows", "-t", session, "-F", "#W"])
            .lines()
            .map(str::to_string)
            .collect()
    }

    fn pane_count(&self, session: &str) -> usize {
        self.capture(&["list-panes", "-s", "-t", session, "-F", "#P"])
            .lines()
            .count()
    }
}

impl Drop for Harness {
    fn drop(&mut self) {
        let _ = self.tmux(&["kill-server"]);
        let _ = self.tmux_on_socket(&["kill-server"]);
    }
}

fn assert_success(output: &Output, operation: &str) {
    assert!(
        output.status.success(),
        "{operation} failed\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn assert_failure(output: &Output, operation: &str) {
    assert!(
        !output.status.success(),
        "{operation} unexpectedly succeeded\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn wait_until(mut condition: impl FnMut() -> bool) -> bool {
    let deadline = Instant::now() + WAIT_TIMEOUT;
    while Instant::now() < deadline {
        if condition() {
            return true;
        }
        sleep(Duration::from_millis(100));
    }
    false
}

/// Pane commands are typed into an asynchronous shell, so their markers are
/// polled instead of read once.
fn wait_for_lines(path: &Path, expected: &[&str]) -> Vec<String> {
    let mut seen = Vec::new();
    wait_until(|| {
        seen = std::fs::read_to_string(path)
            .unwrap_or_default()
            .lines()
            .map(str::to_string)
            .collect();
        seen == expected
    });
    seen
}

fn read_lines(path: &Path) -> Vec<String> {
    std::fs::read_to_string(path)
        .unwrap_or_default()
        .lines()
        .map(str::to_string)
        .collect()
}

fn write_project(path: &Path, body: &str) {
    std::fs::write(path, body).unwrap();
}

#[test]
#[ignore = "requires a real local tmux"]
fn tmux_runtime_matrix() {
    let tmux = PathBuf::from("tmux");
    let version_output = Command::new(&tmux)
        .arg("-V")
        .output()
        .expect("tmux must be installed and resolvable through PATH");
    assert!(version_output.status.success(), "tmux -V failed");
    let version = String::from_utf8_lossy(&version_output.stdout)
        .trim()
        .to_string();
    let resolved = String::from_utf8_lossy(
        &Command::new("/bin/sh")
            .args(["-c", "command -v tmux"])
            .output()
            .unwrap()
            .stdout,
    )
    .trim()
    .to_string();
    assert!(!resolved.is_empty(), "tmux must resolve through PATH");
    let canonical = std::fs::canonicalize(&resolved).unwrap();
    println!("tmux identity: {version} at {}", canonical.display());
    if let Some(expected) = std::env::var_os("BOOTMUX_MATRIX_EXPECT_TMUX") {
        assert_eq!(
            version,
            expected.to_string_lossy(),
            "tmux is not the frozen exact stable target"
        );
    }
    matrix("identity");

    let temp = TempDir::new().unwrap();
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos()
        % 100_000;
    let process = std::process::id();

    let harness = Harness {
        bootmux: PathBuf::from(env!("CARGO_BIN_EXE_bootmux")),
        tmux,
        tmux_tmpdir: temp.path().join("tmuxtmp"),
        projects: temp.path().join("projects"),
        home: temp.path().join("home"),
        root: temp.path().join("root"),
        session: format!("bmtx-{process}-{nonce}"),
        socket_session: format!("bmtx-sock-{process}-{nonce}"),
        socket_name: format!("bmtx-{process}-{nonce}"),
        second_session: format!("bmtx-two-{process}-{nonce}"),
        _temp: temp,
    };
    let app_root = harness.root.join("app");
    for directory in [
        &harness.tmux_tmpdir,
        &harness.projects,
        &harness.home,
        &harness.root,
        &app_root,
    ] {
        std::fs::create_dir_all(directory).unwrap();
    }

    let root = harness.root.display().to_string();
    let hooks_log = harness.root.join("hooks.log");
    let pre_log = harness.root.join("pre.log");
    let window_pre_log = harness.root.join("window-pre.log");
    let editor_log = harness.root.join("editor.log");
    let logs_log = harness.root.join("logs.log");

    let main_project = harness.projects.join(format!("{}.yml", harness.session));
    write_project(
        &main_project,
        &format!(
            "name: {name}\n\
             root: {root}\n\
             attach: false\n\
             startup_window: logs\n\
             startup_pane: watcher\n\
             pre_window: echo pre >> {root}/pre.log\n\
             on_project_start: echo start >> {root}/hooks.log\n\
             on_project_first_start: echo first_start >> {root}/hooks.log\n\
             on_project_restart: echo restart >> {root}/hooks.log\n\
             on_project_exit: echo exit >> {root}/hooks.log\n\
             on_project_stop: echo stop >> {root}/hooks.log\n\
             windows:\n  \
             - editor:\n      \
                 root: {root}/app\n      \
                 pre: echo window-pre >> {root}/window-pre.log\n      \
                 panes:\n        \
                   - main:\n            \
                       commands:\n              \
                         - echo cwd-$(basename \"$PWD\") >> {root}/editor.log\n              \
                         - echo editor-two >> {root}/editor.log\n        \
                   - side:\n            \
                       command: echo side >> {root}/editor.log\n  \
             - logs:\n      \
                 layout: even-vertical\n      \
                 panes:\n        \
                   - tail: echo tail >> {root}/logs.log\n        \
                   - watcher: echo watcher >> {root}/logs.log\n",
            name = harness.session,
            root = root,
        ),
    );

    // The generated script is the tmux CLI contract: it must still emit the
    // documented creation, typing, layout and focus commands.
    let debug = harness.bootmux(&["debug", &harness.session]);
    assert_success(&debug, "bootmux debug");
    let script = String::from_utf8_lossy(&debug.stdout).into_owned();
    for fragment in [
        "new-session",
        "splitw",
        "send-keys",
        "select-layout",
        "select-window",
        "select-pane",
        harness.session.as_str(),
    ] {
        assert!(
            script.contains(fragment),
            "generated tmux script is missing {fragment}:\n{script}"
        );
    }
    matrix("generated_cli");

    assert_success(
        &harness.bootmux(&["start", &harness.session]),
        "first start",
    );
    assert!(
        wait_until(|| harness.has_session(&harness.session)),
        "the project session should exist"
    );
    assert_eq!(
        harness.sessions().lines().count(),
        1,
        "start must create exactly one session"
    );
    assert_eq!(
        harness.window_names(&harness.session),
        vec!["editor", "logs"]
    );
    assert!(
        wait_until(|| harness.pane_count(&harness.session) == 4),
        "expected 4 panes, saw {}",
        harness.pane_count(&harness.session)
    );
    matrix("create_topology");

    let running_version = harness.capture(&[
        "display-message",
        "-p",
        "-t",
        &harness.session,
        "#{version}",
    ]);
    assert!(
        version.contains(running_version.trim()),
        "running server version {running_version} does not match {version}"
    );
    matrix("running_server_version");

    // pre_window runs in every pane, a window `pre` only in its own window's
    // panes, and a pane's own commands run in their configured order.
    assert_eq!(wait_for_lines(&pre_log, &["pre"; 4]).len(), 4);
    assert_eq!(wait_for_lines(&window_pre_log, &["window-pre"; 2]).len(), 2);
    assert_eq!(
        wait_for_lines(&editor_log, &["cwd-app", "editor-two", "side"]),
        vec!["cwd-app", "editor-two", "side"]
    );
    assert_eq!(
        wait_for_lines(&logs_log, &["tail", "watcher"]),
        vec!["tail", "watcher"]
    );
    matrix("root_and_commands");

    let focused = harness.capture(&[
        "display-message",
        "-p",
        "-t",
        &harness.session,
        "#W #{pane_index} #{pane_title}",
    ]);
    let mut focus_fields = focused.split_whitespace();
    assert_eq!(
        focus_fields.next(),
        Some("logs"),
        "startup_window: {focused}"
    );
    assert_eq!(focus_fields.next(), Some("1"), "startup_pane: {focused}");
    matrix("startup_focus");

    assert_success(
        &harness.bootmux(&["start", &harness.session]),
        "second start",
    );
    sleep(Duration::from_secs(1));
    assert_eq!(
        harness.window_names(&harness.session),
        vec!["editor", "logs"],
        "reuse must not rebuild topology"
    );
    assert_eq!(
        harness.pane_count(&harness.session),
        4,
        "reuse must not add panes"
    );
    assert_eq!(
        read_lines(&editor_log),
        vec!["cwd-app", "editor-two", "side"],
        "reuse must not re-run pane commands"
    );
    matrix("reuse");

    assert_eq!(
        read_lines(&hooks_log),
        vec!["start", "first_start", "exit", "start", "restart", "exit"],
        "documented hook order or counts changed"
    );
    matrix("lifecycle_hooks");

    let listed = harness.bootmux(&["list", "--active", "-n"]);
    assert_success(&listed, "list --active");
    let names = String::from_utf8_lossy(&listed.stdout).into_owned();
    assert_eq!(
        names
            .lines()
            .filter(|line| line.trim() == harness.session)
            .count(),
        1,
        "list --active must report the project exactly once: {names}"
    );
    matrix("active_listing");

    // A project selecting its own `-L` socket must land there and stay
    // invisible to the default socket bootmux discovers sessions on.
    let socket_project = harness
        .projects
        .join(format!("{}.yml", harness.socket_session));
    write_project(
        &socket_project,
        &format!(
            "name: {name}\n\
             root: {root}\n\
             tmux_options: -L {socket}\n\
             attach: false\n\
             windows:\n  \
             - shell: echo socket >> {root}/socket.log\n",
            name = harness.socket_session,
            socket = harness.socket_name,
            root = root,
        ),
    );
    assert_success(
        &harness.bootmux(&["start", &harness.socket_session]),
        "isolated socket start",
    );
    let socket_sessions = String::from_utf8_lossy(
        &harness
            .tmux_on_socket(&["list-sessions", "-F", "#S"])
            .stdout,
    )
    .into_owned();
    assert!(
        socket_sessions
            .lines()
            .any(|line| line == harness.socket_session),
        "the project should live on its own socket: {socket_sessions}"
    );
    assert!(
        !harness.has_session(&harness.socket_session),
        "a -L project must not appear on the default socket"
    );
    matrix("isolated_socket");

    // `--append` adds another project's topology to the session bootmux is
    // running inside, and refuses to run without one.
    let tools_session = format!("bmtx-tools-{process}-{nonce}");
    let tools_project = harness.projects.join(format!("{tools_session}.yml"));
    write_project(
        &tools_project,
        &format!(
            "name: {tools_session}\n\
             root: {root}\n\
             attach: false\n\
             windows:\n  \
             - tools:\n      \
                 panes:\n        \
                   - watch: echo watch >> {root}/tools.log\n        \
                   - build: echo build >> {root}/tools.log\n",
            root = root,
        ),
    );
    assert_failure(
        &harness.bootmux(&["start", &tools_session, "--append"]),
        "append outside a session",
    );
    let tmux_env = harness
        .capture(&[
            "display-message",
            "-p",
            "-t",
            &harness.session,
            "#{socket_path},#{pid},#{session_id}",
        ])
        .replace('$', "");
    assert_success(
        &harness.bootmux_inside(&["start", &tools_session, "--append"], &tmux_env),
        "append inside the session",
    );
    assert!(
        wait_until(|| harness.window_names(&harness.session).len() == 3),
        "append must add exactly one copy of the topology, saw {:?}",
        harness.window_names(&harness.session)
    );
    assert_eq!(
        harness.window_names(&harness.session),
        vec!["editor", "logs", "tools"]
    );
    assert!(
        wait_until(|| harness.pane_count(&harness.session) == 6),
        "expected 6 panes after append, saw {}",
        harness.pane_count(&harness.session)
    );
    assert_eq!(
        wait_for_lines(&harness.root.join("tools.log"), &["watch", "build"]),
        vec!["watch", "build"],
        "appended panes must run their commands"
    );
    assert!(
        !harness.has_session(&tools_session),
        "append must not create a container of its own"
    );
    matrix("append");

    assert_success(&harness.bootmux(&["stop", &harness.session]), "stop");
    assert!(
        wait_until(|| !harness.has_session(&harness.session)),
        "stop must remove the session"
    );
    assert_eq!(
        read_lines(&hooks_log)
            .iter()
            .filter(|line| *line == "stop")
            .count(),
        1,
        "the rendered stop hook must run exactly once"
    );
    let socket_sessions = String::from_utf8_lossy(
        &harness
            .tmux_on_socket(&["list-sessions", "-F", "#S"])
            .stdout,
    )
    .into_owned();
    assert!(
        socket_sessions
            .lines()
            .any(|line| line == harness.socket_session),
        "stop must not touch unrelated containers: {socket_sessions}"
    );
    let listed = harness.bootmux(&["list", "--active", "-n"]);
    assert_success(&listed, "list --active after stop");
    assert!(
        !String::from_utf8_lossy(&listed.stdout)
            .lines()
            .any(|line| line.trim() == harness.session),
        "a stopped project must leave list --active"
    );
    matrix("explicit_stop");

    // stop-all discovers managed projects through the real session list.
    let second_root = harness.root.join("second");
    std::fs::create_dir_all(&second_root).unwrap();
    let second_project = harness
        .projects
        .join(format!("{}.yml", harness.second_session));
    write_project(
        &second_project,
        &format!(
            "name: {name}\n\
             root: {second}\n\
             attach: false\n\
             on_project_stop: echo stopped >> {second}/stopped.log\n\
             windows:\n  \
             - shell: echo second >> {second}/second.log\n",
            name = harness.second_session,
            second = second_root.display(),
        ),
    );
    assert_success(
        &harness.bootmux(&["start", &harness.second_session]),
        "second project start",
    );
    assert!(
        wait_until(|| harness.has_session(&harness.second_session)),
        "the second project should be running"
    );
    assert_success(&harness.bootmux(&["stop-all", "-y"]), "stop-all");
    assert!(
        wait_until(|| !harness.has_session(&harness.second_session)),
        "stop-all must remove the managed project"
    );
    assert_eq!(
        wait_for_lines(&second_root.join("stopped.log"), &["stopped"]),
        vec!["stopped"],
        "stop-all must run the rendered stop hook"
    );
    matrix("stop_all");

    // A project whose root cannot be entered must fail without leaving a
    // partial session behind.
    let broken_session = format!("bmtx-bad-{process}-{nonce}");
    let broken_project = harness.projects.join(format!("{broken_session}.yml"));
    write_project(
        &broken_project,
        &format!(
            "name: {broken_session}\n\
             root: {}/definitely-missing\n\
             attach: false\n\
             windows:\n  - editor: echo unreachable\n",
            harness.root.display(),
        ),
    );
    assert_failure(
        &harness.bootmux(&["start", &broken_session]),
        "start with a missing root",
    );
    assert!(
        !harness.has_session(&broken_session),
        "a failed start must not leave a session behind"
    );
    matrix("failure_rollback");

    assert_success(
        &harness.bootmux(&["stop", &harness.socket_session]),
        "isolated socket stop",
    );
}
