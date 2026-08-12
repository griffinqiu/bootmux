use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use anyhow::{anyhow, bail, Context, Result};

use crate::commands::run::StartParams;
use crate::config::{self, ProjectFileQuery};
use crate::env::Env;
use crate::herdr::{
    Endpoint, Herdr, ManagedWorkspace, PaneSplit as HerdrPaneSplit, PaneTarget, ProcessRunner,
    SessionSnapshot, SplitDirection as HerdrSplitDirection, StateIndex, StateStore, TabCreate,
    WorkspaceCreate,
};
use crate::layout::{Layout, SplitDirection};
use crate::project::{parse_settings, LoadOptions, HOOK_ON_PROJECT_STOP};
use crate::settings::Backend;
use crate::spec::{ProjectSpec, WindowSpec};
use crate::template;
use crate::util::{ask_yes, expand_path, report_outcome, say_colored, Color};
use crate::yaml_ext::{get, join_or_string, parse};

struct LoadedSpec {
    spec: ProjectSpec,
}

#[derive(Debug)]
struct Topology {
    workspace_id: String,
    root_pane_id: String,
    tabs: Vec<String>,
    panes: Vec<Vec<String>>,
}

#[derive(Debug)]
struct ExistingWorkspace {
    workspace_id: String,
}

pub fn start(env: &Env, params: &StartParams) -> Result<()> {
    let loaded = load_from_params(env, params)?;
    start_loaded(env, loaded.spec)
}

pub fn debug(env: &Env, params: &StartParams) -> Result<()> {
    let loaded = load_from_params(env, params)?;
    let spec = loaded.spec;
    preflight_spec(&spec)?;
    let endpoint = endpoint_for(&spec, env)?;

    for warning in &spec.warnings {
        println!("warning: {warning}");
    }
    println!("backend: herdr");
    println!("endpoint: {}", describe_endpoint(&endpoint));
    println!("project: {}", spec.name);
    println!("config: {}", spec.source_path.display());
    println!("root: {}", spec.root);
    println!("attach: {}", spec.attach);
    println!("append: {}", spec.append);
    println!("plan:");
    println!(
        "  - ensure compatible Herdr >= {} / protocol {} server",
        crate::herdr::MINIMUM_HERDR_VERSION,
        crate::herdr::describe_protocols(crate::herdr::SUPPORTED_PROTOCOLS)
    );
    if spec.append {
        println!("  - require the active Herdr workspace and append tabs");
    } else {
        println!("  - resolve managed workspace by state ID");
        println!("  - recover only one exact label + root match; otherwise fail closed");
        println!("  - create workspace if no managed or adoptable workspace exists");
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
        let layout = layout_for(window)?;
        println!(
            "  - tab[{window_index}] {:?} cwd={} panes={}",
            window.name,
            window.root,
            layout.pane_count()
        );
        render_layout_plan(&layout, "      ");
        for (pane_index, pane) in window.effective_panes().iter().enumerate() {
            if let Some(title) = &pane.title {
                println!("      pane[{pane_index}] label={title:?}");
            }
            let prefix_count =
                usize::from(spec.pre_window.is_some()) + usize::from(window.pre.is_some());
            println!(
                "      pane[{pane_index}] commands={}",
                prefix_count + pane.commands.len()
            );
        }
    }
    let startup_pane = spec.startup_pane.unwrap_or(
        spec.windows
            .get(spec.startup_window)
            .map(|window| window.focused_pane)
            .unwrap_or(0),
    );
    println!(
        "  - select tab[{}] pane[{}]{}",
        spec.startup_window,
        startup_pane,
        if spec.attach {
            " and attach/focus"
        } else {
            " without keeping global focus"
        }
    );
    if spec.hooks.exit.is_some() {
        println!("  - run on_project_exit");
    }
    Ok(())
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
    let command_endpoint = endpoint_for(&spec, env)?;
    let state = StateStore::xdg()?;
    let _operation_lock = state
        .acquire_operation_lock()
        .context("another bootmux Herdr lifecycle operation is still running")?;
    let client = Herdr::new(command_endpoint);
    let status = client.server_status()?;
    let endpoint = canonical_socket_endpoint(&status.socket);
    let index = state.load()?;
    let managed = managed_for_spec(&index, &endpoint, &spec)?;
    let Some(managed) = managed.cloned() else {
        require_no_managed_config_on_other_endpoint(&index, &endpoint, &spec)?;
        report_workspace("found no managed", &spec.name, &endpoint);
        return Ok(());
    };
    require_managed_project_identity(&managed, &spec)?;
    require_managed_stop_hook_snapshot(&managed, &spec)?;

    let status = if status.running {
        status
    } else {
        client
            .ensure_server()
            .context("failed to start Herdr so the managed workspace can be stopped")?
    };
    require_managed_endpoint_identity(&managed, &status.socket)?;
    let snapshot = client.snapshot()?;
    let Some(existing) = recover_managed_workspace(&snapshot, &managed)? else {
        state.update(|index| {
            index.remove_exact_id(&endpoint, &managed.workspace_id);
            Ok(())
        })?;
        report_workspace("found no managed", &spec.name, &endpoint);
        return Ok(());
    };

    let hook_root = managed.root_cwd.to_string_lossy();
    run_hook(spec.hooks.stop.as_deref(), &hook_root, env).context("on_project_stop hook failed")?;
    client
        .close_workspace(&existing.workspace_id)
        .context("failed to close managed Herdr workspace")?;
    state.update(|index| {
        index.remove_exact_id(&endpoint, &managed.workspace_id);
        if existing.workspace_id != managed.workspace_id {
            index.remove_exact_id(&endpoint, &existing.workspace_id);
        }
        Ok(())
    })?;
    report_workspace("stopped", &spec.name, &endpoint);
    Ok(())
}

