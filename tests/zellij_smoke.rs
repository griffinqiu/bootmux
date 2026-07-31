#![cfg(unix)]

use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::thread::sleep;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use tempfile::TempDir;

/// Owns the isolated session and tears it down even when an assertion fails.
struct Cleanup {
    bootmux: PathBuf,
    zellij: PathBuf,
    session: String,
    home: PathBuf,
    config_home: PathBuf,
    cache_home: PathBuf,
    zellij_config_dir: PathBuf,
    projects: PathBuf,
}

impl Cleanup {
    fn command_env(&self, command: &mut Command) {
        command
            .env("HOME", &self.home)
            .env("XDG_CONFIG_HOME", &self.config_home)
            .env("XDG_CACHE_HOME", &self.cache_home)
            .env("ZELLIJ_CONFIG_DIR", &self.zellij_config_dir)
            .env("TMUXINATOR_CONFIG", &self.projects)
            // zellij inherits these into panes; leaving them set would make
            // bootmux think it is already running inside a session.
            .env_remove("ZELLIJ")
            .env_remove("ZELLIJ_SESSION_NAME")
            .env_remove("ZELLIJ_PANE_ID");
    }

    fn bootmux(&self, args: &[&str]) -> Output {
        let mut command = Command::new(&self.bootmux);
        command.args(args).env("SHELL", "/bin/sh");
        self.command_env(&mut command);
        command.output().unwrap()
    }

    fn zellij(&self, args: &[&str]) -> Output {
        let mut command = Command::new(&self.zellij);
        command.args(args);
        self.command_env(&mut command);
        command.output().unwrap()
    }

    fn session_is_running(&self) -> bool {
        let output = self.zellij(&["list-sessions", "--no-formatting"]);
        String::from_utf8_lossy(&output.stdout)
            .lines()
            .filter(|line| !line.contains("(EXITED"))
            .any(|line| line.split_whitespace().next() == Some(self.session.as_str()))
    }

    /// Ordinary terminal panes only, matching what the backend lays out: a
    /// zellij config can also add plugin panes such as the tab and status bars.
    fn pane_count(&self) -> usize {
        let output = self.zellij(&["--session", &self.session, "action", "list-panes", "--json"]);
        let Ok(serde_json::Value::Array(panes)) =
            serde_json::from_slice::<serde_json::Value>(&output.stdout)
        else {
            return 0;
        };
        panes
            .iter()
            .filter(|pane| {
                ["is_plugin", "is_floating", "is_suppressed"]
                    .iter()
                    .all(|flag| pane.get(flag) != Some(&serde_json::Value::Bool(true)))
            })
            .count()
    }
}

