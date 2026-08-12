#![cfg(unix)]

//! Required real-runtime matrix for the zellij backend.
//!
//! Run explicitly against an exact stable zellij:
//!   cargo test --test zellij_smoke -- --ignored --nocapture --test-threads=1
//!
//! Every row prints `BOOTMUX_MATRIX zellij <row> PASS` only after its
//! assertions and cleanup succeeded, so the coverage is machine-verifiable.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::thread::sleep;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde_json::Value;
use tempfile::TempDir;

const WAIT_TIMEOUT: Duration = Duration::from_secs(30);

fn matrix(row: &str) {
    println!("BOOTMUX_MATRIX zellij {row} PASS");
}

/// A zellij session is created detached, so the terminal that ran the command
/// only learns the outcome from this line.
fn assert_outcome(output: &Output, action: &str, session: &str, operation: &str) {
    let expected = format!("bootmux: {action} zellij session {session:?}");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.lines().any(|line| line == expected),
        "{operation} must report `{expected}` on stdout\nstdout: {stdout}"
    );
}

/// Owns the isolated sessions and tears them down even when an assertion fails.
struct Harness {
    _temp: TempDir,
    bootmux: PathBuf,
    zellij: PathBuf,
    home: PathBuf,
    config_home: PathBuf,
    cache_home: PathBuf,
    zellij_config_dir: PathBuf,
    projects: PathBuf,
    sessions: Vec<String>,
}

impl Harness {
    fn command_env(&self, command: &mut Command) {
        command
            .env("HOME", &self.home)
            .env("SHELL", "/bin/sh")
            .env("XDG_CONFIG_HOME", &self.config_home)
            .env("XDG_CACHE_HOME", &self.cache_home)
            .env("ZELLIJ_CONFIG_DIR", &self.zellij_config_dir)
            .env("TMUXINATOR_CONFIG", &self.projects)
            // zellij exports these into panes; leaving them set would make
            // bootmux believe it is already running inside a session.
            .env_remove("ZELLIJ")
            .env_remove("ZELLIJ_SESSION_NAME")
            .env_remove("ZELLIJ_PANE_ID");
    }

    fn bootmux(&self, args: &[&str]) -> Output {
        let mut command = Command::new(&self.bootmux);
        command.args(["--backend", "zellij"]).args(args);
        self.command_env(&mut command);
        command.output().unwrap()
    }

    /// `--append` only works from inside a session, which zellij signals with
    /// `ZELLIJ_SESSION_NAME`.
    fn bootmux_inside(&self, args: &[&str], session: &str) -> Output {
        let mut command = Command::new(&self.bootmux);
        command.args(["--backend", "zellij"]).args(args);
        self.command_env(&mut command);
        command.env("ZELLIJ_SESSION_NAME", session);
        command.output().unwrap()
    }

    fn zellij(&self, args: &[&str]) -> Output {
        let mut command = Command::new(&self.zellij);
        command.args(args);
        self.command_env(&mut command);
        command.output().unwrap()
    }

    fn session_listing(&self) -> String {
        String::from_utf8_lossy(&self.zellij(&["list-sessions", "--no-formatting"]).stdout)
            .into_owned()
    }

    fn session_is_running(&self, session: &str) -> bool {
        self.session_listing()
            .lines()
            .filter(|line| !line.contains("(EXITED"))
            .any(|line| line.split_whitespace().next() == Some(session))
    }

    fn panes(&self, session: &str) -> Vec<Value> {
        let output = self.zellij(&["--session", session, "action", "list-panes", "--json"]);
        let Ok(Value::Array(panes)) = serde_json::from_slice::<Value>(&output.stdout) else {
            return Vec::new();
        };
        panes
            .into_iter()
            .filter(|pane| {
                ["is_plugin", "is_floating", "is_suppressed"]
                    .iter()
                    .all(|flag| pane.get(flag) != Some(&Value::Bool(true)))
            })
            .collect()
    }

    fn pane_count(&self, session: &str) -> usize {
        self.panes(session).len()
    }

    fn tab_names(&self, session: &str) -> Vec<String> {
        String::from_utf8_lossy(
            &self
                .zellij(&["--session", session, "action", "query-tab-names"])
                .stdout,
        )
        .lines()
        .map(str::to_string)
        .collect()
    }
}