pub fn stop_all(env: &Env, noconfirm: bool) -> Result<()> {
    let state = StateStore::xdg()?;
    let index = state.load()?;
    if index.managed_workspaces.is_empty() {
        return Ok(());
    }

    if !noconfirm {
        say_colored("Stop all active Herdr projects:\n", Color::Yellow);
        let mut names: Vec<_> = index
            .managed_workspaces
            .iter()
            .map(|managed| project_config_name(env, managed))
            .collect();
        names.sort();
        names.dedup();
        println!("{}", names.join("\n"));
        println!();
        if !ask_yes("Are you sure? (n/y)") {
            return Ok(());
        }
    }

    let _operation_lock = state
        .acquire_operation_lock()
        .context("another bootmux Herdr lifecycle operation is still running")?;
    let current_workspace = current_workspace_id(env);
    let current_endpoint = env
        .all
        .get("HERDR_SOCKET_PATH")
        .filter(|value| !value.is_empty())
        .map(|value| Endpoint::SocketPath(PathBuf::from(value)));
    let mut managed = index.managed_workspaces.clone();
    managed.sort_by_key(|entry| {
        current_workspace.as_deref() == Some(entry.workspace_id.as_str())
            && current_endpoint.as_ref() == Some(&entry.endpoint)
    });

    for entry in managed {
        let client = Herdr::new(
            entry
                .launch_endpoint
                .clone()
                .unwrap_or_else(|| entry.endpoint.clone()),
        );
        let status = client.server_status()?;
        let status = if status.running {
            status
        } else {
            client.ensure_server().with_context(|| {
                format!("failed to start Herdr endpoint for {}", entry.project_name)
            })?
        };
        require_managed_endpoint_identity(&entry, &status.socket)?;
        let snapshot = client.snapshot()?;
        let Some(existing) = recover_managed_workspace(&snapshot, &entry)? else {
            state.update(|index| {
                index.remove_exact_id(&entry.endpoint, &entry.workspace_id);
                Ok(())
            })?;
            continue;
        };

        let hook = stop_all_hook(env, &entry)?;
        let root = entry.root_cwd.to_string_lossy();
        run_hook(hook.as_deref(), &root, env)
            .with_context(|| format!("on_project_stop hook failed for {}", entry.project_name))?;
        client
            .close_workspace(&existing.workspace_id)
            .with_context(|| {
                format!("failed to close Herdr workspace {}", existing.workspace_id)
            })?;
        state.update(|index| {
            index.remove_exact_id(&entry.endpoint, &entry.workspace_id);
            if existing.workspace_id != entry.workspace_id {
                index.remove_exact_id(&entry.endpoint, &existing.workspace_id);
            }
            Ok(())
        })?;
    }
    Ok(())
}

pub fn active_project_names(env: &Env) -> Result<Vec<String>> {
    let state = StateStore::xdg()?;
    let index = state.load()?;
    let mut live = Vec::new();
    let mut snapshots: HashMap<Endpoint, Option<SessionSnapshot>> = HashMap::new();

    for managed in &index.managed_workspaces {
        let snapshot = if let Some(snapshot) = snapshots.get(&managed.endpoint) {
            snapshot.as_ref()
        } else {
            let client = Herdr::new(managed.endpoint.clone());
            let snapshot = match client.server_status() {
                Ok(status) if status.running => client.snapshot().ok(),
                _ => None,
            };
            snapshots.insert(managed.endpoint.clone(), snapshot);
            snapshots.get(&managed.endpoint).and_then(Option::as_ref)
        };
        if let Some(snapshot) = snapshot {
            if recover_managed_workspace(snapshot, managed)?.is_some() {
                live.push(project_config_name(env, managed));
            }
        }
    }
    live.sort();
    live.dedup();
    Ok(live)
}

pub fn doctor_compatible(env: &Env) -> bool {
    let endpoint = ambient_endpoint(env);
    Herdr::new(endpoint).probe().is_ok()
}

/// Resolves nested tmux-inside-Herdr without trusting inherited marker
/// variables alone. A Herdr popup is handled earlier by the settings resolver.
pub fn classify_foreground_backend(env: &Env) -> Result<Option<Backend>> {
    let pane_id = env
        .all
        .get("HERDR_PANE_ID")
        .or_else(|| env.all.get("HERDR_ACTIVE_PANE_ID"));
    let Some(pane_id) = pane_id.filter(|value| !value.is_empty()) else {
        return Ok(None);
    };

    let mut command = Command::new("herdr");
    configure_endpoint_command(&mut command, &ambient_endpoint(env));
    let output = command
        .args(["pane", "process-info", "--pane", pane_id])
        .output();
    let Ok(output) = output else {
        return Ok(None);
    };
    if !output.status.success() {
        return Ok(None);
    }
    let json: serde_json::Value = match serde_json::from_slice(&output.stdout) {
        Ok(json) => json,
        Err(_) => return Ok(None),
    };
    let processes = json
        .pointer("/result/process_info/foreground_processes")
        .and_then(serde_json::Value::as_array);
    let Some(processes) = processes else {
        return Ok(None);
    };
    if processes.is_empty() {
        return Ok(None);
    }
    let is_tmux = processes.iter().any(|process| {
        ["name", "argv0"].iter().any(|key| {
            process
                .get(key)
                .and_then(serde_json::Value::as_str)
                .and_then(|value| Path::new(value).file_name())
                .and_then(|value| value.to_str())
                == Some("tmux")
        })
    });
    Ok(Some(if is_tmux {
        Backend::Tmux
    } else {
        Backend::Herdr
    }))
}

