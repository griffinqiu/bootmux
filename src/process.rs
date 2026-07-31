//! Injectable child-process boundary shared by the structured backends.
//!
//! Both the Herdr and zellij adapters drive their multiplexer through a real
//! command-line binary. Routing every spawn through [`CommandRunner`] keeps the
//! argv, environment, and working directory of each call assertable in tests
//! without a live server.

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::io;
use std::path::PathBuf;
use std::process::{Command, Stdio};

/// A process invocation independent of `std::process::Command`, suitable for
/// deterministic tests.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Invocation {
    pub program: PathBuf,
    pub args: Vec<OsString>,
    /// `Some(value)` sets a variable and `None` removes it.
    pub env: BTreeMap<OsString, Option<OsString>>,
    pub current_dir: Option<PathBuf>,
}

impl Invocation {
    pub fn display(&self) -> String {
        let mut fields = Vec::with_capacity(self.args.len() + 1);
        fields.push(self.program.to_string_lossy().into_owned());
        fields.extend(
            self.args
                .iter()
                .map(|arg| format!("{:?}", arg.to_string_lossy())),
        );
        fields.join(" ")
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CommandOutput {
    pub success: bool,
    pub code: Option<i32>,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
}

/// Injectable boundary around all child-process interaction.
pub trait CommandRunner: Send + Sync {
    fn run(&self, invocation: &Invocation) -> io::Result<CommandOutput>;
    fn spawn_detached(&self, invocation: &Invocation) -> io::Result<u32>;
}

#[derive(Clone, Copy, Debug, Default)]
pub struct ProcessRunner;

impl ProcessRunner {
    fn command(invocation: &Invocation) -> Command {
        let mut command = Command::new(&invocation.program);
        command.args(&invocation.args);
        if let Some(current_dir) = &invocation.current_dir {
            command.current_dir(current_dir);
        }
        for (key, value) in &invocation.env {
            match value {
                Some(value) => {
                    command.env(key, value);
                }
                None => {
                    command.env_remove(key);
                }
            }
        }
        command
    }
}

impl CommandRunner for ProcessRunner {
    fn run(&self, invocation: &Invocation) -> io::Result<CommandOutput> {
        let output = Self::command(invocation).output()?;
        Ok(CommandOutput {
            success: output.status.success(),
            code: output.status.code(),
            stdout: output.stdout,
            stderr: output.stderr,
        })
    }

    fn spawn_detached(&self, invocation: &Invocation) -> io::Result<u32> {
        let mut command = Self::command(invocation);
        command
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        detach_command(&mut command);
        command.spawn().map(|child| child.id())
    }
}

#[cfg(unix)]
fn detach_command(command: &mut Command) {
    use std::os::unix::process::CommandExt;

    unsafe extern "C" {
        fn setsid() -> i32;
    }

    // SAFETY: `pre_exec` only calls the async-signal-safe `setsid(2)` and
    // creates no Rust-owned state in the child between fork and exec.
    unsafe {
        command.pre_exec(|| {
            if setsid() == -1 {
                Err(io::Error::last_os_error())
            } else {
                Ok(())
            }
        });
    }
}

#[cfg(windows)]
fn detach_command(command: &mut Command) {
    use std::os::windows::process::CommandExt;

    const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
    const DETACHED_PROCESS: u32 = 0x0000_0008;
    command.creation_flags(CREATE_NEW_PROCESS_GROUP | DETACHED_PROCESS);
}

#[cfg(not(any(unix, windows)))]
fn detach_command(_command: &mut Command) {}
