use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::str::FromStr;

use anyhow::{anyhow, bail, Result};
use serde_norway::Value;

use crate::env::Env;
use crate::layout::{
    Layout, LayoutPreset, PaneChainBuilder, SplitDirection as LayoutSplitDirection,
};
use crate::project::{
    LoadOptions, HOOK_ON_PROJECT_EXIT, HOOK_ON_PROJECT_FIRST_START, HOOK_ON_PROJECT_RESTART,
    HOOK_ON_PROJECT_START, HOOK_ON_PROJECT_STOP,
};
use crate::settings::Backend;
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
                bail!("Pane split direction must be `right` or `down`, got `{other}`.")
            }
            None => bail!("Pane split direction must be `right` or `down`."),
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

    /// Resolves this window's `layout` field, pane chain, or default into the
    /// backend-neutral binary split tree.
    ///
    /// Panes in the returned tree are numbered in configured order, so a
    /// serialized tmux layout is reindexed from its own pane ids.
    pub fn layout_tree(&self) -> Result<Layout> {
        let pane_count = self.effective_panes().len();
        if self.pane_chain {
            let mut builder = PaneChainBuilder::new(0);
            for (index, pane) in self.panes.iter().enumerate().skip(1) {
                let split = pane.split.ok_or_else(|| {
                    anyhow!("pane chain entry {index} is missing its split definition")
                })?;
                builder = builder.split_pane(
                    index,
                    match split.direction {
                        SplitDirection::Right => LayoutSplitDirection::Right,
                        SplitDirection::Down => LayoutSplitDirection::Down,
                    },
                    split.ratio,
                )?;
            }
            return Ok(builder.build());
        }

        match self.layout.as_deref() {
            None | Some("") => Ok(Layout::default_tiled(pane_count)?),
            Some(layout) => match LayoutPreset::from_str(layout) {
                Ok(preset) => Ok(preset.build(pane_count)?),
                Err(_) => {
                    let parsed = Layout::parse_tmux(layout).map_err(|error| {
                        anyhow!("invalid tmux serialized layout `{layout}`: {error}")
                    })?;
                    if parsed.pane_count() != pane_count {
                        bail!(
                            "tmux serialized layout contains {} panes but {pane_count} panes are configured.",
                            parsed.pane_count()
                        );
                    }
                    let pane_indices = parsed
                        .pane_indices()
                        .into_iter()
                        .enumerate()
                        .map(|(configured_index, serialized_id)| (serialized_id, configured_index))
                        .collect::<HashMap<_, _>>();
                    Ok(reindex_layout(&parsed, &pane_indices))
                }
            },
        }
    }
}

