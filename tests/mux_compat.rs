use std::collections::HashMap;
use std::path::PathBuf;
use std::process::Command;

use bootmux::commands::run::{debug_with_backend, StartParams};
use bootmux::env::Env;
use bootmux::project::{LoadOptions, Project};
use bootmux::settings::Backend;
use bootmux::spec::ProjectSpec;
use bootmux::tmux::MockTmux;

struct FixtureCase {
    file: &'static str,
    settings: &'static [(&'static str, &'static str)],
    tmux_error: Option<&'static str>,
    herdr_error: Option<&'static str>,
    zellij_error: Option<&'static str>,
}

const NO_SETTINGS: &[(&str, &str)] = &[];
const TEMPLATE_SETTINGS: &[(&str, &str)] = &[
    ("root", "/tmp/bootmux-mux-template"),
    ("host", "localhost"),
    ("port", "3000"),
];

const CASES: &[FixtureCase] = &[
    FixtureCase {
        file: "detach.yml",
        settings: NO_SETTINGS,
        tmux_error: None,
        herdr_error: None,
        zellij_error: None,
    },
    FixtureCase {
        file: "focused_pane.yml",
        settings: NO_SETTINGS,
        tmux_error: None,
        herdr_error: None,
        zellij_error: None,
    },
    FixtureCase {
        file: "hooks.yml",
        settings: NO_SETTINGS,
        tmux_error: None,
        herdr_error: None,
        zellij_error: None,
    },
    FixtureCase {
        file: "nameless_window.yml",
        settings: NO_SETTINGS,
        tmux_error: None,
        herdr_error: None,
        zellij_error: None,
    },
    FixtureCase {
        file: "noroot.yml",
        settings: NO_SETTINGS,
        tmux_error: None,
        herdr_error: None,
        zellij_error: None,
    },
    FixtureCase {
        file: "nowindows.yml",
        settings: NO_SETTINGS,
        tmux_error: Some("Your project file should include some windows."),
        herdr_error: Some("Your project file should include some windows."),
        zellij_error: Some("Your project file should include some windows."),
    },
    FixtureCase {
        file: "pane_titles.yml",
        settings: NO_SETTINGS,
        tmux_error: None,
        herdr_error: None,
        zellij_error: None,
    },
    FixtureCase {
        file: "sample.yml",
        settings: NO_SETTINGS,
        tmux_error: None,
        herdr_error: None,
        zellij_error: None,
    },
    FixtureCase {
        file: "sample_deprecations.yml",
        settings: NO_SETTINGS,
        tmux_error: None,
        herdr_error: None,
        zellij_error: None,
    },
    FixtureCase {
        file: "sample_emoji_as_name.yml",
        settings: NO_SETTINGS,
        tmux_error: None,
        herdr_error: None,
        zellij_error: None,
    },
    FixtureCase {
        file: "sample_literals_as_window_name.yml",
        settings: NO_SETTINGS,
        tmux_error: None,
        herdr_error: None,
        zellij_error: None,
    },
    FixtureCase {
        file: "sample_number_as_name.yml",
        settings: NO_SETTINGS,
        tmux_error: None,
        herdr_error: None,
        zellij_error: None,
    },
    FixtureCase {
        file: "sample_wemux.yml",
        settings: NO_SETTINGS,
        tmux_error: None,
        herdr_error: None,
        zellij_error: None,
    },
    FixtureCase {
        file: "socket.yml",
        settings: NO_SETTINGS,
        tmux_error: None,
        herdr_error: None,
        zellij_error: None,
    },
    FixtureCase {
        file: "startup.yml",
        settings: NO_SETTINGS,
        tmux_error: None,
        herdr_error: None,
        zellij_error: None,
    },
    FixtureCase {
        file: "synchronize.yml",
        settings: NO_SETTINGS,
        tmux_error: None,
        herdr_error: Some("`synchronize` is not supported by the Herdr backend"),
        zellij_error: None,
    },
    FixtureCase {
        file: "template.yml",
        settings: TEMPLATE_SETTINGS,
        tmux_error: None,
        herdr_error: None,
        zellij_error: None,
    },
    FixtureCase {
        file: "window_root.yml",
        settings: NO_SETTINGS,
        tmux_error: None,
        herdr_error: None,
        zellij_error: None,
    },
    FixtureCase {
        file: "demo.yml",
        settings: NO_SETTINGS,
        tmux_error: None,
        herdr_error: None,
        zellij_error: None,
    },
];

