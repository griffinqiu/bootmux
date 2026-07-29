use std::collections::HashMap;
use std::os::unix::process::CommandExt;

use anyhow::{anyhow, Result};

use crate::config::{self, ProjectFileQuery};
use crate::env::Env;
use crate::project::{parse_settings, LoadOptions, Project};
use crate::script;
use crate::tmux::{TmuxContext, UNSUPPORTED_VERSION_MSG};
use crate::util::{ask_yes, press_enter_to_continue, say_colored, Color};

pub struct StartParams {
    pub project: Option<String>,
    pub args: Vec<String>,
    pub attach: Option<bool>,
    pub custom_name: Option<String>,
    pub project_config: Option<String>,
    pub append: bool,
    pub no_pre_window: bool,
}

impl StartParams {
    // Ruby start_params: -p takes precedence; a positional name given
    // alongside it is shifted into the template args.
    fn normalized(mut self) -> StartParams {
        if self.project_config.is_some() {
            if let Some(name) = self.project.take() {
                self.args.insert(0, name);
            }
        }
        self
    }

    fn load_options(&self) -> LoadOptions {
        LoadOptions {
            custom_name: self.custom_name.clone(),
            force_attach: self.attach == Some(true),
            force_detach: self.attach == Some(false),
            append: self.append,
            no_pre_window: self.no_pre_window,
        }
    }
}

pub fn create_project<'a>(
    env: &'a Env,
    ctx: &'a dyn TmuxContext,
    name: Option<&str>,
    project_config: Option<&str>,
    settings: &HashMap<String, String>,
    args: &[String],
    opts: LoadOptions,
) -> Result<Project<'a>> {
    let file = config::find_project_file(
        env,
        &ProjectFileQuery {
            name,
            project_config,
        },
    )?;
    let content = config::read_project_file(env, &file)?;
    Project::load(&content, settings, args, opts, ctx, env)
}

fn create_from_params<'a>(
    env: &'a Env,
    ctx: &'a dyn TmuxContext,
    params: &StartParams,
) -> Result<Project<'a>> {
    let (settings, args) = parse_settings(params.args.clone());
    create_project(
        env,
        ctx,
        params.project.as_deref(),
        params.project_config.as_deref(),
        &settings,
        &args,
        params.load_options(),
    )
}

fn warn_unsupported_version(ctx: &dyn TmuxContext, suppress: bool) {
    if suppress {
        return;
    }
    let supported = ctx.version().map(|v| v.supported()).unwrap_or(false);
    if !supported {
        say_colored(UNSUPPORTED_VERSION_MSG, Color::Red);
        press_enter_to_continue();
    }
}

fn exec_script(script: String) -> Result<()> {
    let error = std::process::Command::new("/bin/sh")
        .arg("-c")
        .arg(script)
        .exec();
    Err(anyhow!("failed to execute generated script: {error}"))
}

pub fn start(
    env: &Env,
    ctx: &dyn TmuxContext,
    params: StartParams,
    suppress_version_warning: bool,
) -> Result<()> {
    let params = params.normalized();
    warn_unsupported_version(ctx, suppress_version_warning);
    let project = create_from_params(env, ctx, &params)?;
    exec_script(script::render_start(&project))
}

pub fn debug(env: &Env, ctx: &dyn TmuxContext, params: StartParams) -> Result<()> {
    let params = params.normalized();
    let project = create_from_params(env, ctx, &params)?;
    print!("{}", script::render_start(&project));
    Ok(())
}

pub fn stop(
    env: &Env,
    ctx: &dyn TmuxContext,
    project: Option<String>,
    project_config: Option<String>,
    suppress_version_warning: bool,
) -> Result<()> {
    // -p takes precedence over a named project when both are provided.
    let name = if project_config.is_some() {
        None
    } else {
        project
    };
    warn_unsupported_version(ctx, suppress_version_warning);
    let project = create_project(
        env,
        ctx,
        name.as_deref(),
        project_config.as_deref(),
        &HashMap::new(),
        &[],
        LoadOptions::default(),
    )?;
    exec_script(script::render_stop(&project))
}

pub fn local(env: &Env, ctx: &dyn TmuxContext, suppress_version_warning: bool) -> Result<()> {
    warn_unsupported_version(ctx, suppress_version_warning);
    let project = create_project(
        env,
        ctx,
        None,
        None,
        &HashMap::new(),
        &[],
        LoadOptions::default(),
    )?;
    exec_script(script::render_start(&project))
}

pub fn stop_all(env: &Env, ctx: &dyn TmuxContext, noconfirm: bool) -> Result<()> {
    let sessions = ctx.active_sessions();
    let active_configs = config::configs(env, Some((true, &sessions)));

    if !noconfirm {
        say_colored("Stop all active projects:\n", Color::Yellow);
        println!("{}", active_configs.join("\n"));
        println!();
        if !ask_yes("Are you sure? (n/y)") {
            return Ok(());
        }
    }

    let mut projects = Vec::new();
    for name in &active_configs {
        projects.push(create_project(
            env,
            ctx,
            Some(name),
            None,
            &HashMap::new(),
            &[],
            LoadOptions::default(),
        )?);
    }

    // Kill the session we are currently inside last, so the loop is not
    // cut short by detaching ourselves (Ruby Project.stop_all).
    let current_session = ctx.current_session_name(env);
    projects.sort_by_key(|project| project.name().as_deref() == Some(current_session.as_str()));

    for project in projects {
        std::process::Command::new("/bin/sh")
            .arg("-c")
            .arg(script::render_stop(&project))
            .status()?;
    }
    Ok(())
}