fn start_loaded(env: &Env, spec: ProjectSpec) -> Result<()> {
    preflight_spec(&spec)?;
    warn_ignored(&spec);
    let command_endpoint = endpoint_for(&spec, env)?;
    let state = StateStore::xdg()?;
    let operation_lock = state
        .acquire_operation_lock()
        .context("another bootmux Herdr lifecycle operation is still running")?;
    let client = Herdr::new(command_endpoint.clone());
    let server = client
        .ensure_server()
        .context("failed to ensure a compatible Herdr server")?;
    let endpoint = canonical_socket_endpoint(&server.socket);
    if spec.append && !attached_to_endpoint(env, &endpoint) {
        bail!(
            "`--append` can only target the Herdr endpoint containing the current workspace; remove the project socket selector or choose the matching endpoint."
        );
    }
    run_hook(spec.hooks.start.as_deref(), &spec.root, env)
        .context("on_project_start hook failed")?;

    if spec.append {
        let result = append_to_active(env, &client, &spec);
        drop(operation_lock);
        result?;
        report_workspace("appended to", &spec.name, &endpoint);
        return run_hook(spec.hooks.exit.as_deref(), &spec.root, env)
            .context("on_project_exit hook failed");
    }

    let before = client.snapshot()?;
    if let Some(existing) = find_or_adopt(&client, &state, &endpoint, &spec, &before)? {
        run_hook(spec.hooks.restart.as_deref(), &spec.root, env)
            .context("on_project_restart hook failed")?;
        if spec.attach {
            client.focus_workspace(&existing.workspace_id)?;
        }
        drop(operation_lock);
        report_workspace("reused", &spec.name, &endpoint);
        if spec.attach {
            attach_if_outside(env, &command_endpoint, &endpoint)?;
        }
        run_hook(spec.hooks.exit.as_deref(), &spec.root, env)
            .context("on_project_exit hook failed")?;
        return Ok(());
    }

    run_hook(spec.hooks.first_start.as_deref(), &spec.root, env)
        .context("on_project_first_start hook failed")?;
    let created = client.create_workspace(&WorkspaceCreate {
        cwd: Some(PathBuf::from(&spec.root)),
        label: Some(spec.name.clone()),
        focus: false,
        ..WorkspaceCreate::default()
    })?;
    let workspace_id = created.workspace.workspace_id.clone();
    let created_root_pane_id = created.root_pane.pane_id.clone();
    let topology_result = build_new_workspace(
        &client,
        &spec,
        created.workspace.workspace_id,
        created.tab.tab_id,
        created.root_pane.pane_id,
    );
    let topology = match topology_result {
        Ok(topology) => topology,
        Err(error) => {
            let rollback = client.close_workspace(&workspace_id);
            return match rollback {
                Ok(()) => Err(error.context("new Herdr workspace was rolled back")),
                Err(rollback_error) => {
                    let retained = ManagedWorkspace {
                        endpoint: endpoint.clone(),
                        launch_endpoint: Some(command_endpoint.clone()),
                        workspace_id: workspace_id.clone(),
                        label: spec.name.clone(),
                        root_cwd: PathBuf::from(&spec.root),
                        config_path: spec.source_path.clone(),
                        project_name: spec.name.clone(),
                        root_pane_id: Some(created_root_pane_id),
                        stop_hook: stop_hook_snapshot(&spec),
                    };
                    match state.update(|index| {
                        remove_project_entries(index, &endpoint, &spec);
                        index.upsert(retained);
                        Ok(())
                    }) {
                        Ok(()) => Err(error.context(format!(
                            "workspace rollback failed and ownership was retained for a later stop: {rollback_error}"
                        ))),
                        Err(state_error) => Err(error.context(format!(
                            "workspace rollback failed ({rollback_error}) and ownership could not be persisted ({state_error})"
                        ))),
                    }
                }
            };
        }
    };

    let managed = ManagedWorkspace {
        endpoint: endpoint.clone(),
        launch_endpoint: Some(command_endpoint.clone()),
        workspace_id: topology.workspace_id.clone(),
        label: spec.name.clone(),
        root_cwd: PathBuf::from(&spec.root),
        config_path: spec.source_path.clone(),
        project_name: spec.name.clone(),
        root_pane_id: Some(topology.root_pane_id.clone()),
        stop_hook: stop_hook_snapshot(&spec),
    };
    if let Err(error) = state.update(|index| {
        remove_project_entries(index, &endpoint, &spec);
        index.upsert(managed);
        Ok(())
    }) {
        return match client.close_workspace(&topology.workspace_id) {
            Ok(()) => Err(anyhow!(error)
                .context("failed to persist ownership; new Herdr workspace was rolled back")),
            Err(rollback_error) => Err(anyhow!(error).context(format!(
                "failed to persist ownership and workspace rollback failed: {rollback_error}"
            ))),
        };
    }

    if let Err(error) = apply_selection(&client, &spec, &topology, &before, spec.attach) {
        let rollback = rollback_created(&client, &state, &endpoint, &topology.workspace_id);
        return Err(error.context(format!(
            "startup focus failed; {}",
            rollback.unwrap_or_else(|| "new Herdr workspace was rolled back".to_string())
        )));
    }
    drop(operation_lock);
    report_workspace("created", &spec.name, &endpoint);
    if spec.attach {
        attach_if_outside(env, &command_endpoint, &endpoint)
            .context("Herdr workspace started and remains managed, but attaching failed")?;
    }
    run_hook(spec.hooks.exit.as_deref(), &spec.root, env).context("on_project_exit hook failed")?;
    Ok(())
}

fn append_to_active(env: &Env, client: &Herdr<ProcessRunner>, spec: &ProjectSpec) -> Result<()> {
    let workspace_id = current_workspace_id(env).ok_or_else(|| {
        anyhow!("`--append` with Herdr requires running inside a Herdr workspace or popup.")
    })?;
    client.get_workspace(&workspace_id).with_context(|| {
        format!("active Herdr workspace `{workspace_id}` does not exist at the selected endpoint")
    })?;
    run_hook(spec.hooks.first_start.as_deref(), &spec.root, env)
        .context("on_project_first_start hook failed")?;
    let before = client.snapshot()?;
    let topology = build_appended_tabs(client, spec, &workspace_id)?;
    if let Err(error) = apply_selection(client, spec, &topology, &before, spec.attach) {
        let rollback = rollback_appended_tabs(client, &topology.tabs);
        return Err(error.context(
            rollback.unwrap_or_else(|| "appended Herdr tabs were rolled back".to_string()),
        ));
    }
    Ok(())
}

fn build_new_workspace(
    client: &Herdr<ProcessRunner>,
    spec: &ProjectSpec,
    workspace_id: String,
    first_tab_id: String,
    first_pane_id: String,
) -> Result<Topology> {
    let first_name = spec.windows[0].name.as_deref();
    if let Some(name) = first_name {
        client.rename_tab(&first_tab_id, name)?;
    }
    let mut topology = Topology {
        workspace_id: workspace_id.clone(),
        root_pane_id: first_pane_id.clone(),
        tabs: Vec::with_capacity(spec.windows.len()),
        panes: Vec::with_capacity(spec.windows.len()),
    };
    build_window(
        client,
        spec,
        0,
        &first_tab_id,
        &first_pane_id,
        &mut topology,
    )?;

    for index in 1..spec.windows.len() {
        let window = &spec.windows[index];
        let tab = client.create_tab(&TabCreate {
            workspace_id: Some(workspace_id.clone()),
            cwd: Some(PathBuf::from(&window.root)),
            label: window.name.clone(),
            focus: false,
            ..TabCreate::default()
        })?;
        build_window(
            client,
            spec,
            index,
            &tab.tab.tab_id,
            &tab.root_pane.pane_id,
            &mut topology,
        )?;
    }
    Ok(topology)
}

