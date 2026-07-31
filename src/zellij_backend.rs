//! Project lifecycle orchestration for the zellij backend.
//!
//! zellij sessions carry a real name, so unlike Herdr this backend needs no
//! ownership database: a project maps to the session that shares its name, and
//! `stop-all` intersects the live session list with the discoverable configs
//! exactly like the tmux backend does.

use std::collections::HashMap;
use std::os::unix::process::CommandExt;
use std::process::Command;
use std::time::{Duration, Instant};

use anyhow::{bail, Context, Result};

use crate::commands::run::StartParams;
use crate::config::{self, ProjectFileQuery};
use crate::env::Env;
use crate::layout::Layout;
use crate::process::CommandRunner;
use crate::project::{parse_settings, LoadOptions};
use crate::settings::Backend;
use crate::spec::ProjectSpec;
use crate::util::{ask_yes, say_colored, Color};
use crate::zellij::{PaneInfo, Zellij};
use crate::zellij_layout;

/// zellij derives its socket path from the session name, and paths longer than
/// this overflow the platform's socket-path limit.
const MAX_SESSION_NAME_LENGTH: usize = 36;

/// Creating a session returns before zellij has finished building its panes,
/// so the pane listing is polled until the topology matches the config.
const TOPOLOGY_TIMEOUT: Duration = Duration::from_secs(15);
const TOPOLOGY_POLL: Duration = Duration::from_millis(50);

pub fn debug(env: &Env, params: &StartParams) -> Result<()> {
    let spec = load_from_params(env, params)?;
    let session = session_name(&spec)?;
    let layout = preflight(&spec)?;

    for warning in &spec.warnings {
        println!("warning: {warning}");
    }
    println!("backend: zellij");
    println!("session: {session}");
    println!("project: {}", spec.name);
    println!("config: {}", spec.source_path.display());
    println!("root: {}", spec.root);
    println!("attach: {}", spec.attach);
    println!("append: {}", spec.append);

    println!("layout:");
    for line in layout.lines() {
        println!("  {line}");
    }

    println!("plan:");
    println!(
        "  - require zellij >= {} on PATH",
        crate::zellij::MINIMUM_VERSION
    );
    if spec.append {
        println!("  - require the active zellij session and append tabs to it");
    } else {
        println!("  - reuse the session named {session:?} when it is already running");
        println!("  - otherwise create it detached from the layout above");
    }
    if spec.hooks.start.is_some() {
        println!("  - run on_project_start");
    }
    if spec.hooks.first_start.is_some() {
        println!("  - on create: run on_project_first_start");
    }
    if spec.hooks.restart.is_some() {
        println!("  - on reuse: run on_project_restart");
    }
    for (window_index, window) in spec.windows.iter().enumerate() {
        let tree = window.layout_tree()?;
        println!(
            "  - tab[{window_index}] {:?} cwd={} panes={}",
            window.name,
            window.root,
            tree.pane_count()
        );
        let prefix_count =
            usize::from(spec.pre_window.is_some()) + usize::from(window.pre.is_some());
        for (pane_index, pane) in window.effective_panes().iter().enumerate() {
            if let Some(title) = &pane.title {
                println!("      pane[{pane_index}] label={title:?}");
            }
            println!(
                "      pane[{pane_index}] commands={}",
                prefix_count + pane.commands.len()
            );
        }
    }
    println!(
        "  - focus is declared in the layout{}",
        if spec.attach {
            ", then attach the client"
        } else {
            "; the client is left alone"
        }
    );
    if spec.hooks.exit.is_some() {
        println!("  - run on_project_exit");
    }
    Ok(())
}

