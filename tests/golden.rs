use std::collections::HashMap;
use std::path::PathBuf;

use bootmux::env::Env;
use bootmux::project::{LoadOptions, Project};
use bootmux::script;
use bootmux::settings::Backend;
use bootmux::spec::ProjectSpec;
use bootmux::tmux::MockTmux;
use bootmux::zellij_layout;

fn golden_env() -> Env {
    Env {
        shell: Some("/bin/bash".to_string()),
        home: "/home/test".to_string(),
        cwd: PathBuf::from("/workdir"),
        ..Env::default()
    }
}

// Contract from tmuxinator's debug_snapshot_spec.rb: snapshot files are
// written with exactly one trailing newline stripped, so the rendered
// output must equal the snapshot content plus one "\n".
fn assert_matches_snapshot(fixture: &str, snapshot: &str) {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let fixture_content =
        std::fs::read_to_string(root.join("tests/fixtures").join(fixture)).unwrap();
    let snapshot_content =
        std::fs::read_to_string(root.join("tests/snapshots/2.6").join(snapshot)).unwrap();

    let env = golden_env();
    let ctx = MockTmux::default();
    let project = Project::load(
        &fixture_content,
        &HashMap::new(),
        &[],
        LoadOptions::default(),
        &ctx,
        &env,
    )
    .unwrap();

    let rendered = script::render_start(&project);
    let expected = format!("{snapshot_content}\n");

    if rendered != expected {
        for (line_number, (got, want)) in rendered.lines().zip(expected.lines()).enumerate() {
            if got != want {
                panic!(
                    "snapshot mismatch for {snapshot} at line {}:\n  got:  {got:?}\n  want: {want:?}",
                    line_number + 1
                );
            }
        }
        panic!(
            "snapshot length mismatch for {snapshot}: got {} lines, want {} lines\n--- got ---\n{rendered}",
            rendered.lines().count(),
            expected.lines().count()
        );
    }
}

/// The zellij backend's equivalent of the tmux script snapshots: the KDL
/// document is the artifact bootmux hands to zellij, so it is pinned the same
/// way. Snapshot files hold the rendering minus its final newline.
fn assert_matches_zellij_snapshot(fixture: &str, snapshot: &str) {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let fixture_path = root.join("tests/fixtures").join(fixture);
    let fixture_content = std::fs::read_to_string(&fixture_path).unwrap();

    let spec = ProjectSpec::load(
        &fixture_path,
        &fixture_content,
        &HashMap::new(),
        &[],
        LoadOptions::default(),
        &golden_env(),
        Backend::Zellij,
    )
    .unwrap();
    let rendered = zellij_layout::render_project(&spec).unwrap();

    let snapshot_path = root.join("tests/snapshots/zellij").join(snapshot);
    let expected = format!("{}\n", std::fs::read_to_string(&snapshot_path).unwrap());
    assert_eq!(
        rendered, expected,
        "zellij layout snapshot mismatch for {snapshot}"
    );
}

#[test]
fn zellij_basic_layout_snapshot() {
    assert_matches_zellij_snapshot("basic.yml", "basic.kdl");
}

#[test]
fn zellij_pane_titles_layout_snapshot() {
    assert_matches_zellij_snapshot("pane_titles.yml", "pane_titles.kdl");
}

#[test]
fn zellij_session_name_layout_snapshot() {
    assert_matches_zellij_snapshot("session_name.yml", "session_name.kdl");
}

#[test]
fn matches_tmuxinator_basic_snapshot() {
    assert_matches_snapshot("basic.yml", "basic.sh");
}

#[test]
fn matches_tmuxinator_pane_titles_snapshot() {
    assert_matches_snapshot("pane_titles.yml", "pane_titles.sh");
}

#[test]
fn matches_tmuxinator_session_name_snapshot() {
    assert_matches_snapshot("session_name.yml", "session_name.sh");
}