fn reindex_layout(layout: &Layout, pane_indices: &HashMap<usize, usize>) -> Layout {
    match layout {
        Layout::Pane(index) => Layout::Pane(pane_indices[index]),
        Layout::Split {
            direction,
            ratio,
            first,
            second,
        } => Layout::Split {
            direction: *direction,
            ratio: *ratio,
            first: Box::new(reindex_layout(first, pane_indices)),
            second: Box::new(reindex_layout(second, pane_indices)),
        },
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
pub struct ProjectSpec {
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

impl ProjectSpec {
    #[allow(clippy::too_many_arguments)]
    pub fn load(
        source_path: impl AsRef<Path>,
        content: &str,
        settings: &HashMap<String, String>,
        args: &[String],
        opts: LoadOptions,
        env: &Env,
        backend: Backend,
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

        let backend_name = backend.display_name();
        let mut warnings = Vec::new();
        for key in ["tmux_options", "cli_args", "tmux_command"] {
            if truthy(get(&yaml, key)) {
                warnings.push(format!(
                    "`{key}` is tmux-specific and is ignored by the {backend_name} backend."
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
                    "`{key}` only controls tmux pane borders and is ignored by {backend_name}."
                ));
            }
        }
        if backend == Backend::Zellij {
            // zellij derives its socket from the session name and has no
            // endpoint selector of its own.
            for key in ["socket_name", "socket_path"] {
                if truthy(get(&yaml, key)) {
                    warnings.push(format!(
                        "`{key}` selects a tmux or Herdr endpoint and is ignored by zellij."
                    ));
                }
            }
        }

        let no_pre_window = opts.no_pre_window;
        let windows = entries
            .iter()
            .map(|entry| build_window(entry, &root, &env.home, backend, &mut warnings))
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

fn build_window(
    entry: &Value,
    project_root: &str,
    home: &str,
    backend: Backend,
    warnings: &mut Vec<String>,
) -> Result<WindowSpec> {
    if !matches!(entry, Value::Mapping(_)) {
        bail!("Failed to parse config file: window entries must be mappings, e.g. `- editor: vim`");
    }
    let (key, body) = first_entry(entry);
    let name = key.and_then(scalar_to_string);
    let body = body.unwrap_or(&Value::Null);

    if truthy(get(body, "synchronize")) {
        match backend {
            Backend::Herdr => bail!(
                "`synchronize` is not supported by the Herdr backend because it changes interactive input semantics."
            ),
            // zellij has a tab-wide sync mode, but its only CLI entry point
            // toggles the *active* tab and cannot be aimed at a specific one,
            // so bootmux does not claim to reproduce tmux's semantics.
            Backend::Zellij => warnings.push(
                "`synchronize` controls tmux synchronized panes and is ignored by zellij."
                    .to_string(),
            ),
            Backend::Tmux => {}
        }
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
            "A pane chain (`split`/`ratio`/`command(s)`) cannot be combined with a window `layout`."
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
                bail!("Pane {index} cannot specify both `command` and `commands`.");
            }
            let commands = command_list(command.or(commands));
            let direction = get(body, "split").map(SplitDirection::parse).transpose()?;
            let ratio = get(body, "ratio").map(parse_ratio).transpose()?;
            (commands, direction, ratio)
        } else {
            (command_list(body), None, None)
        };

        if index == 0 && (direction.is_some() || ratio.is_some()) {
            bail!("The first pane in a pane chain cannot specify `split` or `ratio`.");
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
    .ok_or_else(|| anyhow!("Pane split `ratio` must be a number from 0.1 through 0.9."))?;
    if !(0.1..=0.9).contains(&ratio) {
        bail!("Pane split `ratio` must be from 0.1 through 0.9, got {ratio}.");
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

    fn load(source: &str) -> Result<ProjectSpec> {
        load_for(source, Backend::Herdr)
    }

    fn load_for(source: &str, backend: Backend) -> Result<ProjectSpec> {
        ProjectSpec::load(
            "/work/demo.yml",
            source,
            &HashMap::new(),
            &[],
            LoadOptions::default(),
            &env(),
            backend,
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
    fn serialized_pane_ids_map_to_configured_visual_order() {
        // tmux numbers panes by its own ids; bootmux renumbers them into the
        // order the panes appear in the config.
        let payload = "100x10,0,0{50x10,0,0,8,49x10,51,0,4}";
        let serialized = format!(
            "{:04x},{payload}",
            crate::layout::tmux_layout_checksum(payload)
        );
        let spec = load(&format!(
            "name: x\nwindows:\n  - grid:\n      layout: \"{serialized}\"\n      \
             panes:\n        - left\n        - right\n"
        ))
        .unwrap();

        let tree = spec.windows[0].layout_tree().unwrap();
        // tmux ids 8 and 4 become configured indices 0 and 1.
        assert_eq!(tree.pane_indices(), vec![0, 1]);
        assert!(matches!(
            tree,
            Layout::Split {
                direction: LayoutSplitDirection::Right,
                ..
            }
        ));
    }

    #[test]
    fn a_serialized_layout_must_match_the_configured_pane_count() {
        let payload = "100x10,0,0{50x10,0,0,8,49x10,51,0,4}";
        let serialized = format!(
            "{:04x},{payload}",
            crate::layout::tmux_layout_checksum(payload)
        );
        let spec = load(&format!(
            "name: x\nwindows:\n  - grid:\n      layout: \"{serialized}\"\n      \
             panes:\n        - only\n"
        ))
        .unwrap();

        assert!(spec.windows[0]
            .layout_tree()
            .unwrap_err()
            .to_string()
            .contains("2 panes but 1 panes are configured"));
    }

    #[test]
    fn ignored_field_warnings_name_the_selected_backend() {
        let source = "name: x\ntmux_command: /usr/local/bin/tmux\nenable_pane_titles: true\n\
                      socket_name: work\n\
                      windows:\n  - x:\n      synchronize: after\n      panes: [vim]\n";

        let herdr = load_for(source, Backend::Herdr).unwrap_err().to_string();
        assert!(herdr.contains("not supported"), "{herdr}");

        let zellij = load_for(source, Backend::Zellij).unwrap();
        assert_eq!(
            zellij.warnings,
            vec![
                "`tmux_command` is tmux-specific and is ignored by the zellij backend.".to_string(),
                "`enable_pane_titles` only controls tmux pane borders and is ignored by zellij."
                    .to_string(),
                "`socket_name` selects a tmux or Herdr endpoint and is ignored by zellij."
                    .to_string(),
                "`synchronize` controls tmux synchronized panes and is ignored by zellij."
                    .to_string(),
            ]
        );
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