fn build_appended_tabs(
    client: &Herdr<ProcessRunner>,
    spec: &ProjectSpec,
    workspace_id: &str,
) -> Result<Topology> {
    let mut topology = Topology {
        workspace_id: workspace_id.to_string(),
        root_pane_id: String::new(),
        tabs: Vec::with_capacity(spec.windows.len()),
        panes: Vec::with_capacity(spec.windows.len()),
    };
    let mut created_tabs = Vec::with_capacity(spec.windows.len());
    for (index, window) in spec.windows.iter().enumerate() {
        let tab = match client.create_tab(&TabCreate {
            workspace_id: Some(workspace_id.to_string()),
            cwd: Some(PathBuf::from(&window.root)),
            label: window.name.clone(),
            focus: false,
            ..TabCreate::default()
        }) {
            Ok(tab) => tab,
            Err(error) => {
                let rollback = rollback_appended_tabs(client, &created_tabs);
                return Err(anyhow!(error).context(rollback.unwrap_or_else(|| {
                    "partially appended Herdr tabs were rolled back".to_string()
                })));
            }
        };
        created_tabs.push(tab.tab.tab_id.clone());
        if topology.root_pane_id.is_empty() {
            topology.root_pane_id = tab.root_pane.pane_id.clone();
        }
        if let Err(error) = build_window(
            client,
            spec,
            index,
            &tab.tab.tab_id,
            &tab.root_pane.pane_id,
            &mut topology,
        ) {
            let rollback = rollback_appended_tabs(client, &created_tabs);
            return Err(error.context(
                rollback.unwrap_or_else(|| {
                    "partially appended Herdr tabs were rolled back".to_string()
                }),
            ));
        }
    }
    Ok(topology)
}

fn build_window(
    client: &Herdr<ProcessRunner>,
    spec: &ProjectSpec,
    window_index: usize,
    tab_id: &str,
    root_pane_id: &str,
    topology: &mut Topology,
) -> Result<()> {
    let window = &spec.windows[window_index];
    let panes = window.effective_panes();
    let layout = layout_for(window)?;
    if layout.pane_count() != panes.len() {
        bail!(
            "window {:?} layout contains {} panes but {} panes are configured.",
            window.name,
            layout.pane_count(),
            panes.len()
        );
    }
    validate_herdr_ratios(&layout)?;

    let mut pane_ids = vec![None; panes.len()];
    realize_layout(
        client,
        &layout,
        root_pane_id,
        Path::new(&window.root),
        &mut pane_ids,
    )?;
    if window_index == 0 && window.root != spec.root {
        client.run_in_pane(
            root_pane_id,
            &format!("cd {}", crate::shellwords::escape(&window.root)),
        )?;
    }
    let pane_ids = pane_ids
        .into_iter()
        .enumerate()
        .map(|(index, pane)| {
            pane.ok_or_else(|| anyhow!("layout did not assign configured pane index {index}"))
        })
        .collect::<Result<Vec<_>>>()?;

    for (pane_index, pane) in panes.iter().enumerate() {
        let pane_id = &pane_ids[pane_index];
        if pane.title.is_some() {
            client.rename_pane(pane_id, pane.title.as_deref())?;
        }
        if let Some(command) = &spec.pre_window {
            client.run_in_pane(pane_id, command)?;
        }
        if let Some(command) = &window.pre {
            client.run_in_pane(pane_id, command)?;
        }
        for command in &pane.commands {
            client.run_in_pane(pane_id, command)?;
        }
    }
    topology.tabs.push(tab_id.to_string());
    topology.panes.push(pane_ids);
    Ok(())
}

fn realize_layout(
    client: &Herdr<ProcessRunner>,
    layout: &Layout,
    existing_pane_id: &str,
    cwd: &Path,
    pane_ids: &mut [Option<String>],
) -> Result<()> {
    match layout {
        Layout::Pane(index) => {
            let slot = pane_ids
                .get_mut(*index)
                .ok_or_else(|| anyhow!("layout refers to missing pane index {index}"))?;
            if slot.is_some() {
                bail!("layout refers to pane index {index} more than once.");
            }
            *slot = Some(existing_pane_id.to_string());
        }
        Layout::Split {
            direction,
            ratio,
            first,
            second,
        } => {
            let created = client.split_pane(&HerdrPaneSplit {
                target: PaneTarget::PaneId(existing_pane_id.to_string()),
                direction: match direction {
                    SplitDirection::Right => HerdrSplitDirection::Right,
                    SplitDirection::Down => HerdrSplitDirection::Down,
                },
                ratio: Some(*ratio as f32),
                cwd: Some(cwd.to_path_buf()),
                env: Default::default(),
                focus: false,
            })?;
            realize_layout(client, first, existing_pane_id, cwd, pane_ids)?;
            realize_layout(client, second, &created.pane_id, cwd, pane_ids)?;
        }
    }
    Ok(())
}

fn apply_selection(
    client: &Herdr<ProcessRunner>,
    spec: &ProjectSpec,
    topology: &Topology,
    previous: &SessionSnapshot,
    keep_focus: bool,
) -> Result<()> {
    // Persist each tab's configured selected pane, then select the project's
    // final startup tab/pane.
    for (window_index, window) in spec.windows.iter().enumerate() {
        let pane_id = topology
            .panes
            .get(window_index)
            .and_then(|panes| panes.get(window.focused_pane))
            .ok_or_else(|| anyhow!("configured focused pane is missing from topology"))?;
        client.focus_pane_direct(pane_id)?;
    }
    let tab_id = topology
        .tabs
        .get(spec.startup_window)
        .ok_or_else(|| anyhow!("configured startup tab is missing from topology"))?;
    client.focus_tab(tab_id)?;
    if let Some(startup_pane) = spec.startup_pane {
        let pane_id = topology
            .panes
            .get(spec.startup_window)
            .and_then(|panes| panes.get(startup_pane))
            .ok_or_else(|| anyhow!("configured startup pane is missing from topology"))?;
        client.focus_pane_direct(pane_id)?;
    }

    if !keep_focus {
        if let Some(previous_pane) = &previous.focused_pane_id {
            if previous.pane_by_exact_id(previous_pane).is_some() {
                client.focus_pane_direct(previous_pane)?;
            }
        } else if let Some(previous_workspace) = &previous.focused_workspace_id {
            client.focus_workspace(previous_workspace)?;
        }
    }
    Ok(())
}

fn layout_for(window: &WindowSpec) -> Result<Layout> {
    window.layout_tree()
}

fn validate_herdr_ratios(layout: &Layout) -> Result<()> {
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
                    "layout ratio {ratio} cannot be represented by Herdr (supported range is 0.1 through 0.9)."
                );
            }
            validate_herdr_ratios(first)?;
            validate_herdr_ratios(second)
        }
    }
}

