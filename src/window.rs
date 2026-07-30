use anyhow::{bail, Result};
use serde_norway::Value;

use crate::pane::Pane;
use crate::project::Project;
use crate::shellwords;
use crate::yaml_ext::{first_entry, get, join_or_string, scalar_to_string};

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum PaneChainDirection {
    Right,
    Down,
}

pub struct Window {
    pub index: usize,
    // Shell-escaped at construction (Ruby Window#initialize).
    pub name: Option<String>,
    pub body: Value,
    pub panes: Vec<Pane>,
}

impl Window {
    pub fn build(entry: &Value, index: usize) -> Result<Window> {
        if !matches!(entry, Value::Mapping(_)) {
            bail!("Failed to parse config file: window entries must be mappings, e.g. `- editor: vim`");
        }
        let (key, body) = first_entry(entry);
        let name = key
            .and_then(scalar_to_string)
            .map(|n| shellwords::escape(&n));
        let body = body.cloned().unwrap_or(Value::Null);
        validate_pane_chain(&body)?;
        let panes = build_panes(get(&body, "panes"));

        Ok(Window {
            index,
            name,
            body,
            panes,
        })
    }

    fn option(&self, key: &str) -> Option<&Value> {
        get(&self.body, key)
    }

    pub fn layout(&self) -> Option<String> {
        self.option("layout")
            .and_then(scalar_to_string)
            .map(|layout| shellwords::escape(&layout))
    }

    pub fn pre(&self) -> Option<String> {
        join_or_string(self.option("pre"), "; ")
    }

    pub fn synchronize_before(&self) -> bool {
        match self.option("synchronize") {
            Some(Value::Bool(true)) => true,
            Some(Value::String(s)) => s == "before",
            _ => false,
        }
    }

    pub fn synchronize_after(&self) -> bool {
        matches!(self.option("synchronize"), Some(Value::String(s)) if s == "after")
    }

    // Window root falls back to the project root; own roots resolve
    // relative to the project root (Ruby Window#root).
    pub fn root(&self, project: &Project) -> Option<String> {
        match self
            .option("root")
            .filter(|v| crate::yaml_ext::truthy(Some(v)))
        {
            Some(own_root) => {
                let own_root = scalar_to_string(own_root).unwrap_or_default();
                let base = project
                    .root_raw()
                    .unwrap_or_else(|| project.env.cwd.to_string_lossy().into_owned());
                Some(shellwords::escape(&crate::util::expand_path(
                    &own_root,
                    &base,
                    &project.env.home,
                )))
            }
            None => project.root(),
        }
    }

    pub fn has_panes(&self) -> bool {
        !self.panes.is_empty()
    }

    pub fn is_pane_chain(&self) -> bool {
        pane_items(get(&self.body, "panes"))
            .iter()
            .any(|item| pane_body(item).is_some_and(is_chain_body))
    }

    /// Returns the split that creates `new_pane_index`. The ratio is the
    /// share retained by the existing pane.
    pub fn pane_chain_split(&self, new_pane_index: usize) -> Option<(PaneChainDirection, f64)> {
        if !self.is_pane_chain() || new_pane_index == 0 {
            return None;
        }
        let items = pane_items(get(&self.body, "panes"));
        let body = items.get(new_pane_index).and_then(|item| pane_body(item));
        let direction = body
            .and_then(|body| get(body, "split"))
            .and_then(scalar_to_string)
            .map(|direction| match direction.as_str() {
                "down" => PaneChainDirection::Down,
                _ => PaneChainDirection::Right,
            })
            .unwrap_or(PaneChainDirection::Right);
        let ratio = body
            .and_then(|body| get(body, "ratio"))
            .and_then(ratio_value)
            .unwrap_or(0.5);
        Some((direction, ratio))
    }

    pub fn target(&self, project: &Project) -> String {
        format!(
            "{}:{}",
            project.name().unwrap_or_default(),
            self.index as i64 + project.base_index()
        )
    }