pub fn start(env: &Env, params: &StartParams) -> Result<()> {
    let spec = load_from_params(env, params)?;
    let session = session_name(&spec)?;
    let layout = preflight(&spec)?;
    warn_ignored(&spec);

    let client = Zellij::new();
    client.require_supported_version()?;

    if spec.append {
        return append_to_active(env, &client, &spec);
    }

    run_hook(spec.hooks.start.as_deref(), &spec.root, env)
        .context("on_project_start hook failed")?;

    if client.has_session(&session)? {
        run_hook(spec.hooks.restart.as_deref(), &spec.root, env)
            .context("on_project_restart hook failed")?;
        focus_session(env, &client, &session, spec.attach)?;
        return run_hook(spec.hooks.exit.as_deref(), &spec.root, env)
            .context("on_project_exit hook failed");
    }

    run_hook(spec.hooks.first_start.as_deref(), &spec.root, env)
        .context("on_project_first_start hook failed")?;
    client
        .create_background_session(&session, &layout)
        .with_context(|| format!("failed to create zellij session {session:?}"))?;

    // Everything after creation can leave a half-built session behind, so a
    // failure takes the whole session down rather than leaving a project that
    // looks started but is not.
    if let Err(error) = populate_session(&client, &spec, &session) {
        return Err(match client.kill_session(&session) {
            Ok(()) => error,
            Err(kill_error) => error.context(format!(
                "the partially created zellij session {session:?} could not be rolled back: \
                 {kill_error}"
            )),
        });
    }

    focus_session(env, &client, &session, spec.attach)?;
    run_hook(spec.hooks.exit.as_deref(), &spec.root, env).context("on_project_exit hook failed")
}

pub fn stop(
    env: &Env,
    project: Option<String>,
    project_config: Option<String>,
    settings: &HashMap<String, String>,
    args: &[String],
) -> Result<()> {
    let (name, project_config) = if project_config.is_some() {
        (None, project_config)
    } else {
        (project, None)
    };
    let spec = load_spec(
        env,
        name.as_deref(),
        project_config.as_deref(),
        settings,
        args,
        LoadOptions::default(),
    )?;
    warn_ignored(&spec);
    stop_session(
        env,
        &Zellij::new(),
        &session_name(&spec)?,
        &spec.root,
        spec.hooks.stop.as_deref(),
    )
}

pub fn stop_all(env: &Env, noconfirm: bool) -> Result<()> {
    let client = Zellij::new();
    let sessions = client.sessions()?;
    let projects = config::configs(env, Some((true, &sessions)));
    if projects.is_empty() {
        return Ok(());
    }

    if !noconfirm {
        say_colored("Stop all active zellij projects:\n", Color::Yellow);
        println!("{}", projects.join("\n"));
        println!();
        if !ask_yes("Are you sure? (n/y)") {
            return Ok(());
        }
    }

    // Leaving the session we are attached to for last keeps a detach from
    // cutting the loop short, matching the tmux backend.
    let current = env.all.get("ZELLIJ_SESSION_NAME").cloned();
    let mut ordered = projects;
    ordered.sort_by_key(|project| current.as_deref() == Some(project.as_str()));

    for project in ordered {
        let spec = load_spec(
            env,
            Some(&project),
            None,
            &HashMap::new(),
            &[],
            LoadOptions::default(),
        )?;
        stop_session(
            env,
            &client,
            &session_name(&spec)?,
            &spec.root,
            spec.hooks.stop.as_deref(),
        )?;
    }
    Ok(())
}

/// Runs the stop hook and closes the session.
///
/// The session is killed even when the hook fails, so a project whose root was
/// deleted or whose hook is broken can still be shut down; the hook's failure
/// is still what the command reports.
fn stop_session<R: CommandRunner>(
    env: &Env,
    client: &Zellij<R>,
    session: &str,
    root: &str,
    stop_hook: Option<&str>,
) -> Result<()> {
    let hook_result = run_hook(stop_hook, root, env).context("on_project_stop hook failed");
    let kill_result = client
        .kill_session(session)
        .with_context(|| format!("failed to stop zellij session {session:?}"));
    hook_result.and(kill_result)
}

/// Adds the project's tabs to the zellij session bootmux is running inside.
fn append_to_active<R: CommandRunner>(
    env: &Env,
    client: &Zellij<R>,
    spec: &ProjectSpec,
) -> Result<()> {
    let Some(session) = env
        .all
        .get("ZELLIJ_SESSION_NAME")
        .filter(|value| !value.is_empty())
    else {
        bail!(
            "`--append` adds tabs to the current zellij session, so it must run inside one. \
             Start the project without `--append` to create its own session."
        );
    };

    run_hook(spec.hooks.start.as_deref(), &spec.root, env)
        .context("on_project_start hook failed")?;
    run_hook(spec.hooks.first_start.as_deref(), &spec.root, env)
        .context("on_project_first_start hook failed")?;

    let mut created = Vec::new();
    let result = (|| -> Result<()> {
        for window_index in 0..spec.windows.len() {
            let layout = zellij_layout::render_window(spec, window_index)?;
            let window = &spec.windows[window_index];
            let tab = client.new_tab(session, window.name.as_deref(), &layout)?;
            created.push(tab);

            let panes = wait_for_tab_panes(
                client,
                session,
                tab,
                window.effective_panes().len(),
                TOPOLOGY_TIMEOUT,
            )?;
            run_pane_commands(client, session, spec, window_index, &panes)?;
        }
        Ok(())
    })();

    if let Err(error) = result {
        return Err(match rollback_tabs(client, session, &created) {
            None => error,
            Some(detail) => error.context(detail),
        });
    }

    run_hook(spec.hooks.exit.as_deref(), &spec.root, env).context("on_project_exit hook failed")
}

