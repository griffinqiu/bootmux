//! Typed command-line integration for Herdr 0.7.5 (socket protocol 17).
//!
//! The adapter deliberately drives Herdr through its documented JSON CLI
//! rather than duplicating the socket protocol.  Every invocation pins one
//! [`Endpoint`] and clears ambient endpoint variables first, so a bootmux
//! process running inside Herdr cannot accidentally address the wrong server.

use std::collections::BTreeMap;
use std::env;
use std::ffi::{OsStr, OsString};
use std::fmt;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

pub use crate::process::{CommandOutput, CommandRunner, Invocation, ProcessRunner};

pub const MINIMUM_HERDR_VERSION: &str = "0.7.5";
pub const REQUIRED_PROTOCOL: u32 = 17;
pub const STATE_INDEX_VERSION: u32 = 1;
pub const STATE_INDEX_FILE_NAME: &str = "herdr-workspaces.json";

const HERDR_SOCKET_PATH: &str = "HERDR_SOCKET_PATH";
const HERDR_CLIENT_SOCKET_PATH: &str = "HERDR_CLIENT_SOCKET_PATH";
const HERDR_SESSION: &str = "HERDR_SESSION";
const HERDR_STARTUP_CWD: &str = "HERDR_STARTUP_CWD";
const DEFAULT_READY_TIMEOUT: Duration = Duration::from_secs(15);
const DEFAULT_READY_POLL: Duration = Duration::from_millis(50);
const DEFAULT_LOCK_TIMEOUT: Duration = Duration::from_secs(2);
const DEFAULT_LOCK_POLL: Duration = Duration::from_millis(20);
const DEFAULT_OPERATION_LOCK_TIMEOUT: Duration = Duration::from_secs(30);

pub type Result<T> = std::result::Result<T, Error>;

/// Selects the Herdr runtime namespace used by every command.
#[derive(Clone, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
pub enum Endpoint {
    #[default]
    Default,
    NamedSession(String),
    SocketPath(PathBuf),
}

impl Endpoint {
    pub fn named_session(name: impl Into<String>) -> Self {
        Self::NamedSession(name.into())
    }

    pub fn socket_path(path: impl Into<PathBuf>) -> Self {
        Self::SocketPath(path.into())
    }

    fn apply(&self, args: &mut Vec<OsString>, env: &mut BTreeMap<OsString, Option<OsString>>) {
        // Remove all inherited selectors before applying the configured one.
        env.insert(OsString::from(HERDR_SOCKET_PATH), None);
        env.insert(OsString::from(HERDR_CLIENT_SOCKET_PATH), None);
        env.insert(OsString::from(HERDR_SESSION), None);

        match self {
            Endpoint::Default => {}
            Endpoint::NamedSession(name) => {
                args.push(OsString::from("--session"));
                args.push(OsString::from(name));
            }
            Endpoint::SocketPath(path) => {
                env.insert(
                    OsString::from(HERDR_SOCKET_PATH),
                    Some(path.as_os_str().to_owned()),
                );
            }
        }
    }
}

/// Herdr JSON CLI client.
#[derive(Clone, Debug)]
pub struct Herdr<R = ProcessRunner> {
    binary: PathBuf,
    endpoint: Endpoint,
    runner: R,
}

impl Herdr<ProcessRunner> {
    pub fn new(endpoint: Endpoint) -> Self {
        Self::with_binary("herdr", endpoint)
    }

    pub fn with_binary(binary: impl Into<PathBuf>, endpoint: Endpoint) -> Self {
        Self {
            binary: binary.into(),
            endpoint,
            runner: ProcessRunner,
        }
    }
}

impl Default for Herdr<ProcessRunner> {
    fn default() -> Self {
        Self::new(Endpoint::Default)
    }
}

impl<R: CommandRunner> Herdr<R> {
    pub fn with_runner(binary: impl Into<PathBuf>, endpoint: Endpoint, runner: R) -> Self {
        Self {
            binary: binary.into(),
            endpoint,
            runner,
        }
    }

    pub fn endpoint(&self) -> &Endpoint {
        &self.endpoint
    }

    pub fn binary(&self) -> &Path {
        &self.binary
    }

    /// Probes both the installed CLI and the selected server.
    pub fn probe(&self) -> Result<Probe> {
        let wire: FullStatusWire = self.run_json(["status", "--json"])?;
        validate_component("client", &wire.client.version, wire.client.protocol)?;
        if wire.server.running {
            validate_server(&wire.client, &wire.server)?;
        }
        Ok(Probe {
            client: wire.client,
            server: wire.server,
        })
    }

    pub fn server_status(&self) -> Result<ServerStatus> {
        let status: ServerStatus = self.run_json(["status", "server", "--json"])?;
        if status.running {
            validate_component(
                "server",
                status.version.as_deref().ok_or(Error::MissingStatusField {
                    component: "server",
                    field: "version",
                })?,
                status.protocol.ok_or(Error::MissingStatusField {
                    component: "server",
                    field: "protocol",
                })?,
            )?;
        }
        Ok(status)
    }

    /// Starts `herdr server` in a detached OS session. Readiness is handled by
    /// [`ensure_server`](Self::ensure_server).
    pub fn spawn_server_detached(&self) -> Result<u32> {
        let mut invocation = self.invocation(["server"]);
        invocation.current_dir = env::current_dir().ok();
        if let Some(cwd) = &invocation.current_dir {
            invocation.env.insert(
                OsString::from(HERDR_STARTUP_CWD),
                Some(cwd.as_os_str().to_owned()),
            );
        }
        self.runner
            .spawn_detached(&invocation)
            .map_err(|source| Error::Spawn {
                command: invocation.display(),
                source,
            })
    }

    /// Ensures a compatible server exists, starting it detached if necessary.
    pub fn ensure_server(&self) -> Result<ServerStatus> {
        self.ensure_server_with(ReadinessOptions::default())
    }

    pub fn ensure_server_with(&self, options: ReadinessOptions) -> Result<ServerStatus> {
        let probe = self.probe()?;
        if probe.server.running {
            return Ok(probe.server);
        }

        self.spawn_server_detached()?;
        let deadline = Instant::now() + options.timeout;
        let mut last_error = None;
        loop {
            match self.server_status() {
                Ok(status) if status.running => {
                    let server_protocol = status.protocol.ok_or(Error::MissingStatusField {
                        component: "server",
                        field: "protocol",
                    })?;
                    if server_protocol != probe.client.protocol {
                        return Err(Error::ProtocolMismatch {
                            client: probe.client.protocol,
                            server: server_protocol,
                        });
                    }
                    return Ok(status);
                }
                Ok(_) => {}
                Err(err @ Error::UnsupportedVersion { .. })
                | Err(err @ Error::UnsupportedProtocol { .. })
                | Err(err @ Error::ProtocolMismatch { .. }) => return Err(err),
                Err(err) => last_error = Some(err.to_string()),
            }

            let now = Instant::now();
            if now >= deadline {
                return Err(Error::ServerNotReady {
                    timeout: options.timeout,
                    last_error,
                });
            }
            thread::sleep(options.poll_interval.min(deadline - now));
        }
    }

    pub fn snapshot(&self) -> Result<SessionSnapshot> {
        let response: SnapshotResponse = self.run_json(["api", "snapshot"])?;
        expect_result_type(&response.result.result_type, "session_snapshot")?;
        validate_component(
            "snapshot",
            &response.result.snapshot.version,
            response.result.snapshot.protocol,
        )?;
        Ok(response.result.snapshot)
    }

    pub fn list_workspaces(&self) -> Result<Vec<WorkspaceInfo>> {
        let response: WorkspaceListResponse = self.run_json(["workspace", "list"])?;
        expect_result_type(&response.result.result_type, "workspace_list")?;
        Ok(response.result.workspaces)
    }

    pub fn get_workspace(&self, workspace_id: &str) -> Result<WorkspaceInfo> {
        let response: WorkspaceInfoResponse = self.run_json_os([
            OsStr::new("workspace"),
            OsStr::new("get"),
            OsStr::new(workspace_id),
        ])?;
        expect_result_type(&response.result.result_type, "workspace_info")?;
        Ok(response.result.workspace)
    }

    pub fn create_workspace(&self, options: &WorkspaceCreate) -> Result<WorkspaceCreated> {
        let mut args = vec![OsString::from("workspace"), OsString::from("create")];
        push_path_option(&mut args, "--cwd", options.cwd.as_deref());
        push_string_option(&mut args, "--label", options.label.as_deref());
        push_env_options(&mut args, &options.env);
        args.push(OsString::from(if options.focus {
            "--focus"
        } else {
            "--no-focus"
        }));

        let response: WorkspaceCreatedResponse = self.run_json_vec(args)?;
        expect_result_type(&response.result.result_type, "workspace_created")?;
        Ok(WorkspaceCreated {
            workspace: response.result.workspace,
            tab: response.result.tab,
            root_pane: response.result.root_pane,
        })
    }