fn preflight_spec(spec: &ProjectSpec) -> Result<()> {
    for window in &spec.windows {
        let pane_count = window.effective_panes().len();
        let layout = layout_for(window)?;
        if layout.pane_count() != pane_count {
            bail!(
                "window {:?} layout contains {} panes but {} panes are configured.",
                window.name,
                layout.pane_count(),
                pane_count
            );
        }
        validate_herdr_ratios(&layout)?;
    }
    Ok(())
}

fn find_or_adopt(
    client: &Herdr<ProcessRunner>,
    state: &StateStore,
    endpoint: &Endpoint,
    spec: &ProjectSpec,
    snapshot: &SessionSnapshot,
) -> Result<Option<ExistingWorkspace>> {
    let index = state.load()?;
    if let Some(managed) = managed_for_spec(&index, endpoint, spec)?.cloned() {
        if let Some(existing) = recover_managed_workspace(snapshot, &managed)? {
            require_managed_project_identity(&managed, spec)?;
            let mut recovered = managed.clone();
            let mut state_changed = false;
            if existing.workspace_id != managed.workspace_id {
                recovered.workspace_id = existing.workspace_id.clone();
                recovered.root_pane_id =
                    root_pane_for(snapshot, &existing.workspace_id, &spec.root);
                state_changed = true;
            }
            if recovered.launch_endpoint.is_none() {
                recovered.launch_endpoint = Some(client.endpoint().clone());
                state_changed = true;
            }
            let stop_hook = stop_hook_snapshot(spec);
            if recovered.stop_hook != stop_hook {
                recovered.stop_hook = stop_hook;
                state_changed = true;
            }
            if state_changed {
                state.update(|index| {
                    index.remove_exact_id(endpoint, &managed.workspace_id);
                    index.upsert(recovered);
                    Ok(())
                })?;
            }
            return Ok(Some(existing));
        }
        state.update(|index| {
            index.remove_exact_id(endpoint, &managed.workspace_id);
            Ok(())
        })?;
    }

    if let Some(workspace) =
        snapshot.workspace_by_unique_exact_label_and_root_cwd(&spec.name, Path::new(&spec.root))?
    {
        let root_pane_id = root_pane_for(snapshot, &workspace.workspace_id, &spec.root);
        let adopted = ManagedWorkspace {
            endpoint: endpoint.clone(),
            launch_endpoint: Some(client.endpoint().clone()),
            workspace_id: workspace.workspace_id.clone(),
            label: spec.name.clone(),
            root_cwd: PathBuf::from(&spec.root),
            config_path: spec.source_path.clone(),
            project_name: spec.name.clone(),
            root_pane_id: root_pane_id.clone(),
            stop_hook: stop_hook_snapshot(spec),
        };
        state.update(|index| {
            remove_project_entries(index, endpoint, spec);
            index.upsert(adopted);
            Ok(())
        })?;
        return Ok(Some(ExistingWorkspace {
            workspace_id: workspace.workspace_id.clone(),
        }));
    }
    Ok(None)
}

fn managed_for_spec<'a>(
    index: &'a StateIndex,
    endpoint: &Endpoint,
    spec: &ProjectSpec,
) -> Result<Option<&'a ManagedWorkspace>> {
    let by_config: Vec<_> = index
        .managed_workspaces
        .iter()
        .filter(|managed| &managed.endpoint == endpoint && managed.config_path == spec.source_path)
        .collect();
    match by_config.len() {
        0 => Ok(None),
        1 => Ok(by_config.into_iter().next()),
        _ => {
            let exact: Vec<_> = by_config
                .into_iter()
                .filter(|managed| managed.project_name == spec.name)
                .collect();
            match exact.len() {
                1 => Ok(exact.into_iter().next()),
                count => bail!(
                    "state contains multiple ownership entries for config {} and {count} match project name `{}`; refusing an ambiguous operation.",
                    spec.source_path.display(),
                    spec.name
                ),
            }
        }
    }
}

fn require_managed_project_identity(managed: &ManagedWorkspace, spec: &ProjectSpec) -> Result<()> {
    if managed.project_name != spec.name
        || managed.label != spec.name
        || managed.root_cwd != Path::new(&spec.root)
    {
        bail!(
            "config {} now resolves to label `{}` and root {}, but bootmux owns workspace `{}` as project `{}`, label `{}`, and root {}; refusing an identity-mismatched lifecycle operation. Pass the same template settings used when the workspace was started.",
            spec.source_path.display(),
            spec.name,
            spec.root,
            managed.workspace_id,
            managed.project_name,
            managed.label,
            managed.root_cwd.display()
        );
    }
    Ok(())
}

fn require_no_managed_config_on_other_endpoint(
    index: &StateIndex,
    endpoint: &Endpoint,
    spec: &ProjectSpec,
) -> Result<()> {
    let mut other_endpoints = index
        .managed_workspaces
        .iter()
        .filter(|managed| managed.config_path == spec.source_path && &managed.endpoint != endpoint)
        .map(|managed| describe_endpoint(&managed.endpoint))
        .collect::<Vec<_>>();
    other_endpoints.sort();
    other_endpoints.dedup();
    if !other_endpoints.is_empty() {
        bail!(
            "config {} is managed on Herdr endpoint(s) {}, but the current template settings select {}; refusing a silent no-op. Pass the same socket settings used when the workspace was started.",
            spec.source_path.display(),
            other_endpoints.join(", "),
            describe_endpoint(endpoint)
        );
    }
    Ok(())
}

fn require_managed_stop_hook_snapshot(
    managed: &ManagedWorkspace,
    spec: &ProjectSpec,
) -> Result<()> {
    let Some(snapshot) = &managed.stop_hook else {
        return Ok(());
    };
    let rendered = spec.hooks.stop.as_deref().unwrap_or_default();
    if snapshot != rendered {
        bail!(
            "config {} now renders a different on_project_stop hook than the managed workspace snapshot; refusing to execute an unverified hook. Pass the same template settings used at start, restore the config, or start the project once to refresh its managed snapshot.",
            spec.source_path.display()
        );
    }
    Ok(())
}

fn remove_project_entries(index: &mut StateIndex, endpoint: &Endpoint, spec: &ProjectSpec) {
    index.managed_workspaces.retain(|managed| {
        &managed.endpoint != endpoint
            || managed.config_path != spec.source_path
            || managed.project_name != spec.name
    });
}

