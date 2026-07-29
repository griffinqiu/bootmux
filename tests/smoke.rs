// End-to-end smoke test against a real tmux server on an isolated socket.
// Run explicitly with: cargo test --test smoke -- --ignored

use assert_cmd::Command;
use tempfile::TempDir;

const SOCKET: &str = "bootmux-smoke";

fn tmux(args: &[&str]) -> std::process::Output {
    std::process::Command::new("tmux")
        .arg("-L")
        .arg(SOCKET)
        .args(args)
        .output()
        .unwrap()
}

fn kill_server() {
    tmux(&["kill-server"]);
}

#[test]
#[ignore]
fn starts_and_stops_a_real_session() {
    kill_server();

    let config_dir = TempDir::new().unwrap();
    let project = format!(
        "name: smoke\nroot: /tmp\ntmux_options: -L {SOCKET}\nattach: false\nwindows:\n  - editor:\n      layout: main-vertical\n      panes:\n        - echo pane0\n        - echo pane1\n  - shell: echo hello\n"
    );
    std::fs::write(config_dir.path().join("smoke.yml"), project).unwrap();

    let run = |args: &[&str]| {
        let mut cmd = Command::cargo_bin("bootmux").unwrap();
        cmd.env("TMUXINATOR_CONFIG", config_dir.path())
            .env_remove("TMUX")
            .args(args)
            .assert()
            .success();
    };

    run(&["start", "smoke"]);

    let windows =
        String::from_utf8(tmux(&["list-windows", "-t", "smoke", "-F", "#W"]).stdout).unwrap();
    assert_eq!(windows.lines().collect::<Vec<_>>(), vec!["editor", "shell"]);

    let panes =
        String::from_utf8(tmux(&["list-panes", "-s", "-t", "smoke", "-F", "#W"]).stdout).unwrap();
    assert_eq!(panes.lines().filter(|w| *w == "editor").count(), 2);

    run(&["stop", "smoke"]);

    let sessions = tmux(&["list-sessions"]);
    let listing = String::from_utf8(sessions.stdout).unwrap();
    assert!(
        !listing.contains("smoke:"),
        "session should be gone: {listing}"
    );

    kill_server();
}
