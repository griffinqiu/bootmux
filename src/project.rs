use std::cell::OnceCell;
use std::collections::HashMap;

use anyhow::{anyhow, bail, Result};
use serde_norway::Value;

use crate::env::Env;
use crate::shellwords;
use crate::template;
use crate::tmux::TmuxContext;
use crate::util::expand_path;
use crate::window::Window;
use crate::yaml_ext::{get, get_string, join_or_string, scalar_to_string, truthy};

pub const HOOK_ON_PROJECT_START: &str = "on_project_start";
pub const HOOK_ON_PROJECT_FIRST_START: &str = "on_project_first_start";
pub const HOOK_ON_PROJECT_RESTART: &str = "on_project_restart";
pub const HOOK_ON_PROJECT_EXIT: &str = "on_project_exit";
pub const HOOK_ON_PROJECT_STOP: &str = "on_project_stop";

const PANE_TITLE_POSITIONS: [&str; 3] = ["top", "bottom", "off"];

#[derive(Clone, Debug, Default)]
pub struct LoadOptions {
    pub custom_name: Option<String>,
    pub force_attach: bool,
    pub force_detach: bool,
    pub append: bool,
    pub no_pre_window: bool,
}

// Ruby Project.parse_settings: `key=value` arguments become template
// settings, everything else stays in args.
pub fn parse_settings(raw_args: Vec<String>) -> (HashMap<String, String>, Vec<String>) {
    let mut settings = HashMap::new();
    let mut args = Vec::new();
    for arg in raw_args {
        match arg.split_once('=') {
            Some((key, value)) => {
                settings.insert(key.to_string(), value.to_string());
            }
            None => args.push(arg),
        }
    }
    (settings, args)
}

pub struct Project<'a> {
    pub yaml: Value,
    pub opts: LoadOptions,
    pub ctx: &'a dyn TmuxContext,
    pub env: &'a Env,
    windows: Vec<Window>,
    tmux_config: OnceCell<HashMap<String, String>>,
}

impl std::fmt::Debug for Project<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Project")
            .field("name", &self.name())
            .finish()
    }
}

impl<'a> Project<'a> {
    pub fn load(
        content: &str,
        settings: &HashMap<String, String>,
        args: &[String],
        opts: LoadOptions,
        ctx: &'a dyn TmuxContext,
        env: &'a Env,
    ) -> Result<Project<'a>> {
        let rendered = template::render_config(content, settings, args, env)?;
        let mut yaml: Value = serde_norway::from_str(&rendered)
            .map_err(|e| anyhow!("Failed to parse config file: {e}"))?;
        yaml.apply_merge()
            .map_err(|e| anyhow!("Failed to parse config file: {e}"))?;

        check_unsupported_options(&yaml)?;

