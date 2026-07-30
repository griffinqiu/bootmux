use std::collections::HashMap;
use std::path::{Path, PathBuf};

use anyhow::{anyhow, bail, Result};
use serde_norway::Value;

use crate::env::Env;
use crate::project::{
    LoadOptions, HOOK_ON_PROJECT_EXIT, HOOK_ON_PROJECT_FIRST_START, HOOK_ON_PROJECT_RESTART,
    HOOK_ON_PROJECT_START, HOOK_ON_PROJECT_STOP,
};
use crate::template;
use crate::util::expand_path;
use crate::yaml_ext::{
    first_entry, get, get_aliased_nonempty_sequence, get_aliased_scalar, get_string,
    join_or_string, mux_attach, parse, scalar_to_string, truthy,
};

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum SplitDirection {
    Right,
    Down,
}

impl SplitDirection {
    fn parse(value: &Value) -> Result<Self> {
        match scalar_to_string(value).as_deref() {
            Some("right") => Ok(Self::Right),
            Some("down") => Ok(Self::Down),
            Some(other) => {
                bail!("Herdr pane split direction must be `right` or `down`, got `{other}`.")
            }
            None => bail!("Herdr pane split direction must be `right` or `down`."),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PaneSplit {
    pub direction: SplitDirection,
    /// The share retained by the existing (first) pane.
    pub ratio: f64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct PaneSpec {
    pub title: Option<String>,
    pub commands: Vec<String>,
    pub split: Option<PaneSplit>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct WindowSpec {
    pub name: Option<String>,
    pub root: String,
    pub pre: Option<String>,
    pub commands: Vec<String>,
    pub panes: Vec<PaneSpec>,
    pub layout: Option<String>,
    pub pane_chain: bool,
    pub focused_pane: usize,
}

impl WindowSpec {
    pub fn effective_panes(&self) -> Vec<PaneSpec> {
        if self.panes.is_empty() {
            vec![PaneSpec {
                title: None,
                commands: self.commands.clone(),
                split: None,
            }]
        } else {
            self.panes.clone()
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Hooks {
    pub start: Option<String>,
    pub first_start: Option<String>,
    pub restart: Option<String>,
    pub exit: Option<String>,
    pub stop: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct HerdrProjectSpec {
    pub source_path: PathBuf,
    pub name: String,
    pub root: String,
    pub attach: bool,
    pub append: bool,
    pub socket_name: Option<String>,
    pub socket_path: Option<String>,
    pub startup_window: usize,
    /// Explicit project-level pane selection. When omitted, the startup tab
    /// retains its own `focused_pane` selection.
    pub startup_pane: Option<usize>,
    pub pre_window: Option<String>,
    pub hooks: Hooks,
    pub windows: Vec<WindowSpec>,
    pub warnings: Vec<String>,
}

impl HerdrProjectSpec {
    #[allow(clippy::too_many_arguments)]
    pub fn load(
        source_path: impl AsRef<Path>,
        content: &str,
        settings: &HashMap<String, String>,
        args: &[String],
        opts: LoadOptions,
        env: &Env,
    ) -> Result<Self> {
        if opts.force_attach && opts.force_detach {
            bail!("Cannot force_attach and force_detach at the same time");
        }

        let rendered = template::render_config(content, settings, args, env)?;
        let mut yaml: Value =
            parse(&rendered).map_err(|error| anyhow!("Failed to parse config file: {error}"))?;
        yaml.apply_merge()
            .map_err(|error| anyhow!("Failed to parse config file: {error}"))?;
        let raw_name = opts
            .custom_name
            .clone()
            .or_else(|| {
                get_aliased_scalar(&yaml, "name", &["project_name"]).and_then(scalar_to_string)
            })
            .filter(|name| !name.is_empty())
            .ok_or_else(|| anyhow!("Your project file didn't specify a 'project_name'"))?;

        let root = get_aliased_scalar(&yaml, "root", &["project_root"])
            .and_then(scalar_to_string)
            .filter(|root| !root.is_empty())
            .map(|root| expand_path(&root, &env.cwd.to_string_lossy(), &env.home))
            .unwrap_or_else(|| env.cwd.to_string_lossy().into_owned());
        let root = canonicalize_existing(&root);

        let entries = get_aliased_nonempty_sequence(&yaml, "windows", &["tabs"]).unwrap_or(&[]);
        if entries.is_empty() {
            bail!("Your project file should include some windows.");
        }

        let no_pre_window = opts.no_pre_window;
        let windows = entries
            .iter()
            .map(|entry| build_window(entry, &root, &env.home))
            .collect::<Result<Vec<_>>>()?;

        let startup_window =
            resolve_window(get(&yaml, "startup_window"), &windows, "startup_window")?;
        let startup_pane = get(&yaml, "startup_pane")
            .filter(|value| truthy(Some(value)))
            .map(|value| {
                resolve_pane(
                    Some(value),
                    &windows[startup_window].effective_panes(),
                    "startup_pane",
                )
            })
            .transpose()?;

        let yaml_attach = mux_attach(get(&yaml, "attach"));
        let attach = opts.force_attach || (!opts.force_detach && yaml_attach);

        let mut warnings = Vec::new();
        for key in ["tmux_options", "cli_args", "tmux_command"] {
            if truthy(get(&yaml, key)) {
                warnings.push(format!(
                    "`{key}` is tmux-specific and is ignored by the Herdr backend."
                ));
            }
        }
        for key in [
            "enable_pane_titles",
            "pane_title_position",
            "pane_title_format",
        ] {
            if truthy(get(&yaml, key)) {
                warnings.push(format!(
                    "`{key}` only controls tmux pane borders and is ignored by Herdr."
                ));
            }
        }

        let source_path = source_path.as_ref();
        let source_path = std::fs::canonicalize(source_path)
            .unwrap_or_else(|_| absolute_path(source_path, &env.cwd));

        Ok(Self {
            source_path,
            name: raw_name,
            root,
            attach,
            append: opts.append,
            socket_name: get_string(&yaml, "socket_name").filter(|value| !value.is_empty()),
            socket_path: get_string(&yaml, "socket_path").filter(|value| !value.is_empty()),
            startup_window,
            startup_pane,
            pre_window: if no_pre_window {
                None
            } else {
                join_or_string(get(&yaml, "pre_window"), "; ")
            },
            hooks: Hooks {
                start: join_or_string(get(&yaml, HOOK_ON_PROJECT_START), "; "),
                first_start: join_or_string(get(&yaml, HOOK_ON_PROJECT_FIRST_START), "; "),
                restart: join_or_string(get(&yaml, HOOK_ON_PROJECT_RESTART), "; "),
                exit: join_or_string(get(&yaml, HOOK_ON_PROJECT_EXIT), "; "),
                stop: join_or_string(get(&yaml, HOOK_ON_PROJECT_STOP), "; "),
            },
            windows,
            warnings,
        })
    }
}

fn absolute_path(path: &Path, cwd: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        cwd.join(path)
    }
}

fn canonicalize_existing(path: &str) -> String {
    std::fs::canonicalize(path)
        .unwrap_or_else(|_| PathBuf::from(path))
        .to_string_lossy()
        .into_owned()
}

fn build_window(entry: &Value, project_root: &str, home: &str) -> Result<WindowSpec> {
    if !matches!(entry, Value::Mapping(_)) {
        bail!("Failed to parse config file: window entries must be mappings, e.g. `- editor: vim`");
    }
    let (key, body) = first_entry(entry);
    let name = key.and_then(scalar_to_string);
    let body = body.unwrap_or(&Value::Null);

    if truthy(get(body, "synchronize")) {
        bail!(
            "`synchronize` is not supported by the Herdr backend because it changes interactive input semantics."
        );
    }

    let root = get_string(body, "root")
        .filter(|value| !value.is_empty())
        .map(|value| expand_path(&value, project_root, home))
        .unwrap_or_else(|| project_root.to_string());
    let root = canonicalize_existing(&root);
    let pre = join_or_string(get(body, "pre"), "; ");
    let layout = get_string(body, "layout").filter(|value| !value.is_empty());

    let (panes, pane_chain) = parse_panes(get(body, "panes"))?;
    if pane_chain && layout.is_some() {
        bail!(
            "A Herdr pane chain (`split`/`ratio`/`command(s)`) cannot be combined with a window `layout`."
        );
    }

    let commands = if matches!(body, Value::Mapping(_)) {
        Vec::new()
    } else {
        command_list(Some(body))
    };
    let effective = if panes.is_empty() {
        vec![PaneSpec {
            title: None,
            commands: commands.clone(),
            split: None,
        }]
    } else {
        panes.clone()
    };
    let focused_pane = resolve_pane(get(body, "focused_pane"), &effective, "focused_pane")?;

    Ok(WindowSpec {
        name,
        root,
        pre,
        commands,
        panes,
        layout,
        pane_chain,
        focused_pane,
    })
}

fn parse_panes(value: Option<&Value>) -> Result<(Vec<PaneSpec>, bool)> {
    let value = match value {
        None | Some(Value::Null) => return Ok((Vec::new(), false)),
        Some(value) => value,
    };
    let items: Vec<&Value> = match value {
        Value::Sequence(items) => items.iter().collect(),
        other => vec![other],
    };

    let mut panes = Vec::with_capacity(items.len());
    let mut chain = false;
    for (index, item) in items.into_iter().enumerate() {
        let (title, body) = match item {
            Value::Mapping(map) if !map.is_empty() => {
                let (key, body) = first_entry(item);
                (key.and_then(scalar_to_string), body)
            }
            Value::Mapping(_) => (None, None),
            Value::Null => (None, None),
            other => (None, Some(other)),
        };

        let structured = body.map(is_chain_body).unwrap_or(false);
        chain |= structured;
        let (commands, direction, ratio) = if structured {
            let body = body.unwrap();
            let command = get(body, "command");
            let commands = get(body, "commands");
            if command.is_some() && commands.is_some() {
                bail!("Herdr pane {index} cannot specify both `command` and `commands`.");
            }
            let commands = command_list(command.or(commands));
            let direction = get(body, "split").map(SplitDirection::parse).transpose()?;
            let ratio = get(body, "ratio").map(parse_ratio).transpose()?;
            (commands, direction, ratio)
        } else {
            (command_list(body), None, None)
        };

        if index == 0 && (direction.is_some() || ratio.is_some()) {
            bail!("The first pane in a Herdr pane chain cannot specify `split` or `ratio`.");
        }
        let split = if index == 0 {
            None
        } else if chain || direction.is_some() || ratio.is_some() {
            Some(PaneSplit {
                direction: direction.unwrap_or(SplitDirection::Right),
                ratio: ratio.unwrap_or(0.5),
            })
        } else {
            None
        };
        panes.push(PaneSpec {
            title,
            commands,
            split,
        });
    }

    // A structured item turns the whole list into a chain. Fill defaults
    // for earlier/later traditional entries after the mode is known.
    if chain {
        for pane in panes.iter_mut().skip(1) {
            if pane.split.is_none() {
                pane.split = Some(PaneSplit {
                    direction: SplitDirection::Right,
                    ratio: 0.5,
                });
            }
        }
    }
    Ok((panes, chain))
}

fn is_chain_body(value: &Value) -> bool {
    matches!(value, Value::Mapping(_))
        && ["command", "commands", "split", "ratio"]
            .iter()
            .any(|key| get(value, key).is_some())
}

fn command_list(value: Option<&Value>) -> Vec<String> {
    match value {
        None | Some(Value::Null) => Vec::new(),
        Some(Value::Sequence(items)) => items.iter().filter_map(scalar_to_string).collect(),
        Some(value) => scalar_to_string(value).into_iter().collect(),
    }
}

fn parse_ratio(value: &Value) -> Result<f64> {
    let ratio = match value {
        Value::Number(number) => number.as_f64(),
        Value::String(string) => string.parse().ok(),
        _ => None,
    }
    .ok_or_else(|| anyhow!("Herdr pane split `ratio` must be a number from 0.1 through 0.9."))?;
    if !(0.1..=0.9).contains(&ratio) {
        bail!("Herdr pane split `ratio` must be from 0.1 through 0.9, got {ratio}.");
    }
    Ok(ratio)
}

fn resolve_window(value: Option<&Value>, windows: &[WindowSpec], field: &str) -> Result<usize> {
    let Some(value) = value.filter(|value| truthy(Some(value))) else {
        return Ok(0);
    };
    if let Some(index) = as_index(value) {
        if index < windows.len() {
            return Ok(index);
        }
        bail!("`{field}` index {index} is outside the configured windows.");
    }
    let wanted = scalar_to_string(value).unwrap_or_default();
    windows
        .iter()
        .position(|window| window.name.as_deref() == Some(wanted.as_str()))
        .ok_or_else(|| anyhow!("`{field}` refers to unknown window `{wanted}`."))
}

fn resolve_pane(value: Option<&Value>, panes: &[PaneSpec], field: &str) -> Result<usize> {
    let Some(value) = value.filter(|value| truthy(Some(value))) else {
        return Ok(0);
    };
    if let Some(index) = as_index(value) {
        if index < panes.len() {
            return Ok(index);
        }
        // tmuxinator falls back for focused_pane, but an invalid final
        // Herdr focus should not silently select a different process.
        bail!("`{field}` index {index} is outside the configured panes.");
    }
    let wanted = scalar_to_string(value).unwrap_or_default();
    panes
        .iter()
        .position(|pane| pane.title.as_deref() == Some(wanted.as_str()))
        .ok_or_else(|| anyhow!("`{field}` refers to unknown pane `{wanted}`."))
}

fn as_index(value: &Value) -> Option<usize> {
    match value {
        Value::Number(number) => number.as_u64().map(|number| number as usize),
        Value::String(string) => string.trim().parse().ok(),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env() -> Env {
        Env {
            home: "/home/test".into(),
            cwd: PathBuf::from("/work"),
            ..Env::default()
        }
    }

    fn load(source: &str) -> Result<HerdrProjectSpec> {
        HerdrProjectSpec::load(
            "/work/demo.yml",
            source,
            &HashMap::new(),
            &[],
            LoadOptions::default(),
            &env(),
        )
    }

    #[test]
    fn preserves_raw_names_and_builds_chain() {
        let spec = load(
            r#"
name: "api.dev:one"
root: ~/code
attach: false
windows:
  - "web server":
      panes:
        - editor:
            command: nvim
        - dev:
            split: down
            ratio: 0.65
            commands: [npm run dev, echo ready]
"#,
        )
        .unwrap();
        assert_eq!(spec.name, "api.dev:one");
        assert_eq!(spec.root, "/home/test/code");
        assert!(!spec.attach);
        let window = &spec.windows[0];
        assert_eq!(window.name.as_deref(), Some("web server"));
        assert!(window.pane_chain);
        assert_eq!(
            window.panes[1].split.unwrap().direction,
            SplitDirection::Down
        );
        assert_eq!(window.panes[1].split.unwrap().ratio, 0.65);
    }

    #[test]
    fn omitted_startup_pane_preserves_the_tabs_focused_pane() {
        let spec = load(
            "name: x\nstartup_window: editor\nwindows:\n  - editor:\n      \
             focused_pane: shell\n      panes:\n        - editor: vim\n        - shell: bash\n",
        )
        .unwrap();
        assert_eq!(spec.startup_window, 0);
        assert_eq!(spec.startup_pane, None);
        assert_eq!(spec.windows[0].focused_pane, 1);

        let explicit = load(
            "name: x\nstartup_window: editor\nstartup_pane: editor\nwindows:\n  - editor:\n      \
             focused_pane: shell\n      panes:\n        - editor: vim\n        - shell: bash\n",
        )
        .unwrap();
        assert_eq!(explicit.startup_pane, Some(0));
    }

    #[test]
    fn mux_alias_order_and_attach_scalars_match_the_tmux_backend() {
        let spec = load(
            "name: modern\nproject_name: legacy\nroot: /modern\nproject_root: /legacy\n\
             windows:\n  - modern: echo modern\ntabs:\n  - legacy: echo legacy\nattach: \"0\"\n",
        )
        .unwrap();
        assert_eq!(spec.name, "legacy");
        assert_eq!(spec.root, "/legacy");
        assert_eq!(spec.windows[0].name.as_deref(), Some("legacy"));
        assert!(!spec.attach);

        let spec = load(
            "project_name: legacy\nname: modern\nproject_root: /legacy\nroot: /modern\n\
             tabs:\n  - legacy: echo legacy\nwindows:\n  - modern: echo modern\nattach: \"False\"\n",
        )
        .unwrap();
        assert_eq!(spec.name, "modern");
        assert_eq!(spec.root, "/modern");
        assert_eq!(spec.windows[0].name.as_deref(), Some("modern"));
        assert!(spec.attach);

        for value in ["False", "FALSE", "+0", "00", "0x0"] {
            let spec = load(&format!(
                "name: lexical\nattach: {value}\nwindows:\n  - app: echo ready\n"
            ))
            .unwrap();
            assert!(spec.attach, "{value}");
        }

        let spec =
            load("name: valid\nproject_name: [invalid]\ntabs:\n  - kept: echo kept\nwindows: []\n")
                .unwrap();
        assert_eq!(spec.name, "valid");
        assert_eq!(spec.windows[0].name.as_deref(), Some("kept"));
    }

    #[test]
    fn rejects_chain_layout_and_synchronize() {
        let error = load(
            "name: x\nwindows:\n  - x:\n      layout: tiled\n      panes:\n        - a: {command: vim}\n",
        )
        .unwrap_err()
        .to_string();
        assert!(error.contains("cannot be combined"));

        let error =
            load("name: x\nwindows:\n  - x:\n      synchronize: after\n      panes: [vim]\n")
                .unwrap_err()
                .to_string();
        assert!(error.contains("not supported"));
    }

    #[test]
    fn rejects_bad_chain_values() {
        for source in [
            "name: x\nwindows:\n  - x:\n      panes:\n        - a: {split: right, command: vim}\n",
            "name: x\nwindows:\n  - x:\n      panes:\n        - a: vim\n        - b: {ratio: 0.95, command: top}\n",
            "name: x\nwindows:\n  - x:\n      panes:\n        - a: vim\n        - b: {split: left, command: top}\n",
            "name: x\nwindows:\n  - x:\n      panes:\n        - a: vim\n        - b: {command: top, commands: [top]}\n",
        ] {
            assert!(load(source).is_err(), "{source}");
        }
    }
}