/// Closes tabs an aborted append created, newest first.
fn rollback_tabs<R: CommandRunner>(
    client: &Zellij<R>,
    session: &str,
    tabs: &[u32],
) -> Option<String> {
    let failures: Vec<String> = tabs
        .iter()
        .rev()
        .filter_map(|tab| {
            client
                .close_tab(session, *tab)
                .err()
                .map(|error| format!("tab {tab}: {error}"))
        })
        .collect();
    if failures.is_empty() {
        None
    } else {
        Some(format!(
            "append rollback was incomplete ({})",
            failures.join("; ")
        ))
    }
}

/// Runs each pane's commands once the session's topology exists.
fn populate_session<R: CommandRunner>(
    client: &Zellij<R>,
    spec: &ProjectSpec,
    session: &str,
) -> Result<()> {
    let expected: Vec<usize> = spec
        .windows
        .iter()
        .map(|window| window.effective_panes().len())
        .collect();
    let panes = wait_for_session_panes(client, session, &expected, TOPOLOGY_TIMEOUT)?;
    for (window_index, tab_panes) in panes.iter().enumerate() {
        run_pane_commands(client, session, spec, window_index, tab_panes)?;
    }
    Ok(())
}

/// Types each pane's commands into its shell, in the same order the tmux
/// backend sends them: project `pre_window`, then the window's `pre`, then the
/// pane's own commands.
fn run_pane_commands<R: CommandRunner>(
    client: &Zellij<R>,
    session: &str,
    spec: &ProjectSpec,
    window_index: usize,
    pane_ids: &[String],
) -> Result<()> {
    let window = &spec.windows[window_index];
    for (pane_index, pane) in window.effective_panes().iter().enumerate() {
        let pane_id = &pane_ids[pane_index];
        let commands = spec
            .pre_window
            .iter()
            .chain(window.pre.iter())
            .chain(pane.commands.iter());
        for command in commands {
            client
                .run_in_pane(session, pane_id, command)
                .with_context(|| {
                    format!("failed to run `{command}` in tab {window_index} pane {pane_index}")
                })?;
        }
    }
    Ok(())
}

/// Waits for a freshly created session to report the panes its layout
/// declares, then maps them onto the configured pane order.
///
/// Session creation returns before zellij has finished building every pane, so
/// the listing is polled instead of read once.
fn wait_for_session_panes<R: CommandRunner>(
    client: &Zellij<R>,
    session: &str,
    expected: &[usize],
    timeout: Duration,
) -> Result<Vec<Vec<String>>> {
    let deadline = Instant::now() + timeout;
    loop {
        let last_seen = ordered_panes(client, session)?;
        if last_seen.len() == expected.len()
            && last_seen
                .iter()
                .zip(expected)
                .all(|(actual, wanted)| actual.len() == *wanted)
        {
            return Ok(last_seen);
        }
        if Instant::now() >= deadline {
            bail!(
                "zellij session {session:?} settled with {:?} panes per tab but the project \
                 configures {expected:?}",
                last_seen.iter().map(Vec::len).collect::<Vec<_>>()
            );
        }
        std::thread::sleep(TOPOLOGY_POLL);
    }
}

/// Waits for one appended tab to report its panes.
fn wait_for_tab_panes<R: CommandRunner>(
    client: &Zellij<R>,
    session: &str,
    tab_id: u32,
    expected: usize,
    timeout: Duration,
) -> Result<Vec<String>> {
    let deadline = Instant::now() + timeout;
    loop {
        let mut panes: Vec<PaneInfo> = client
            .list_panes(session)?
            .into_iter()
            .filter(|pane| pane.is_ordinary_terminal() && pane.tab_id == tab_id)
            .collect();
        if panes.len() == expected {
            panes.sort_by_key(|pane| (pane.pane_y, pane.pane_x));
            return Ok(panes.iter().map(PaneInfo::target).collect());
        }
        if Instant::now() >= deadline {
            bail!(
                "appended zellij tab {tab_id} settled with {} panes but {expected} are configured",
                panes.len()
            );
        }
        std::thread::sleep(TOPOLOGY_POLL);
    }
}