fn recover_managed_workspace(
    snapshot: &SessionSnapshot,
    managed: &ManagedWorkspace,
) -> Result<Option<ExistingWorkspace>> {
    Ok(snapshot
        .recover_workspace(managed)?
        .map(|recovered| ExistingWorkspace {
            workspace_id: recovered.workspace.workspace_id.clone(),
        }))
}

fn root_pane_for(snapshot: &SessionSnapshot, workspace_id: &str, root: &str) -> Option<String> {
    snapshot
        .panes
        .iter()
        .filter(|pane| {
            pane.workspace_id == workspace_id && pane.cwd.as_deref() == Some(Path::new(root))
        })
        .map(|pane| pane.pane_id.clone())
        .min()
}

fn endpoint_for(spec: &ProjectSpec, env: &Env) -> Result<Endpoint> {
    if let Some(path) = &spec.socket_path {
        return Ok(Endpoint::SocketPath(PathBuf::from(expand_path(
            path,
            &env.cwd.to_string_lossy(),
            &env.home,
        ))));
    }
    if let Some(name) = &spec.socket_name {
        validate_session_name(name)?;
        return Ok(Endpoint::NamedSession(name.clone()));
    }
    Ok(ambient_endpoint(env))
}

fn ambient_endpoint(env: &Env) -> Endpoint {
    if let Some(path) = env
        .all
        .get("HERDR_SOCKET_PATH")
        .filter(|value| !value.is_empty())
    {
        Endpoint::SocketPath(PathBuf::from(path))
    } else if let Some(name) = env
        .all
        .get("HERDR_SESSION")
        .filter(|value| !value.is_empty())
    {
        Endpoint::NamedSession(name.clone())
    } else {
        Endpoint::Default
    }
}

fn validate_session_name(name: &str) -> Result<()> {
    if name.len() > 64
        || name.is_empty()
        || !name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
    {
        bail!("Herdr session name `{name}` must be 1-64 ASCII letters, digits, `.`, `_`, or `-`.");
    }
    Ok(())
}

fn report_workspace(action: &str, project: &str, endpoint: &Endpoint) {
    report_outcome(&format!(
        "{action} herdr workspace {project:?} ({})",
        describe_endpoint(endpoint)
    ));
}

fn describe_endpoint(endpoint: &Endpoint) -> String {
    match endpoint {
        Endpoint::Default => "default".to_string(),
        Endpoint::NamedSession(name) => format!("session:{name}"),
        Endpoint::SocketPath(path) => format!("socket:{}", path.display()),
    }
}

fn current_workspace_id(env: &Env) -> Option<String> {
    env.all
        .get("HERDR_ACTIVE_WORKSPACE_ID")
        .or_else(|| env.all.get("HERDR_WORKSPACE_ID"))
        .filter(|value| !value.is_empty())
        .cloned()
}

fn attach_if_outside(
    env: &Env,
    launch_endpoint: &Endpoint,
    canonical_endpoint: &Endpoint,
) -> Result<()> {
    if attached_to_endpoint(env, canonical_endpoint) {
        return Ok(());
    }
    let mut command = Command::new("herdr");
    configure_endpoint_command(&mut command, launch_endpoint);
    let status = command
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status()
        .context("failed to attach the Herdr client")?;
    if !status.success() {
        bail!("Herdr client exited with status {status}.");
    }
    Ok(())
}

fn running_inside_herdr(env: &Env) -> bool {
    env.all.iter().any(|(key, value)| {
        !value.is_empty()
            && (key.starts_with("HERDR_ACTIVE_")
                || matches!(
                    key.as_str(),
                    "HERDR_ENV" | "HERDR_WORKSPACE_ID" | "HERDR_TAB_ID" | "HERDR_PANE_ID"
                ))
    })
}

fn attached_to_endpoint(env: &Env, canonical_endpoint: &Endpoint) -> bool {
    if !running_inside_herdr(env) {
        return false;
    }
    let Endpoint::SocketPath(target_socket) = canonical_endpoint else {
        return false;
    };
    ["HERDR_SOCKET_PATH", "HERDR_CLIENT_SOCKET_PATH"]
        .into_iter()
        .filter_map(|key| env.all.get(key))
        .filter(|value| !value.is_empty())
        .map(|socket| std::fs::canonicalize(socket).unwrap_or_else(|_| PathBuf::from(socket)))
        .any(|socket| socket.as_path() == target_socket.as_path())
}

fn canonical_socket_endpoint(socket: impl AsRef<Path>) -> Endpoint {
    let socket = socket.as_ref();
    Endpoint::SocketPath(std::fs::canonicalize(socket).unwrap_or_else(|_| socket.to_path_buf()))
}

fn require_managed_endpoint_identity(
    managed: &ManagedWorkspace,
    resolved_socket: impl AsRef<Path>,
) -> Result<()> {
    let resolved = canonical_socket_endpoint(resolved_socket);
    if resolved != managed.endpoint {
        bail!(
            "refusing to stop managed Herdr project `{}`: its saved endpoint {} now resolves to {}",
            managed.project_name,
            describe_endpoint(&managed.endpoint),
            describe_endpoint(&resolved)
        );
    }
    Ok(())
}

fn configure_endpoint_command(command: &mut Command, endpoint: &Endpoint) {
    command.env_remove("HERDR_SOCKET_PATH");
    command.env_remove("HERDR_CLIENT_SOCKET_PATH");
    command.env_remove("HERDR_SESSION");
    match endpoint {
        Endpoint::Default => {}
        Endpoint::NamedSession(name) => {
            command.args(["--session", name]);
        }
        Endpoint::SocketPath(path) => {
            command.env("HERDR_SOCKET_PATH", path);
        }
    }
}

fn rollback_created(
    client: &Herdr<ProcessRunner>,
    state: &StateStore,
    endpoint: &Endpoint,
    workspace_id: &str,
) -> Option<String> {
    let mut failures = Vec::new();
    match client.close_workspace(workspace_id) {
        Ok(()) => {
            if let Err(error) = state.update(|index| {
                index.remove_exact_id(endpoint, workspace_id);
                Ok(())
            }) {
                failures.push(format!("state cleanup failed: {error}"));
            }
        }
        Err(error) => {
            failures.push(format!(
                "workspace close failed and ownership state was retained: {error}"
            ));
        }
    }
    if failures.is_empty() {
        None
    } else {
        Some(format!("rollback was incomplete ({})", failures.join("; ")))
    }
}