    pub fn focus_workspace(&self, workspace_id: &str) -> Result<WorkspaceInfo> {
        let response: WorkspaceInfoResponse = self.run_json_os([
            OsStr::new("workspace"),
            OsStr::new("focus"),
            OsStr::new(workspace_id),
        ])?;
        expect_result_type(&response.result.result_type, "workspace_info")?;
        Ok(response.result.workspace)
    }

    pub fn close_workspace(&self, workspace_id: &str) -> Result<()> {
        let response: OkResponse = self.run_json_os([
            OsStr::new("workspace"),
            OsStr::new("close"),
            OsStr::new(workspace_id),
        ])?;
        expect_result_type(&response.result.result_type, "ok")
    }

    pub fn create_tab(&self, options: &TabCreate) -> Result<TabCreated> {
        let mut args = vec![OsString::from("tab"), OsString::from("create")];
        push_string_option(&mut args, "--workspace", options.workspace_id.as_deref());
        push_path_option(&mut args, "--cwd", options.cwd.as_deref());
        push_string_option(&mut args, "--label", options.label.as_deref());
        push_env_options(&mut args, &options.env);
        args.push(OsString::from(if options.focus {
            "--focus"
        } else {
            "--no-focus"
        }));

        let response: TabCreatedResponse = self.run_json_vec(args)?;
        expect_result_type(&response.result.result_type, "tab_created")?;
        Ok(TabCreated {
            tab: response.result.tab,
            root_pane: response.result.root_pane,
        })
    }

    pub fn focus_tab(&self, tab_id: &str) -> Result<TabInfo> {
        let response: TabInfoResponse =
            self.run_json_os([OsStr::new("tab"), OsStr::new("focus"), OsStr::new(tab_id)])?;
        expect_result_type(&response.result.result_type, "tab_info")?;
        Ok(response.result.tab)
    }

    pub fn rename_tab(&self, tab_id: &str, label: &str) -> Result<TabInfo> {
        let response: TabInfoResponse = self.run_json_os([
            OsStr::new("tab"),
            OsStr::new("rename"),
            OsStr::new(tab_id),
            OsStr::new(label),
        ])?;
        expect_result_type(&response.result.result_type, "tab_info")?;
        Ok(response.result.tab)
    }

    pub fn close_tab(&self, tab_id: &str) -> Result<()> {
        let response: OkResponse =
            self.run_json_os([OsStr::new("tab"), OsStr::new("close"), OsStr::new(tab_id)])?;
        expect_result_type(&response.result.result_type, "ok")
    }

    pub fn split_pane(&self, options: &PaneSplit) -> Result<PaneInfo> {
        let mut args = vec![OsString::from("pane"), OsString::from("split")];
        match &options.target {
            PaneTarget::PaneId(pane_id) => args.push(OsString::from(pane_id)),
            PaneTarget::Current => args.push(OsString::from("--current")),
            PaneTarget::Focused => {}
        }
        args.push(OsString::from("--direction"));
        args.push(OsString::from(options.direction.as_str()));
        if let Some(ratio) = options.ratio {
            if !ratio.is_finite() {
                return Err(Error::InvalidArgument {
                    name: "ratio",
                    message: "must be finite".to_string(),
                });
            }
            args.push(OsString::from("--ratio"));
            args.push(OsString::from(ratio.to_string()));
        }
        push_path_option(&mut args, "--cwd", options.cwd.as_deref());
        push_env_options(&mut args, &options.env);
        args.push(OsString::from(if options.focus {
            "--focus"
        } else {
            "--no-focus"
        }));

        let response: PaneInfoResponse = self.run_json_vec(args)?;
        expect_result_type(&response.result.result_type, "pane_info")?;
        Ok(response.result.pane)
    }

    /// Runs a shell command through the official `pane run` helper. Herdr
    /// intentionally emits no JSON body for this successful command.
    pub fn run_in_pane(&self, pane_id: &str, command: &str) -> Result<()> {
        self.run_empty_os([
            OsStr::new("pane"),
            OsStr::new("run"),
            OsStr::new(pane_id),
            OsStr::new(command),
        ])
    }

    /// Focuses a neighboring pane. Protocol 17 has no CLI operation that
    /// focuses an arbitrary pane ID directly.
    pub fn focus_pane_neighbor(&self, options: &PaneFocus) -> Result<PaneFocusResult> {
        let mut args = vec![
            OsString::from("pane"),
            OsString::from("focus"),
            OsString::from("--direction"),
            OsString::from(options.direction.as_str()),
        ];
        match &options.source {
            PaneTarget::PaneId(pane_id) => {
                args.push(OsString::from("--pane"));
                args.push(OsString::from(pane_id));
            }
            PaneTarget::Current => args.push(OsString::from("--current")),
            PaneTarget::Focused => {}
        }
        let response: PaneFocusResponse = self.run_json_vec(args)?;
        expect_result_type(&response.result.result_type, "pane_focus_direction")?;
        Ok(response.result.focus)
    }

    /// Focuses an exact pane through Herdr's protocol-17 newline-delimited
    /// socket API. The 0.7.5 CLI only exposes directional neighbor focus.
    #[cfg(unix)]
    pub fn focus_pane_direct(&self, pane_id: &str) -> Result<PaneInfo> {
        use std::io::{BufRead, BufReader};
        use std::os::unix::net::UnixStream;

        const REQUEST_ID: &str = "bootmux:pane:focus";
        const SOCKET_TIMEOUT: Duration = Duration::from_secs(5);

        let status = self.server_status()?;
        if !status.running {
            return Err(Error::ServerNotRunning {
                socket: PathBuf::from(status.socket),
            });
        }
        let socket = PathBuf::from(status.socket);
        let mut stream = UnixStream::connect(&socket).map_err(|source| Error::SocketIo {
            operation: "connect",
            path: socket.clone(),
            source,
        })?;
        stream
            .set_read_timeout(Some(SOCKET_TIMEOUT))
            .map_err(|source| Error::SocketIo {
                operation: "set read timeout",
                path: socket.clone(),
                source,
            })?;
        stream
            .set_write_timeout(Some(SOCKET_TIMEOUT))
            .map_err(|source| Error::SocketIo {
                operation: "set write timeout",
                path: socket.clone(),
                source,
            })?;

        let request = serde_json::json!({
            "id": REQUEST_ID,
            "method": "pane.focus",
            "params": { "pane_id": pane_id },
        });
        serde_json::to_writer(&mut stream, &request).map_err(|source| Error::SocketJson {
            operation: "serialize pane.focus request",
            source,
        })?;
        stream.write_all(b"\n").map_err(|source| Error::SocketIo {
            operation: "write request",
            path: socket.clone(),
            source,
        })?;
        stream.flush().map_err(|source| Error::SocketIo {
            operation: "flush request",
            path: socket.clone(),
            source,
        })?;

        let mut response_bytes = Vec::new();
        BufReader::new(stream)
            .read_until(b'\n', &mut response_bytes)
            .map_err(|source| Error::SocketIo {
                operation: "read response",
                path: socket.clone(),
                source,
            })?;
        if response_bytes.is_empty() {
            return Err(Error::UnexpectedOutput {
                command: format!("Herdr socket {} pane.focus", socket.display()),
                output: "server closed the socket without a response".to_string(),
            });
        }
        if let Some(error) = parse_cli_error(&response_bytes) {
            return Err(Error::Cli {
                command: format!("Herdr socket {} pane.focus", socket.display()),
                status: None,
                code: error.code,
                message: error.message,
            });
        }
        let response: DirectPaneFocusResponse =
            serde_json::from_slice(&response_bytes).map_err(|source| Error::InvalidJson {
                command: format!("Herdr socket {} pane.focus", socket.display()),
                output: String::from_utf8_lossy(&response_bytes).trim().to_string(),
                source,
            })?;
        if response.id != REQUEST_ID {
            return Err(Error::UnexpectedResponseId {
                expected: REQUEST_ID,
                found: response.id,
            });
        }
        expect_result_type(&response.result.result_type, "pane_info")?;
        Ok(response.result.pane)
    }

    #[cfg(not(unix))]
    pub fn focus_pane_direct(&self, _pane_id: &str) -> Result<PaneInfo> {
        Err(Error::UnsupportedOperation {
            operation: "exact Herdr pane focus",
            reason: "Herdr protocol-17 local socket focus is only implemented on Unix",
        })
    }

    pub fn rename_pane(&self, pane_id: &str, label: Option<&str>) -> Result<PaneInfo> {
        let mut args = vec![
            OsString::from("pane"),
            OsString::from("rename"),
            OsString::from(pane_id),
        ];
        match label {
            Some(label) => args.push(OsString::from(label)),
            None => args.push(OsString::from("--clear")),
        }
        let response: PaneInfoResponse = self.run_json_vec(args)?;
        expect_result_type(&response.result.result_type, "pane_info")?;
        Ok(response.result.pane)
    }

