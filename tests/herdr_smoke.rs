#![cfg(unix)]

//! Required real-runtime matrix for the Herdr backend.
//!
//! Run explicitly against an exact stable Herdr:
//!   cargo test --test herdr_smoke -- --ignored --nocapture --test-threads=1
//!
//! Every row prints `BOOTMUX_MATRIX herdr <row> PASS` only after its assertions
//! and cleanup succeeded, so the coverage is machine-verifiable.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::{Arc, Barrier};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde_json::Value;
use tempfile::TempDir;

const WAIT_TIMEOUT: Duration = Duration::from_secs(30);

fn matrix(row: &str) {
    println!("BOOTMUX_MATRIX herdr {row} PASS");
}

/// Herdr settles outside the calling terminal, so every successful lifecycle
/// command has to say what it did. The endpoint suffix is omitted because the
/// server reports its socket through the platform's canonical path.
fn outcome_prefix(action: &str, project: &str) -> String {
    format!("bootmux: {action} herdr workspace {project:?} (")
}

fn reports_outcome(output: &Output, action: &str, project: &str) -> bool {
    let prefix = outcome_prefix(action, project);
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .any(|line| line.starts_with(&prefix))
}

fn assert_outcome(output: &Output, action: &str, project: &str, operation: &str) {
    assert!(
        reports_outcome(output, action, project),
        "{operation} must report `{}` on stdout\nstdout: {}",
        outcome_prefix(action, project),
        String::from_utf8_lossy(&output.stdout)
    );
}

struct Harness {
    _temp: TempDir,
    bootmux: PathBuf,
    herdr: PathBuf,
    project: PathBuf,
    home: PathBuf,
    config_home: PathBuf,
    data_home: PathBuf,
    state_home: PathBuf,
    cache_home: PathBuf,
    config_path: PathBuf,
    socket_path: PathBuf,
    projects: PathBuf,
    label_setting: String,
    socket_setting: String,
}

impl Harness {
    fn command_env(&self, command: &mut Command) {
        command
            .env("HERDR_CONFIG_PATH", &self.config_path)
            .env("HERDR_SOCKET_PATH", &self.socket_path)
            .env("HOME", &self.home)
            .env("TMUXINATOR_CONFIG", &self.projects)
            .env("XDG_CONFIG_HOME", &self.config_home)
            .env("XDG_DATA_HOME", &self.data_home)
            .env("XDG_STATE_HOME", &self.state_home)
            .env("XDG_CACHE_HOME", &self.cache_home);
    }

    fn bootmux(&self, args: &[&str]) -> Output {
        let mut command = Command::new(&self.bootmux);
        command.args(args).env("SHELL", "/bin/sh");
        self.command_env(&mut command);
        command.output().unwrap()
    }

    fn bootmux_in_workspace(&self, args: &[&str], workspace_id: &str) -> Output {
        let mut command = Command::new(&self.bootmux);
        command
            .args(args)
            .env("SHELL", "/bin/sh")
            .env("HERDR_ACTIVE_WORKSPACE_ID", workspace_id);
        self.command_env(&mut command);
        command.output().unwrap()
    }

    fn herdr(&self, args: &[&str]) -> Output {
        let mut command = Command::new(&self.herdr);
        command.args(args);
        self.command_env(&mut command);
        command.output().unwrap()
    }

    fn start_args(&self) -> [&str; 8] {
        [
            "--backend",
            "herdr",
            "start",
            "--project-config",
            self.project.to_str().unwrap(),
            self.label_setting.as_str(),
            self.socket_setting.as_str(),
            "--no-attach",
        ]
    }

    fn snapshot(&self) -> Value {
        let output = self.herdr(&["api", "snapshot"]);
        assert_success(&output, "Herdr snapshot");
        serde_json::from_slice(&output.stdout).unwrap()
    }

    fn workspaces(&self, label: &str) -> Vec<Value> {
        self.snapshot()
            .pointer("/result/snapshot/workspaces")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default()
            .into_iter()
            .filter(|workspace| workspace.get("label").and_then(Value::as_str) == Some(label))
            .collect()
    }

    fn tabs_of(&self, workspace_id: &str) -> Vec<Value> {
        self.snapshot()
            .pointer("/result/snapshot/tabs")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default()
            .into_iter()
            .filter(|tab| tab.get("workspace_id").and_then(Value::as_str) == Some(workspace_id))
            .collect()
    }