    pub fn name_option(&self) -> String {
        match &self.name {
            Some(name) => format!("-n {name}"),
            None => String::new(),
        }
    }

    pub fn new_window_command(&self, project: &Project) -> String {
        let path = self
            .root(project)
            .map(|root| format!("-c {root}"))
            .unwrap_or_default();
        format!(
            "{} new-window {} -k -t {} {}",
            project.tmux(),
            path,
            self.target(project),
            self.name_option()
        )
    }

    // Commands for windows without panes; nil entries are skipped, empty
    // strings escape to '' (Ruby Window#build_commands).
    pub fn commands(&self, project: &Project) -> Vec<String> {
        let prefix = format!("{} send-keys -t {}", project.tmux(), self.target(project));
        match &self.body {
            Value::Sequence(items) => items
                .iter()
                .filter(|item| !matches!(item, Value::Null))
                .filter_map(scalar_to_string)
                .map(|command| format!("{prefix} {} C-m", shellwords::escape(&command)))
                .collect(),
            Value::String(command) if !command.is_empty() => {
                vec![format!("{prefix} {} C-m", shellwords::escape(command))]
            }
            _ => Vec::new(),
        }
    }

    pub fn pre_window_command(&self, project: &Project) -> Option<String> {
        project.pre_window().map(|pre_window| {
            format!(
                "{} send-keys -t {} {} C-m",
                project.tmux(),
                self.target(project),
                shellwords::escape(&pre_window)
            )
        })
    }

    pub fn tiled_layout_command(&self, project: &Project) -> String {
        format!(
            "{} select-layout -t {} tiled",
            project.tmux(),
            self.target(project)
        )
    }

    pub fn layout_command(&self, project: &Project) -> String {
        format!(
            "{} select-layout -t {} {}",
            project.tmux(),
            self.target(project),
            self.layout().unwrap_or_default()
        )
    }

    pub fn synchronize_command(&self, project: &Project) -> String {
        format!(
            "{} set-window-option -t {} synchronize-panes on",
            project.tmux(),
            self.target(project)
        )
    }

    pub fn focus_pane_command(&self, project: &Project) -> String {
        format!(
            "{} select-pane -t {}.{}",
            project.tmux(),
            self.target(project),
            self.focused_pane_index(project)
        )
    }

    // Integer indices must be in range, strings match against escaped pane
    // titles; anything else falls back to the first pane (Ruby Window#pane_index).
    pub(crate) fn focused_pane_index(&self, project: &Project) -> i64 {
        self.pane_index(self.option("focused_pane"), project)
    }

    pub(crate) fn pane_index(&self, configured: Option<&Value>, project: &Project) -> i64 {
        let index = match configured {
            None | Some(Value::Null) | Some(Value::Bool(false)) => 0,
            Some(value) => match as_integer(value) {
                Some(idx) if idx >= 0 && (idx as usize) < self.panes.len() => idx,
                Some(_) => 0,
                None => {
                    let wanted = scalar_to_string(value)
                        .map(|title| shellwords::escape(&title))
                        .unwrap_or_default();
                    self.panes
                        .iter()
                        .position(|pane| pane.title.as_deref() == Some(wanted.as_str()))
                        .unwrap_or(0) as i64
                }
            },
        };
        index + project.pane_base_index()
    }
}

fn as_integer(value: &Value) -> Option<i64> {
    match value {
        Value::Number(n) => n.as_i64().or_else(|| n.as_f64().map(|f| f.trunc() as i64)),
        Value::String(s) => s.trim().parse().ok(),
        _ => None,
    }
}