impl Drop for Cleanup {
    fn drop(&mut self) {
        let _ = self.zellij(&["kill-session", &self.session]);
        let _ = self.zellij(&["delete-session", &self.session, "--force"]);
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

/// Pane commands are typed into a shell asynchronously, so the file a pane
/// writes is polled rather than read once.
fn wait_for_contents(path: &Path, expected: &str) -> String {
    let deadline = Instant::now() + Duration::from_secs(20);
    let mut seen = String::new();
    while Instant::now() < deadline {
        seen = std::fs::read_to_string(path).unwrap_or_default();
        if seen.trim() == expected {
            return seen;
        }
        sleep(Duration::from_millis(100));
    }
    seen
}

fn wait_until(mut condition: impl FnMut() -> bool) -> bool {
    let deadline = Instant::now() + Duration::from_secs(20);
    while Instant::now() < deadline {
        if condition() {
            return true;
        }
        sleep(Duration::from_millis(100));
    }
    false
}

#[test]
#[ignore = "requires a local zellij >= 0.44"]
fn creates_reuses_and_stops_a_real_zellij_session() {
    let zellij = match std::env::var_os("ZELLIJ_BIN") {
        Some(path) => PathBuf::from(path),
        None => PathBuf::from("zellij"),
    };
    if !Command::new(&zellij)
        .arg("--version")
        .output()
        .map(|output| output.status.success())
        .unwrap_or(false)
    {
        eprintln!("skipping: zellij is not installed");
        return;
    }

    let temp = TempDir::new().unwrap();
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    // zellij caps session names at 36 characters.
    let session = format!("bmzj-{}-{}", std::process::id(), nonce % 100_000);

    let projects = temp.path().join("projects");
    let root = temp.path().join("root");
    for directory in [&projects, &root] {
        std::fs::create_dir_all(directory).unwrap();
    }

    let cleanup = Cleanup {
        bootmux: PathBuf::from(env!("CARGO_BIN_EXE_bootmux")),
        zellij,
        session: session.clone(),
        home: temp.path().join("home"),
        config_home: temp.path().join("config"),
        cache_home: temp.path().join("cache"),
        zellij_config_dir: temp.path().join("zellij"),
        projects: projects.clone(),
    };
    for directory in [
        &cleanup.home,
        &cleanup.config_home,
        &cleanup.cache_home,
        &cleanup.zellij_config_dir,
    ] {
        std::fs::create_dir_all(directory).unwrap();
    }

    let project_file = projects.join(format!("{session}.yml"));
    std::fs::write(
        &project_file,
        format!(
            "name: {session}\n\
             root: {}\n\
             attach: false\n\
             pre_window: echo pre >> pre.log\n\
             on_project_stop: echo stopped >> stopped.log\n\
             windows:\n  \
             - editor:\n      \
                 panes:\n        \
                   - left:\n            \
                       command: echo left >> left.log\n        \
                   - right:\n            \
                       split: right\n            \
                       ratio: 0.4\n            \
                       commands:\n              \
                         - echo right-one >> right.log\n              \
                         - echo right-two >> right.log\n  \
             - shell: echo shell >> shell.log\n",
            root.display()
        ),
    )
    .unwrap();
    let project_arg = project_file.to_str().unwrap();

    // Creating the session lays out two tabs with three panes total.
    assert_success(
        &cleanup.bootmux(&["--backend", "zellij", "start", "-p", project_arg]),
        "zellij start",
    );
    assert!(
        cleanup.session_is_running(),
        "the session should be running"
    );
    assert!(
        wait_until(|| cleanup.pane_count() == 3),
        "expected 3 panes, saw {}",
        cleanup.pane_count()
    );

    // pre_window runs in every pane, and a pane's own commands run in order.
    assert_eq!(
        wait_for_contents(&root.join("pre.log"), "pre\npre\npre"),
        "pre\npre\npre\n"
    );
    assert_eq!(wait_for_contents(&root.join("left.log"), "left"), "left\n");
    assert_eq!(
        wait_for_contents(&root.join("right.log"), "right-one\nright-two"),
        "right-one\nright-two\n"
    );
    assert_eq!(
        wait_for_contents(&root.join("shell.log"), "shell"),
        "shell\n"
    );

    // Starting again reuses the session instead of re-running pane commands.
    assert_success(
        &cleanup.bootmux(&["--backend", "zellij", "start", "-p", project_arg]),
        "second zellij start",
    );
    sleep(Duration::from_secs(1));
    assert_eq!(
        std::fs::read_to_string(root.join("left.log")).unwrap(),
        "left\n",
        "a reused session must not re-run pane commands"
    );
    assert_eq!(cleanup.pane_count(), 3, "reuse must not add panes");

    // `list --active` finds the project by its session name.
    let listed = cleanup.bootmux(&["--backend", "zellij", "list", "--active", "-n"]);
    assert_success(&listed, "list --active");
    assert!(
        String::from_utf8_lossy(&listed.stdout).contains(&session),
        "list --active did not report {session}: {}",
        String::from_utf8_lossy(&listed.stdout)
    );

    // Stop runs the stop hook and closes the session.
    assert_success(
        &cleanup.bootmux(&["--backend", "zellij", "stop", "-p", project_arg]),
        "zellij stop",
    );
    assert_eq!(
        wait_for_contents(&root.join("stopped.log"), "stopped"),
        "stopped\n"
    );
    assert!(
        wait_until(|| !cleanup.session_is_running()),
        "the session should be gone after stop"
    );
}

#[test]
#[ignore = "requires a local zellij >= 0.44"]
fn a_failed_project_root_leaves_no_session_behind() {
    let zellij = match std::env::var_os("ZELLIJ_BIN") {
        Some(path) => PathBuf::from(path),
        None => PathBuf::from("zellij"),
    };
    if !Command::new(&zellij)
        .arg("--version")
        .output()
        .map(|output| output.status.success())
        .unwrap_or(false)
    {
        eprintln!("skipping: zellij is not installed");
        return;
    }

    let temp = TempDir::new().unwrap();
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let session = format!("bmzj-bad-{}-{}", std::process::id(), nonce % 10_000);
    let projects = temp.path().join("projects");
    std::fs::create_dir_all(&projects).unwrap();

    let cleanup = Cleanup {
        bootmux: PathBuf::from(env!("CARGO_BIN_EXE_bootmux")),
        zellij,
        session: session.clone(),
        home: temp.path().join("home"),
        config_home: temp.path().join("config"),
        cache_home: temp.path().join("cache"),
        zellij_config_dir: temp.path().join("zellij"),
        projects: projects.clone(),
    };
    for directory in [
        &cleanup.home,
        &cleanup.config_home,
        &cleanup.cache_home,
        &cleanup.zellij_config_dir,
    ] {
        std::fs::create_dir_all(directory).unwrap();
    }

    // A window declaring more panes than its layout can hold is rejected
    // during preflight, before any session is created.
    let project_file = projects.join(format!("{session}.yml"));
    std::fs::write(
        &project_file,
        format!(
            "name: {session}\nroot: {}\nattach: false\nwindows:\n  - broken:\n      \
             layout: main-vertical\n      panes:\n        - only:\n            \
             split: right\n            ratio: 0.99\n",
            temp.path().display()
        ),
    )
    .unwrap();

    let output = cleanup.bootmux(&[
        "--backend",
        "zellij",
        "start",
        "-p",
        project_file.to_str().unwrap(),
    ]);
    assert!(
        !output.status.success(),
        "an unrepresentable project must not start"
    );
    assert!(
        !cleanup.session_is_running(),
        "a rejected project must not leave a session behind"
    );
}
