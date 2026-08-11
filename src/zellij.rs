//! Typed command-line integration for zellij 0.44.
//!
//! 0.44 is the first release whose CLI can build and drive a session from
//! outside it: `attach --create-background` creates a detached session,
//! `new-tab` reports the tab it created, and `--pane-id` aims input at a
//! specific pane. Everything here goes through those documented commands
//! rather than zellij's internal socket protocol.

use std::ffi::{OsStr, OsString};
use std::fmt;
use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::de::DeserializeOwned;
use serde::Deserialize;

use crate::process::{CommandRunner, Invocation, ProcessRunner};

pub const MINIMUM_VERSION: ZellijVersion = ZellijVersion {
    major: 0,
    minor: 44,
    patch: 0,
};

/// zellij prints this instead of an empty list, and exits non-zero while doing
/// so, so it has to be recognized rather than treated as a failure.
const NO_SESSIONS_MARKER: &str = "No active zellij sessions";
/// Suffix zellij appends to sessions that are dead but still resurrectable.
/// They remain listed, so "the session exists" is not the same as "the session
/// is running".
const EXITED_MARKER: &str = "(EXITED";

/// A rendered layout, handed to zellij as a file.
///
/// Inline layouts (`--layout-string`) only exist from zellij 0.44.1 onwards,
/// while `--layout` has accepted a path for the whole supported range, so the
/// file keeps the advertised minimum honest.
///
/// zellij's server reads the file after the client has already returned, so the
/// caller must keep this alive until the topology it describes has settled.
pub struct LayoutFile {
    path: PathBuf,
}

impl LayoutFile {
    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn new(layout: &str) -> Result<Self> {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|elapsed| elapsed.as_nanos())
            .unwrap_or_default();
        let path = std::env::temp_dir().join(format!(
            "bootmux-layout-{}-{unique}.kdl",
            std::process::id()
        ));
        OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .and_then(|mut file| file.write_all(layout.as_bytes()))
            .map_err(|source| Error::LayoutFile {
                path: path.display().to_string(),
                source,
            })?;
        Ok(Self { path })
    }
}

impl Drop for LayoutFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug)]
pub enum Error {
    Spawn {
        invocation: String,
        source: std::io::Error,
    },
    CommandFailed {
        invocation: String,
        code: Option<i32>,
        output: String,
    },
    InvalidJson {
        invocation: String,
        source: serde_json::Error,
    },
    UnexpectedOutput {
        invocation: String,
        detail: String,
    },
    LayoutFile {
        path: String,
        source: std::io::Error,
    },
    UnparsableVersion {
        found: String,
    },
    UnsupportedVersion {
        found: ZellijVersion,
        minimum: ZellijVersion,
    },
}

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Spawn { invocation, source } => {
                write!(formatter, "failed to run `{invocation}`: {source}")
            }
            Self::CommandFailed {
                invocation,
                code,
                output,
            } => {
                let status = match code {
                    Some(code) => format!("exit status {code}"),
                    None => "a signal".to_string(),
                };
                write!(formatter, "`{invocation}` failed with {status}")?;
                if !output.is_empty() {
                    write!(formatter, ": {output}")?;
                }
                Ok(())
            }
            Self::InvalidJson { invocation, source } => {
                write!(formatter, "`{invocation}` returned invalid JSON: {source}")
            }
            Self::UnexpectedOutput { invocation, detail } => {
                write!(formatter, "`{invocation}` returned {detail}")
            }
            Self::LayoutFile { path, source } => {
                write!(
                    formatter,
                    "could not write the zellij layout {path:?}: {source}"
                )
            }
            Self::UnparsableVersion { found } => {
                write!(formatter, "could not read a zellij version from {found:?}")
            }
            Self::UnsupportedVersion { found, minimum } => write!(
                formatter,
                "zellij {found} is too old; bootmux needs zellij {minimum} or newer for \
                 background sessions and pane-targeted input"
            ),
        }
    }
}

impl std::error::Error for Error {}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub struct ZellijVersion {
    pub major: u32,
    pub minor: u32,
    pub patch: u32,
}