/// Groups the session's terminal panes by tab and orders each tab's panes the
/// way the config lists them.
///
/// Pane titles are not usable for this: a shell overwrites the title of any
/// pane bootmux did not explicitly name. Geometry is reliable instead, because
/// bootmux renders the layout itself and zellij lays panes out left-to-right,
/// top-to-bottom within a tab.
fn ordered_panes<R: CommandRunner>(client: &Zellij<R>, session: &str) -> Result<Vec<Vec<String>>> {
    let mut panes: Vec<PaneInfo> = client
        .list_panes(session)?
        .into_iter()
        .filter(PaneInfo::is_ordinary_terminal)
        .collect();
    panes.sort_by_key(|pane| (pane.tab_position, pane.pane_y, pane.pane_x));

    let mut tabs: Vec<Vec<String>> = Vec::new();
    let mut current_tab = None;
    for pane in panes {
        if current_tab != Some(pane.tab_position) {
            current_tab = Some(pane.tab_position);
            tabs.push(Vec::new());
        }
        tabs.last_mut()
            .expect("a tab was pushed before its first pane")
            .push(pane.target());
    }
    Ok(tabs)
}

/// Brings the client to the project, if it was asked to.
fn focus_session<R: CommandRunner>(
    env: &Env,
    client: &Zellij<R>,
    session: &str,
    attach: bool,
) -> Result<()> {
    if !attach {
        return Ok(());
    }
    match env
        .all
        .get("ZELLIJ_SESSION_NAME")
        .filter(|value| !value.is_empty())
    {
        // Already in the project's own session; the layout placed the focus.
        Some(current) if current == session => Ok(()),
        Some(current) => client
            .switch_session(current, session)
            .with_context(|| format!("failed to switch to zellij session {session:?}")),
        None => attach_client(client, session),
    }
}

/// Hands the terminal to zellij. This replaces the bootmux process, so it
/// never returns on success.
fn attach_client<R: CommandRunner>(client: &Zellij<R>, session: &str) -> Result<()> {
    let error = Command::new(client.binary())
        .arg("attach")
        .arg(session)
        .exec();
    Err(anyhow::anyhow!(
        "failed to attach to zellij session {session:?}: {error}"
    ))
}

fn run_hook(command: Option<&str>, root: &str, env: &Env) -> Result<()> {
    let Some(command) = command.filter(|command| !command.is_empty()) else {
        return Ok(());
    };
    let status = Command::new(env.shell_or_default())
        .arg("-c")
        .arg(command)
        .current_dir(root)
        .status()
        .with_context(|| format!("failed to run hook `{command}`"))?;
    if !status.success() {
        bail!("hook `{command}` exited with status {status}.");
    }
    Ok(())
}

fn warn_ignored(spec: &ProjectSpec) {
    for warning in &spec.warnings {
        eprintln!("warning: {warning}");
    }
}

pub fn active_project_names(_env: &Env) -> Result<Vec<String>> {
    Zellij::new().sessions().map_err(Into::into)
}

pub fn doctor_compatible(_env: &Env) -> bool {
    Zellij::new().require_supported_version().is_ok()
}

/// Validates everything that can be checked without contacting zellij, and
/// returns the rendered layout so a later failure cannot be a rendering bug.
fn preflight(spec: &ProjectSpec) -> Result<String> {
    for window in &spec.windows {
        let pane_count = window.effective_panes().len();
        let tree = window.layout_tree()?;
        if tree.pane_count() != pane_count {
            bail!(
                "window {:?} layout contains {} panes but {pane_count} panes are configured.",
                window.name,
                tree.pane_count()
            );
        }
        validate_ratios(&tree)?;
    }
    zellij_layout::render_project(spec)
}

