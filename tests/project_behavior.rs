use std::collections::HashMap;
use std::path::PathBuf;

use bootmux::env::Env;
use bootmux::project::{LoadOptions, Project};
use bootmux::script;
use bootmux::tmux::MockTmux;

fn test_env() -> Env {
    Env {
        shell: Some("/bin/bash".to_string()),
        home: "/home/test".to_string(),
        cwd: PathBuf::from("/workdir"),
        ..Env::default()
    }
}

fn fixture(name: &str) -> String {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name);
    std::fs::read_to_string(path).unwrap()
}

fn load<'a>(content: &'a str, ctx: &'a MockTmux, env: &'a Env) -> anyhow::Result<Project<'a>> {
    Project::load(
        content,
        &HashMap::new(),
        &[],
        LoadOptions::default(),
        ctx,
        env,
    )
}

#[test]
fn parses_the_full_sample_config() {
    let env = test_env();
    let ctx = MockTmux::default();
    let content = fixture("sample.yml");
    let project = load(&content, &ctx, &env).unwrap();

    assert_eq!(project.name().unwrap(), "sample");
    assert_eq!(project.root().unwrap(), "/home/test/test");
    assert_eq!(project.tmux(), "tmux -f ~/.tmux.mac.conf -L foo");
    assert_eq!(project.windows().len(), 9);

    let editor = &project.windows()[0];
    assert_eq!(editor.name.as_deref(), Some("editor"));
    assert_eq!(editor.layout().as_deref(), Some("main-vertical"));
    assert_eq!(editor.panes.len(), 4);
    assert_eq!(editor.panes[0].commands, vec![Some("vim".to_string())]);
    assert!(editor.panes[1].commands.is_empty());
    let titled = &editor.panes[3];
    assert_eq!(titled.title.as_deref(), Some("pane_with_multiple_commands"));
    assert_eq!(
        titled.commands,
        vec![
            Some("ssh server".to_string()),
            Some("echo \"Hello\"".to_string())
        ]
    );

    let shell = &project.windows()[1];
    assert!(shell.panes.is_empty());
    assert_eq!(shell.commands(&project).len(), 2);

    let capistrano = &project.windows()[7];
    assert_eq!(capistrano.name.as_deref(), Some("capistrano"));
    assert!(capistrano.panes.is_empty());
    assert!(capistrano.commands(&project).is_empty());
}

#[test]
fn resolves_yaml_aliases_and_merge_keys() {
    let env = test_env();
    let ctx = MockTmux::default();
    let content = fixture("sample_alias.yml");
    let project = load(&content, &ctx, &env).unwrap();

    let editor = &project.windows()[0];
    assert_eq!(editor.pre().as_deref(), Some("echo \"alias_is_working\""));
}

#[test]
fn stringifies_odd_window_names_and_project_names() {
    let env = test_env();
    let ctx = MockTmux::default();

    let literals = fixture("sample_literals_as_window_name.yml");
    let project = load(&literals, &ctx, &env).unwrap();
    let names: Vec<Option<String>> = project
        .windows()
        .iter()
        .map(|window| window.name.clone())
        .collect();
    assert_eq!(names[0].as_deref(), Some("222"));
    assert_eq!(names[6].as_deref(), Some("true"));
    assert_eq!(names[7].as_deref(), Some("false"));
    assert_eq!(names[8].as_deref(), Some("nil"));

    let numeric = fixture("sample_number_as_name.yml");
    let project = load(&numeric, &ctx, &env).unwrap();
    assert_eq!(project.name().unwrap(), "222");

    let emoji = fixture("sample_emoji_as_name.yml");
    let project = load(&emoji, &ctx, &env).unwrap();
    assert_eq!(project.name().unwrap(), "\\🍩");
    assert_eq!(project.unescaped_name().unwrap(), "🍩");

    let nameless = fixture("nameless_window.yml");
    let project = load(&nameless, &ctx, &env).unwrap();
    assert_eq!(project.windows()[0].name, None);
    assert_eq!(project.windows()[1].name.as_deref(), Some("other"));
}