impl ZellijVersion {
    /// Reads a version out of `zellij --version` output, which looks like
    /// `zellij 0.44.3`.
    pub fn parse(output: &str) -> Option<Self> {
        let token = output
            .split_whitespace()
            .find(|token| token.starts_with(|character: char| character.is_ascii_digit()))?;
        // Tolerate pre-release and build metadata suffixes.
        let core = token
            .split(['-', '+'])
            .next()
            .unwrap_or(token)
            .trim_start_matches('v');
        let mut parts = core.split('.');
        let mut next = || parts.next().and_then(|part| part.parse::<u32>().ok());
        let major = next()?;
        let minor = next()?;
        let patch = next().unwrap_or(0);
        Some(Self {
            major,
            minor,
            patch,
        })
    }

    pub fn supported(self) -> bool {
        self >= MINIMUM_VERSION
    }
}

impl fmt::Display for ZellijVersion {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}.{}.{}", self.major, self.minor, self.patch)
    }
}

/// One terminal pane as reported by `action list-panes --json`.
#[derive(Clone, Debug, Deserialize)]
pub struct PaneInfo {
    pub id: u32,
    #[serde(default)]
    pub is_plugin: bool,
    #[serde(default)]
    pub is_floating: bool,
    #[serde(default)]
    pub is_suppressed: bool,
    #[serde(default)]
    pub title: String,
    pub tab_id: u32,
    pub tab_position: u32,
    #[serde(default)]
    pub tab_name: String,
    pub pane_x: u32,
    pub pane_y: u32,
}

impl PaneInfo {
    /// The `--pane-id` spelling for this pane. zellij also accepts a bare
    /// integer, but that is ambiguous between terminal and plugin panes.
    pub fn target(&self) -> String {
        format!("terminal_{}", self.id)
    }

    /// Whether this pane is one of the ordinary tiled terminals bootmux lays
    /// out, as opposed to a plugin, floating, or suppressed pane.
    pub fn is_ordinary_terminal(&self) -> bool {
        !self.is_plugin && !self.is_floating && !self.is_suppressed
    }
}

#[derive(Clone, Debug, Deserialize)]
pub struct TabInfo {
    pub tab_id: u32,
    pub position: u32,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub active: bool,
}

/// zellij CLI client.
#[derive(Clone, Debug)]
pub struct Zellij<R = ProcessRunner> {
    binary: PathBuf,
    runner: R,
}

impl Zellij<ProcessRunner> {
    pub fn new() -> Self {
        Self::with_binary("zellij")
    }

    pub fn with_binary(binary: impl Into<PathBuf>) -> Self {
        Self {
            binary: binary.into(),
            runner: ProcessRunner,
        }
    }
}

impl Default for Zellij<ProcessRunner> {
    fn default() -> Self {
        Self::new()
    }
}

impl<R: CommandRunner> Zellij<R> {
    pub fn with_runner(binary: impl Into<PathBuf>, runner: R) -> Self {
        Self {
            binary: binary.into(),
            runner,
        }
    }

    pub fn binary(&self) -> &Path {
        &self.binary
    }

    pub fn version(&self) -> Result<ZellijVersion> {
        let invocation = self.invocation([OsStr::new("--version")]);
        let stdout = self.capture(&invocation)?;
        let parsed = ZellijVersion::parse(&stdout);
        parsed.ok_or(Error::UnparsableVersion { found: stdout })
    }

    pub fn require_supported_version(&self) -> Result<ZellijVersion> {
        let version = self.version()?;
        if version.supported() {
            Ok(version)
        } else {
            Err(Error::UnsupportedVersion {
                found: version,
                minimum: MINIMUM_VERSION,
            })
        }
    }

    /// Names of the sessions that are actually running.
    ///
    /// The short listing is deliberately avoided: it also reports dead
    /// sessions that zellij keeps around for resurrection, which would make a
    /// stopped project look like a running one.
    pub fn sessions(&self) -> Result<Vec<String>> {
        let invocation =
            self.invocation([OsStr::new("list-sessions"), OsStr::new("--no-formatting")]);
        let output = self
            .runner
            .run(&invocation)
            .map_err(|source| Error::Spawn {
                invocation: invocation.display(),
                source,
            })?;

        let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
        let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
        if !output.success {
            // An empty list is reported as a failure, so it has to be told
            // apart from a real one.
            if stdout.contains(NO_SESSIONS_MARKER) || stderr.contains(NO_SESSIONS_MARKER) {
                return Ok(Vec::new());
            }
            return Err(Error::CommandFailed {
                invocation: invocation.display(),
                code: output.code,
                output: combined(&stdout, &stderr),
            });
        }

        Ok(parse_session_listing(&stdout))
    }