/// zellij sizes panes in whole percent, so a ratio outside this range would
/// round to a pane the layout cannot express.
fn validate_ratios(layout: &Layout) -> Result<()> {
    match layout {
        Layout::Pane(_) => Ok(()),
        Layout::Split {
            ratio,
            first,
            second,
            ..
        } => {
            if !(0.1..=0.9).contains(ratio) {
                bail!(
                    "layout ratio {ratio} cannot be represented by zellij (supported range is 0.1 through 0.9)."
                );
            }
            validate_ratios(first)?;
            validate_ratios(second)
        }
    }
}

/// The zellij session a project maps to.
///
/// Unlike tmux, zellij does not need `.` and `:` rewritten, but it does derive
/// a socket path from the name, so length and path separators are rejected
/// rather than silently mangled.
fn session_name(spec: &ProjectSpec) -> Result<String> {
    let name = spec.name.trim();
    if name.is_empty() {
        bail!("a zellij session name cannot be empty");
    }
    if name.chars().count() > MAX_SESSION_NAME_LENGTH {
        bail!(
            "zellij session name {name:?} is {} characters; zellij allows at most \
             {MAX_SESSION_NAME_LENGTH}. Use `-n NAME` or a shorter project name.",
            name.chars().count()
        );
    }
    if name.contains('/') {
        bail!("zellij session name {name:?} cannot contain `/`.");
    }
    if let Some(character) = name.chars().find(|character| character.is_control()) {
        bail!("zellij session name {name:?} cannot contain the control character {character:?}.");
    }
    Ok(name.to_string())
}

fn load_from_params(env: &Env, params: &StartParams) -> Result<ProjectSpec> {
    let (settings, args) = parse_settings(params.args.clone());
    load_spec(
        env,
        params.project.as_deref(),
        params.project_config.as_deref(),
        &settings,
        &args,
        params.load_options(),
    )
}

