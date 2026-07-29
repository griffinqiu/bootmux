use std::collections::HashMap;
use std::path::PathBuf;

use bootmux::env::Env;
use bootmux::project::{LoadOptions, Project};
use bootmux::script;
use bootmux::tmux::MockTmux;

// Contract from tmuxinator's debug_snapshot_spec.rb: snapshot files are
// written with exactly one trailing newline stripped, so the rendered
// output must equal the snapshot content plus one "\n".
fn assert_matches_snapshot(fixture: &str, snapshot: &str) {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let fixture_content =
        std::fs::read_to_string(root.join("tests/fixtures").join(fixture)).unwrap();
    let snapshot_content =
        std::fs::read_to_string(root.join("tests/snapshots/2.6").join(snapshot)).unwrap();

    let env = Env {
        shell: Some("/bin/bash".to_string()),
        home: "/home/test".to_string(),
        cwd: PathBuf::from("/workdir"),
        ..Env::default()
    };
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