    pub fn has_session(&self, name: &str) -> Result<bool> {
        Ok(self.sessions()?.iter().any(|session| session == name))
    }

    /// Creates a detached session from a rendered KDL layout.
    ///
    /// zellij treats this as idempotent: creating a session that already
    /// exists reports "Session already exists" and succeeds.
    pub fn create_background_session(&self, name: &str, layout: &LayoutFile) -> Result<()> {
        self.run_discarding_output(self.invocation([
            OsStr::new("--layout"),
            layout.path().as_os_str(),
            OsStr::new("attach"),
            OsStr::new("--create-background"),
            OsStr::new(name),
        ]))
    }

    pub fn kill_session(&self, name: &str) -> Result<()> {
        self.run_discarding_output(self.invocation([OsStr::new("kill-session"), OsStr::new(name)]))
    }

    pub fn list_panes(&self, session: &str) -> Result<Vec<PaneInfo>> {
        self.action_json(session, [OsStr::new("list-panes"), OsStr::new("--json")])
    }

    pub fn list_tabs(&self, session: &str) -> Result<Vec<TabInfo>> {
        self.action_json(session, [OsStr::new("list-tabs"), OsStr::new("--json")])
    }

    /// Appends a tab described by a KDL layout and returns its tab id.
    ///
    /// `layout` must be a complete `layout { tab { … } }` document written
    /// across multiple lines; zellij rejects a bare tab body and rejects
    /// semicolon-separated nodes on one line.
    pub fn new_tab(&self, session: &str, name: Option<&str>, layout: &LayoutFile) -> Result<u32> {
        let mut args = vec![OsString::from("new-tab")];
        if let Some(name) = name {
            args.push(OsString::from("--name"));
            args.push(OsString::from(name));
        }
        args.push(OsString::from("--layout"));
        args.push(OsString::from(layout.path().as_os_str()));

        let invocation = self.action_invocation(session, args);
        let stdout = self.capture(&invocation)?;
        stdout
            .trim()
            .parse::<u32>()
            .map_err(|_| Error::UnexpectedOutput {
                invocation: invocation.display(),
                detail: format!("{:?} instead of a tab id", stdout.trim()),
            })
    }

    pub fn close_tab(&self, session: &str, tab_id: u32) -> Result<()> {
        self.run_action(
            session,
            [
                OsString::from("close-tab"),
                OsString::from("--tab-id"),
                OsString::from(tab_id.to_string()),
            ],
        )
    }

    pub fn write_chars(&self, session: &str, pane_id: &str, chars: &str) -> Result<()> {
        self.run_action(
            session,
            [
                OsString::from("write-chars"),
                OsString::from("--pane-id"),
                OsString::from(pane_id),
                OsString::from(chars),
            ],
        )
    }

    pub fn send_keys(&self, session: &str, pane_id: &str, keys: &[&str]) -> Result<()> {
        let mut args = vec![
            OsString::from("send-keys"),
            OsString::from("--pane-id"),
            OsString::from(pane_id),
        ];
        args.extend(keys.iter().map(OsString::from));
        self.run_action(session, args)
    }

    /// Types a command into a pane's shell and submits it, matching tmux's
    /// `send-keys … C-m` semantics: the shell survives the command.
    pub fn run_in_pane(&self, session: &str, pane_id: &str, command: &str) -> Result<()> {
        self.write_chars(session, pane_id, command)?;
        self.send_keys(session, pane_id, &["Enter"])
    }

    pub fn rename_pane(&self, session: &str, pane_id: &str, name: &str) -> Result<()> {
        self.run_action(
            session,
            [
                OsString::from("rename-pane"),
                OsString::from("--pane-id"),
                OsString::from(pane_id),
                OsString::from(name),
            ],
        )
    }

