#![cfg(unix)]

use std::path::PathBuf;
use std::process::{Command, Output};
use std::sync::{Arc, Barrier};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde_json::Value;
use tempfile::TempDir;

struct Cleanup {
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
    label_setting: String,
    socket_setting: String,
}

impl Cleanup {
    fn command_env(&self, command: &mut Command) {
        command
            .env("HERDR_CONFIG_PATH", &self.config_path)
            .env("HERDR_SOCKET_PATH", &self.socket_path)
            .env("HOME", &self.home)
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
}

impl Drop for Cleanup {
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

fn workspace_snapshot(cleanup: &Cleanup) -> Value {
    let output = cleanup.herdr(&["api", "snapshot"]);
    assert_success(&output, "Herdr snapshot");
    serde_json::from_slice(&output.stdout).unwrap()
}

#[test]
#[ignore = "requires a local Herdr >= 0.7.5 (protocol 17)"]
fn creates_reuses_and_stops_a_real_herdr_workspace() {
    let herdr = match std::env::var_os("HERDR_BIN") {
        Some(path) => PathBuf::from(path),
        None => PathBuf::from("herdr"),
    };
    let version = Command::new(&herdr).arg("--version").output();
    if !version
        .map(|output| output.status.success())
        .unwrap_or(false)
    {
        eprintln!("skipping: Herdr is not installed");
        return;
    }

    let temp = TempDir::new().unwrap();
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let label = format!("bootmux-smoke-{}-{nonce}", std::process::id());
    let root = temp.path().join("work");
    let app_root = root.join("app");
    let home = temp.path().join("home");
    let config_home = temp.path().join("config");
    let data_home = temp.path().join("data");
    let state_home = temp.path().join("state");
    let cache_home = temp.path().join("cache");
    std::fs::create_dir_all(&app_root).unwrap();
    std::fs::create_dir_all(&home).unwrap();
    std::fs::create_dir_all(&state_home).unwrap();
    let project = temp.path().join("project.yml");
    let stop_marker = temp.path().join("stop-hook-ran");
    let socket_path = temp.path().join("herdr.sock");
    let config_path = temp.path().join("herdr-config.toml");
    std::fs::write(
        &project,
        format!(
            r#"name: <%= @settings["label"] %>
root: {}
socket_path: <%= @settings["socket"] %>
attach: false
on_project_stop: test '<%= @settings["label"] %>' = '{label}' && touch {}
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
            root.display(),
            bootmux::shellwords::escape(&stop_marker.to_string_lossy())
        ),
    )
    .unwrap();

    let socket_setting = format!("socket={}", socket_path.display());
    let cleanup = Cleanup {
        bootmux: PathBuf::from(env!("CARGO_BIN_EXE_bootmux")),
        herdr,
        project,
        home,
        config_home,
        data_home,
        state_home,
        cache_home,
        config_path,
        socket_path,
        label_setting: format!("label={label}"),
        socket_setting,
    };
    let project_path = cleanup.project.to_str().unwrap();
    let label_setting = cleanup.label_setting.as_str();
    let socket_setting = cleanup.socket_setting.as_str();
    let start_args = [
        "--backend",
        "herdr",
        "start",
        "--project-config",
        project_path,
        label_setting,
        socket_setting,
        "--no-attach",
    ];
    let barrier = Arc::new(Barrier::new(3));
    let concurrent_outputs = thread::scope(|scope| {
        let workers = (0..2)
            .map(|_| {
                let barrier = barrier.clone();
                let cleanup = &cleanup;
                let start_args = &start_args;
                scope.spawn(move || {
                    barrier.wait();
                    cleanup.bootmux(start_args)
                })
            })
            .collect::<Vec<_>>();
        barrier.wait();
        workers
            .into_iter()
            .map(|worker| worker.join().unwrap())
            .collect::<Vec<_>>()
    });
    for output in &concurrent_outputs {
        assert_success(output, "concurrent bootmux Herdr start");
    }

    let output = cleanup.bootmux(&[
        "--backend",
        "herdr",
        "start",
        "--project-config",
        project_path,
        label_setting,
        socket_setting,
        "--no-attach",
    ]);
    assert_success(&output, "repeated bootmux Herdr start");

    let snapshot = workspace_snapshot(&cleanup);
    let workspaces = snapshot
        .pointer("/result/snapshot/workspaces")
        .and_then(Value::as_array)
        .unwrap();
    let matching: Vec<_> = workspaces
        .iter()
        .filter(|workspace| workspace.get("label").and_then(Value::as_str) == Some(&label))
        .collect();
    assert_eq!(matching.len(), 1, "repeated start must reuse one workspace");
    let workspace_id = matching[0]
        .get("workspace_id")
        .and_then(Value::as_str)
        .unwrap()
        .to_string();
    assert_eq!(
        matching[0].get("tab_count").and_then(Value::as_u64),
        Some(2)
    );
    assert_eq!(
        matching[0].get("pane_count").and_then(Value::as_u64),
        Some(4)
    );

    let pane_ids: Vec<String> = snapshot
        .pointer("/result/snapshot/panes")
        .and_then(Value::as_array)
        .unwrap()
        .iter()
        .filter(|pane| {
            pane.get("workspace_id").and_then(Value::as_str) == Some(workspace_id.as_str())
        })
        .filter_map(|pane| pane.get("pane_id").and_then(Value::as_str))
        .map(str::to_string)
        .collect();
    assert_eq!(pane_ids.len(), 4);

    let deadline = Instant::now() + Duration::from_secs(10);
    let expected = [
        format!(
            "editor-cwd={}",
            app_root.file_name().unwrap().to_string_lossy()
        ),
        "server-ok".to_string(),
        "tail-ready".to_string(),
        "watcher-ready".to_string(),
    ];
    loop {
        let combined = pane_ids
            .iter()
            .map(|pane_id| cleanup.herdr(&["pane", "read", pane_id]))
            .map(|output| String::from_utf8_lossy(&output.stdout).into_owned())
            .collect::<Vec<_>>()
            .join("\n");
        if expected
            .iter()
            .all(|marker| combined.contains(marker.as_str()))
        {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "pane commands did not finish before timeout:\n{combined}"
        );
        thread::sleep(Duration::from_millis(100));
    }

    let output = cleanup.bootmux_in_workspace(
        &[
            "--backend",
            "herdr",
            "start",
            "--project-config",
            project_path,
            label_setting,
            socket_setting,
            "--append",
            "--no-attach",
        ],
        &workspace_id,
    );
    assert_success(&output, "bootmux Herdr append");
    let snapshot = workspace_snapshot(&cleanup);
    let appended = snapshot
        .pointer("/result/snapshot/workspaces")
        .and_then(Value::as_array)
        .unwrap()
        .iter()
        .find(|workspace| workspace.get("label").and_then(Value::as_str) == Some(&label))
        .unwrap();
    assert_eq!(appended.get("tab_count").and_then(Value::as_u64), Some(4));
    assert_eq!(appended.get("pane_count").and_then(Value::as_u64), Some(8));

    let wrong_socket_setting = format!("socket={}", cleanup.home.join("wrong.sock").display());
    let output = cleanup.bootmux(&[
        "--backend",
        "herdr",
        "stop",
        "--project-config",
        project_path,
        label_setting,
        &wrong_socket_setting,
    ]);
    assert!(
        !output.status.success(),
        "wrong socket settings unexpectedly produced a successful stop"
    );

    let output = cleanup.bootmux(&[
        "--backend",
        "herdr",
        "stop",
        "--project-config",
        project_path,
        "label=wrong-template-value",
        socket_setting,
    ]);
    assert!(
        !output.status.success(),
        "wrong template identity unexpectedly stopped the workspace"
    );
    let snapshot = workspace_snapshot(&cleanup);
    assert!(snapshot
        .pointer("/result/snapshot/workspaces")
        .and_then(Value::as_array)
        .unwrap()
        .iter()
        .any(|workspace| workspace.get("label").and_then(Value::as_str) == Some(&label)));

    let output = cleanup.bootmux(&[
        "--backend",
        "herdr",
        "stop",
        "--project-config",
        project_path,
        label_setting,
        socket_setting,
    ]);
    assert_success(&output, "bootmux Herdr stop");
    assert!(stop_marker.is_file(), "rendered stop hook did not run");
    let snapshot = workspace_snapshot(&cleanup);
    assert!(snapshot
        .pointer("/result/snapshot/workspaces")
        .and_then(Value::as_array)
        .unwrap()
        .iter()
        .all(|workspace| workspace.get("label").and_then(Value::as_str) != Some(&label)));

    std::fs::remove_file(&stop_marker).unwrap();
    let output = cleanup.bootmux(&[
        "--backend",
        "herdr",
        "start",
        "--project-config",
        project_path,
        label_setting,
        socket_setting,
        "--no-attach",
    ]);
    assert_success(&output, "second bootmux Herdr start");
    std::fs::remove_file(&cleanup.project).unwrap();
    let output = cleanup.bootmux(&["--backend", "herdr", "stop-all", "-y"]);
    assert_success(&output, "bootmux Herdr stop-all");
    assert!(
        stop_marker.is_file(),
        "stop-all did not use the persisted rendered stop hook"
    );
    let snapshot = workspace_snapshot(&cleanup);
    assert!(snapshot
        .pointer("/result/snapshot/workspaces")
        .and_then(Value::as_array)
        .unwrap()
        .iter()
        .all(|workspace| workspace.get("label").and_then(Value::as_str) != Some(&label)));
}
