use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{bail, Result};

use crate::env::Env;
use crate::util::expand_path;

pub const LOCAL_DEFAULTS: [&str; 2] = ["./.tmuxinator.yml", "./.tmuxinator.yaml"];
pub const NO_LOCAL_FILE_MSG: &str = "Project file at ./.tmuxinator.yml doesn't exist.";
pub const NO_PROJECT_FOUND_MSG: &str = "Project could not be found.";

pub fn home_dir(env: &Env) -> String {
    format!("{}/.tmuxinator", env.home)
}

pub fn xdg_dir(env: &Env) -> String {
    let xdg_config = env.xdg_config_home.as_deref().unwrap_or("~/.config");
    let config_home = expand_path(xdg_config, &env.cwd.to_string_lossy(), &env.home);
    format!("{config_home}/tmuxinator")
}

// $TMUXINATOR_CONFIG, created on access when set (matches Ruby's
// Config.environment side effect: once set, it always exists and wins).
pub fn environment_dir(env: &Env) -> Option<String> {
    let value = env.tmuxinator_config.as_deref().unwrap_or("");
    if value.is_empty() {
        return None;
    }
    if !Path::new(value).is_dir() {
        fs::create_dir_all(value).ok();
    }
    Some(value.to_string())
}

pub fn directory(env: &Env) -> String {
    if let Some(dir) = environment_dir(env) {
        if Path::new(&dir).is_dir() {
            return dir;
        }
    }
    let xdg = xdg_dir(env);
    if Path::new(&xdg).is_dir() {
        return xdg;
    }
    let home = home_dir(env);
    if Path::new(&home).is_dir() {
        return home;
    }
    fs::create_dir_all(&xdg).ok();
    xdg
}

pub fn local_project(env: &Env) -> Option<String> {
    LOCAL_DEFAULTS
        .iter()
        .find(|file| env.cwd.join(file).exists())
        .map(|file| file.to_string())
}

pub fn default_project(env: &Env, name: &str) -> String {
    format!("{}/{name}.yml", directory(env))
}

fn project_in(dir: &str, name: &str) -> Option<String> {
    if dir.is_empty() || !Path::new(dir).is_dir() {
        return None;
    }
    let mut candidates: Vec<PathBuf> = walkdir::WalkDir::new(dir)
        .follow_links(true)
        .into_iter()
        .filter_map(|entry| entry.ok())
        .filter(|entry| entry.file_type().is_file())
        .map(|entry| entry.into_path())
        .filter(|path| {
            matches!(
                path.extension().and_then(|e| e.to_str()),
                Some("yml") | Some("yaml")
            )
        })
        .collect();
    candidates.sort();
    candidates
        .into_iter()
        .find(|path| path.file_stem().map(|stem| stem == name).unwrap_or(false))
        .map(|path| path.to_string_lossy().into_owned())
}

pub fn global_project(env: &Env, name: &str) -> Option<String> {
    environment_dir(env)
        .and_then(|dir| project_in(&dir, name))
        .or_else(|| project_in(&xdg_dir(env), name))
        .or_else(|| project_in(&home_dir(env), name))
}

pub fn project(env: &Env, name: &str) -> String {
    global_project(env, name)
        .or_else(|| local_project(env))
        .unwrap_or_else(|| default_project(env, name))
}

pub fn project_exists(env: &Env, name: &str) -> bool {
    let path = project(env, name);
    resolve(env, &path).exists()
}

fn resolve(env: &Env, path: &str) -> PathBuf {
    let candidate = PathBuf::from(path);
    if candidate.is_absolute() {
        candidate
    } else {
        env.cwd.join(candidate)
    }
}

// Search-ordered existing config directories; used by `implode` and `list`.
pub fn directories(env: &Env) -> Vec<String> {
    if let Some(dir) = environment_dir(env) {
        if Path::new(&dir).is_dir() {
            return vec![dir];
        }
    }
    [xdg_dir(env), home_dir(env)]
        .into_iter()
        .filter(|dir| Path::new(dir).is_dir())
        .collect()
}

// Quirk preserved from Ruby: listing only picks up `.yml` files (lookup
// accepts `.yaml` too), and every ".yml" occurrence in the relative path
// is stripped, not just the extension.
pub fn config_file_basenames(env: &Env) -> Vec<String> {
    let mut names: Vec<String> = directories(env)
        .iter()
        .flat_map(|dir| {
            walkdir::WalkDir::new(dir)
                .follow_links(true)
                .into_iter()
                .filter_map(|entry| entry.ok())
                .filter(|entry| entry.file_type().is_file())
                .map(|entry| entry.into_path())
                .filter(|path| path.extension().and_then(|e| e.to_str()) == Some("yml"))
                .map(|path| {
                    path.to_string_lossy()
                        .replacen(&format!("{dir}/"), "", 1)
                        .replace(".yml", "")
                })
                .collect::<Vec<_>>()
        })
        .collect();
    names.sort();
    names
}

pub fn configs(env: &Env, active_sessions: Option<(bool, &[String])>) -> Vec<String> {
    let names = config_file_basenames(env);
    match active_sessions {
        Some((true, sessions)) => {
            let mut seen = std::collections::HashSet::new();
            names
                .into_iter()
                .filter(|name| sessions.contains(name) && seen.insert(name.clone()))
                .collect()
        }
        Some((false, sessions)) => names
            .into_iter()
            .filter(|name| !sessions.contains(name))
            .collect(),
        None => names,
    }
}

pub struct ProjectFileQuery<'a> {
    pub name: Option<&'a str>,
    pub project_config: Option<&'a str>,
}

// Precedence port of Config.validate: -p path, then local file (only when
// no name was given), then a named project.
pub fn find_project_file(env: &Env, query: &ProjectFileQuery) -> Result<String> {
    if let Some(path) = query.project_config {
        if !resolve(env, path).exists() {
            bail!("Project config ({path}) doesn't exist.");
        }
        return Ok(path.to_string());
    }
    match query.name {
        None => local_project(env).ok_or_else(|| anyhow::anyhow!(NO_LOCAL_FILE_MSG)),
        Some(name) => {
            if !project_exists(env, name) {
                bail!("Project {name} doesn't exist.");
            }
            Ok(project(env, name))
        }
    }
}

pub fn read_project_file(env: &Env, path: &str) -> Result<String> {
    Ok(fs::read_to_string(resolve(env, path))?)
}