#[test]
fn rejects_projects_without_name_or_windows() {
    let env = test_env();
    let ctx = MockTmux::default();

    let err = load(&fixture("noname.yml"), &ctx, &env)
        .unwrap_err()
        .to_string();
    assert_eq!(err, "Your project file didn't specify a 'project_name'");

    let err = load(&fixture("nowindows.yml"), &ctx, &env)
        .unwrap_err()
        .to_string();
    assert_eq!(err, "Your project file should include some windows.");
}

#[test]
fn rejects_deprecated_tmuxinator_options_with_migration_hints() {
    let env = test_env();
    let ctx = MockTmux::default();

    let err = load(&fixture("detach.yml"), &ctx, &env)
        .unwrap_err()
        .to_string();
    assert!(err.contains("`pre` option"));
    assert!(err.contains("on_project_start"));

    for (config, expected_hint) in [
        ("name: x\nrbenv: 2.0.0\nwindows:\n  - a: b\n", "pre_window"),
        ("name: x\ntabs:\n  - a: b\n", "windows"),
        (
            "name: x\ncli_args: -f x\nwindows:\n  - a: b\n",
            "tmux_options",
        ),
        (
            "name: x\npost: cmd\nwindows:\n  - a: b\n",
            "on_project_stop",
        ),
    ] {
        let err = load(config, &ctx, &env).unwrap_err().to_string();
        assert!(err.contains(expected_hint), "{err}");
    }

    let err = load(
        "name: x\ntmux_command: wemux\nwindows:\n  - a: b\n",
        &ctx,
        &env,
    )
    .unwrap_err()
    .to_string();
    assert_eq!(err, "wemux is not supported by bootmux.");
}

#[test]
fn respects_attach_false_and_force_flags() {
    let env = test_env();
    let ctx = MockTmux::default();
    let config = "name: detached\nattach: false\nwindows:\n  - a: b\n";

    let project = load(config, &ctx, &env).unwrap();
    assert!(!project.attach());
    let rendered = script::render_start(&project);
    assert!(!rendered.contains("attach-session"));

    let forced = Project::load(
        config,
        &HashMap::new(),
        &[],
        LoadOptions {
            force_attach: true,
            ..Default::default()
        },
        &ctx,
        &env,
    )
    .unwrap();
    assert!(forced.attach());

    let err = Project::load(
        config,
        &HashMap::new(),
        &[],
        LoadOptions {
            force_attach: true,
            force_detach: true,
            ..Default::default()
        },
        &ctx,
        &env,
    )
    .unwrap_err()
    .to_string();
    assert_eq!(err, "Cannot force_attach and force_detach at the same time");
}

#[test]
fn renders_hooks_and_synchronize_and_startup_selection() {
    let env = test_env();
    let ctx = MockTmux::default();
    let config = r#"
name: hooked
startup_window: logs
startup_pane: 1
on_project_start: echo start
on_project_first_start:
  - echo one
  - echo two
windows:
  - editor:
      synchronize: after
      panes:
        - vim
        - top
  - logs: tail -f log
"#;
    let project = load(config, &ctx, &env).unwrap();
    let rendered = script::render_start(&project);

    assert!(rendered.contains("# Run on_project_start command.\necho start\n"));
    assert!(rendered.contains("  echo one; echo two\n"));
    assert!(rendered.contains("set-window-option -t hooked:0 synchronize-panes on"));
    assert!(rendered.contains("select-window -t hooked:logs"));
    assert!(rendered.contains("select-pane -t hooked:logs.1"));
}

#[test]
fn append_mode_skips_session_creation_and_offsets_windows() {
    let env = test_env();
    let ctx = MockTmux {
        session_exists: true,
        last_window_index: 3,
        current_session: "existing".to_string(),
        ..Default::default()
    };
    let config = "name: ignored\nwindows:\n  - extra: echo hi\n";
    let project = Project::load(
        config,
        &HashMap::new(),
        &[],
        LoadOptions {
            append: true,
            ..Default::default()
        },
        &ctx,
        &env,
    )
    .unwrap();

    assert_eq!(project.name().unwrap(), "existing");
    assert_eq!(project.base_index(), 4);

    let rendered = script::render_start(&project);
    assert!(!rendered.contains("start-server"));
    assert!(!rendered.contains("new-session"));
    assert!(!rendered.contains("attach-session"));
    assert!(rendered.contains("new-window  -k -t existing:4 -n extra"));
}