        let mut project = Project {
            yaml,
            opts,
            ctx,
            env,
            windows: Vec::new(),
            tmux_config: OnceCell::new(),
        };
        project.validate_options()?;
        project.windows = build_windows(&project.yaml)?;
        project.validate()?;
        Ok(project)
    }

    fn validate_options(&self) -> Result<()> {
        if self.opts.force_attach && self.opts.force_detach {
            bail!("Cannot force_attach and force_detach at the same time");
        }
        if self.opts.append && !self.has_session() {
            bail!("Cannot append to a session that does not exist");
        }
        Ok(())
    }

    fn validate(&self) -> Result<()> {
        if self.windows.is_empty() {
            bail!("Your project file should include some windows.");
        }
        if self.name().is_none() {
            bail!("Your project file didn't specify a 'project_name'");
        }
        Ok(())
    }

    pub fn windows(&self) -> &[Window] {
        &self.windows
    }

    // Sanitized session name: `.` and `:` become `_` (they would break
    // tmux target syntax), then shell-escaped.
    pub fn name(&self) -> Option<String> {
        let raw_name = if self.opts.append {
            let current = self.ctx.current_session_name(self.env);
            if current.is_empty() {
                None
            } else {
                Some(current)
            }
        } else {
            self.opts
                .custom_name
                .clone()
                .or_else(|| get_string(&self.yaml, "project_name"))
                .or_else(|| get_string(&self.yaml, "name"))
        };

        raw_name
            .filter(|name| !name.is_empty())
            .map(|name| shellwords::escape(&name.replace(['.', ':'], "_")))
    }

    pub fn unescaped_name(&self) -> Option<String> {
        self.name().map(|name| shellwords::unescape(&name))
    }

    pub fn root_raw(&self) -> Option<String> {
        get_string(&self.yaml, "project_root")
            .or_else(|| get_string(&self.yaml, "root"))
            .filter(|root| !root.is_empty())
            .map(|root| expand_path(&root, &self.env.cwd.to_string_lossy(), &self.env.home))
    }

    pub fn root(&self) -> Option<String> {
        self.root_raw().map(|root| shellwords::escape(&root))
    }

    pub fn attach(&self) -> bool {
        let yaml_attach = match get(&self.yaml, "attach") {
            None | Some(Value::Null) => true,
            Some(Value::Bool(b)) => *b,
            Some(_) => true,
        };
        self.opts.force_attach || (!self.opts.force_detach && yaml_attach)
    }

    pub fn tmux(&self) -> String {
        format!(
            "{}{}{}",
            self.tmux_command(),
            self.tmux_options_part(),
            self.socket_part()
        )
    }

    pub fn tmux_command(&self) -> String {
        get(&self.yaml, "tmux_command")
            .filter(|value| truthy(Some(value)))
            .and_then(scalar_to_string)
            .unwrap_or_else(|| "tmux".to_string())
    }

    fn tmux_options_part(&self) -> String {
        let options = get(&self.yaml, "tmux_options");
        if truthy(options) {
            let value = options.and_then(scalar_to_string).unwrap_or_default();
            format!(" {}", value.trim())
        } else {
            String::new()
        }
    }

    fn socket_part(&self) -> String {
        let socket_path = get(&self.yaml, "socket_path").filter(|v| truthy(Some(v)));
        let socket_name = get(&self.yaml, "socket_name").filter(|v| truthy(Some(v)));
        if let Some(path) = socket_path.and_then(scalar_to_string) {
            format!(" -S {path}")
        } else if let Some(name) = socket_name.and_then(scalar_to_string) {
            format!(" -L {name}")
        } else {
            String::new()
        }
    }

    fn tmux_config_map(&self) -> &HashMap<String, String> {
        self.tmux_config
            .get_or_init(|| self.ctx.extract_tmux_config(&self.tmux()))
    }

    pub fn base_index(&self) -> i64 {
        if self.opts.append {
            return self.ctx.last_window_index(&self.tmux()) + 1;
        }
        self.tmux_config_map()
            .get("base-index")
            .and_then(|value| value.parse().ok())
            .unwrap_or(0)
    }

    pub fn pane_base_index(&self) -> i64 {
        self.tmux_config_map()
            .get("pane-base-index")
            .and_then(|value| value.parse().ok())
            .unwrap_or(0)
    }

    // Raw injection like Ruby: startup_window/startup_pane values are not
    // escaped, and may be a window name or an index.
    pub fn startup_window(&self) -> String {
        let window = match get(&self.yaml, "startup_window") {
            value if truthy(value) => value.and_then(scalar_to_string).unwrap_or_default(),
            _ => self.base_index().to_string(),
        };
        format!("{}:{}", self.name().unwrap_or_default(), window)
    }

    pub fn startup_pane(&self) -> String {
        let pane = match get(&self.yaml, "startup_pane") {
            Some(Value::String(s)) if !s.is_empty() => s.clone(),
            Some(Value::Number(n)) => {
                scalar_to_string(&Value::Number(n.clone())).unwrap_or_default()
            }
            Some(Value::Bool(true)) => "true".to_string(),
            _ => self.pane_base_index().to_string(),
        };
        format!("{}.{}", self.startup_window(), pane)
    }

    pub fn startup_pane_command(&self) -> String {
        format!("{} select-pane -t {}", self.tmux(), self.startup_pane())
    }

    pub fn pre_window(&self) -> Option<String> {
        if self.opts.no_pre_window {
            return None;
        }
        join_or_string(get(&self.yaml, "pre_window"), "; ")
    }

    pub fn hook(&self, hook_name: &str) -> Option<String> {
        join_or_string(get(&self.yaml, hook_name), "; ")
    }

    pub fn has_session(&self) -> bool {
        match self.unescaped_name() {
            Some(name) => self.ctx.has_session(&self.tmux(), &name),
            None => false,
        }
    }

    pub fn new_session_command(&self) -> Option<String> {
        if self.opts.append {
            return None;
        }
        let first_window_name = self
            .windows
            .first()
            .map(|window| window.name_option())
            .unwrap_or_default();
        Some(format!(
            "{} new-session -d -s {} {}",
            self.tmux(),
            self.name().unwrap_or_default(),
            first_window_name
        ))
    }

    pub fn kill_session_command(&self) -> String {
        format!(
            "{} kill-session -t {}",
            self.tmux(),
            self.name().unwrap_or_default()
        )
    }

    pub fn enable_pane_titles(&self) -> bool {
        truthy(get(&self.yaml, "enable_pane_titles"))
    }

    pub fn pane_title_position(&self) -> Option<String> {
        get(&self.yaml, "pane_title_position")
            .filter(|value| truthy(Some(value)))
            .and_then(scalar_to_string)
    }

    pub fn pane_title_position_valid(&self) -> bool {
        matches!(
            self.pane_title_position().as_deref(),
            Some(position) if PANE_TITLE_POSITIONS.contains(&position)
        )
    }

    pub fn set_pane_title_position_command(&self, window_target: &str) -> String {
        let position = if self.pane_title_position_valid() {
            self.pane_title_position().unwrap_or_default()
        } else {
            "top".to_string()
        };
        format!(
            "{} set-window-option -t {window_target} pane-border-status {position}",
            self.tmux()
        )
    }

    pub fn set_pane_title_format_command(&self, window_target: &str) -> String {
        let format_value = get(&self.yaml, "pane_title_format")
            .filter(|value| truthy(Some(value)))
            .and_then(scalar_to_string)
            .unwrap_or_else(|| "#{pane_index}: #{pane_title}".to_string());
        format!(
            "{} set-window-option -t {window_target} pane-border-format \"{format_value}\"",
            self.tmux()
        )
    }

    // The warning is a printf line embedded in the generated script; the
    // message contains a real newline before the color reset, exactly like
    // Ruby's Project#print_warning.
    pub fn pane_title_position_warning(&self) -> String {
        let position = self.pane_title_position().unwrap_or_default();
        format!(
            "printf \"\\033[1;33mWARNING: The specified pane title position '{position}' \
             is not valid. Please choose one of: top, bottom, or off.\n\\033[0m\""
        )
    }
}