fn test_env() -> Env {
    Env {
        shell: Some("/bin/bash".to_string()),
        home: "/home/test".to_string(),
        cwd: PathBuf::from("/workdir"),
        ..Env::default()
    }
}

fn fixture_path(file: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/mux")
        .join(file)
}

fn fixture(file: &str) -> String {
    std::fs::read_to_string(fixture_path(file)).unwrap()
}

fn settings(case: &FixtureCase) -> HashMap<String, String> {
    case.settings
        .iter()
        .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
        .collect()
}

fn params(case: &FixtureCase) -> StartParams {
    StartParams {
        project: None,
        args: case
            .settings
            .iter()
            .map(|(key, value)| format!("{key}={value}"))
            .collect(),
        attach: None,
        custom_name: None,
        project_config: Some(fixture_path(case.file).to_string_lossy().into_owned()),
        append: false,
        no_pre_window: false,
    }
}

fn assert_backend_case(case: &FixtureCase, backend: Backend) {
    let expected_error = match backend {
        Backend::Tmux => case.tmux_error,
        Backend::Herdr => case.herdr_error,
        Backend::Zellij => case.zellij_error,
    };
    let result = debug_with_backend(&test_env(), &MockTmux::default(), backend, params(case));

    match (result, expected_error) {
        (Ok(()), None) => {}
        (Ok(()), Some(expected)) => {
            panic!(
                "{} unexpectedly accepted {} (expected error containing {expected:?})",
                backend, case.file
            );
        }
        (Err(error), None) => {
            panic!("{} unexpectedly rejected {}: {error:#}", backend, case.file);
        }
        (Err(error), Some(expected)) => {
            let message = format!("{error:#}");
            assert!(
                message.contains(expected),
                "{} rejected {}, but {message:?} did not contain {expected:?}",
                backend,
                case.file
            );
        }
    }
}

#[test]
fn tmux_debug_matches_the_vendored_mux_fixture_matrix() {
    assert_eq!(CASES.len(), 19);
    for case in CASES {
        assert_backend_case(case, Backend::Tmux);
    }
}

#[test]
fn herdr_debug_matches_the_vendored_mux_fixture_matrix() {
    assert_eq!(CASES.len(), 19);
    for case in CASES {
        assert_backend_case(case, Backend::Herdr);
    }
}

#[test]
fn zellij_debug_matches_the_vendored_mux_fixture_matrix() {
    assert_eq!(CASES.len(), 19);
    for case in CASES {
        assert_backend_case(case, Backend::Zellij);
    }
}

#[test]
fn vendored_mux_fixture_matrix_is_exhaustive() {
    let fixture_dir = fixture_path("README.md").parent().unwrap().to_path_buf();
    let mut actual = std::fs::read_dir(fixture_dir)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|name| name.ends_with(".yml"))
        .collect::<Vec<_>>();
    let mut expected = CASES
        .iter()
        .map(|case| case.file.to_owned())
        .collect::<Vec<_>>();
    actual.sort();
    expected.sort();
    assert_eq!(actual, expected);
}