#[test]
fn append_to_missing_session_is_rejected() {
    let env = test_env();
    let ctx = MockTmux {
        session_exists: false,
        current_session: "existing".to_string(),
        ..Default::default()
    };
    let err = Project::load(
        "name: x\nwindows:\n  - a: b\n",
        &HashMap::new(),
        &[],
        LoadOptions {
            append: true,
            ..Default::default()
        },
        &ctx,
        &env,
    )
    .unwrap_err()
    .to_string();
    assert_eq!(err, "Cannot append to a session that does not exist");
}

#[test]
fn renders_restart_branch_when_session_exists() {
    let env = test_env();
    let ctx = MockTmux {
        session_exists: true,
        ..Default::default()
    };
    let config = "name: running\non_project_restart: echo back\nwindows:\n  - a: b\n";
    let project = load(config, &ctx, &env).unwrap();
    let rendered = script::render_start(&project);

    assert!(!rendered.contains("new-session"));
    assert!(rendered.contains("  # Run on_project_restart command.\n  echo back\n"));
    assert!(rendered.contains("attach-session -t running"));
}

#[test]
fn renders_stop_script_with_hook() {
    let env = test_env();
    let ctx = MockTmux {
        session_exists: true,
        ..Default::default()
    };
    let config =
        "name: doomed\nroot: /workspace/doomed\non_project_stop: echo bye\nwindows:\n  - a: b\n";
    let project = load(config, &ctx, &env).unwrap();

    let rendered = script::render_stop(&project);
    let expected = "#!/bin/bash\n\n  cd /workspace/doomed\n\n  # Run on_project_stop command\n  echo bye\n\n  tmux kill-session -t doomed\n";
    assert_eq!(rendered, expected);

    let gone = MockTmux::default();
    let project = load(config, &gone, &env).unwrap();
    assert_eq!(script::render_stop(&project), "#!/bin/bash\n\n");
}

#[test]
fn focused_pane_resolves_by_index_and_title() {
    let env = test_env();
    let ctx = MockTmux::default();
    let by_title = r#"
name: focus
windows:
  - editor:
      focused_pane: logs
      panes:
        - editor: vim
        - logs: tail -f x
"#;
    let project = load(by_title, &ctx, &env).unwrap();
    let rendered = script::render_start(&project);
    assert!(rendered.contains("  tmux select-pane -t focus:0.1\n\n\n  tmux select-window"));

    let out_of_range = r#"
name: focus
windows:
  - editor:
      focused_pane: 7
      panes:
        - vim
        - top
"#;
    let project = load(out_of_range, &ctx, &env).unwrap();
    let rendered = script::render_start(&project);
    assert!(rendered.contains("  tmux select-pane -t focus:0.0\n\n\n  tmux select-window"));
}

#[test]
fn sanitizes_session_names_with_separators() {
    let env = test_env();
    let ctx = MockTmux::default();
    let project = load("name: my.project:x\nwindows:\n  - a: b\n", &ctx, &env).unwrap();
    assert_eq!(project.name().unwrap(), "my_project_x");
}

#[test]
fn window_pre_runs_before_pane_commands() {
    let env = test_env();
    let ctx = MockTmux::default();
    let config = r#"
name: prewin
pre_window: rbenv shell 2.0.0
windows:
  - editor:
      pre:
        - echo a
        - echo b
      panes:
        - vim
"#;
    let project = load(config, &ctx, &env).unwrap();
    let rendered = script::render_start(&project);
    assert!(rendered.contains("send-keys -t prewin:0.0 rbenv\\ shell\\ 2.0.0 C-m"));
    assert!(rendered.contains("send-keys -t prewin:0.0 echo\\ a\\ \\&\\&\\ echo\\ b C-m"));
}