    fn focused_pane_id(&self) -> String {
        self.snapshot()
            .pointer("/result/snapshot/focused_pane_id")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string()
    }

    fn panes_of(&self, workspace_id: &str) -> Vec<Value> {
        self.snapshot()
            .pointer("/result/snapshot/panes")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default()
            .into_iter()
            .filter(|pane| pane.get("workspace_id").and_then(Value::as_str) == Some(workspace_id))
            .collect()
    }
}

impl Drop for Harness {
    fn drop(&mut self) {
        let _ = self.bootmux(&[
            "--backend",
            "herdr",
            "stop",
            "--project-config",
            self.project.to_str().unwrap(),
            self.label_setting.as_str(),
            self.socket_setting.as_str(),
        ]);
        let _ = self.herdr(&["server", "stop"]);
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
        thread::sleep(Duration::from_millis(100));
    }
    false
}

fn read_lines(path: &Path) -> Vec<String> {
    std::fs::read_to_string(path)
        .unwrap_or_default()
        .lines()
        .map(str::to_string)
        .collect()
}

#[test]
#[ignore = "requires a real local Herdr"]
fn herdr_runtime_matrix() {
    let herdr = std::env::var_os("HERDR_BIN")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("herdr"));
    let version_output = Command::new(&herdr)
        .arg("--version")
        .output()
        .expect("Herdr must be installed and resolvable");
    assert!(version_output.status.success(), "herdr --version failed");
    let version = String::from_utf8_lossy(&version_output.stdout)
        .trim()
        .to_string();
    let resolved = String::from_utf8_lossy(
        &Command::new("/bin/sh")
            .args(["-c", "command -v herdr"])
            .output()
            .unwrap()
            .stdout,
    )
    .trim()
    .to_string();
    assert!(!resolved.is_empty(), "Herdr must resolve through PATH");
    println!(
        "herdr identity: {version} at {}",
        std::fs::canonicalize(&resolved).unwrap().display()
    );
    if let Some(expected) = std::env::var_os("BOOTMUX_MATRIX_EXPECT_HERDR") {
        assert_eq!(
            version,
            expected.to_string_lossy(),
            "Herdr is not the frozen exact stable target"
        );
    }
    matrix("identity");

    let temp = TempDir::new().unwrap();
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let label = format!("bootmux-smoke-{}-{nonce}", std::process::id());
    let root = temp.path().join("work");
    let app_root = root.join("app");
    let projects = temp.path().join("projects");
    for directory in [
        &app_root,
        &projects,
        &temp.path().join("home"),
        &temp.path().join("state"),
    ] {
        std::fs::create_dir_all(directory).unwrap();
    }
    let project = projects.join("project.yml");
    let hooks_log = root.join("hooks.log");
    let socket_path = temp.path().join("herdr.sock");

    std::fs::write(
        &project,
        format!(
            r#"name: <%= @settings["label"] %>
root: {root}
socket_path: <%= @settings["socket"] %>
attach: false
on_project_start: echo start >> {root}/hooks.log
on_project_first_start: echo first_start >> {root}/hooks.log
on_project_restart: echo restart >> {root}/hooks.log
on_project_exit: echo exit >> {root}/hooks.log
on_project_stop: test '<%= @settings["label"] %>' = '{label}' && echo stop >> {root}/hooks.log
startup_window: logs
startup_pane: watcher
pre_window: export BOOTMUX_SMOKE=ok
windows:
  - app:
      root: app
      panes:
        - editor:
            command: printf 'editor-cwd=%s\n' "$(basename "$PWD")"
        - server:
            split: right
            ratio: 0.6
            commands:
              - printf 'server-%s\n' "$BOOTMUX_SMOKE"
  - logs:
      layout: even-vertical
      panes:
        - tail: printf 'tail-ready\n'
        - watcher: printf 'watcher-ready\n'
"#,
            root = root.display(),
            label = label,
        ),
    )
    .unwrap();

    let harness = Harness {
        bootmux: PathBuf::from(env!("CARGO_BIN_EXE_bootmux")),
        herdr,
        project,
        home: temp.path().join("home"),
        config_home: temp.path().join("config"),
        data_home: temp.path().join("data"),
        state_home: temp.path().join("state"),
        cache_home: temp.path().join("cache"),
        config_path: temp.path().join("herdr-config.toml"),
        socket_setting: format!("socket={}", socket_path.display()),
        socket_path,
        projects: projects.clone(),
        label_setting: format!("label={label}"),
        _temp: temp,
    };
    let project_path = harness.project.to_str().unwrap().to_string();
    let label_setting = harness.label_setting.clone();
    let socket_setting = harness.socket_setting.clone();

    // Two starts racing on the same rendered identity must converge on one
    // workspace rather than creating a second.
    let barrier = Arc::new(Barrier::new(3));
    let concurrent = thread::scope(|scope| {
        let workers = (0..2)
            .map(|_| {
                let barrier = barrier.clone();
                let harness = &harness;
                scope.spawn(move || {
                    barrier.wait();
                    harness.bootmux(&harness.start_args())
                })
            })
            .collect::<Vec<_>>();
        barrier.wait();
        workers
            .into_iter()
            .map(|worker| worker.join().unwrap())
            .collect::<Vec<_>>()
    });
    for output in &concurrent {
        assert_success(output, "concurrent bootmux Herdr start");
    }
    assert_eq!(
        harness.workspaces(&label).len(),
        1,
        "concurrent starts must converge on one workspace"
    );
    assert_eq!(
        concurrent
            .iter()
            .filter(|output| reports_outcome(output, "created", &label))
            .count(),
        1,
        "exactly one concurrent start may report a creation"
    );
    assert_eq!(
        concurrent
            .iter()
            .filter(|output| reports_outcome(output, "reused", &label))
            .count(),
        1,
        "the losing concurrent start must report a reuse"
    );
    matrix("concurrent_start");

    // The status document is how bootmux checks that it can talk to Herdr.
    let status_output = harness.herdr(&["status", "--json"]);
    assert_success(&status_output, "herdr status --json");
    let status: Value = serde_json::from_slice(&status_output.stdout).unwrap();
    for pointer in [
        "/client/version",
        "/client/protocol",
        "/server/version",
        "/server/protocol",
        "/server/running",
        "/server/socket",
    ] {
        assert!(
            status.pointer(pointer).is_some(),
            "herdr status --json lost {pointer}: {status}"
        );
    }
    assert_eq!(
        status.pointer("/server/running"),
        Some(&Value::Bool(true)),
        "the isolated Herdr server should be running"
    );
    matrix("status_json");

    let client_protocol = status
        .pointer("/client/protocol")
        .and_then(Value::as_u64)
        .unwrap();
    let server_protocol = status
        .pointer("/server/protocol")
        .and_then(Value::as_u64)
        .unwrap();
    assert_eq!(
        client_protocol, server_protocol,
        "client and server must speak the same protocol"
    );
    assert!(
        bootmux::herdr::SUPPORTED_PROTOCOLS.contains(&(server_protocol as u32)),
        "Herdr protocol {server_protocol} is outside bootmux's supported set {:?}",
        bootmux::herdr::SUPPORTED_PROTOCOLS
    );
    assert_eq!(
        status
            .pointer("/client/version")
            .and_then(Value::as_str)
            .map(|value| format!("herdr {value}")),
        Some(version.clone()),
        "status --json disagrees with herdr --version"
    );
    matrix("client_server_protocol");

    let workspace = harness.workspaces(&label).remove(0);
    let workspace_id = workspace
        .get("workspace_id")
        .and_then(Value::as_str)
        .unwrap()
        .to_string();
    assert_eq!(workspace.get("tab_count").and_then(Value::as_u64), Some(2));
    assert_eq!(workspace.get("pane_count").and_then(Value::as_u64), Some(4));
    assert_eq!(harness.panes_of(&workspace_id).len(), 4);
    matrix("create_topology");

    // pre_window reaches every pane, a pane's own commands run in order, and
    // each window's root is where its panes start.
    let pane_ids: Vec<String> = harness
        .panes_of(&workspace_id)
        .iter()
        .filter_map(|pane| pane.get("pane_id").and_then(Value::as_str))
        .map(str::to_string)
        .collect();
    let expected_output = [
        format!(
            "editor-cwd={}",
            app_root.file_name().unwrap().to_string_lossy()
        ),
        "server-ok".to_string(),
        "tail-ready".to_string(),
        "watcher-ready".to_string(),
    ];
    assert!(
        wait_until(|| {
            let combined = pane_ids
                .iter()
                .map(|pane_id| harness.herdr(&["pane", "read", pane_id]))
                .map(|output| String::from_utf8_lossy(&output.stdout).into_owned())
                .collect::<Vec<_>>()
                .join("\n");
            expected_output
                .iter()
                .all(|marker| combined.contains(marker.as_str()))
        }),
        "pane commands did not produce their markers"
    );
    matrix("root_and_commands");

    // startup_window selects the configured tab inside the workspace.
    let logs_tab_id = harness
        .tabs_of(&workspace_id)
        .into_iter()
        .find(|tab| tab.get("label").and_then(Value::as_str) == Some("logs"))
        .and_then(|tab| {
            tab.get("tab_id")
                .and_then(Value::as_str)
                .map(str::to_string)
        })
        .expect("the logs tab must exist");
    assert_eq!(
        workspace.get("active_tab_id").and_then(Value::as_str),
        Some(logs_tab_id.as_str()),
        "startup_window did not select the configured tab"
    );
    matrix("startup_focus");

    // Herdr focuses a workspace as it is created, so a focus that sits outside
    // it proves bootmux moved it back through the direct socket `pane.focus`
    // call that `--no-attach` promises.
    let focused_pane = harness.focused_pane_id();
    assert!(
        !focused_pane.is_empty(),
        "the server should report a focused pane"
    );
    assert!(
        !harness
            .panes_of(&workspace_id)
            .iter()
            .any(|pane| pane.get("pane_id").and_then(Value::as_str) == Some(focused_pane.as_str())),
        "a detached start must not leave the focus inside its own workspace"
    );
    assert_eq!(
        workspace.get("focused"),
        Some(&Value::Bool(false)),
        "a detached start must not focus its own workspace"
    );
    matrix("direct_pane_focus");

    let repeated = harness.bootmux(&harness.start_args());
    assert_success(&repeated, "repeated start");
    assert_outcome(&repeated, "reused", &label, "repeated start");
    assert_eq!(
        harness.workspaces(&label).len(),
        1,
        "repeated start must reuse one workspace"
    );
    assert_eq!(
        harness.panes_of(&workspace_id).len(),
        4,
        "reuse must not add panes"
    );
    matrix("reuse");

    // Two concurrent creations plus one reuse: the create ran once, the other
    // two starts took the restart path.
    let hooks = read_lines(&hooks_log);
    assert_eq!(
        hooks.iter().filter(|line| *line == "first_start").count(),
        1,
        "topology must be created exactly once: {hooks:?}"
    );
    assert_eq!(
        hooks.iter().filter(|line| *line == "start").count(),
        3,
        "every start must run on_project_start: {hooks:?}"
    );
    assert_eq!(
        hooks.iter().filter(|line| *line == "restart").count(),
        2,
        "reusing starts must run on_project_restart: {hooks:?}"
    );
    assert_eq!(
        hooks.iter().filter(|line| *line == "exit").count(),
        3,
        "every start must run on_project_exit: {hooks:?}"
    );
    assert_eq!(hooks.first().map(String::as_str), Some("start"));
    assert_eq!(hooks.get(1).map(String::as_str), Some("first_start"));
    matrix("lifecycle_hooks");

    let listed = harness.bootmux(&["--backend", "herdr", "list", "--active", "-n"]);
    assert_success(&listed, "list --active");
    let names = String::from_utf8_lossy(&listed.stdout).into_owned();
    assert_eq!(
        names
            .lines()
            .filter(|line| line.trim() == "project")
            .count(),
        1,
        "list --active must report the project exactly once: {names}"
    );
    matrix("active_listing");

    let appended = harness.bootmux_in_workspace(
        &[
            "--backend",
            "herdr",
            "start",
            "--project-config",
            &project_path,
            &label_setting,
            &socket_setting,
            "--append",
            "--no-attach",
        ],
        &workspace_id,
    );
    assert_success(&appended, "bootmux Herdr append");
    assert_outcome(&appended, "appended to", &label, "bootmux Herdr append");
    let workspace = harness.workspaces(&label).remove(0);
    assert_eq!(workspace.get("tab_count").and_then(Value::as_u64), Some(4));
    assert_eq!(workspace.get("pane_count").and_then(Value::as_u64), Some(8));
    assert_failure(
        &harness.bootmux(&[
            "--backend",
            "herdr",
            "start",
            "--project-config",
            &project_path,
            &label_setting,
            &socket_setting,
            "--append",
            "--no-attach",
        ]),
        "append outside a workspace",
    );
    matrix("append");

    // A stop that cannot prove the managed identity must be refused, and must
    // leave the ownership record and workspace untouched.
    let wrong_socket = format!("socket={}", harness.home.join("wrong.sock").display());
    assert_failure(
        &harness.bootmux(&[
            "--backend",
            "herdr",
            "stop",
            "--project-config",
            &project_path,
            &label_setting,
            &wrong_socket,
        ]),
        "stop with the wrong endpoint",
    );
    assert_failure(
        &harness.bootmux(&[
            "--backend",
            "herdr",
            "stop",
            "--project-config",
            &project_path,
            "label=wrong-template-value",
            &socket_setting,
        ]),
        "stop with the wrong rendered identity",
    );
    assert_eq!(
        harness.workspaces(&label).len(),
        1,
        "a refused stop must not remove the workspace"
    );
    assert!(
        !read_lines(&hooks_log).contains(&"stop".to_string()),
        "a refused stop must not run the stop hook"
    );
    matrix("ownership_rollback");

    let stopped = harness.bootmux(&[
        "--backend",
        "herdr",
        "stop",
        "--project-config",
        &project_path,
        &label_setting,
        &socket_setting,
    ]);
    assert_success(&stopped, "bootmux Herdr stop");
    assert_outcome(&stopped, "stopped", &label, "bootmux Herdr stop");
    assert_eq!(
        read_lines(&hooks_log)
            .iter()
            .filter(|line| *line == "stop")
            .count(),
        1,
        "the rendered stop hook must run exactly once"
    );
    assert!(
        harness.workspaces(&label).is_empty(),
        "stop must close the workspace"
    );
    let listed = harness.bootmux(&["--backend", "herdr", "list", "--active", "-n"]);
    assert_success(&listed, "list --active after stop");
    assert!(
        !String::from_utf8_lossy(&listed.stdout)
            .lines()
            .any(|line| line.trim() == "project"),
        "a stopped project must leave list --active"
    );
    matrix("explicit_stop");

    assert_success(
        &harness.bootmux(&harness.start_args()),
        "start before stop-all",
    );
    assert!(wait_until(|| harness.workspaces(&label).len() == 1));
    std::fs::remove_file(&harness.project).unwrap();
    assert_success(
        &harness.bootmux(&["--backend", "herdr", "stop-all", "-y"]),
        "bootmux Herdr stop-all",
    );
    assert!(
        harness.workspaces(&label).is_empty(),
        "stop-all must close the managed workspace"
    );
    assert_eq!(
        read_lines(&hooks_log)
            .iter()
            .filter(|line| *line == "stop")
            .count(),
        2,
        "stop-all must run the persisted rendered stop hook"
    );
    matrix("stop_all");

    // A window that cannot be laid out is rejected before anything is created.
    let broken = harness.projects.join("broken.yml");
    std::fs::write(
        &broken,
        format!(
            "name: broken-{nonce}\nroot: {}\nattach: false\nsocket_path: {}\nwindows:\n  \
             - broken:\n      layout: main-vertical\n      panes:\n        - only:\n            \
             split: right\n            ratio: 0.99\n",
            root.display(),
            harness.socket_path.display(),
        ),
    )
    .unwrap();
    assert_failure(
        &harness.bootmux(&[
            "--backend",
            "herdr",
            "start",
            "--project-config",
            broken.to_str().unwrap(),
            "--no-attach",
        ]),
        "start with an unrepresentable topology",
    );
    assert!(
        harness.workspaces(&format!("broken-{nonce}")).is_empty(),
        "a rejected project must not leave a workspace behind"
    );
    let listed = harness.bootmux(&["--backend", "herdr", "list", "--active", "-n"]);
    assert_success(&listed, "list --active after a rejected start");
    assert!(
        !String::from_utf8_lossy(&listed.stdout)
            .lines()
            .any(|line| line.trim() == "broken"),
        "a rejected project must not leave an ownership record"
    );
    matrix("failure_rollback");
}
