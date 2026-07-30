pub mod info;
pub mod manage;
pub mod run;

use anyhow::Result;

use crate::cli::{Command, ConfigAction};
use crate::env::Env;
use crate::settings::{self, Backend};
use crate::tmux::TmuxContext;

fn attach_option(attach: bool, no_attach: bool) -> Option<bool> {
    match (attach, no_attach) {
        (true, _) => Some(true),
        (_, true) => Some(false),
        _ => None,
    }
}

fn selected_backend(explicit: Option<Backend>, env: &Env) -> Result<Backend> {
    let classifier = |env: &Env| crate::herdr_backend::classify_foreground_backend(env);
    settings::resolve_backend_with_classifier(explicit, env, Some(&classifier))
}

pub fn dispatch(command: Command, env: &Env, ctx: &dyn TmuxContext) -> Result<()> {
    dispatch_with_backend(command, None, env, ctx)
}

pub fn dispatch_with_backend(
    command: Command,
    explicit_backend: Option<Backend>,
    env: &Env,
    ctx: &dyn TmuxContext,
) -> Result<()> {
    match command {
        Command::Start {
            project,
            args,
            attach,
            no_attach,
            name,
            project_config,
            suppress_tmux_version_warning,
            append,
            no_pre_window,
        } => {
            let backend = selected_backend(explicit_backend, env)?;
            run::start_with_backend(
                env,
                ctx,
                backend,
                run::StartParams {
                    project,
                    args,
                    attach: attach_option(attach, no_attach),
                    custom_name: name,
                    project_config,
                    append,
                    no_pre_window,
                },
                suppress_tmux_version_warning,
            )
        }
        Command::Debug {
            project,
            args,
            attach,
            no_attach,
            name,
            project_config,
            append,
            no_pre_window,
        } => {
            let backend = selected_backend(explicit_backend, env)?;
            run::debug_with_backend(
                env,
                ctx,
                backend,
                run::StartParams {
                    project,
                    args,
                    attach: attach_option(attach, no_attach),
                    custom_name: name,
                    project_config,
                    append,
                    no_pre_window,
                },
            )
        }
        Command::Stop {
            project,
            args,
            project_config,
            suppress_tmux_version_warning,
        } => {
            let backend = selected_backend(explicit_backend, env)?;
            run::stop_with_backend(
                env,
                ctx,
                backend,
                project,
                project_config,
                args,
                suppress_tmux_version_warning,
            )
        }
        Command::StopAll { noconfirm } => run::stop_all_with_backend(
            env,
            ctx,
            selected_backend(explicit_backend, env)?,
            noconfirm,
        ),
        Command::Local {
            suppress_tmux_version_warning,
        } => run::local_with_backend(
            env,
            ctx,
            selected_backend(explicit_backend, env)?,
            suppress_tmux_version_warning,
        ),
        Command::New {
            name,
            session,
            local,
        } => {
            if session.is_some() && selected_backend(explicit_backend, env)? == Backend::Herdr {
                anyhow::bail!(
                    "`bootmux new NAME SESSION` introspects tmux and is only available with `--backend tmux`."
                );
            }
            manage::new(env, ctx, &name, session.as_deref(), local)
        }
        Command::Open { name, local } => manage::open(env, &name, local),
        Command::Edit { name, local } => manage::edit(env, name.as_deref(), local),
        Command::Copy { existing, new } => manage::copy(env, &existing, &new),
        Command::Delete { projects } => manage::delete(env, &projects),
        Command::Implode => manage::implode(env),
        Command::List { newline, active } => {
            let backend = if active {
                Some(selected_backend(explicit_backend, env)?)
            } else {
                None
            };
            info::list_with_backend(env, ctx, backend, newline, active)
        }
        Command::Version => info::version(),
        Command::Doctor => info::doctor_with_backend(env, selected_backend(explicit_backend, env)?),
        Command::Picker => {
            let backend = selected_backend(explicit_backend, env)?;
            if let Some(project) = crate::picker::pick_project(&crate::config::configs(env, None))?
            {
                run::start_with_backend(
                    env,
                    ctx,
                    backend,
                    run::StartParams {
                        project: Some(project),
                        args: Vec::new(),
                        attach: None,
                        custom_name: None,
                        project_config: None,
                        append: false,
                        no_pre_window: false,
                    },
                    false,
                )
            } else {
                Ok(())
            }
        }
        Command::Bindings { backend, key } => {
            let backend: Backend = backend.parse()?;
            let snippet = match (backend, key.as_deref()) {
                (Backend::Tmux, Some(key)) => {
                    crate::bindings::tmux_snippet_with(key, crate::bindings::PICKER_COMMAND)?
                }
                (Backend::Herdr, Some(key)) => crate::bindings::herdr_snippet_with(
                    key,
                    crate::bindings::PICKER_COMMAND,
                    crate::bindings::DEFAULT_HERDR_POPUP_SIZE,
                    crate::bindings::DEFAULT_HERDR_POPUP_SIZE,
                )?,
                (_, None) => crate::bindings::snippet(backend),
            };
            print!("{snippet}");
            Ok(())
        }
        Command::Config { action } => match action {
            ConfigAction::Get { key } => {
                let key = key.replace('-', "_");
                if let Some(value) = settings::get(env, &key)? {
                    println!("{value}");
                }
                Ok(())
            }
            ConfigAction::Set { key, value } => {
                settings::set(env, &key.replace('-', "_"), &value)?;
                println!("{} = \"{}\"", key.replace('-', "_"), value);
                Ok(())
            }
            ConfigAction::Path => {
                println!("{}", settings::path(env)?.display());
                Ok(())
            }
        },
        Command::Completions { arg } => info::completions(env, &arg),
        Command::Commands { shell } => info::commands(shell.as_deref()),
    }
}
