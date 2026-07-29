pub mod info;
pub mod manage;
pub mod run;

use anyhow::Result;

use crate::cli::Command;
use crate::env::Env;
use crate::tmux::TmuxContext;

fn attach_option(attach: bool, no_attach: bool) -> Option<bool> {
    match (attach, no_attach) {
        (true, _) => Some(true),
        (_, true) => Some(false),
        _ => None,
    }
}

pub fn dispatch(command: Command, env: &Env, ctx: &dyn TmuxContext) -> Result<()> {
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
        } => run::start(
            env,
            ctx,
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
        ),
        Command::Debug {
            project,
            args,
            attach,
            no_attach,
            name,
            project_config,
            append,
            no_pre_window,
        } => run::debug(
            env,
            ctx,
            run::StartParams {
                project,
                args,
                attach: attach_option(attach, no_attach),
                custom_name: name,
                project_config,
                append,
                no_pre_window,
            },
        ),
        Command::Stop {
            project,
            project_config,
            suppress_tmux_version_warning,
        } => run::stop(
            env,
            ctx,
            project,
            project_config,
            suppress_tmux_version_warning,
        ),
        Command::StopAll { noconfirm } => run::stop_all(env, ctx, noconfirm),
        Command::Local {
            suppress_tmux_version_warning,
        } => run::local(env, ctx, suppress_tmux_version_warning),
        Command::New {
            name,
            session,
            local,
        } => manage::new(env, ctx, &name, session.as_deref(), local),
        Command::Open { name, local } => manage::open(env, &name, local),
        Command::Edit { name, local } => manage::edit(env, name.as_deref(), local),
        Command::Copy { existing, new } => manage::copy(env, &existing, &new),
        Command::Delete { projects } => manage::delete(env, &projects),
        Command::Implode => manage::implode(env),
        Command::List { newline, active } => info::list(env, ctx, newline, active),
        Command::Version => info::version(),
        Command::Doctor => info::doctor(env),
        Command::Completions { arg } => info::completions(env, &arg),
        Command::Commands { shell } => info::commands(shell.as_deref()),
    }
}
