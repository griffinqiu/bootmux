use std::fs;
use std::path::Path;
use std::process::Command;

use anyhow::{anyhow, bail, Result};
use serde_norway::{Mapping, Value};

use crate::config;
use crate::env::Env;
use crate::template;
use crate::tmux::TmuxContext;
use crate::util::{ask_yes, exit_with_message, say_colored, Color};

const SAMPLE_CONFIG: &str = include_str!("../../assets/sample.yml");

fn config_path(env: &Env, name: &str, local: bool) -> String {
    if local {
        config::LOCAL_DEFAULTS[0].to_string()
    } else {
        config::default_project(env, name)
    }
}

fn open_in_editor(env: &Env, path: &str) -> Result<()> {
    let status = Command::new("sh")
        .arg("-c")
        .arg(format!("$EDITOR {path}"))
        .status();
    if !status.map(|s| s.success()).unwrap_or(false) {
        super::info::doctor_environment(env)?;
    }
    Ok(())
}

fn generate_project_file(env: &Env, name: &str, path: &str) -> Result<()> {
    let template_source = config::global_project(env, "default")
        .and_then(|default_path| fs::read_to_string(default_path).ok())
        .unwrap_or_else(|| SAMPLE_CONFIG.to_string());
    let rendered = template::render_sample(&template_source, name, path)?;
    if let Some(parent) = Path::new(path).parent() {
        fs::create_dir_all(parent).ok();
    }
    fs::write(path, rendered)?;
    Ok(())
}

fn create_or_open(env: &Env, name: &str, local: bool) -> Result<()> {
    let path = config_path(env, name, local);
    if !Path::new(&path).exists() {
        generate_project_file(env, name, &path)?;
    }
    open_in_editor(env, &path)
}

pub fn new(
    env: &Env,
    ctx: &dyn TmuxContext,
    name: &str,
    session: Option<&str>,
    local: bool,
) -> Result<()> {
    match session {
        Some(session) => new_from_session(env, ctx, name, session, local),
        None => create_or_open(env, name, local),
    }
}

pub fn open(env: &Env, name: &str, local: bool) -> Result<()> {
    create_or_open(env, name, local)
}

pub fn edit(env: &Env, name: Option<&str>, local: bool) -> Result<()> {
    if name.is_none() && !local {
        bail!("`bootmux edit` requires a project name (or --local).");
    }
    let path = config_path(env, name.unwrap_or_default(), local);
    if Path::new(&path).exists() {
        open_in_editor(env, &path)
    } else if local && name.is_none() {
        exit_with_message(config::NO_LOCAL_FILE_MSG);
    } else {
        exit_with_message(&format!(
            "Project {} doesn't exist!",
            name.unwrap_or_default()
        ));
    }
}

pub fn copy(env: &Env, existing: &str, new: &str) -> Result<()> {
    let existing_path = config::project(env, existing);
    let new_path = config::project(env, new);

    if !config::project_exists(env, existing) {
        exit_with_message(&format!("Project {existing} doesn't exist!"));
    }

    let new_exists = config::project_exists(env, new);
    if !new_exists
        || ask_yes(&format!(
            "{new} already exists, would you like to overwrite it?"
        ))
    {
        if new_exists {
            println!("Overwriting {new}");
        }
        fs::copy(&existing_path, &new_path)?;
    }

    open_in_editor(env, &new_path)
}

pub fn delete(env: &Env, projects: &[String]) -> Result<()> {
    for project in projects {
        if config::project_exists(env, project) {
            let path = config::project(env, project);
            if ask_yes(&format!("Are you sure you want to delete {project}?(y/n)")) {
                fs::remove_file(&path)?;
                println!("Deleted {project}");
            }
        } else {
            println!("{project} does not exist!");
        }
    }
    Ok(())
}

pub fn implode(env: &Env) -> Result<()> {
    if ask_yes("Are you sure you want to delete all bootmux projects?") {
        for directory in config::directories(env) {
            fs::remove_dir_all(&directory)?;
        }
        say_colored("Deleted all bootmux projects.", Color::Green);
    }
    Ok(())
}

// Port of Ruby new_project_with_session: introspect a running session and
// serialize it as a project file.
fn new_from_session(
    env: &Env,
    ctx: &dyn TmuxContext,
    name: &str,
    session: &str,
    local: bool,
) -> Result<()> {
    let session_missing = || anyhow!("Session '{session}' doesn't exist.");

    let windows_output = ctx
        .capture(&format!(
            "tmux list-windows -t {session} -F \"#W #{{window_layout}} #{{window_active}} #{{pane_current_path}}\""
        ))
        .map_err(|_| session_missing())?;
    let panes_output = ctx
        .capture(&format!(
            "tmux list-panes -s -t {session} -F \"#W #{{pane_current_path}}\""
        ))
        .map_err(|_| session_missing())?;
    let options_output = ctx
        .capture(&format!("tmux show-options -t {session}"))
        .map_err(|_| session_missing())?;

    let mut project_root = options_output.lines().find_map(|line| {
        line.strip_prefix("default-path \"")
            .and_then(|rest| rest.strip_suffix('"'))
            .map(str::to_string)
    });

    let mut panes_by_window: std::collections::HashMap<&str, Vec<&str>> =
        std::collections::HashMap::new();
    for line in panes_output.lines() {
        let mut parts = line.split_whitespace();
        if let (Some(window), Some(path)) = (parts.next(), parts.next()) {
            panes_by_window.entry(window).or_default().push(path);
        }
    }

    let mut windows = Vec::new();
    for line in windows_output.lines() {
        let fields: Vec<&str> = line.split_whitespace().collect();
        let [window_name, layout, active, path, ..] = fields.as_slice() else {
            continue;
        };
        if project_root.is_none() && *active == "1" {
            project_root = Some(path.to_string());
        }
        let pane_commands: Vec<Value> = panes_by_window
            .get(window_name)
            .map(|paths| {
                paths
                    .iter()
                    .map(|pane_path| Value::String(format!("cd {pane_path}")))
                    .collect()
            })
            .unwrap_or_default();

        let mut window_options = Mapping::new();
        window_options.insert(
            Value::String("layout".to_string()),
            Value::String(layout.to_string()),
        );
        window_options.insert(
            Value::String("panes".to_string()),
            Value::Sequence(pane_commands),
        );
        let mut window_entry = Mapping::new();
        window_entry.insert(
            Value::String(window_name.to_string()),
            Value::Mapping(window_options),
        );
        windows.push(Value::Mapping(window_entry));
    }

    let mut yaml = Mapping::new();
    yaml.insert(
        Value::String("name".to_string()),
        Value::String(name.to_string()),
    );
    yaml.insert(
        Value::String("project_root".to_string()),
        project_root.map(Value::String).unwrap_or(Value::Null),
    );
    yaml.insert(
        Value::String("windows".to_string()),
        Value::Sequence(windows),
    );

    let path = config_path(env, name, local);
    fs::write(&path, serde_norway::to_string(&Value::Mapping(yaml))?)?;
    Ok(())
}