impl Drop for Harness {
    fn drop(&mut self) {
        for session in &self.sessions {
            let _ = self.zellij(&["kill-session", session]);
            let _ = self.zellij(&["delete-session", session, "--force"]);
        }
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
        "{operation} unexpectedly succeeded\nstdout: {}",
        String::from_utf8_lossy(&output.stdout)
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
/// polled rather than read once.
fn wait_for_lines(path: &Path, expected: &[&str]) -> Vec<String> {
    let mut seen = Vec::new();
    wait_until(|| {
        seen = read_lines(path);
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

fn zellij_binary() -> PathBuf {
    std::env::var_os("ZELLIJ_BIN")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("zellij"))
}

fn isolated_directories(temp: &TempDir) -> (PathBuf, PathBuf, PathBuf, PathBuf, PathBuf, PathBuf) {
    let home = temp.path().join("home");
    let config_home = temp.path().join("config");
    let cache_home = temp.path().join("cache");
    let zellij_config_dir = temp.path().join("zellij");
    let projects = temp.path().join("projects");
    let root = temp.path().join("root");
    for directory in [
        &home,
        &config_home,
        &cache_home,
        &zellij_config_dir,
        &projects,
        &root,
    ] {
        std::fs::create_dir_all(directory).unwrap();
    }
    // zellij only keeps a stopped session resurrectable once it has serialized
    // it, which the default one-minute interval would make untestable.
    std::fs::write(
        zellij_config_dir.join("config.kdl"),
        "session_serialization true\nserialization_interval 1\n",
    )
    .unwrap();
    (
        home,
        config_home,
        cache_home,
        zellij_config_dir,
        projects,
        root,
    )
}

fn unique_suffix() -> String {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos()
        % 100_000;
    // zellij caps session names at 36 characters.
    format!("{}-{nonce}", std::process::id())
}

#[test]
#[ignore = "requires a real local zellij"]
fn zellij_runtime_matrix() {
    let zellij = zellij_binary();
    let version_output = Command::new(&zellij)
        .arg("--version")
        .output()
        .expect("zellij must be installed and resolvable");
    assert!(version_output.status.success(), "zellij --version failed");
    let version = String::from_utf8_lossy(&version_output.stdout)
        .trim()
        .to_string();
    let resolved = String::from_utf8_lossy(
        &Command::new("/bin/sh")
            .args(["-c", "command -v zellij"])
            .output()
            .unwrap()
            .stdout,
    )
    .trim()
    .to_string();
    assert!(!resolved.is_empty(), "zellij must resolve through PATH");
    println!(
        "zellij identity: {version} at {}",
        std::fs::canonicalize(&resolved).unwrap().display()
    );
    if let Some(expected) = std::env::var_os("BOOTMUX_MATRIX_EXPECT_ZELLIJ") {
        assert_eq!(
            version,
            expected.to_string_lossy(),
            "zellij is not the frozen exact stable target"
        );
    }
    matrix("identity");

    let temp = TempDir::new().unwrap();
    let (home, config_home, cache_home, zellij_config_dir, projects, root) =
        isolated_directories(&temp);
    let suffix = unique_suffix();
    let session = format!("bmzj-{suffix}");
    let second_session = format!("bmzj2-{suffix}");
    let app_root = root.join("app");
    std::fs::create_dir_all(&app_root).unwrap();

    let harness = Harness {
        bootmux: PathBuf::from(env!("CARGO_BIN_EXE_bootmux")),
        zellij,
        home,
        config_home,
        cache_home,
        zellij_config_dir,
        projects: projects.clone(),
        sessions: vec![session.clone(), second_session.clone()],
        _temp: temp,
    };

    let root_display = root.display().to_string();
    let hooks_log = root.join("hooks.log");
    let editor_log = root.join("editor.log");
    let logs_log = root.join("logs.log");

    let main_project = projects.join(format!("{session}.yml"));
    std::fs::write(
        &main_project,
        format!(
            "name: {session}\n\
             root: {root_display}\n\
             attach: false\n\
             startup_window: logs\n\
             startup_pane: watcher\n\
             pre_window: echo pre >> {root_display}/pre.log\n\
             on_project_start: echo start >> {root_display}/hooks.log\n\
             on_project_first_start: echo first_start >> {root_display}/hooks.log\n\
             on_project_restart: echo restart >> {root_display}/hooks.log\n\
             on_project_exit: echo exit >> {root_display}/hooks.log\n\
             on_project_stop: echo stop >> {root_display}/hooks.log\n\
             windows:\n  \
             - editor:\n      \
                 root: {root_display}/app\n      \
                 pre: echo window-pre >> {root_display}/window-pre.log\n      \
                 panes:\n        \
                   - main:\n            \
                       commands:\n              \
                         - echo cwd-$(basename \"$PWD\") >> {root_display}/editor.log\n              \
                         - echo editor-two >> {root_display}/editor.log\n        \
                   - side:\n            \
                       command: echo side >> {root_display}/editor.log\n  \
             - logs:\n      \
                 panes:\n        \
                   - tail: echo tail >> {root_display}/logs.log\n        \
                   - watcher: echo watcher >> {root_display}/logs.log\n",
        ),
    )
    .unwrap();
    let main_path = main_project.to_str().unwrap().to_string();

    // The rendered KDL is the zellij contract; the live session proves zellij
    // actually loaded it.
    let debug = harness.bootmux(&["debug", "-p", &main_path]);
    assert_success(&debug, "bootmux debug");
    let plan = String::from_utf8_lossy(&debug.stdout).into_owned();
    for fragment in [
        "tab name=\"editor\"",
        "tab name=\"logs\"",
        "focus=true",
        &format!(
            "cwd=\"{}\"",
            std::fs::canonicalize(&app_root).unwrap().display()
        ),
        "pane name=\"watcher\"",
    ] {
        assert!(
            plan.contains(fragment),
            "rendered zellij layout is missing {fragment}:\n{plan}"
        );
    }

    let first_start = harness.bootmux(&["start", "-p", &main_path]);
    assert_success(&first_start, "first start");
    assert_outcome(&first_start, "created", &session, "first start");
    assert!(
        wait_until(|| harness.session_is_running(&session)),
        "the session should be running"
    );
    assert!(
        wait_until(|| harness.tab_names(&session) == ["editor", "logs"]),
        "zellij did not load the rendered tabs, saw {:?}",
        harness.tab_names(&session)
    );
    matrix("kdl_load");

    assert!(
        wait_until(|| harness.pane_count(&session) == 4),
        "expected 4 panes, saw {}",
        harness.pane_count(&session)
    );
    assert_eq!(
        harness
            .session_listing()
            .lines()
            .filter(|line| line.split_whitespace().next() == Some(session.as_str()))
            .count(),
        1,
        "start must create exactly one session"
    );
    matrix("create_topology");

    // pre_window runs in every pane, a window `pre` only in its own window's
    // panes, and a pane's own commands run in their configured order.
    assert_eq!(wait_for_lines(&root.join("pre.log"), &["pre"; 4]).len(), 4);
    assert_eq!(
        wait_for_lines(&root.join("window-pre.log"), &["window-pre"; 2]).len(),
        2
    );
    assert_eq!(
        wait_for_lines(&editor_log, &["cwd-app", "editor-two", "side"]),
        vec!["cwd-app", "editor-two", "side"]
    );
    assert_eq!(
        wait_for_lines(&logs_log, &["tail", "watcher"]),
        vec!["tail", "watcher"]
    );
    matrix("root_and_commands");

    // The JSON listing is how bootmux finds and targets panes.
    let panes = harness.panes(&session);
    assert_eq!(panes.len(), 4);
    for pane in &panes {
        for field in [
            "id",
            "tab_id",
            "tab_position",
            "tab_name",
            "pane_x",
            "pane_y",
            "title",
            "is_plugin",
            "is_floating",
            "is_suppressed",
        ] {
            assert!(
                pane.get(field).is_some(),
                "list-panes --json lost the {field} field: {pane}"
            );
        }
    }
    assert_eq!(
        panes
            .iter()
            .filter(|pane| pane.get("tab_name").and_then(Value::as_str) == Some("logs"))
            .count(),
        2,
        "panes must report the tab they belong to"
    );
    matrix("list_panes_json");

    // A detached session has no attached client, so zellij reports the focus
    // the layout declared rather than an active tab: the configured tab is the
    // one marked focused, and its focused pane is the configured one.
    let logs_tab = plan
        .lines()
        .find(|line| line.contains("tab name=\"logs\""))
        .expect("the rendered layout must declare the logs tab");
    assert!(
        logs_tab.contains("focus=true"),
        "startup_window did not focus the configured tab: {logs_tab}"
    );
    let focused = panes
        .iter()
        .filter(|pane| pane.get("tab_name").and_then(Value::as_str) == Some("logs"))
        .find(|pane| pane.get("is_focused") == Some(&Value::Bool(true)))
        .and_then(|pane| pane.get("title"))
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    assert_eq!(
        focused, "watcher",
        "startup_pane did not select the configured pane"
    );
    matrix("startup_focus");

    let second_start = harness.bootmux(&["start", "-p", &main_path]);
    assert_success(&second_start, "second start");
    assert_outcome(&second_start, "reused", &session, "second start");
    sleep(Duration::from_secs(1));
    assert_eq!(
        harness.tab_names(&session),
        vec!["editor", "logs"],
        "reuse must not rebuild topology"
    );
    assert_eq!(harness.pane_count(&session), 4, "reuse must not add panes");
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
        names.lines().filter(|line| line.trim() == session).count(),
        1,
        "list --active must report the project exactly once: {names}"
    );
    matrix("active_listing");

    // Append adds another project's tabs to the session bootmux runs inside.
    let tools_project = projects.join("tools.yml");
    std::fs::write(
        &tools_project,
        format!(
            "name: tools-{suffix}\n\
             root: {root_display}\n\
             attach: false\n\
             windows:\n  \
             - tools:\n      \
                 panes:\n        \
                   - watch: echo watch >> {root_display}/tools.log\n        \
                   - build: echo build >> {root_display}/tools.log\n",
        ),
    )
    .unwrap();
    let tools_path = tools_project.to_str().unwrap().to_string();
    assert_failure(
        &harness.bootmux(&["start", "-p", &tools_path, "--append"]),
        "append outside a session",
    );
    let appended = harness.bootmux_inside(&["start", "-p", &tools_path, "--append"], &session);
    assert_success(&appended, "append inside the session");
    assert_outcome(
        &appended,
        "appended to",
        &session,
        "append inside the session",
    );
    assert!(
        wait_until(|| harness.tab_names(&session) == ["editor", "logs", "tools"]),
        "append must add exactly one copy of the topology, saw {:?}",
        harness.tab_names(&session)
    );
    assert!(
        wait_until(|| harness.pane_count(&session) == 6),
        "expected 6 panes after append, saw {}",
        harness.pane_count(&session)
    );
    assert_eq!(
        wait_for_lines(&root.join("tools.log"), &["watch", "build"]),
        vec!["watch", "build"],
        "appended panes must run their commands"
    );
    assert!(
        !harness.session_is_running(&format!("tools-{suffix}")),
        "append must not create a container of its own"
    );
    matrix("append");

    // An append that fails after creating tabs must close them again. A pane
    // command larger than the argument limit can never be delivered, so the
    // failure lands after the tabs exist.
    let oversized_project = projects.join("oversized.yml");
    std::fs::write(
        &oversized_project,
        format!(
            "name: oversized-{suffix}\n\
             root: {root_display}\n\
             attach: false\n\
             windows:\n  \
             - created:\n      \
                 panes:\n        \
                   - one: echo one\n        \
                   - two: echo two\n  \
             - undeliverable: echo {}\n",
            "x".repeat(4_000_000),
        ),
    )
    .unwrap();
    let before_rollback = harness.tab_names(&session);
    assert_failure(
        &harness.bootmux_inside(
            &[
                "start",
                "-p",
                oversized_project.to_str().unwrap(),
                "--append",
            ],
            &session,
        ),
        "append with an undeliverable command",
    );
    assert!(
        wait_until(|| harness.tab_names(&session) == before_rollback),
        "a failed append must close the tabs it created, saw {:?}",
        harness.tab_names(&session)
    );
    assert_eq!(
        harness.pane_count(&session),
        6,
        "a failed append must not leave panes behind"
    );
    matrix("append_rollback");

    // A second managed project doubles as the container stop must not touch.
    let second_root = root.join("second");
    std::fs::create_dir_all(&second_root).unwrap();
    let second_project = projects.join(format!("{second_session}.yml"));
    std::fs::write(
        &second_project,
        format!(
            "name: {second_session}\n\
             root: {}\n\
             attach: false\n\
             on_project_stop: echo stopped >> {}/stopped.log\n\
             windows:\n  \
             - shell: echo second >> {}/second.log\n",
            second_root.display(),
            second_root.display(),
            second_root.display(),
        ),
    )
    .unwrap();
    assert_success(
        &harness.bootmux(&["start", "-p", second_project.to_str().unwrap()]),
        "second project start",
    );
    assert!(
        wait_until(|| harness.session_is_running(&second_session)),
        "the second project should be running"
    );

    let stopped = harness.bootmux(&["stop", "-p", &main_path]);
    assert_success(&stopped, "stop");
    assert_outcome(&stopped, "stopped", &session, "stop");
    assert!(
        wait_until(|| !harness.session_is_running(&session)),
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
    assert!(
        harness.session_is_running(&second_session),
        "stop must not touch unrelated sessions"
    );
    matrix("explicit_stop");

    // zellij keeps a stopped session listed for resurrection; bootmux must not
    // report it as active.
    assert!(
        wait_until(|| harness
            .session_listing()
            .lines()
            .any(|line| line.starts_with(&session) && line.contains("(EXITED"))),
        "zellij should still list the stopped session as exited: {}",
        harness.session_listing()
    );
    let listed = harness.bootmux(&["list", "--active", "-n"]);
    assert_success(&listed, "list --active after stop");
    assert!(
        !String::from_utf8_lossy(&listed.stdout)
            .lines()
            .any(|line| line.trim() == session),
        "a dead session must not be reported as active"
    );
    matrix("dead_session_filter");

    assert_success(&harness.bootmux(&["stop-all", "-y"]), "stop-all");
    assert!(
        wait_until(|| !harness.session_is_running(&second_session)),
        "stop-all must remove the managed project"
    );
    assert_eq!(
        wait_for_lines(&second_root.join("stopped.log"), &["stopped"]),
        vec!["stopped"],
        "stop-all must run the rendered stop hook"
    );
    matrix("stop_all");
}

#[test]
#[ignore = "requires a real local zellij"]
fn a_failed_project_root_leaves_no_session_behind() {
    let zellij = zellij_binary();
    assert!(
        Command::new(&zellij)
            .arg("--version")
            .output()
            .map(|output| output.status.success())
            .unwrap_or(false),
        "zellij must be installed"
    );

    let temp = TempDir::new().unwrap();
    let (home, config_home, cache_home, zellij_config_dir, projects, root) =
        isolated_directories(&temp);
    let session = format!("bmzjb-{}", unique_suffix());

    let harness = Harness {
        bootmux: PathBuf::from(env!("CARGO_BIN_EXE_bootmux")),
        zellij,
        home,
        config_home,
        cache_home,
        zellij_config_dir,
        projects: projects.clone(),
        sessions: vec![session.clone()],
        _temp: temp,
    };

    // A window declaring more panes than its layout can hold is rejected
    // during preflight, before any session is created.
    let project = projects.join(format!("{session}.yml"));
    std::fs::write(
        &project,
        format!(
            "name: {session}\nroot: {}\nattach: false\nwindows:\n  - broken:\n      \
             layout: main-vertical\n      panes:\n        - only:\n            \
             split: right\n            ratio: 0.99\n",
            root.display(),
        ),
    )
    .unwrap();

    assert_failure(
        &harness.bootmux(&["start", "-p", project.to_str().unwrap()]),
        "start with an unrepresentable topology",
    );
    assert!(
        !harness.session_is_running(&session),
        "a rejected project must not leave a session behind"
    );
    assert!(
        !harness
            .session_listing()
            .lines()
            .any(|line| line.starts_with(&session)),
        "a rejected project must not leave a resurrectable session either"
    );
    matrix("failure_rollback");
}