fn rollback_appended_tabs(client: &Herdr<ProcessRunner>, tab_ids: &[String]) -> Option<String> {
    let failures = tab_ids
        .iter()
        .rev()
        .filter_map(|tab_id| {
            client
                .close_tab(tab_id)
                .err()
                .map(|error| format!("tab {tab_id}: {error}"))
        })
        .collect::<Vec<_>>();
    if failures.is_empty() {
        None
    } else {
        Some(format!(
            "append rollback was incomplete ({})",
            failures.join("; ")
        ))
    }
}

fn load_from_params(env: &Env, params: &StartParams) -> Result<LoadedSpec> {
    let (settings, args) = parse_settings(params.args.clone());
    Ok(LoadedSpec {
        spec: load_spec(
            env,
            params.project.as_deref(),
            params.project_config.as_deref(),
            &settings,
            &args,
            params.load_options(),
        )?,
    })
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
    let content = config::read_project_file(env, &file)?;
    ProjectSpec::load(&file, &content, settings, args, opts, env, Backend::Herdr)
}

fn load_stop_hook(env: &Env, managed: &ManagedWorkspace) -> Option<Result<Option<String>>> {
    if !managed.config_path.exists() {
        return None;
    }
    Some(
        std::fs::read_to_string(&managed.config_path)
            .map_err(anyhow::Error::from)
            .and_then(|content| template::render_config(&content, &HashMap::new(), &[], env))
            .and_then(|rendered| {
                let mut yaml = parse(&rendered)
                    .map_err(|error| anyhow!("Failed to parse config file: {error}"))?;
                yaml.apply_merge()
                    .map_err(|error| anyhow!("Failed to parse config file: {error}"))?;
                Ok(join_or_string(get(&yaml, HOOK_ON_PROJECT_STOP), "; "))
            }),
    )
}

fn stop_all_hook(env: &Env, managed: &ManagedWorkspace) -> Result<Option<String>> {
    match &managed.stop_hook {
        Some(hook) => Ok(Some(hook.clone())),
        None => Ok(load_stop_hook(env, managed).transpose()?.flatten()),
    }
}

/// `None` is reserved for state written before stop hooks were snapshotted.
/// New state records an empty string when no hook exists, preventing stop-all
/// from executing a hook added to the config after the workspace was started.
fn stop_hook_snapshot(spec: &ProjectSpec) -> Option<String> {
    Some(spec.hooks.stop.clone().unwrap_or_default())
}