    pub fn rename_tab(&self, session: &str, tab_id: u32, name: &str) -> Result<()> {
        self.run_action(
            session,
            [
                OsString::from("rename-tab"),
                OsString::from("--tab-id"),
                OsString::from(tab_id.to_string()),
                OsString::from(name),
            ],
        )
    }

    pub fn focus_pane(&self, session: &str, pane_id: &str) -> Result<()> {
        self.run_action(
            session,
            [OsString::from("focus-pane-id"), OsString::from(pane_id)],
        )
    }

    /// Focuses a tab by its one-based screen position.
    pub fn go_to_tab(&self, session: &str, position: u32) -> Result<()> {
        self.run_action(
            session,
            [
                OsString::from("go-to-tab"),
                OsString::from(position.to_string()),
            ],
        )
    }

    pub fn switch_session(&self, session: &str, target: &str) -> Result<()> {
        self.run_action(
            session,
            [OsString::from("switch-session"), OsString::from(target)],
        )
    }

    fn invocation<'a>(&self, args: impl IntoIterator<Item = &'a OsStr>) -> Invocation {
        Invocation {
            program: self.binary.clone(),
            args: args.into_iter().map(OsString::from).collect(),
            env: Default::default(),
            current_dir: None,
        }
    }

    /// Every session-scoped call pins `--session` so an inherited
    /// `ZELLIJ_SESSION_NAME` can never redirect it at the wrong session.
    fn action_invocation(
        &self,
        session: &str,
        args: impl IntoIterator<Item = OsString>,
    ) -> Invocation {
        let mut all = vec![
            OsString::from("--session"),
            OsString::from(session),
            OsString::from("action"),
        ];
        all.extend(args);
        Invocation {
            program: self.binary.clone(),
            args: all,
            env: Default::default(),
            current_dir: None,
        }
    }

    fn run_action(&self, session: &str, args: impl IntoIterator<Item = OsString>) -> Result<()> {
        self.run_discarding_output(self.action_invocation(session, args))
    }

    fn action_json<'a, T: DeserializeOwned>(
        &self,
        session: &str,
        args: impl IntoIterator<Item = &'a OsStr>,
    ) -> Result<T> {
        let invocation = self.action_invocation(
            session,
            args.into_iter().map(OsString::from).collect::<Vec<_>>(),
        );
        let output = self.execute(&invocation)?;
        serde_json::from_slice(&output).map_err(|source| Error::InvalidJson {
            invocation: invocation.display(),
            source,
        })
    }

    fn run_discarding_output(&self, invocation: Invocation) -> Result<()> {
        self.execute(&invocation).map(|_| ())
    }

    fn capture(&self, invocation: &Invocation) -> Result<String> {
        let stdout = self.execute(invocation)?;
        Ok(String::from_utf8_lossy(&stdout).into_owned())
    }

    fn execute(&self, invocation: &Invocation) -> Result<Vec<u8>> {
        let output = self.runner.run(invocation).map_err(|source| Error::Spawn {
            invocation: invocation.display(),
            source,
        })?;
        if output.success {
            return Ok(output.stdout);
        }
        Err(Error::CommandFailed {
            invocation: invocation.display(),
            code: output.code,
            output: combined(
                &String::from_utf8_lossy(&output.stdout),
                &String::from_utf8_lossy(&output.stderr),
            ),
        })
    }
}

/// Reads names out of `list-sessions --no-formatting`, whose lines look like
/// `name [Created 2s ago]` with an `(EXITED - attach to resurrect)` suffix on
/// sessions that are no longer running.
fn parse_session_listing(stdout: &str) -> Vec<String> {
    stdout
        .lines()
        .filter(|line| !line.contains(EXITED_MARKER))
        .filter_map(|line| line.split_whitespace().next())
        .filter(|name| !name.is_empty())
        .map(str::to_owned)
        .collect()
}

fn combined(stdout: &str, stderr: &str) -> String {
    let mut parts = Vec::new();
    for part in [stderr.trim(), stdout.trim()] {
        if !part.is_empty() {
            parts.push(part);
        }
    }
    parts.join("; ")
}