fn build_windows(yaml: &Value) -> Result<Vec<Window>> {
    let entries = match get(yaml, "windows") {
        Some(Value::Sequence(entries)) => entries.as_slice(),
        _ => &[],
    };
    entries
        .iter()
        .enumerate()
        .map(|(index, entry)| Window::build(entry, index))
        .collect()
}

fn check_unsupported_options(yaml: &Value) -> Result<()> {
    let unsupported: [(&str, &str); 7] = [
        ("rbenv", "use `pre_window: rbenv shell <version>` instead"),
        ("rvm", "use `pre_window: rvm use <version>` instead"),
        ("pre_tab", "rename it to `pre_window`"),
        ("tabs", "rename it to `windows`"),
        ("cli_args", "rename it to `tmux_options`"),
        (
            "pre",
            "use the `on_project_start` / `on_project_first_start` hooks instead",
        ),
        (
            "post",
            "use the `on_project_stop` / `on_project_exit` hooks instead",
        ),
    ];

    for (key, hint) in unsupported {
        if get(yaml, key).is_some() {
            bail!(
                "The `{key}` option was deprecated in tmuxinator and is not supported by \
                 bootmux: {hint}."
            );
        }
    }

    if get_string(yaml, "tmux_command").as_deref() == Some("wemux") {
        bail!("wemux is not supported by bootmux.");
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_key_value_settings_from_args() {
        let (settings, args) = parse_settings(vec![
            "foo=bar".to_string(),
            "extra".to_string(),
            "a=b=c".to_string(),
        ]);
        assert_eq!(settings.get("foo").unwrap(), "bar");
        assert_eq!(settings.get("a").unwrap(), "b=c");
        assert_eq!(args, vec!["extra".to_string()]);
    }
}