fn project_config_name(env: &Env, managed: &ManagedWorkspace) -> String {
    config::config_file_basenames(env)
        .into_iter()
        .find(|name| {
            config::global_project(env, name)
                .and_then(|path| std::fs::canonicalize(path).ok())
                .as_deref()
                == Some(managed.config_path.as_path())
        })
        .unwrap_or_else(|| managed.project_name.clone())
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

fn render_layout_plan(layout: &Layout, indent: &str) {
    match layout {
        Layout::Pane(index) => println!("{indent}pane[{index}]"),
        Layout::Split {
            direction,
            ratio,
            first,
            second,
        } => {
            println!("{indent}split {direction} ratio={ratio:.4}");
            let child_indent = format!("{indent}  ");
            render_layout_plan(first, &child_indent);
            render_layout_plan(second, &child_indent);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn simple_window(layout: Option<&str>, pane_count: usize) -> WindowSpec {
        WindowSpec {
            name: Some("test".into()),
            root: "/tmp".into(),
            pre: None,
            commands: Vec::new(),
            panes: (0..pane_count)
                .map(|_| crate::spec::PaneSpec {
                    title: None,
                    commands: Vec::new(),
                    split: None,
                })
                .collect(),
            layout: layout.map(str::to_string),
            pane_chain: false,
            focused_pane: 0,
        }
    }

    #[test]
    fn named_layouts_and_tiled_have_the_configured_panes() {
        for layout in [
            None,
            Some("tiled"),
            Some("even-horizontal"),
            Some("even-vertical"),
            Some("main-horizontal"),
            Some("main-vertical"),
        ] {
            assert_eq!(
                layout_for(&simple_window(layout, 4)).unwrap().pane_count(),
                4
            );
        }
    }

    #[test]
    fn endpoint_precedence_is_socket_then_name_then_ambient() {
        let mut spec = ProjectSpec {
            source_path: "/tmp/a.yml".into(),
            name: "a".into(),
            root: "/tmp".into(),
            attach: false,
            append: false,
            socket_name: Some("named".into()),
            socket_path: Some("/tmp/explicit.sock".into()),
            startup_window: 0,
            startup_pane: None,
            pre_window: None,
            hooks: Default::default(),
            windows: vec![simple_window(None, 1)],
            warnings: Vec::new(),
        };
        let mut env = Env {
            cwd: "/work".into(),
            home: "/home/u".into(),
            ..Env::default()
        };
        env.all
            .insert("HERDR_SOCKET_PATH".into(), "/tmp/ambient.sock".into());
        assert_eq!(
            endpoint_for(&spec, &env).unwrap(),
            Endpoint::SocketPath("/tmp/explicit.sock".into())
        );
        spec.socket_path = None;
        assert_eq!(
            endpoint_for(&spec, &env).unwrap(),
            Endpoint::NamedSession("named".into())
        );
        spec.socket_name = None;
        assert_eq!(
            endpoint_for(&spec, &env).unwrap(),
            Endpoint::SocketPath("/tmp/ambient.sock".into())
        );
    }

    #[test]
    fn endpoint_commands_clear_every_ambient_selector_before_pinning() {
        use std::ffi::OsStr;

        let mut command = Command::new("herdr");
        configure_endpoint_command(&mut command, &Endpoint::NamedSession("work".into()));

        assert_eq!(
            command.get_args().collect::<Vec<_>>(),
            [OsStr::new("--session"), OsStr::new("work")]
        );
        let environment = command.get_envs().collect::<HashMap<_, _>>();
        assert_eq!(
            environment.get(OsStr::new("HERDR_SOCKET_PATH")),
            Some(&None)
        );
        assert_eq!(
            environment.get(OsStr::new("HERDR_CLIENT_SOCKET_PATH")),
            Some(&None)
        );
        assert_eq!(environment.get(OsStr::new("HERDR_SESSION")), Some(&None));
    }

    #[test]
    fn socket_selection_alone_does_not_claim_an_attached_herdr_client() {
        let mut env = Env::default();
        env.all
            .insert("HERDR_SOCKET_PATH".into(), "/tmp/herdr.sock".into());
        assert!(!running_inside_herdr(&env));

        env.all.insert("HERDR_PANE_ID".into(), "w:t:p".into());
        assert!(running_inside_herdr(&env));
        assert!(attached_to_endpoint(
            &env,
            &Endpoint::SocketPath("/tmp/herdr.sock".into())
        ));
        assert!(!attached_to_endpoint(
            &env,
            &Endpoint::SocketPath("/tmp/other.sock".into())
        ));
    }

    #[test]
    fn stop_all_endpoint_identity_check_fails_closed_on_selector_drift() {
        let managed = ManagedWorkspace {
            endpoint: Endpoint::SocketPath("/tmp/original.sock".into()),
            launch_endpoint: Some(Endpoint::NamedSession("work".into())),
            workspace_id: "workspace-1".into(),
            label: "work".into(),
            root_cwd: "/tmp/work".into(),
            config_path: "/tmp/work.yml".into(),
            project_name: "work".into(),
            root_pane_id: None,
            stop_hook: None,
        };
        require_managed_endpoint_identity(&managed, "/tmp/original.sock").unwrap();
        let error = require_managed_endpoint_identity(&managed, "/tmp/rebound.sock")
            .unwrap_err()
            .to_string();
        assert!(error.contains("refusing to stop"));
        assert!(error.contains("original.sock"));
        assert!(error.contains("rebound.sock"));
    }

    #[test]
    fn regular_stop_identity_check_rejects_wrong_template_settings() {
        let managed = ManagedWorkspace {
            endpoint: Endpoint::Default,
            launch_endpoint: None,
            workspace_id: "workspace-1".into(),
            label: "correct".into(),
            root_cwd: "/tmp/correct".into(),
            config_path: "/tmp/work.yml".into(),
            project_name: "correct".into(),
            root_pane_id: None,
            stop_hook: None,
        };
        let mut spec = ProjectSpec {
            source_path: "/tmp/work.yml".into(),
            name: "correct".into(),
            root: "/tmp/correct".into(),
            attach: false,
            append: false,
            socket_name: None,
            socket_path: None,
            startup_window: 0,
            startup_pane: None,
            pre_window: None,
            hooks: Default::default(),
            windows: vec![simple_window(None, 1)],
            warnings: Vec::new(),
        };
        require_managed_project_identity(&managed, &spec).unwrap();

        spec.name = "wrong-setting".into();
        let error = require_managed_project_identity(&managed, &spec)
            .unwrap_err()
            .to_string();
        assert!(error.contains("identity-mismatched"));
        assert!(error.contains("same template settings"));

        spec.name = "correct".into();
        spec.root = "/tmp/wrong-root".into();
        assert!(require_managed_project_identity(&managed, &spec).is_err());

        spec.root = "/tmp/correct".into();
        spec.hooks.stop = Some("echo changed".into());
        let managed_with_hook = ManagedWorkspace {
            stop_hook: Some("echo original".into()),
            ..managed.clone()
        };
        let error = require_managed_stop_hook_snapshot(&managed_with_hook, &spec)
            .unwrap_err()
            .to_string();
        assert!(error.contains("unverified hook"));
        assert!(error.contains("same template settings"));

        let index = StateIndex {
            managed_workspaces: vec![managed],
            ..StateIndex::default()
        };
        let wrong_endpoint = Endpoint::SocketPath("/tmp/wrong.sock".into());
        let error = require_no_managed_config_on_other_endpoint(&index, &wrong_endpoint, &spec)
            .unwrap_err()
            .to_string();
        assert!(error.contains("same socket settings"));
        assert!(error.contains("refusing a silent no-op"));
    }

    #[test]
    fn an_absent_stop_hook_is_still_a_persisted_snapshot() {
        let spec = ProjectSpec {
            source_path: "/tmp/work.yml".into(),
            name: "work".into(),
            root: "/tmp/work".into(),
            attach: false,
            append: false,
            socket_name: None,
            socket_path: None,
            startup_window: 0,
            startup_pane: None,
            pre_window: None,
            hooks: Default::default(),
            windows: vec![simple_window(None, 1)],
            warnings: Vec::new(),
        };
        assert_eq!(stop_hook_snapshot(&spec), Some(String::new()));

        let temp = tempfile::tempdir().unwrap();
        let config_path = temp.path().join("work.yml");
        std::fs::write(
            &config_path,
            "name: work\non_project_stop: echo newly-added\nwindows:\n  - app: true\n",
        )
        .unwrap();
        let managed = ManagedWorkspace {
            endpoint: Endpoint::Default,
            launch_endpoint: None,
            workspace_id: "workspace-1".into(),
            label: "work".into(),
            root_cwd: temp.path().into(),
            config_path,
            project_name: "work".into(),
            root_pane_id: None,
            stop_hook: Some(String::new()),
        };
        assert_eq!(
            stop_all_hook(&Env::default(), &managed).unwrap(),
            Some(String::new())
        );

        let legacy = ManagedWorkspace {
            stop_hook: None,
            ..managed
        };
        assert_eq!(
            stop_all_hook(&Env::default(), &legacy).unwrap(),
            Some("echo newly-added".into())
        );
    }

    #[test]
    fn unrepresentable_ratios_fail_instead_of_clamping() {
        let layout = Layout::Split {
            direction: SplitDirection::Right,
            ratio: 0.05,
            first: Box::new(Layout::Pane(0)),
            second: Box::new(Layout::Pane(1)),
        };
        assert!(validate_herdr_ratios(&layout)
            .unwrap_err()
            .to_string()
            .contains("cannot be represented"));
    }

    #[test]
    fn preflight_rejects_a_late_invalid_layout_before_lifecycle_work() {
        let payload = "100x10,0,0{5x10,0,0,8,94x10,6,0,4}";
        let serialized = format!(
            "{:04x},{payload}",
            crate::layout::tmux_layout_checksum(payload)
        );
        let spec = ProjectSpec {
            source_path: "/tmp/a.yml".into(),
            name: "a".into(),
            root: "/tmp".into(),
            attach: false,
            append: false,
            socket_name: None,
            socket_path: None,
            startup_window: 0,
            startup_pane: None,
            pre_window: None,
            hooks: Default::default(),
            windows: vec![simple_window(None, 1), simple_window(Some(&serialized), 2)],
            warnings: Vec::new(),
        };

        assert!(preflight_spec(&spec)
            .unwrap_err()
            .to_string()
            .contains("cannot be represented"));
    }
}
