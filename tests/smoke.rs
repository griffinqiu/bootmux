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
    let live_root = config_dir.path().join("live-root");
    std::fs::create_dir(&live_root).unwrap();
    let project = format!(
        "name: smoke\nroot: {}\ntmux_options: -L {SOCKET}\nattach: false\nstartup_window: \"owner's logs\"\nwindows:\n  - editor:\n      layout: main-vertical\n      panes:\n        - echo pane0\n        - echo pane1\n  - \"owner's logs\": echo hello\n",
        live_root.display()
    );
    std::fs::write(config_dir.path().join("smoke.yml"), project).unwrap();
    std::fs::write(
        config_dir.path().join("broken.yml"),
        format!(
            "name: broken\nroot: /definitely/missing/bootmux-smoke\ntmux_options: -L {SOCKET}\nattach: false\nwindows:\n  - editor: echo unreachable\n"
        ),
    )
    .unwrap();

    let run = |args: &[&str]| {
        let mut cmd = Command::cargo_bin("bootmux").unwrap();
        cmd.env("TMUXINATOR_CONFIG", config_dir.path())
            .env_remove("TMUX")
            .arg("--backend")
            .arg("tmux")
            .args(args)
            .assert()
            .success();
    };

    run(&["start", "smoke"]);
    run(&["start", "smoke"]);

    let windows =
        String::from_utf8(tmux(&["list-windows", "-t", "smoke", "-F", "#W"]).stdout).unwrap();
    assert_eq!(
        windows.lines().collect::<Vec<_>>(),
        vec!["editor", "owner's logs"]
    );
    let active =
        String::from_utf8(tmux(&["display-message", "-p", "-t", "smoke", "#W"]).stdout).unwrap();
    assert_eq!(active.trim(), "owner's logs");

    let panes =
        String::from_utf8(tmux(&["list-panes", "-s", "-t", "smoke", "-F", "#W"]).stdout).unwrap();
    assert_eq!(panes.lines().filter(|w| *w == "editor").count(), 2);

    std::fs::remove_dir(&live_root).unwrap();
    run(&["stop", "smoke"]);

    let sessions = tmux(&["list-sessions"]);
    let listing = String::from_utf8(sessions.stdout).unwrap();
    assert!(
        !listing.contains("smoke:"),
        "session should be gone: {listing}"
    );

    let mut broken = Command::cargo_bin("bootmux").unwrap();
    broken
        .env("TMUXINATOR_CONFIG", config_dir.path())
        .env_remove("TMUX")
        .args(["--backend", "tmux", "start", "broken"])
        .assert()
        .failure();
    assert!(
        !tmux(&["has-session", "-t", "broken"]).status.success(),
        "a failed root change must not leave a partial session"
    );

    kill_server();
}