    fn invocation<I, S>(&self, args: I) -> Invocation
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        let mut endpoint_args = Vec::new();
        let mut invocation_env = BTreeMap::new();
        self.endpoint.apply(&mut endpoint_args, &mut invocation_env);
        endpoint_args.extend(
            args.into_iter()
                .map(|arg| arg.as_ref().to_owned())
                .collect::<Vec<_>>(),
        );
        Invocation {
            program: self.binary.clone(),
            args: endpoint_args,
            env: invocation_env,
            current_dir: None,
        }
    }

    fn run_json<I, S, T>(&self, args: I) -> Result<T>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
        T: DeserializeOwned,
    {
        self.run_json_invocation(self.invocation(args))
    }

    fn run_json_os<const N: usize, T>(&self, args: [&OsStr; N]) -> Result<T>
    where
        T: DeserializeOwned,
    {
        self.run_json_invocation(self.invocation(args))
    }

    fn run_json_vec<T>(&self, args: Vec<OsString>) -> Result<T>
    where
        T: DeserializeOwned,
    {
        self.run_json_invocation(self.invocation(args))
    }

    fn run_json_invocation<T>(&self, invocation: Invocation) -> Result<T>
    where
        T: DeserializeOwned,
    {
        let output = self.execute(&invocation)?;
        serde_json::from_slice(&output.stdout).map_err(|source| Error::InvalidJson {
            command: invocation.display(),
            output: String::from_utf8_lossy(&output.stdout).trim().to_string(),
            source,
        })
    }

    fn run_empty_os<const N: usize>(&self, args: [&OsStr; N]) -> Result<()> {
        let invocation = self.invocation(args);
        let output = self.execute(&invocation)?;
        if output.stdout.iter().all(u8::is_ascii_whitespace) {
            Ok(())
        } else {
            Err(Error::UnexpectedOutput {
                command: invocation.display(),
                output: String::from_utf8_lossy(&output.stdout).trim().to_string(),
            })
        }
    }

    fn execute(&self, invocation: &Invocation) -> Result<CommandOutput> {
        let output = self
            .runner
            .run(invocation)
            .map_err(|source| Error::Execute {
                command: invocation.display(),
                source,
            })?;
        if output.success {
            return Ok(output);
        }

        if let Some(error) =
            parse_cli_error(&output.stderr).or_else(|| parse_cli_error(&output.stdout))
        {
            return Err(Error::Cli {
                command: invocation.display(),
                status: output.code,
                code: error.code,
                message: error.message,
            });
        }

        Err(Error::CommandFailed {
            command: invocation.display(),
            status: output.code,
            stdout: String::from_utf8_lossy(&output.stdout).trim().to_string(),
            stderr: String::from_utf8_lossy(&output.stderr).trim().to_string(),
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ReadinessOptions {
    pub timeout: Duration,
    pub poll_interval: Duration,
}

impl Default for ReadinessOptions {
    fn default() -> Self {
        Self {
            timeout: DEFAULT_READY_TIMEOUT,
            poll_interval: DEFAULT_READY_POLL,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Probe {
    pub client: ClientStatus,
    pub server: ServerStatus,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClientStatus {
    pub version: String,
    pub protocol: u32,
    #[serde(default)]
    pub channel: Option<String>,
    #[serde(default)]
    pub binary: Option<String>,
    #[serde(default)]
    pub session: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServerStatus {
    pub status: String,
    pub running: bool,
    #[serde(default)]
    pub version: Option<String>,
    #[serde(default)]
    pub protocol: Option<u32>,
    #[serde(default)]
    pub compatible: Option<bool>,
    pub socket: String,
    #[serde(default)]
    pub session: Option<String>,
    #[serde(default)]
    pub restart_needed: Option<bool>,
}

#[derive(Deserialize)]
struct FullStatusWire {
    client: ClientStatus,
    server: ServerStatus,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HerdrVersion {
    pub major: u64,
    pub minor: u64,
    pub patch: u64,
    pub prerelease: Option<String>,
}

impl HerdrVersion {
    pub fn parse(raw: &str) -> Option<Self> {
        let raw = raw.trim().strip_prefix('v').unwrap_or(raw.trim());
        let without_build = raw.split_once('+').map_or(raw, |(version, _)| version);
        let (core, prerelease) = match without_build.split_once('-') {
            Some((_core, "")) => return None,
            Some((core, prerelease)) => (core, Some(prerelease.to_string())),
            None => (without_build, None),
        };
        let mut components = core.split('.');
        let major = components.next()?.parse().ok()?;
        let minor = components.next().unwrap_or("0").parse().ok()?;
        let patch = components.next().unwrap_or("0").parse().ok()?;
        if components.next().is_some() {
            return None;
        }
        Some(Self {
            major,
            minor,
            patch,
            prerelease,
        })
    }

    fn meets_minimum(&self, minimum: &Self) -> bool {
        self >= minimum
    }
}

impl PartialOrd for HerdrVersion {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for HerdrVersion {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        (self.major, self.minor, self.patch)
            .cmp(&(other.major, other.minor, other.patch))
            .then_with(|| match (&self.prerelease, &other.prerelease) {
                (None, None) => std::cmp::Ordering::Equal,
                (None, Some(_)) => std::cmp::Ordering::Greater,
                (Some(_), None) => std::cmp::Ordering::Less,
                (Some(left), Some(right)) => left.cmp(right),
            })
    }
}

impl fmt::Display for HerdrVersion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}.{}", self.major, self.minor, self.patch)?;
        if let Some(prerelease) = &self.prerelease {
            write!(f, "-{prerelease}")?;
        }
        Ok(())
    }
}

fn validate_component(component: &'static str, version: &str, protocol: u32) -> Result<()> {
    let parsed = HerdrVersion::parse(version).ok_or_else(|| Error::InvalidVersion {
        component,
        found: version.to_string(),
    })?;
    let minimum =
        HerdrVersion::parse(MINIMUM_HERDR_VERSION).expect("minimum Herdr version is valid");
    if !parsed.meets_minimum(&minimum) {
        return Err(Error::UnsupportedVersion {
            component,
            found: version.to_string(),
            minimum: MINIMUM_HERDR_VERSION,
        });
    }
    if protocol != REQUIRED_PROTOCOL {
        return Err(Error::UnsupportedProtocol {
            component,
            found: protocol,
            required: REQUIRED_PROTOCOL,
        });
    }
    Ok(())
}

fn validate_server(client: &ClientStatus, server: &ServerStatus) -> Result<()> {
    let server_version = server.version.as_deref().ok_or(Error::MissingStatusField {
        component: "server",
        field: "version",
    })?;
    let server_protocol = server.protocol.ok_or(Error::MissingStatusField {
        component: "server",
        field: "protocol",
    })?;
    validate_component("server", server_version, server_protocol)?;
    if client.protocol != server_protocol {
        return Err(Error::ProtocolMismatch {
            client: client.protocol,
            server: server_protocol,
        });
    }
    Ok(())
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct WorkspaceCreate {
    pub cwd: Option<PathBuf>,
    pub label: Option<String>,
    pub env: BTreeMap<String, String>,
    pub focus: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TabCreate {
    pub workspace_id: Option<String>,
    pub cwd: Option<PathBuf>,
    pub label: Option<String>,
    pub env: BTreeMap<String, String>,
    pub focus: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SplitDirection {
    Right,
    Down,
}

impl SplitDirection {
    fn as_str(self) -> &'static str {
        match self {
            Self::Right => "right",
            Self::Down => "down",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PaneDirection {
    Left,
    Right,
    Up,
    Down,
}

impl PaneDirection {
    fn as_str(self) -> &'static str {
        match self {
            Self::Left => "left",
            Self::Right => "right",
            Self::Up => "up",
            Self::Down => "down",
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum PaneTarget {
    PaneId(String),
    Current,
    #[default]
    Focused,
}

#[derive(Clone, Debug, PartialEq)]
pub struct PaneSplit {
    pub target: PaneTarget,
    pub direction: SplitDirection,
    pub ratio: Option<f32>,
    pub cwd: Option<PathBuf>,
    pub env: BTreeMap<String, String>,
    pub focus: bool,
}

impl PaneSplit {
    pub fn new(target: PaneTarget, direction: SplitDirection) -> Self {
        Self {
            target,
            direction,
            ratio: None,
            cwd: None,
            env: BTreeMap::new(),
            focus: false,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PaneFocus {
    pub source: PaneTarget,
    pub direction: PaneDirection,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkspaceInfo {
    pub workspace_id: String,
    #[serde(default)]
    pub number: usize,
    #[serde(default)]
    pub label: String,
    #[serde(default)]
    pub focused: bool,
    #[serde(default)]
    pub pane_count: usize,
    #[serde(default)]
    pub tab_count: usize,
    #[serde(default)]
    pub active_tab_id: String,
    #[serde(default)]
    pub worktree: Option<WorkspaceWorktreeInfo>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkspaceWorktreeInfo {
    pub repo_key: String,
    pub repo_name: String,
    pub repo_root: PathBuf,
    pub checkout_path: PathBuf,
    pub is_linked_worktree: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TabInfo {
    pub tab_id: String,
    pub workspace_id: String,
    #[serde(default)]
    pub number: usize,
    #[serde(default)]
    pub label: String,
    #[serde(default)]
    pub focused: bool,
    #[serde(default)]
    pub pane_count: usize,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PaneInfo {
    pub pane_id: String,
    pub terminal_id: String,
    pub workspace_id: String,
    pub tab_id: String,
    #[serde(default)]
    pub focused: bool,
    #[serde(default)]
    pub cwd: Option<PathBuf>,
    #[serde(default)]
    pub foreground_cwd: Option<PathBuf>,
    #[serde(default)]
    pub label: Option<String>,
    #[serde(default)]
    pub agent: Option<String>,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub revision: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WorkspaceCreated {
    pub workspace: WorkspaceInfo,
    pub tab: TabInfo,
    pub root_pane: PaneInfo,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TabCreated {
    pub tab: TabInfo,
    pub root_pane: PaneInfo,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PaneFocusResult {
    pub changed: bool,
    #[serde(default)]
    pub reason: Option<String>,
    pub source_pane_id: String,
    #[serde(default)]
    pub focused_pane_id: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionSnapshot {
    pub version: String,
    pub protocol: u32,
    #[serde(default)]
    pub focused_workspace_id: Option<String>,
    #[serde(default)]
    pub focused_tab_id: Option<String>,
    #[serde(default)]
    pub focused_pane_id: Option<String>,
    #[serde(default)]
    pub workspaces: Vec<WorkspaceInfo>,
    #[serde(default)]
    pub tabs: Vec<TabInfo>,
    #[serde(default)]
    pub panes: Vec<PaneInfo>,
}

impl SessionSnapshot {
    pub fn workspace_by_exact_id(&self, workspace_id: &str) -> Option<&WorkspaceInfo> {
        self.workspaces
            .iter()
            .find(|workspace| workspace.workspace_id == workspace_id)
    }

    pub fn pane_by_exact_id(&self, pane_id: &str) -> Option<&PaneInfo> {
        self.panes.iter().find(|pane| pane.pane_id == pane_id)
    }

    /// Herdr protocol 17 does not put a cwd on `WorkspaceInfo`; the immutable
    /// launch cwd lives on its panes (and, for worktrees, `checkout_path`).
    pub fn workspace_has_exact_launch_cwd(
        &self,
        workspace: &WorkspaceInfo,
        root_cwd: &Path,
    ) -> bool {
        workspace
            .worktree
            .as_ref()
            .is_some_and(|worktree| worktree.checkout_path == root_cwd)
            || self.panes.iter().any(|pane| {
                pane.workspace_id == workspace.workspace_id && pane.cwd.as_deref() == Some(root_cwd)
            })
    }

    pub fn workspace_by_unique_exact_label_and_root_cwd(
        &self,
        label: &str,
        root_cwd: &Path,
    ) -> Result<Option<&WorkspaceInfo>> {
        let matches: Vec<_> = self
            .workspaces
            .iter()
            .filter(|workspace| {
                workspace.label == label && self.workspace_has_exact_launch_cwd(workspace, root_cwd)
            })
            .collect();
        unique_recovery_match(matches, label, root_cwd)
    }

    /// Recovers by stable ID first, then by a unique exact label + launch-cwd
    /// match. Ambiguity is an error rather than an arbitrary adoption.
    pub fn recover_workspace(
        &self,
        managed: &ManagedWorkspace,
    ) -> Result<Option<RecoveredWorkspace<'_>>> {
        if let Some(workspace) = self.workspace_by_exact_id(&managed.workspace_id) {
            if workspace.label != managed.label
                || !self.workspace_has_exact_launch_cwd(workspace, &managed.root_cwd)
            {
                return Err(Error::OwnershipMismatch {
                    workspace_id: managed.workspace_id.clone(),
                    label: managed.label.clone(),
                    root_cwd: managed.root_cwd.clone(),
                });
            }
            return Ok(Some(RecoveredWorkspace {
                workspace,
                strategy: RecoveryStrategy::ExactId,
            }));
        }
        Ok(self
            .workspace_by_unique_exact_label_and_root_cwd(&managed.label, &managed.root_cwd)?
            .map(|workspace| RecoveredWorkspace {
                workspace,
                strategy: RecoveryStrategy::ExactLabelAndRootCwd,
            }))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RecoveryStrategy {
    ExactId,
    ExactLabelAndRootCwd,
}

#[derive(Clone, Copy, Debug)]
pub struct RecoveredWorkspace<'a> {
    pub workspace: &'a WorkspaceInfo,
    pub strategy: RecoveryStrategy,
}

fn unique_recovery_match<'a>(
    matches: Vec<&'a WorkspaceInfo>,
    label: &str,
    root_cwd: &Path,
) -> Result<Option<&'a WorkspaceInfo>> {
    match matches.len() {
        0 => Ok(None),
        1 => Ok(matches.into_iter().next()),
        count => Err(Error::AmbiguousRecovery {
            label: label.to_string(),
            root_cwd: root_cwd.to_path_buf(),
            matches: count,
        }),
    }
}

#[derive(Deserialize)]
struct SnapshotResponse {
    result: SnapshotResult,
}

#[derive(Deserialize)]
struct SnapshotResult {
    #[serde(rename = "type")]
    result_type: String,
    snapshot: SessionSnapshot,
}

#[derive(Deserialize)]
struct WorkspaceListResponse {
    result: WorkspaceListResult,
}

#[derive(Deserialize)]
struct WorkspaceListResult {
    #[serde(rename = "type")]
    result_type: String,
    workspaces: Vec<WorkspaceInfo>,
}

#[derive(Deserialize)]
struct WorkspaceInfoResponse {
    result: WorkspaceInfoResult,
}

#[derive(Deserialize)]
struct WorkspaceInfoResult {
    #[serde(rename = "type")]
    result_type: String,
    workspace: WorkspaceInfo,
}

#[derive(Deserialize)]
struct WorkspaceCreatedResponse {
    result: WorkspaceCreatedResult,
}

#[derive(Deserialize)]
struct WorkspaceCreatedResult {
    #[serde(rename = "type")]
    result_type: String,
    workspace: WorkspaceInfo,
    tab: TabInfo,
    root_pane: PaneInfo,
}

#[derive(Deserialize)]
struct TabInfoResponse {
    result: TabInfoResult,
}

#[derive(Deserialize)]
struct TabInfoResult {
    #[serde(rename = "type")]
    result_type: String,
    tab: TabInfo,
}

#[derive(Deserialize)]
struct TabCreatedResponse {
    result: TabCreatedResult,
}

#[derive(Deserialize)]
struct TabCreatedResult {
    #[serde(rename = "type")]
    result_type: String,
    tab: TabInfo,
    root_pane: PaneInfo,
}

#[derive(Deserialize)]
struct PaneInfoResponse {
    result: PaneInfoResult,
}

#[derive(Deserialize)]
struct PaneInfoResult {
    #[serde(rename = "type")]
    result_type: String,
    pane: PaneInfo,
}

#[cfg(unix)]
#[derive(Deserialize)]
struct DirectPaneFocusResponse {
    id: String,
    result: PaneInfoResult,
}

#[derive(Deserialize)]
struct PaneFocusResponse {
    result: PaneFocusResponseResult,
}

#[derive(Deserialize)]
struct PaneFocusResponseResult {
    #[serde(rename = "type")]
    result_type: String,
    focus: PaneFocusResult,
}

#[derive(Deserialize)]
struct OkResponse {
    result: OkResult,
}

#[derive(Deserialize)]
struct OkResult {
    #[serde(rename = "type")]
    result_type: String,
}

fn expect_result_type(found: &str, expected: &'static str) -> Result<()> {
    if found == expected {
        Ok(())
    } else {
        Err(Error::UnexpectedResult {
            expected,
            found: found.to_string(),
        })
    }
}

fn push_string_option(args: &mut Vec<OsString>, flag: &str, value: Option<&str>) {
    if let Some(value) = value {
        args.push(OsString::from(flag));
        args.push(OsString::from(value));
    }
}

fn push_path_option(args: &mut Vec<OsString>, flag: &str, value: Option<&Path>) {
    if let Some(value) = value {
        args.push(OsString::from(flag));
        args.push(value.as_os_str().to_owned());
    }
}

fn push_env_options(args: &mut Vec<OsString>, env: &BTreeMap<String, String>) {
    for (key, value) in env {
        args.push(OsString::from("--env"));
        args.push(OsString::from(format!("{key}={value}")));
    }
}

#[derive(Deserialize)]
struct CliErrorEnvelope {
    error: CliErrorBody,
}

#[derive(Deserialize)]
struct CliErrorBody {
    code: String,
    message: String,
}

fn parse_cli_error(output: &[u8]) -> Option<CliErrorBody> {
    serde_json::from_slice::<CliErrorEnvelope>(output)
        .ok()
        .map(|envelope| envelope.error)
}

/// One bootmux-owned Herdr workspace. Paths are intentionally compared
/// lexically: adoption must not silently cross symlink or case boundaries.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ManagedWorkspace {
    /// Canonical socket identity used for ownership comparisons.
    pub endpoint: Endpoint,
    /// Selector used to restart the same server, especially a named session.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub launch_endpoint: Option<Endpoint>,
    pub workspace_id: String,
    pub label: String,
    pub root_cwd: PathBuf,
    pub config_path: PathBuf,
    pub project_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub root_pane_id: Option<String>,
    /// Fully rendered at start time so stop-all does not need template inputs.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stop_hook: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct StateIndex {
    pub version: u32,
    #[serde(default)]
    pub managed_workspaces: Vec<ManagedWorkspace>,
}

impl Default for StateIndex {
    fn default() -> Self {
        Self {
            version: STATE_INDEX_VERSION,
            managed_workspaces: Vec::new(),
        }
    }
}

impl StateIndex {
    pub fn managed_by_exact_id(
        &self,
        endpoint: &Endpoint,
        workspace_id: &str,
    ) -> Option<&ManagedWorkspace> {
        self.managed_workspaces
            .iter()
            .find(|managed| &managed.endpoint == endpoint && managed.workspace_id == workspace_id)
    }

    pub fn managed_by_unique_exact_label_and_root_cwd(
        &self,
        endpoint: &Endpoint,
        label: &str,
        root_cwd: &Path,
    ) -> Result<Option<&ManagedWorkspace>> {
        let mut matches = self.managed_workspaces.iter().filter(|managed| {
            &managed.endpoint == endpoint && managed.label == label && managed.root_cwd == root_cwd
        });
        let first = matches.next();
        if first.is_some() && matches.next().is_some() {
            let count = self
                .managed_workspaces
                .iter()
                .filter(|managed| {
                    &managed.endpoint == endpoint
                        && managed.label == label
                        && managed.root_cwd == root_cwd
                })
                .count();
            return Err(Error::AmbiguousRecovery {
                label: label.to_string(),
                root_cwd: root_cwd.to_path_buf(),
                matches: count,
            });
        }
        Ok(first)
    }

    pub fn recover_managed(
        &self,
        endpoint: &Endpoint,
        workspace_id: &str,
        label: &str,
        root_cwd: &Path,
    ) -> Result<Option<&ManagedWorkspace>> {
        if let Some(managed) = self.managed_by_exact_id(endpoint, workspace_id) {
            return Ok(Some(managed));
        }
        self.managed_by_unique_exact_label_and_root_cwd(endpoint, label, root_cwd)
    }

    pub fn upsert(&mut self, managed: ManagedWorkspace) {
        if let Some(existing) = self.managed_workspaces.iter_mut().find(|existing| {
            existing.endpoint == managed.endpoint && existing.workspace_id == managed.workspace_id
        }) {
            *existing = managed;
        } else {
            self.managed_workspaces.push(managed);
        }
    }

    pub fn remove_exact_id(&mut self, endpoint: &Endpoint, workspace_id: &str) -> bool {
        let previous_len = self.managed_workspaces.len();
        self.managed_workspaces.retain(|managed| {
            &managed.endpoint != endpoint || managed.workspace_id != workspace_id
        });
        self.managed_workspaces.len() != previous_len
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LockOptions {
    pub timeout: Duration,
    pub poll_interval: Duration,
}

impl Default for LockOptions {
    fn default() -> Self {
        Self {
            timeout: DEFAULT_LOCK_TIMEOUT,
            poll_interval: DEFAULT_LOCK_POLL,
        }
    }
}

/// Versioned JSON store under the XDG state directory.
#[derive(Clone, Debug)]
pub struct StateStore {
    path: PathBuf,
    lock_options: LockOptions,
}

impl StateStore {
    pub fn xdg() -> Result<Self> {
        Ok(Self::new(default_state_index_path()?))
    }

    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self {
            path: path.into(),
            lock_options: LockOptions::default(),
        }
    }

    pub fn with_lock_options(mut self, options: LockOptions) -> Self {
        self.lock_options = options;
        self
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn lock_path(&self) -> PathBuf {
        let mut name = self
            .path
            .file_name()
            .unwrap_or_else(|| OsStr::new(STATE_INDEX_FILE_NAME))
            .to_os_string();
        name.push(".lock");
        self.path.with_file_name(name)
    }

    pub fn operation_lock_path(&self) -> PathBuf {
        let mut name = self
            .path
            .file_name()
            .unwrap_or_else(|| OsStr::new(STATE_INDEX_FILE_NAME))
            .to_os_string();
        name.push(".operation.lock");
        self.path.with_file_name(name)
    }

    /// Serializes Herdr lifecycle decisions across bootmux processes.
    ///
    /// This advisory OS lock is intentionally separate from the short-lived
    /// JSON transaction lock. The kernel releases it if a process exits, so a
    /// crashed start cannot leave a stale lifecycle lock behind.
    pub fn acquire_operation_lock(&self) -> Result<StateOperationLock> {
        let lock_path = self.operation_lock_path();
        let file = acquire_advisory_file_lock(
            &lock_path,
            DEFAULT_OPERATION_LOCK_TIMEOUT,
            DEFAULT_LOCK_POLL,
        )?;
        Ok(StateOperationLock { _file: file })
    }

    pub fn load(&self) -> Result<StateIndex> {
        self.load_unlocked()
    }

    pub fn save(&self, index: &StateIndex) -> Result<()> {
        let _lock = self.acquire_lock()?;
        self.save_unlocked(index)
    }

    /// Holds the create-new lock across the full read/modify/atomic-write
    /// transaction.
    pub fn update<T>(&self, update: impl FnOnce(&mut StateIndex) -> Result<T>) -> Result<T> {
        let _lock = self.acquire_lock()?;
        let mut index = self.load_unlocked()?;
        let result = update(&mut index)?;
        self.save_unlocked(&index)?;
        Ok(result)
    }

    fn load_unlocked(&self) -> Result<StateIndex> {
        let mut file = match File::open(&self.path) {
            Ok(file) => file,
            Err(source) if source.kind() == io::ErrorKind::NotFound => {
                return Ok(StateIndex::default())
            }
            Err(source) => {
                return Err(Error::StateIo {
                    operation: "open",
                    path: self.path.clone(),
                    source,
                })
            }
        };
        let mut bytes = Vec::new();
        file.read_to_end(&mut bytes)
            .map_err(|source| Error::StateIo {
                operation: "read",
                path: self.path.clone(),
                source,
            })?;
        let index: StateIndex =
            serde_json::from_slice(&bytes).map_err(|source| Error::StateJson {
                operation: "parse",
                path: self.path.clone(),
                source,
            })?;
        if index.version != STATE_INDEX_VERSION {
            return Err(Error::UnsupportedStateVersion {
                path: self.path.clone(),
                found: index.version,
                supported: STATE_INDEX_VERSION,
            });
        }
        Ok(index)
    }

    fn save_unlocked(&self, index: &StateIndex) -> Result<()> {
        if index.version != STATE_INDEX_VERSION {
            return Err(Error::UnsupportedStateVersion {
                path: self.path.clone(),
                found: index.version,
                supported: STATE_INDEX_VERSION,
            });
        }
        let parent = usable_parent(&self.path);
        create_private_dir(parent)?;
        let temp_path = temporary_path_for(&self.path);
        let mut cleanup = TempFileCleanup(Some(temp_path.clone()));
        let mut file = open_private_create_new(&temp_path).map_err(|source| Error::StateIo {
            operation: "create temporary file",
            path: temp_path.clone(),
            source,
        })?;
        serde_json::to_writer_pretty(&mut file, index).map_err(|source| Error::StateJson {
            operation: "serialize",
            path: temp_path.clone(),
            source,
        })?;
        file.write_all(b"\n").map_err(|source| Error::StateIo {
            operation: "write",
            path: temp_path.clone(),
            source,
        })?;
        file.sync_all().map_err(|source| Error::StateIo {
            operation: "sync",
            path: temp_path.clone(),
            source,
        })?;
        drop(file);
        fs::rename(&temp_path, &self.path).map_err(|source| Error::StateIo {
            operation: "atomic rename",
            path: self.path.clone(),
            source,
        })?;
        cleanup.0 = None;
        Ok(())
    }

    fn acquire_lock(&self) -> Result<StateLock> {
        let lock_path = self.lock_path();
        let file = acquire_advisory_file_lock(
            &lock_path,
            self.lock_options.timeout,
            self.lock_options.poll_interval,
        )?;
        Ok(StateLock { _file: file })
    }
}

pub struct StateOperationLock {
    _file: File,
}

struct StateLock {
    _file: File,
}

fn acquire_advisory_file_lock(
    path: &Path,
    timeout: Duration,
    poll_interval: Duration,
) -> Result<File> {
    let parent = usable_parent(path);
    create_private_dir(parent)?;
    let mut options = OpenOptions::new();
    options.read(true).write(true).create(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let file = options.open(path).map_err(|source| Error::StateIo {
        operation: "open lock file",
        path: path.to_path_buf(),
        source,
    })?;
    set_lock_close_on_exec(&file, path)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        file.set_permissions(fs::Permissions::from_mode(0o600))
            .map_err(|source| Error::StateIo {
                operation: "set lock file permissions",
                path: path.to_path_buf(),
                source,
            })?;
    }

    let deadline = Instant::now() + timeout;
    loop {
        match file.try_lock() {
            Ok(()) => return Ok(file),
            Err(fs::TryLockError::WouldBlock) => {
                let now = Instant::now();
                if now >= deadline {
                    return Err(Error::StateLocked {
                        path: path.to_path_buf(),
                        timeout,
                    });
                }
                thread::sleep(poll_interval.min(deadline - now));
            }
            Err(fs::TryLockError::Error(source)) => {
                return Err(Error::StateIo {
                    operation: "lock file",
                    path: path.to_path_buf(),
                    source,
                })
            }
        }
    }
}

#[cfg(unix)]
fn set_lock_close_on_exec(file: &File, path: &Path) -> Result<()> {
    use std::os::fd::AsRawFd;

    const F_GETFD: i32 = 1;
    const F_SETFD: i32 = 2;
    const FD_CLOEXEC: i32 = 1;

    unsafe extern "C" {
        fn fcntl(fd: i32, command: i32, ...) -> i32;
    }

    let descriptor = file.as_raw_fd();
    // SAFETY: `fcntl` is called with the documented integer commands for the
    // valid descriptor borrowed from `file`; neither call takes a pointer.
    let flags = unsafe { fcntl(descriptor, F_GETFD) };
    if flags == -1 {
        return Err(Error::StateIo {
            operation: "read lock close-on-exec flag",
            path: path.to_path_buf(),
            source: io::Error::last_os_error(),
        });
    }
    if unsafe { fcntl(descriptor, F_SETFD, flags | FD_CLOEXEC) } == -1 {
        return Err(Error::StateIo {
            operation: "set lock close-on-exec flag",
            path: path.to_path_buf(),
            source: io::Error::last_os_error(),
        });
    }
    Ok(())
}

#[cfg(not(unix))]
fn set_lock_close_on_exec(_file: &File, _path: &Path) -> Result<()> {
    Ok(())
}

struct TempFileCleanup(Option<PathBuf>);

impl Drop for TempFileCleanup {
    fn drop(&mut self) {
        if let Some(path) = self.0.take() {
            let _ = fs::remove_file(path);
        }
    }
}

static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

fn temporary_path_for(path: &Path) -> PathBuf {
    let counter = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
    let mut name = path
        .file_name()
        .unwrap_or_else(|| OsStr::new(STATE_INDEX_FILE_NAME))
        .to_os_string();
    name.push(format!(".tmp.{}.{}", std::process::id(), counter));
    path.with_file_name(name)
}

fn create_private_dir(path: &Path) -> Result<()> {
    let existed = path.exists();
    fs::create_dir_all(path).map_err(|source| Error::StateIo {
        operation: "create directory",
        path: path.to_path_buf(),
        source,
    })?;
    #[cfg(unix)]
    if !existed {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o700)).map_err(|source| {
            Error::StateIo {
                operation: "set directory permissions",
                path: path.to_path_buf(),
                source,
            }
        })?;
    }
    Ok(())
}

fn usable_parent(path: &Path) -> &Path {
    path.parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."))
}

fn open_private_create_new(path: &Path) -> io::Result<File> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let file = options.open(path)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        file.set_permissions(fs::Permissions::from_mode(0o600))?;
    }
    Ok(file)
}

pub fn default_state_index_path() -> Result<PathBuf> {
    Ok(xdg_state_home()?
        .join("bootmux")
        .join(STATE_INDEX_FILE_NAME))
}

pub fn xdg_state_home() -> Result<PathBuf> {
    if let Some(path) = env::var_os("XDG_STATE_HOME")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
    {
        return Ok(path);
    }
    env::var_os("HOME")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .map(|home| home.join(".local").join("state"))
        .ok_or(Error::StateHomeUnavailable)
}

#[derive(Debug)]
pub enum Error {
    Execute {
        command: String,
        source: io::Error,
    },
    Spawn {
        command: String,
        source: io::Error,
    },
    CommandFailed {
        command: String,
        status: Option<i32>,
        stdout: String,
        stderr: String,
    },
    Cli {
        command: String,
        status: Option<i32>,
        code: String,
        message: String,
    },
    InvalidJson {
        command: String,
        output: String,
        source: serde_json::Error,
    },
    UnexpectedOutput {
        command: String,
        output: String,
    },
    UnexpectedResult {
        expected: &'static str,
        found: String,
    },
    InvalidArgument {
        name: &'static str,
        message: String,
    },
    InvalidVersion {
        component: &'static str,
        found: String,
    },
    UnsupportedVersion {
        component: &'static str,
        found: String,
        minimum: &'static str,
    },
    UnsupportedProtocol {
        component: &'static str,
        found: u32,
        required: u32,
    },
    ProtocolMismatch {
        client: u32,
        server: u32,
    },
    MissingStatusField {
        component: &'static str,
        field: &'static str,
    },
    ServerNotRunning {
        socket: PathBuf,
    },
    ServerNotReady {
        timeout: Duration,
        last_error: Option<String>,
    },
    SocketIo {
        operation: &'static str,
        path: PathBuf,
        source: io::Error,
    },
    SocketJson {
        operation: &'static str,
        source: serde_json::Error,
    },
    UnexpectedResponseId {
        expected: &'static str,
        found: String,
    },
    UnsupportedOperation {
        operation: &'static str,
        reason: &'static str,
    },
    AmbiguousRecovery {
        label: String,
        root_cwd: PathBuf,
        matches: usize,
    },
    OwnershipMismatch {
        workspace_id: String,
        label: String,
        root_cwd: PathBuf,
    },
    StateHomeUnavailable,
    StatePath {
        path: PathBuf,
        message: String,
    },
    StateIo {
        operation: &'static str,
        path: PathBuf,
        source: io::Error,
    },
    StateJson {
        operation: &'static str,
        path: PathBuf,
        source: serde_json::Error,
    },
    UnsupportedStateVersion {
        path: PathBuf,
        found: u32,
        supported: u32,
    },
    StateLocked {
        path: PathBuf,
        timeout: Duration,
    },
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Execute { command, source } => {
                write!(f, "failed to execute `{command}`: {source}")
            }
            Error::Spawn { command, source } => {
                write!(f, "failed to start detached `{command}`: {source}")
            }
            Error::CommandFailed {
                command,
                status,
                stdout,
                stderr,
            } => {
                write!(
                    f,
                    "`{command}` failed with status {}",
                    status.map_or_else(|| "unknown".to_string(), |code| code.to_string())
                )?;
                if !stderr.is_empty() {
                    write!(f, ": {stderr}")?;
                } else if !stdout.is_empty() {
                    write!(f, ": {stdout}")?;
                }
                Ok(())
            }
            Error::Cli {
                command,
                status,
                code,
                message,
            } => write!(
                f,
                "`{command}` failed with status {} ({code}): {message}",
                status.map_or_else(|| "unknown".to_string(), |value| value.to_string())
            ),
            Error::InvalidJson {
                command,
                output,
                source,
            } => write!(
                f,
                "`{command}` returned invalid JSON ({source}): {output}"
            ),
            Error::UnexpectedOutput { command, output } => {
                write!(f, "`{command}` returned unexpected output: {output}")
            }
            Error::UnexpectedResult { expected, found } => {
                write!(f, "expected Herdr result `{expected}`, got `{found}`")
            }
            Error::InvalidArgument { name, message } => {
                write!(f, "invalid {name}: {message}")
            }
            Error::InvalidVersion { component, found } => {
                write!(f, "{component} reported invalid Herdr version `{found}`")
            }
            Error::UnsupportedVersion {
                component,
                found,
                minimum,
            } => write!(
                f,
                "{component} Herdr version {found} is unsupported; require >= {minimum}"
            ),
            Error::UnsupportedProtocol {
                component,
                found,
                required,
            } => write!(
                f,
                "{component} Herdr protocol {found} is unsupported; require protocol {required}"
            ),
            Error::ProtocolMismatch { client, server } => write!(
                f,
                "Herdr client/server protocol mismatch (client {client}, server {server})"
            ),
            Error::MissingStatusField { component, field } => {
                write!(f, "{component} Herdr status is missing `{field}`")
            }
            Error::ServerNotRunning { socket } => {
                write!(f, "Herdr server is not running at {}", socket.display())
            }
            Error::ServerNotReady {
                timeout,
                last_error,
            } => {
                write!(
                    f,
                    "Herdr server did not become ready within {}ms",
                    timeout.as_millis()
                )?;
                if let Some(last_error) = last_error {
                    write!(f, ": {last_error}")?;
                }
                Ok(())
            }
            Error::SocketIo {
                operation,
                path,
                source,
            } => write!(
                f,
                "failed to {operation} Herdr socket at {}: {source}",
                path.display()
            ),
            Error::SocketJson { operation, source } => {
                write!(f, "failed to {operation}: {source}")
            }
            Error::UnexpectedResponseId { expected, found } => write!(
                f,
                "Herdr socket response id mismatch: expected `{expected}`, got `{found}`"
            ),
            Error::UnsupportedOperation { operation, reason } => {
                write!(f, "{operation} is unsupported: {reason}")
            }
            Error::AmbiguousRecovery {
                label,
                root_cwd,
                matches,
            } => write!(
                f,
                "cannot recover Herdr workspace: {matches} exact matches for label `{label}` and cwd {}",
                root_cwd.display()
            ),
            Error::OwnershipMismatch {
                workspace_id,
                label,
                root_cwd,
            } => write!(
                f,
                "managed Herdr workspace `{workspace_id}` still exists but no longer matches owned label `{label}` and root {}; refusing to discard ownership",
                root_cwd.display()
            ),
            Error::StateHomeUnavailable => {
                write!(f, "cannot locate XDG state home (HOME is unset)")
            }
            Error::StatePath { path, message } => {
                write!(f, "invalid state path {}: {message}", path.display())
            }
            Error::StateIo {
                operation,
                path,
                source,
            } => write!(
                f,
                "failed to {operation} state at {}: {source}",
                path.display()
            ),
            Error::StateJson {
                operation,
                path,
                source,
            } => write!(
                f,
                "failed to {operation} state JSON at {}: {source}",
                path.display()
            ),
            Error::UnsupportedStateVersion {
                path,
                found,
                supported,
            } => write!(
                f,
                "state index {} has version {found}; supported version is {supported}",
                path.display()
            ),
            Error::StateLocked { path, timeout } => write!(
                f,
                "state index is locked at {} after waiting {}ms",
                path.display(),
                timeout.as_millis()
            ),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Error::Execute { source, .. }
            | Error::Spawn { source, .. }
            | Error::SocketIo { source, .. }
            | Error::StateIo { source, .. } => Some(source),
            Error::InvalidJson { source, .. }
            | Error::SocketJson { source, .. }
            | Error::StateJson { source, .. } => Some(source),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;
    use std::process::{Command, Stdio};
    use std::sync::Mutex;

    #[derive(Default)]
    struct FakeRunner {
        outputs: Mutex<VecDeque<CommandOutput>>,
        invocations: Mutex<Vec<Invocation>>,
        detached: Mutex<Vec<Invocation>>,
    }

    impl FakeRunner {
        fn with_outputs(outputs: impl IntoIterator<Item = CommandOutput>) -> Self {
            Self {
                outputs: Mutex::new(outputs.into_iter().collect()),
                ..Self::default()
            }
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
            self.detached.lock().unwrap().push(invocation.clone());
            Ok(42)
        }
    }

    fn success(json: &str) -> CommandOutput {
        CommandOutput {
            success: true,
            code: Some(0),
            stdout: json.as_bytes().to_vec(),
            stderr: Vec::new(),
        }
    }

    #[test]
    fn endpoint_clears_ambient_selectors_and_applies_named_session() {
        let runner = FakeRunner::with_outputs([success(
            r#"{"result":{"type":"workspace_list","workspaces":[]}}"#,
        )]);
        let herdr = Herdr::with_runner("herdr", Endpoint::NamedSession("work".into()), runner);
        herdr.list_workspaces().unwrap();

        let invocation = herdr.runner.invocations.lock().unwrap()[0].clone();
        assert_eq!(
            invocation.args,
            ["--session", "work", "workspace", "list"]
                .into_iter()
                .map(OsString::from)
                .collect::<Vec<_>>()
        );
        assert_eq!(
            invocation.env.get(OsStr::new(HERDR_SOCKET_PATH)),
            Some(&None)
        );
        assert_eq!(invocation.env.get(OsStr::new(HERDR_SESSION)), Some(&None));
    }

    #[test]
    fn pane_focus_uses_directional_cli_shape() {
        let runner = FakeRunner::with_outputs([success(
            r#"{"result":{"type":"pane_focus_direction","focus":{"changed":true,"source_pane_id":"w:p1","focused_pane_id":"w:p2"}}}"#,
        )]);
        let herdr = Herdr::with_runner("herdr", Endpoint::Default, runner);
        let result = herdr
            .focus_pane_neighbor(&PaneFocus {
                source: PaneTarget::PaneId("w:p1".into()),
                direction: PaneDirection::Right,
            })
            .unwrap();
        assert_eq!(result.focused_pane_id.as_deref(), Some("w:p2"));
        assert_eq!(
            herdr.runner.invocations.lock().unwrap()[0].args,
            ["pane", "focus", "--direction", "right", "--pane", "w:p1"]
                .into_iter()
                .map(OsString::from)
                .collect::<Vec<_>>()
        );
    }

    #[cfg(unix)]
    #[test]
    fn direct_pane_focus_uses_protocol_17_jsonl_without_a_live_server() {
        use std::io::{BufRead, BufReader};
        use std::os::unix::net::UnixListener;

        let temp = tempfile::tempdir().unwrap();
        let socket = temp.path().join("herdr.sock");
        let listener = UnixListener::bind(&socket).unwrap();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request_line = String::new();
            BufReader::new(stream.try_clone().unwrap())
                .read_line(&mut request_line)
                .unwrap();
            let request: serde_json::Value = serde_json::from_str(&request_line).unwrap();
            assert_eq!(request["method"], "pane.focus");
            assert_eq!(request["params"]["pane_id"], "w:p2");
            stream
                .write_all(
                    br#"{"id":"bootmux:pane:focus","result":{"type":"pane_info","pane":{"pane_id":"w:p2","terminal_id":"t2","workspace_id":"w","tab_id":"w:1","focused":true,"cwd":"/repo","revision":1}}}
"#,
                )
                .unwrap();
        });

        let runner = FakeRunner::with_outputs([success(&format!(
            r#"{{"status":"running","running":true,"version":"0.7.5","protocol":17,"compatible":true,"socket":{}}}"#,
            serde_json::to_string(&socket.to_string_lossy()).unwrap()
        ))]);
        let herdr = Herdr::with_runner("herdr", Endpoint::Default, runner);
        let pane = herdr.focus_pane_direct("w:p2").unwrap();
        assert_eq!(pane.pane_id, "w:p2");
        assert!(pane.focused);
        server.join().unwrap();
    }

    #[test]
    fn ensure_server_spawns_then_polls_without_touching_a_real_server() {
        let runner = FakeRunner::with_outputs([
            success(
                r#"{"client":{"version":"0.7.5","protocol":17},"server":{"status":"not_running","running":false,"socket":"/tmp/h.sock"}}"#,
            ),
            success(
                r#"{"status":"running","running":true,"version":"0.7.5","protocol":17,"compatible":true,"socket":"/tmp/h.sock"}"#,
            ),
        ]);
        let herdr = Herdr::with_runner("herdr", Endpoint::SocketPath("/tmp/h.sock".into()), runner);
        let status = herdr
            .ensure_server_with(ReadinessOptions {
                timeout: Duration::ZERO,
                poll_interval: Duration::ZERO,
            })
            .unwrap();
        assert!(status.running);
        assert_eq!(herdr.runner.detached.lock().unwrap().len(), 1);
    }

    #[test]
    fn snapshot_recovery_prefers_id_and_rejects_ambiguous_fallback() {
        let workspace = |id: &str| WorkspaceInfo {
            workspace_id: id.into(),
            number: 1,
            label: "api".into(),
            focused: false,
            pane_count: 1,
            tab_count: 1,
            active_tab_id: format!("{id}:1"),
            worktree: None,
        };
        let pane = |id: &str, workspace_id: &str| PaneInfo {
            pane_id: id.into(),
            terminal_id: format!("t-{id}"),
            workspace_id: workspace_id.into(),
            tab_id: format!("{workspace_id}:1"),
            focused: false,
            cwd: Some("/repo".into()),
            foreground_cwd: None,
            label: None,
            agent: None,
            title: None,
            revision: 0,
        };
        let snapshot = SessionSnapshot {
            version: "0.7.5".into(),
            protocol: 17,
            focused_workspace_id: None,
            focused_tab_id: None,
            focused_pane_id: None,
            workspaces: vec![workspace("w1"), workspace("w2")],
            tabs: vec![],
            panes: vec![pane("w1:p1", "w1"), pane("w2:p1", "w2")],
        };
        let managed = ManagedWorkspace {
            endpoint: Endpoint::Default,
            launch_endpoint: None,
            workspace_id: "w1".into(),
            label: "api".into(),
            root_cwd: "/repo".into(),
            config_path: "/repo/.bootmux.yml".into(),
            project_name: "api".into(),
            root_pane_id: Some("w1:p1".into()),
            stop_hook: Some("echo stopped".into()),
        };
        assert_eq!(
            snapshot
                .recover_workspace(&managed)
                .unwrap()
                .unwrap()
                .strategy,
            RecoveryStrategy::ExactId
        );

        let renamed = ManagedWorkspace {
            label: "renamed".into(),
            ..managed.clone()
        };
        assert!(matches!(
            snapshot.recover_workspace(&renamed),
            Err(Error::OwnershipMismatch { .. })
        ));

        let missing = ManagedWorkspace {
            workspace_id: "gone".into(),
            ..managed
        };
        assert!(matches!(
            snapshot.recover_workspace(&missing),
            Err(Error::AmbiguousRecovery { matches: 2, .. })
        ));
    }

    #[test]
    fn state_store_round_trips_with_private_atomic_file() {
        let temp = tempfile::tempdir().unwrap();
        let store = StateStore::new(temp.path().join("state").join(STATE_INDEX_FILE_NAME))
            .with_lock_options(LockOptions {
                timeout: Duration::ZERO,
                poll_interval: Duration::ZERO,
            });
        let managed = ManagedWorkspace {
            endpoint: Endpoint::Default,
            launch_endpoint: None,
            workspace_id: "w1".into(),
            label: "api".into(),
            root_cwd: "/repo".into(),
            config_path: "/repo/.bootmux.yml".into(),
            project_name: "api".into(),
            root_pane_id: Some("w1:p1".into()),
            stop_hook: Some("echo stopped".into()),
        };
        store
            .update(|index| {
                index.upsert(managed.clone());
                Ok(())
            })
            .unwrap();
        assert_eq!(
            store
                .load()
                .unwrap()
                .managed_by_exact_id(&Endpoint::Default, "w1"),
            Some(&managed)
        );
        assert!(store.lock_path().is_file());

        let mut legacy_json = serde_json::to_value(&managed).unwrap();
        legacy_json.as_object_mut().unwrap().remove("stop_hook");
        let legacy: ManagedWorkspace = serde_json::from_value(legacy_json).unwrap();
        assert_eq!(legacy.stop_hook, None);

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(store.path()).unwrap().permissions().mode() & 0o777,
                0o600
            );
            assert_eq!(
                fs::metadata(store.lock_path())
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777,
                0o600
            );
        }
    }

    #[test]
    fn state_store_serializes_concurrent_updates() {
        let temp = tempfile::tempdir().unwrap();
        let store = std::sync::Arc::new(StateStore::new(
            temp.path().join("state").join(STATE_INDEX_FILE_NAME),
        ));
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(8));
        let workers = (0..8)
            .map(|index| {
                let store = store.clone();
                let barrier = barrier.clone();
                std::thread::spawn(move || {
                    barrier.wait();
                    store
                        .update(|state| {
                            state.upsert(ManagedWorkspace {
                                endpoint: Endpoint::Default,
                                launch_endpoint: None,
                                workspace_id: format!("w{index}"),
                                label: format!("project-{index}"),
                                root_cwd: format!("/repo/{index}").into(),
                                config_path: format!("/repo/{index}.yml").into(),
                                project_name: format!("project-{index}"),
                                root_pane_id: Some(format!("w{index}:p1")),
                                stop_hook: None,
                            });
                            Ok(())
                        })
                        .unwrap();
                })
            })
            .collect::<Vec<_>>();
        for worker in workers {
            worker.join().unwrap();
        }
        let state = store.load().unwrap();
        assert_eq!(state.managed_workspaces.len(), 8);
        assert!(store.lock_path().is_file());
    }

    #[cfg(unix)]
    #[test]
    fn advisory_lock_is_not_inherited_by_spawned_processes() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("lifecycle.lock");
        let lock = acquire_advisory_file_lock(&path, Duration::ZERO, Duration::ZERO).unwrap();
        let mut child = Command::new("/bin/sh")
            .args(["-c", "sleep 5"])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        drop(lock);

        let reacquired = acquire_advisory_file_lock(
            &path,
            Duration::from_millis(500),
            Duration::from_millis(10),
        );
        let _ = child.kill();
        let _ = child.wait();
        assert!(
            reacquired.is_ok(),
            "a spawned child retained the lifecycle lock: {reacquired:?}"
        );
    }

    #[test]
    fn topology_operations_use_protocol_17_cli_shapes_and_thread_ids() {
        let workspace = serde_json::json!({
            "workspace_id": "w1", "number": 1, "label": "api",
            "pane_count": 1, "tab_count": 1, "active_tab_id": "w1:t1"
        });
        let tab1 = serde_json::json!({
            "tab_id": "w1:t1", "workspace_id": "w1", "number": 1,
            "label": "1", "pane_count": 1
        });
        let tab2 = serde_json::json!({
            "tab_id": "w1:t2", "workspace_id": "w1", "number": 2,
            "label": "logs", "pane_count": 1
        });
        let pane = |id: &str, tab: &str| {
            serde_json::json!({
                "pane_id": id, "terminal_id": format!("term-{id}"),
                "workspace_id": "w1", "tab_id": tab, "cwd": "/repo"
            })
        };
        let outputs = [
            success(
                &serde_json::json!({
                    "result": {
                        "type": "workspace_created",
                        "workspace": workspace,
                        "tab": tab1,
                        "root_pane": pane("w1:p1", "w1:t1")
                    }
                })
                .to_string(),
            ),
            success(
                &serde_json::json!({
                    "result": {
                        "type": "tab_created",
                        "tab": tab2,
                        "root_pane": pane("w1:p2", "w1:t2")
                    }
                })
                .to_string(),
            ),
            success(
                &serde_json::json!({
                    "result": {"type": "pane_info", "pane": pane("w1:p3", "w1:t2")}
                })
                .to_string(),
            ),
            success(
                &serde_json::json!({
                    "result": {"type": "pane_info", "pane": pane("w1:p3", "w1:t2")}
                })
                .to_string(),
            ),
            success(""),
            success(r#"{"result":{"type":"ok"}}"#),
        ];
        let runner = FakeRunner::with_outputs(outputs);
        let herdr = Herdr::with_runner("herdr", Endpoint::NamedSession("isolated".into()), runner);

        let created = herdr
            .create_workspace(&WorkspaceCreate {
                cwd: Some("/repo".into()),
                label: Some("api".into()),
                focus: false,
                ..WorkspaceCreate::default()
            })
            .unwrap();
        assert_eq!(created.root_pane.pane_id, "w1:p1");
        let tab = herdr
            .create_tab(&TabCreate {
                workspace_id: Some(created.workspace.workspace_id),
                cwd: Some("/repo/logs".into()),
                label: Some("logs".into()),
                focus: false,
                ..TabCreate::default()
            })
            .unwrap();
        let split = herdr
            .split_pane(&PaneSplit {
                target: PaneTarget::PaneId(tab.root_pane.pane_id),
                direction: SplitDirection::Down,
                ratio: Some(0.65),
                cwd: Some("/repo/logs".into()),
                env: BTreeMap::new(),
                focus: false,
            })
            .unwrap();
        herdr.rename_pane(&split.pane_id, Some("watcher")).unwrap();
        herdr
            .run_in_pane(
                &split.pane_id,
                r#"printf '%s\n' "$VALUE"; touch /tmp/not-run-here"#,
            )
            .unwrap();
        herdr.close_tab("w1:t2").unwrap();

        let invocations = herdr.runner.invocations.lock().unwrap();
        let args = |index: usize| {
            invocations[index]
                .args
                .iter()
                .map(|arg| arg.to_string_lossy().into_owned())
                .collect::<Vec<_>>()
        };
        assert_eq!(
            args(0),
            vec![
                "--session",
                "isolated",
                "workspace",
                "create",
                "--cwd",
                "/repo",
                "--label",
                "api",
                "--no-focus"
            ]
        );
        assert_eq!(
            args(2),
            vec![
                "--session",
                "isolated",
                "pane",
                "split",
                "w1:p2",
                "--direction",
                "down",
                "--ratio",
                "0.65",
                "--cwd",
                "/repo/logs",
                "--no-focus"
            ]
        );
        assert_eq!(
            args(4).last().unwrap(),
            r#"printf '%s\n' "$VALUE"; touch /tmp/not-run-here"#
        );
        assert_eq!(
            args(5),
            vec!["--session", "isolated", "tab", "close", "w1:t2"]
        );
    }
}