#[cfg(test)]
impl<R: CommandRunner> Zellij<R> {
    /// Exposes the injected runner so tests can assert on the exact argv the
    /// client produced.
    pub(crate) fn runner_for_test(&self) -> &R {
        &self.runner
    }
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;
    use std::io;
    use std::sync::Mutex;

    use super::*;
    use crate::process::CommandOutput;

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

        fn args(&self, index: usize) -> Vec<String> {
            self.invocations.lock().unwrap()[index]
                .args
                .iter()
                .map(|arg| arg.to_string_lossy().into_owned())
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

        fn spawn_detached(&self, invocation: &Invocation) -> io::Result<u32> {
            self.invocations.lock().unwrap().push(invocation.clone());
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

    fn failed(code: i32, stderr: &str) -> CommandOutput {
        CommandOutput {
            success: false,
            code: Some(code),
            stdout: Vec::new(),
            stderr: stderr.as_bytes().to_vec(),
        }
    }

    #[test]
    fn version_parsing_tolerates_prefixes_and_suffixes() {
        assert_eq!(
            ZellijVersion::parse("zellij 0.44.3\n"),
            Some(ZellijVersion {
                major: 0,
                minor: 44,
                patch: 3
            })
        );
        assert_eq!(
            ZellijVersion::parse("zellij 1.0"),
            Some(ZellijVersion {
                major: 1,
                minor: 0,
                patch: 0
            })
        );
        assert_eq!(
            ZellijVersion::parse("zellij 0.45.0-alpha.1+deadbeef"),
            Some(ZellijVersion {
                major: 0,
                minor: 45,
                patch: 0
            })
        );
        assert_eq!(ZellijVersion::parse("zellij unknown"), None);

        assert!(!ZellijVersion::parse("zellij 0.43.9").unwrap().supported());
        assert!(ZellijVersion::parse("zellij 0.44.0").unwrap().supported());
        assert!(ZellijVersion::parse("zellij 0.44.3").unwrap().supported());
        assert!(ZellijVersion::parse("zellij 1.2.0").unwrap().supported());
    }

    #[test]
    fn an_old_zellij_is_rejected_with_both_versions_named() {
        let runner = FakeRunner::with(vec![ok("zellij 0.43.1\n")]);
        let error = Zellij::with_runner("zellij", runner)
            .require_supported_version()
            .unwrap_err()
            .to_string();
        assert!(error.contains("0.43.1"), "{error}");
        assert!(error.contains("0.44.0"), "{error}");
    }

    #[test]
    fn session_listing_skips_resurrectable_sessions_and_survives_an_empty_list() {
        let runner = FakeRunner::with(vec![ok(
            "api [Created 2s ago] \nold [Created 16days ago] (EXITED - attach to resurrect)\nweb [Created 1m ago] \n",
        )]);
        let client = Zellij::with_runner("zellij", runner);
        assert_eq!(client.sessions().unwrap(), vec!["api", "web"]);
        assert_eq!(
            client.runner_for_test().args(0),
            vec!["list-sessions".to_string(), "--no-formatting".to_string()]
        );

        // zellij exits non-zero when nothing is running, which is not a failure.
        let empty = Zellij::with_runner(
            "zellij",
            FakeRunner::with(vec![failed(1, "No active zellij sessions found.")]),
        );
        assert!(empty.sessions().unwrap().is_empty());

        let broken = Zellij::with_runner(
            "zellij",
            FakeRunner::with(vec![failed(2, "permission denied")]),
        );
        assert!(broken
            .sessions()
            .unwrap_err()
            .to_string()
            .contains("permission denied"));
    }

    #[test]
    fn a_layout_file_carries_the_rendered_document_and_is_removed_afterwards() {
        let layout = "layout {\n    tab name=\"editor\"\n}\n";
        let path = {
            let file = LayoutFile::new(layout).unwrap();
            assert_eq!(
                file.path.extension().and_then(std::ffi::OsStr::to_str),
                Some("kdl")
            );
            assert_eq!(std::fs::read_to_string(&file.path).unwrap(), layout);
            file.path.clone()
        };
        assert!(
            !path.exists(),
            "the temporary layout must not be left behind"
        );
    }

    #[test]
    fn topology_commands_pin_the_session_and_use_documented_cli_shapes() {
        let runner = FakeRunner::with(vec![
            ok(""),
            ok("2\n"),
            ok(""),
            ok(""),
            ok(""),
            ok(""),
            ok(""),
            ok(""),
        ]);
        let client = Zellij::with_runner("zellij", runner);

        let layout = LayoutFile::new("layout {\n    tab\n}\n").unwrap();
        client.create_background_session("api", &layout).unwrap();
        let tab = client.new_tab("api", Some("editor"), &layout).unwrap();
        assert_eq!(tab, 2);
        client.rename_pane("api", "terminal_3", "vim").unwrap();
        client
            .run_in_pane("api", "terminal_3", "npm run dev")
            .unwrap();
        client.focus_pane("api", "terminal_3").unwrap();
        client.go_to_tab("api", 1).unwrap();
        client.close_tab("api", 2).unwrap();

        let runner = client.runner_for_test();
        let created = runner.args(0);
        assert_eq!(created[0], "--layout");
        assert!(
            created[1].ends_with(".kdl"),
            "the layout must be handed over as a file: {created:?}"
        );
        assert_eq!(created[2..], ["attach", "--create-background", "api"]);
        let appended = runner.args(1);
        assert_eq!(
            appended[..6],
            ["--session", "api", "action", "new-tab", "--name", "editor"]
        );
        assert_eq!(appended[6], "--layout");
        assert!(
            appended[7].ends_with(".kdl"),
            "the appended layout must be handed over as a file: {appended:?}"
        );
        assert_eq!(
            runner.args(2),
            vec![
                "--session",
                "api",
                "action",
                "rename-pane",
                "--pane-id",
                "terminal_3",
                "vim",
            ]
        );
        // A pane command is typed and then submitted, exactly like tmux's
        // `send-keys … C-m`.
        assert_eq!(
            runner.args(3),
            vec![
                "--session",
                "api",
                "action",
                "write-chars",
                "--pane-id",
                "terminal_3",
                "npm run dev",
            ]
        );
        assert_eq!(
            runner.args(4),
            vec![
                "--session",
                "api",
                "action",
                "send-keys",
                "--pane-id",
                "terminal_3",
                "Enter",
            ]
        );
        assert_eq!(
            runner.args(5),
            vec!["--session", "api", "action", "focus-pane-id", "terminal_3"]
        );
        assert_eq!(
            runner.args(6),
            vec!["--session", "api", "action", "go-to-tab", "1"]
        );
        assert_eq!(
            runner.args(7),
            vec!["--session", "api", "action", "close-tab", "--tab-id", "2"]
        );
    }

    #[test]
    fn a_new_tab_that_does_not_report_an_id_is_an_error() {
        let client = Zellij::with_runner("zellij", FakeRunner::with(vec![ok("Session not found")]));
        let layout = LayoutFile::new("layout {}").unwrap();
        let error = client.new_tab("api", None, &layout).unwrap_err();
        assert!(error.to_string().contains("instead of a tab id"), "{error}");
    }

    #[test]
    fn pane_listing_decodes_the_real_json_shape_and_classifies_panes() {
        let json = r#"[
            {"id":0,"is_plugin":false,"is_focused":true,"is_floating":false,
             "is_suppressed":false,"title":"vim","tab_id":0,"tab_position":0,
             "tab_name":"editor","pane_x":0,"pane_y":0,"pane_rows":30,
             "pane_columns":50,"pane_command":"/bin/zsh","pane_cwd":"/repo"},
            {"id":7,"is_plugin":true,"is_floating":false,"is_suppressed":false,
             "title":"tab-bar","tab_id":0,"tab_position":0,"tab_name":"editor",
             "pane_x":0,"pane_y":0}
        ]"#;
        let client = Zellij::with_runner("zellij", FakeRunner::with(vec![ok(json)]));
        let panes = client.list_panes("api").unwrap();

        assert_eq!(panes.len(), 2);
        assert_eq!(panes[0].target(), "terminal_0");
        assert_eq!(panes[0].title, "vim");
        assert!(panes[0].is_ordinary_terminal());
        assert!(!panes[1].is_ordinary_terminal());
        assert_eq!(
            client.runner_for_test().args(0),
            vec!["--session", "api", "action", "list-panes", "--json"]
        );
    }
}