#[test]
fn tmux_scripts_for_mux_fixtures_are_valid_sh() {
    let env = test_env();
    let tmux = MockTmux::default();
    for case in CASES.iter().filter(|case| case.tmux_error.is_none()) {
        let project = Project::load(
            &fixture(case.file),
            &settings(case),
            &[],
            LoadOptions::default(),
            &tmux,
            &env,
        )
        .unwrap_or_else(|error| panic!("failed to load {}: {error:#}", case.file));
        let script = bootmux::script::render_start(&project);
        let output = Command::new("/bin/sh")
            .args(["-n", "-c", &script])
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{} generated invalid /bin/sh syntax: {}",
            case.file,
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

#[test]
fn mux_fixture_semantics_are_preserved_across_backends() {
    let env = test_env();
    let tmux = MockTmux::default();

    let deprecated = Project::load(
        &fixture("sample_deprecations.yml"),
        &HashMap::new(),
        &[],
        LoadOptions::default(),
        &tmux,
        &env,
    )
    .unwrap();
    assert_eq!(deprecated.name().as_deref(), Some("sample"));
    assert_eq!(deprecated.root().as_deref(), Some("/home/test/test"));
    assert_eq!(deprecated.tmux(), "tmux -f ~/.tmux.mac.conf -L foo");
    assert_eq!(deprecated.windows().len(), 9);

    let template_case = CASES
        .iter()
        .find(|case| case.file == "template.yml")
        .unwrap();
    let template = Project::load(
        &fixture(template_case.file),
        &settings(template_case),
        &[],
        LoadOptions::default(),
        &tmux,
        &env,
    )
    .unwrap();
    assert_eq!(
        template.root().as_deref(),
        Some("/tmp/bootmux-mux-template")
    );
    assert!(template.windows()[0]
        .commands(&template)
        .iter()
        .any(|command| command.contains("host\\=localhost\\ port\\=3000")));

    let running_tmux = MockTmux {
        session_exists: true,
        ..Default::default()
    };
    let template_for_stop = Project::load(
        &fixture(template_case.file),
        &HashMap::new(),
        &[],
        LoadOptions::default(),
        &running_tmux,
        &env,
    )
    .unwrap();
    assert!(
        bootmux::script::render_stop(&template_for_stop).contains("kill-session -t template_test")
    );
    let herdr_template_for_stop = ProjectSpec::load(
        fixture_path(template_case.file),
        &fixture(template_case.file),
        &HashMap::new(),
        &[],
        LoadOptions::default(),
        &env,
        Backend::Herdr,
    )
    .unwrap();
    assert_eq!(herdr_template_for_stop.name, "template_test");

    let literals = Project::load(
        &fixture("sample_literals_as_window_name.yml"),
        &HashMap::new(),
        &[],
        LoadOptions::default(),
        &tmux,
        &env,
    )
    .unwrap();
    assert_eq!(
        literals.windows()[2].name.as_deref(),
        Some("111222333444555666777")
    );

    let wemux = Project::load(
        &fixture("sample_wemux.yml"),
        &HashMap::new(),
        &[],
        LoadOptions::default(),
        &tmux,
        &env,
    )
    .unwrap();
    assert_eq!(wemux.tmux_command(), "wemux");

    let sample = ProjectSpec::load(
        fixture_path("sample.yml"),
        &fixture("sample.yml"),
        &HashMap::new(),
        &[],
        LoadOptions::default(),
        &env,
        Backend::Herdr,
    )
    .unwrap();
    assert_eq!(sample.windows.len(), 9);
    assert_eq!(
        sample.windows[0].pre.as_deref(),
        Some("echo \"I get run in each pane, before each pane command!\"; ")
    );

    let focused_tmux = Project::load(
        &fixture("focused_pane.yml"),
        &HashMap::new(),
        &[],
        LoadOptions::default(),
        &tmux,
        &env,
    )
    .unwrap();
    let focused_script = bootmux::script::render_start(&focused_tmux);
    let final_pane_selection = focused_script
        .lines()
        .rev()
        .find(|line| line.contains("select-pane -t focused_pane"))
        .unwrap();
    assert!(
        final_pane_selection.contains("focused_pane:0.1"),
        "project startup selection overrode the window's focused_pane: {final_pane_selection}"
    );

    let focused = ProjectSpec::load(
        fixture_path("focused_pane.yml"),
        &fixture("focused_pane.yml"),
        &HashMap::new(),
        &[],
        LoadOptions::default(),
        &env,
        Backend::Herdr,
    )
    .unwrap();
    assert_eq!(focused.startup_pane, None);
    assert_eq!(focused.windows[0].focused_pane, 1);

    let startup = ProjectSpec::load(
        fixture_path("startup.yml"),
        &fixture("startup.yml"),
        &HashMap::new(),
        &[],
        LoadOptions::default(),
        &env,
        Backend::Herdr,
    )
    .unwrap();
    assert_eq!(startup.startup_window, 2);
    assert_eq!(startup.startup_pane, Some(1));

    let detached = ProjectSpec::load(
        fixture_path("detach.yml"),
        &fixture("detach.yml"),
        &HashMap::new(),
        &[],
        LoadOptions::default(),
        &env,
        Backend::Herdr,
    )
    .unwrap();
    assert!(!detached.attach);
    assert_eq!(detached.socket_name.as_deref(), Some("foo"));
}