fn build_panes(panes_value: Option<&Value>) -> Vec<Pane> {
    let panes_value = match panes_value {
        None | Some(Value::Null) => return Vec::new(),
        Some(value) => value,
    };

    let items: Vec<&Value> = match panes_value {
        Value::Sequence(items) => items.iter().collect(),
        other => vec![other],
    };

    items
        .iter()
        .enumerate()
        .map(|(index, item)| {
            let (title, commands) = match item {
                Value::Mapping(_) => {
                    let (key, body) = first_entry(item);
                    let title = key
                        .and_then(scalar_to_string)
                        .map(|t| shellwords::escape(&t));
                    let commands = match body {
                        Some(body) if is_chain_body(body) => structured_pane_commands(body),
                        body => pane_commands(body),
                    };
                    (title, commands)
                }
                Value::Sequence(_) => (None, pane_commands(Some(item))),
                Value::Null => (None, Vec::new()),
                scalar => (None, vec![scalar_to_string(scalar)]),
            };
            Pane {
                index,
                title,
                commands,
            }
        })
        .collect()
}

fn pane_items(value: Option<&Value>) -> Vec<&Value> {
    match value {
        None | Some(Value::Null) => Vec::new(),
        Some(Value::Sequence(items)) => items.iter().collect(),
        Some(value) => vec![value],
    }
}

fn pane_body(value: &Value) -> Option<&Value> {
    match value {
        Value::Mapping(_) => first_entry(value).1,
        Value::Null => None,
        value => Some(value),
    }
}

fn is_chain_body(value: &Value) -> bool {
    matches!(value, Value::Mapping(_))
        && ["command", "commands", "split", "ratio"]
            .iter()
            .any(|key| get(value, key).is_some())
}

fn structured_pane_commands(body: &Value) -> Vec<Option<String>> {
    pane_commands(get(body, "command").or_else(|| get(body, "commands")))
}

fn validate_pane_chain(window_body: &Value) -> Result<()> {
    let items = pane_items(get(window_body, "panes"));
    let chain = items
        .iter()
        .any(|item| pane_body(item).is_some_and(is_chain_body));
    if !chain {
        return Ok(());
    }
    if get(window_body, "layout").is_some() {
        bail!(
            "A pane chain (`split`/`ratio`/`command(s)`) cannot be combined with a window `layout`."
        );
    }
    for (index, item) in items.iter().enumerate() {
        let Some(body) = pane_body(item).filter(|body| is_chain_body(body)) else {
            continue;
        };
        if get(body, "command").is_some() && get(body, "commands").is_some() {
            bail!("Pane {index} cannot specify both `command` and `commands`.");
        }
        if index == 0 && (get(body, "split").is_some() || get(body, "ratio").is_some()) {
            bail!("The first pane in a pane chain cannot specify `split` or `ratio`.");
        }
        if let Some(direction) = get(body, "split").and_then(scalar_to_string) {
            if !matches!(direction.as_str(), "right" | "down") {
                bail!("Pane {index} split direction must be `right` or `down`, got `{direction}`.");
            }
        }
        if let Some(value) = get(body, "ratio") {
            let ratio = ratio_value(value).ok_or_else(|| {
                anyhow::anyhow!("Pane {index} split `ratio` must be a number from 0.1 through 0.9.")
            })?;
            if !(0.1..=0.9).contains(&ratio) {
                bail!("Pane {index} split `ratio` must be from 0.1 through 0.9, got {ratio}.");
            }
        }
    }
    Ok(())
}

fn ratio_value(value: &Value) -> Option<f64> {
    match value {
        Value::Number(number) => number.as_f64(),
        Value::String(string) => string.parse().ok(),
        _ => None,
    }
}

// Ruby splats the pane body into the command list: a nil body means no
// commands at all, while a nil *entry* in a command array renders as an
// empty line.
fn pane_commands(body: Option<&Value>) -> Vec<Option<String>> {
    match body {
        None | Some(Value::Null) => Vec::new(),
        Some(Value::Sequence(items)) => items.iter().map(scalar_to_string).collect(),
        Some(scalar) => vec![scalar_to_string(scalar)],
    }
}