fn load_spec(
    env: &Env,
    name: Option<&str>,
    project_config: Option<&str>,
    settings: &HashMap<String, String>,
    args: &[String],
    opts: LoadOptions,
) -> Result<ProjectSpec> {
    let file = config::find_project_file(
        env,
        &ProjectFileQuery {
            name,
            project_config,
        },
    )?;
    let content =
        config::read_project_file(env, &file).with_context(|| format!("failed to read {file}"))?;
    ProjectSpec::load(&file, &content, settings, args, opts, env, Backend::Zellij)
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;
    use std::io;
    use std::path::PathBuf;
    use std::sync::Mutex;

    use super::*;
    use crate::process::{CommandOutput, Invocation};
    use crate::spec::{Hooks, WindowSpec};

    #[derive(Default)]
    struct FakeRunner {
        outputs: Mutex<VecDeque<CommandOutput>>,
        invocations: Mutex<Vec<Invocation>>,
    }

    impl FakeRunner {
        fn with(outputs: Vec<CommandOutput>) -> Self {
            Self {
                outputs: Mutex::new(outputs.into()),
                invocations: Mutex::new(Vec::new()),
            }
        }

        /// Every recorded call as a flat argv, with the `--session NAME action`
        /// prefix dropped so assertions read like the action itself.
        fn actions(&self) -> Vec<Vec<String>> {
            self.invocations
                .lock()
                .unwrap()
                .iter()
                .map(|invocation| {
                    invocation
                        .args
                        .iter()
                        .map(|arg| arg.to_string_lossy().into_owned())
                        .skip_while(|arg| arg != "action")
                        .skip(1)
                        .collect()
                })
                .collect()
        }
    }

    impl CommandRunner for FakeRunner {
        fn run(&self, invocation: &Invocation) -> io::Result<CommandOutput> {
            self.invocations.lock().unwrap().push(invocation.clone());
            self.outputs
                .lock()
                .unwrap()
                .pop_front()
                .ok_or_else(|| io::Error::other("no fake output"))
        }

        fn spawn_detached(&self, _invocation: &Invocation) -> io::Result<u32> {
            Ok(42)
        }
    }

    fn ok(stdout: &str) -> CommandOutput {
        CommandOutput {
            success: true,
            code: Some(0),
            stdout: stdout.as_bytes().to_vec(),
            stderr: Vec::new(),
        }
    }

    fn failed(stderr: &str) -> CommandOutput {
        CommandOutput {
            success: false,
            code: Some(2),
            stdout: Vec::new(),
            stderr: stderr.as_bytes().to_vec(),
        }
    }

    fn pane_json(entries: &[(u32, u32, u32, u32)]) -> String {
        let panes: Vec<String> = entries
            .iter()
            .map(|(id, tab_position, x, y)| {
                format!(
                    r#"{{"id":{id},"is_plugin":false,"is_floating":false,"is_suppressed":false,
                        "title":"","tab_id":{tab_position},"tab_position":{tab_position},
                        "tab_name":"t","pane_x":{x},"pane_y":{y}}}"#
                )
            })
            .collect();
        format!("[{}]", panes.join(","))
    }

    /// The elements of a JSON array literal, without its brackets.
    fn inner(array: &str) -> String {
        array[1..array.len() - 1].to_string()
    }

    fn project_spec(source: &str) -> ProjectSpec {
        ProjectSpec::load(
            "/work/demo.yml",
            source,
            &HashMap::new(),
            &[],
            LoadOptions::default(),
            &Env {
                home: "/home/test".into(),
                cwd: PathBuf::from("/work"),
                ..Env::default()
            },
            Backend::Zellij,
        )
        .unwrap()
    }

    #[test]
    fn panes_are_grouped_by_tab_and_ordered_by_geometry() {
        // Reported out of order, and with a plugin and a floating pane mixed in.
        let json = format!(
            "[{},{},{}]",
            r#"{"id":9,"is_plugin":true,"is_floating":false,"is_suppressed":false,"title":"tab-bar",
                "tab_id":0,"tab_position":0,"tab_name":"t","pane_x":0,"pane_y":0}"#,
            r#"{"id":8,"is_plugin":false,"is_floating":true,"is_suppressed":false,"title":"float",
                "tab_id":0,"tab_position":0,"tab_name":"t","pane_x":0,"pane_y":0}"#,
            inner(&pane_json(&[
                (3, 1, 0, 0),
                (2, 0, 25, 30),
                (0, 0, 0, 0),
                (1, 0, 0, 30)
            ]))
        );
        let client = Zellij::with_runner("zellij", FakeRunner::with(vec![ok(&json)]));

        assert_eq!(
            ordered_panes(&client, "api").unwrap(),
            vec![
                vec![
                    "terminal_0".to_string(),
                    "terminal_1".to_string(),
                    "terminal_2".to_string()
                ],
                vec!["terminal_3".to_string()],
            ]
        );
    }

    #[test]
    fn pane_commands_run_pre_window_then_window_pre_then_the_panes_own() {
        let spec = project_spec(
            "name: order\nroot: /work\npre_window: nvm use\nwindows:\n  - app:\n      \
             pre: source .env\n      panes:\n        - one: echo first\n        \
             - two:\n            commands: [echo a, echo b]\n",
        );
        // Two write-chars/send-keys pairs per command: 3 for pane 0, 4 for pane 1.
        let client = Zellij::with_runner("zellij", FakeRunner::with(vec![ok(""); (3 + 4) * 2]));

        run_pane_commands(
            &client,
            "api",
            &spec,
            0,
            &["terminal_0".to_string(), "terminal_1".to_string()],
        )
        .unwrap();

        let typed: Vec<String> = client
            .runner_for_test()
            .actions()
            .into_iter()
            .filter(|action| action.first().map(String::as_str) == Some("write-chars"))
            .map(|action| format!("{} {}", action[2], action[3]))
            .collect();
        assert_eq!(
            typed,
            vec![
                "terminal_0 nvm use",
                "terminal_0 source .env",
                "terminal_0 echo first",
                "terminal_1 nvm use",
                "terminal_1 source .env",
                "terminal_1 echo a",
                "terminal_1 echo b",
            ]
        );
        // Each typed command is submitted with Enter, matching tmux's C-m.
        let submitted = client
            .runner_for_test()
            .actions()
            .into_iter()
            .filter(|action| action.first().map(String::as_str) == Some("send-keys"))
            .count();
        assert_eq!(submitted, typed.len());
    }

    #[test]
    fn a_tab_whose_pane_count_never_matches_reports_both_counts() {
        // The layout only ever yields one pane, but two are configured.
        let client = Zellij::with_runner(
            "zellij",
            FakeRunner::with(vec![ok(&pane_json(&[(0, 0, 0, 0)])); 512]),
        );
        let error = wait_for_tab_panes(&client, "api", 0, 2, Duration::from_millis(0))
            .unwrap_err()
            .to_string();
        assert!(error.contains("1 panes"), "{error}");
        assert!(error.contains("2 are configured"), "{error}");
    }

    #[test]
    fn append_rollback_closes_tabs_newest_first_and_reports_what_it_could_not_close() {
        let client = Zellij::with_runner("zellij", FakeRunner::with(vec![ok(""), ok("")]));
        assert_eq!(rollback_tabs(&client, "api", &[4, 7]), None);
        assert_eq!(
            client
                .runner_for_test()
                .actions()
                .iter()
                .map(|action| action.join(" "))
                .collect::<Vec<_>>(),
            vec!["close-tab --tab-id 7", "close-tab --tab-id 4"]
        );

        let stubborn = Zellij::with_runner(
            "zellij",
            FakeRunner::with(vec![failed("tab is busy"), ok("")]),
        );
        let detail = rollback_tabs(&stubborn, "api", &[4, 7]).unwrap();
        assert!(detail.contains("rollback was incomplete"), "{detail}");
        assert!(detail.contains("tab 7"), "{detail}");
    }

    #[test]
    fn focus_is_skipped_when_the_project_does_not_attach() {
        let client = Zellij::with_runner("zellij", FakeRunner::default());
        focus_session(&Env::default(), &client, "api", false).unwrap();
        assert!(client.runner_for_test().actions().is_empty());
    }

    #[test]
    fn attaching_from_another_session_switches_instead_of_nesting() {
        let env = Env {
            all: HashMap::from([("ZELLIJ_SESSION_NAME".to_string(), "other".to_string())]),
            ..Env::default()
        };
        let client = Zellij::with_runner("zellij", FakeRunner::with(vec![ok("")]));
        focus_session(&env, &client, "api", true).unwrap();
        assert_eq!(
            client.runner_for_test().actions(),
            vec![vec!["switch-session".to_string(), "api".to_string()]]
        );

        // Already inside the project's own session, nothing to do.
        let same = Env {
            all: HashMap::from([("ZELLIJ_SESSION_NAME".to_string(), "api".to_string())]),
            ..Env::default()
        };
        let client = Zellij::with_runner("zellij", FakeRunner::default());
        focus_session(&same, &client, "api", true).unwrap();
        assert!(client.runner_for_test().actions().is_empty());
    }

    fn spec_named(name: &str) -> ProjectSpec {
        ProjectSpec {
            source_path: "/tmp/a.yml".into(),
            name: name.into(),
            root: "/tmp".into(),
            attach: false,
            append: false,
            socket_name: None,
            socket_path: None,
            startup_window: 0,
            startup_pane: None,
            pre_window: None,
            hooks: Hooks::default(),
            windows: vec![WindowSpec {
                name: None,
                root: "/tmp".into(),
                pre: None,
                commands: Vec::new(),
                panes: Vec::new(),
                layout: None,
                pane_chain: false,
                focused_pane: 0,
            }],
            warnings: Vec::new(),
        }
    }

    #[test]
    fn session_names_keep_characters_tmux_would_rewrite() {
        // The tmux backend replaces `.` and `:`; zellij has no such target
        // syntax, so the project name is used verbatim.
        assert_eq!(
            session_name(&spec_named("api.dev:one")).unwrap(),
            "api.dev:one"
        );
        assert_eq!(session_name(&spec_named("  padded  ")).unwrap(), "padded");
    }

    #[test]
    fn session_names_that_zellij_cannot_hold_are_rejected_with_a_fix() {
        let long = "x".repeat(MAX_SESSION_NAME_LENGTH + 1);
        let error = session_name(&spec_named(&long)).unwrap_err().to_string();
        assert!(error.contains("37 characters"), "{error}");
        assert!(error.contains("-n NAME"), "{error}");

        assert!(session_name(&spec_named(&"x".repeat(MAX_SESSION_NAME_LENGTH))).is_ok());
        assert!(session_name(&spec_named("a/b"))
            .unwrap_err()
            .to_string()
            .contains('/'));
        assert!(session_name(&spec_named("a\nb")).is_err());
        assert!(session_name(&spec_named("   ")).is_err());
    }

    #[test]
    fn ratios_outside_the_representable_range_fail_instead_of_clamping() {
        let layout = Layout::Split {
            direction: crate::layout::SplitDirection::Right,
            ratio: 0.05,
            first: Box::new(Layout::Pane(0)),
            second: Box::new(Layout::Pane(1)),
        };
        assert!(validate_ratios(&layout)
            .unwrap_err()
            .to_string()
            .contains("cannot be represented"));
    }
}
