use std::collections::HashMap;
use std::process::Command;

use anyhow::{bail, Result};

use crate::env::Env;

pub const MINIMUM_SUPPORTED_VERSION: f64 = 2.6;
pub const UNSUPPORTED_VERSION_MSG: &str =
    "WARNING: You are running bootmux with an unsupported version of tmux.\n\
     Please consider using a supported version (tmux >= 2.6).";

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum TmuxVersion {
    Master,
    Numeric(f64),
}

impl TmuxVersion {
    pub fn supported(&self) -> bool {
        match self {
            TmuxVersion::Master => true,
            TmuxVersion::Numeric(version) => *version >= MINIMUM_SUPPORTED_VERSION,
        }
    }

    // Port of Config.version: second token of `tmux -V`; "master" means a
    // development build, letter suffixes collapse to their numeric base.
    pub fn parse(tmux_v_output: &str) -> Option<TmuxVersion> {
        let token = tmux_v_output.split_whitespace().nth(1)?;
        if token == "master" {
            return Some(TmuxVersion::Master);
        }
        let digits: String = token
            .chars()
            .skip_while(|c| !c.is_ascii_digit())
            .take_while(|c| c.is_ascii_digit() || *c == '.')
            .collect();
        let numeric_part = match digits.split('.').collect::<Vec<_>>().as_slice() {
            [] | [""] => return None,
            [major] => major.to_string(),
            [major, minor, ..] => format!("{major}.{minor}"),
        };
        numeric_part.parse().ok().map(TmuxVersion::Numeric)
    }
}

pub trait TmuxContext {
    fn extract_tmux_config(&self, tmux_cmd: &str) -> HashMap<String, String>;
    fn has_session(&self, tmux_cmd: &str, unescaped_name: &str) -> bool;
    fn last_window_index(&self, tmux_cmd: &str) -> i64;
    fn current_session_name(&self, env: &Env) -> String;
    fn version(&self) -> Option<TmuxVersion>;
    fn active_sessions(&self) -> Vec<String>;
    fn capture(&self, command: &str) -> Result<String>;
}

pub struct RealTmux;

impl RealTmux {
    fn run(&self, command: &str) -> Option<String> {
        let output = Command::new("sh").arg("-c").arg(command).output().ok()?;
        Some(String::from_utf8_lossy(&output.stdout).into_owned())
    }
}

impl TmuxContext for RealTmux {
    fn extract_tmux_config(&self, tmux_cmd: &str) -> HashMap<String, String> {
        let command = format!(
            "{tmux_cmd} start-server\\; show-option -g base-index\\; \
             show-window-option -g pane-base-index\\;"
        );
        let mut options = HashMap::new();
        if let Some(output) = self.run(&command) {
            for line in output.lines() {
                let mut parts = line.split_whitespace();
                if let Some(key) = parts.next() {
                    options.insert(key.to_string(), parts.next().unwrap_or("").to_string());
                }
            }
        }
        options
    }

    fn has_session(&self, tmux_cmd: &str, unescaped_name: &str) -> bool {
        let sessions = self
            .run(&format!("{tmux_cmd} ls 2> /dev/null"))
            .unwrap_or_default();
        let prefix = format!("{unescaped_name}:");
        sessions.lines().any(|line| line.starts_with(&prefix))
    }

    fn last_window_index(&self, tmux_cmd: &str) -> i64 {
        self.run(&format!("{tmux_cmd} list-windows -F '#I'"))
            .unwrap_or_default()
            .split_whitespace()
            .last()
            .and_then(|index| index.parse().ok())
            .unwrap_or(0)
    }

    fn current_session_name(&self, env: &Env) -> String {
        if !env.inside_tmux() {
            return String::new();
        }
        self.run("tmux display-message -p \"#S\"")
            .unwrap_or_default()
            .trim()
            .to_string()
    }

    fn version(&self) -> Option<TmuxVersion> {
        TmuxVersion::parse(&self.run("tmux -V")?)
    }

    fn active_sessions(&self) -> Vec<String> {
        self.run("tmux list-sessions -F \"#S\" 2> /dev/null")
            .unwrap_or_default()
            .lines()
            .map(str::to_string)
            .collect()
    }

    fn capture(&self, command: &str) -> Result<String> {
        let output = Command::new("sh").arg("-c").arg(command).output()?;
        if !output.status.success() {
            bail!("command failed: {command}");
        }
        Ok(String::from_utf8_lossy(&output.stdout).into_owned())
    }
}

// Canned introspection matching the stubs in tmuxinator's
// debug_snapshot_spec.rb, used by golden-file tests.
#[derive(Default)]
pub struct MockTmux {
    pub base_index: i64,
    pub pane_base_index: i64,
    pub session_exists: bool,
    pub last_window_index: i64,
    pub current_session: String,
}

impl TmuxContext for MockTmux {
    fn extract_tmux_config(&self, _tmux_cmd: &str) -> HashMap<String, String> {
        HashMap::from([
            ("base-index".to_string(), self.base_index.to_string()),
            (
                "pane-base-index".to_string(),
                self.pane_base_index.to_string(),
            ),
        ])
    }

    fn has_session(&self, _tmux_cmd: &str, _unescaped_name: &str) -> bool {
        self.session_exists
    }

    fn last_window_index(&self, _tmux_cmd: &str) -> i64 {
        self.last_window_index
    }

    fn current_session_name(&self, _env: &Env) -> String {
        self.current_session.clone()
    }

    fn version(&self) -> Option<TmuxVersion> {
        Some(TmuxVersion::Numeric(2.6))
    }

    fn active_sessions(&self) -> Vec<String> {
        Vec::new()
    }

    fn capture(&self, _command: &str) -> Result<String> {
        Ok(String::new())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_tmux_versions() {
        assert_eq!(
            TmuxVersion::parse("tmux 3.4"),
            Some(TmuxVersion::Numeric(3.4))
        );
        assert_eq!(
            TmuxVersion::parse("tmux 3.2a"),
            Some(TmuxVersion::Numeric(3.2))
        );
        assert_eq!(TmuxVersion::parse("tmux master"), Some(TmuxVersion::Master));
        assert_eq!(
            TmuxVersion::parse("tmux next-3.5"),
            Some(TmuxVersion::Numeric(3.5))
        );
        assert_eq!(TmuxVersion::parse(""), None);
    }

    #[test]
    fn version_support_cutoff_is_2_6() {
        assert!(TmuxVersion::Numeric(2.6).supported());
        assert!(TmuxVersion::Numeric(3.4).supported());
        assert!(TmuxVersion::Master.supported());
        assert!(!TmuxVersion::Numeric(2.5).supported());
    }
}
