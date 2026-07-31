//! Renders a project into the KDL layout documents zellij consumes.
//!
//! zellij is declarative where tmux is imperative: one layout document
//! expresses tab names, working directories, split geometry, pane titles, and
//! initial focus, so bootmux builds the whole topology in a single call
//! instead of a chain of split commands.
//!
//! Two constraints are load-bearing and were confirmed against zellij 0.44:
//! the document must be a complete `layout { … }` node (a bare tab body is
//! rejected), and every node needs its own line (semicolon-separated nodes on
//! one line fail to parse).

use std::fmt::Write as _;

use anyhow::Result;

use crate::layout::{Layout, SplitDirection};
use crate::spec::{ProjectSpec, WindowSpec};

const INDENT: &str = "    ";

/// Renders every window of the project as one document, used to create the
/// session in a single `attach --create-background` call.
pub fn render_project(spec: &ProjectSpec) -> Result<String> {
    render_tabs(spec, 0..spec.windows.len())
}

/// Renders a single window as its own document, used by `--append` through
/// `zellij action new-tab --layout-string`.
pub fn render_window(spec: &ProjectSpec, window_index: usize) -> Result<String> {
    render_tabs(spec, window_index..window_index + 1)
}

fn render_tabs(spec: &ProjectSpec, windows: std::ops::Range<usize>) -> Result<String> {
    let mut document = String::from("layout {\n");
    for window_index in windows {
        let window = &spec.windows[window_index];
        render_tab(&mut document, spec, window, window_index)?;
    }
    document.push_str("}\n");
    Ok(document)
}

fn render_tab(
    document: &mut String,
    spec: &ProjectSpec,
    window: &WindowSpec,
    window_index: usize,
) -> Result<()> {
    let layout = window.layout_tree()?;
    let panes = window.effective_panes();
    let focused_pane = focused_pane_for(spec, window, window_index);

    let mut attributes = Vec::new();
    if let Some(name) = &window.name {
        attributes.push(format!("name={}", kdl_string(name)));
    }
    attributes.push(format!("cwd={}", kdl_string(&window.root)));
    if window_index == spec.startup_window {
        attributes.push("focus=true".to_string());
    }
    // A tab node carries the direction of its own children, so a window whose
    // root is a split declares it here instead of wrapping in an extra pane.
    if let Layout::Split { direction, .. } = &layout {
        attributes.push(format!(
            "split_direction={}",
            kdl_string(split_direction(*direction))
        ));
    }

    writeln!(document, "{INDENT}tab {} {{", attributes.join(" "))
        .expect("writing to a String cannot fail");
    render_layout(document, &layout, &panes, focused_pane, 2, None, true);
    writeln!(document, "{INDENT}}}").expect("writing to a String cannot fail");
    Ok(())
}

/// Which pane this tab should open focused on.
///
/// A project-level `startup_pane` only overrides the tab bootmux starts in;
/// every other tab keeps its own `focused_pane`.
fn focused_pane_for(spec: &ProjectSpec, window: &WindowSpec, window_index: usize) -> usize {
    if window_index == spec.startup_window {
        spec.startup_pane.unwrap_or(window.focused_pane)
    } else {
        window.focused_pane
    }
}

/// `is_tab_root` marks the split whose direction the enclosing `tab` node
/// already declared, so it contributes children rather than another container.
fn render_layout(
    document: &mut String,
    layout: &Layout,
    panes: &[crate::spec::PaneSpec],
    focused_pane: usize,
    depth: usize,
    size: Option<String>,
    is_tab_root: bool,
) {
    let indent = INDENT.repeat(depth);
    match layout {
        Layout::Pane(pane_index) => {
            let mut attributes = Vec::new();
            if let Some(title) = panes.get(*pane_index).and_then(|pane| pane.title.as_ref()) {
                attributes.push(format!("name={}", kdl_string(title)));
            }
            if let Some(size) = size {
                attributes.push(format!("size={}", kdl_string(&size)));
            }
            if *pane_index == focused_pane {
                attributes.push("focus=true".to_string());
            }

            if attributes.is_empty() {
                writeln!(document, "{indent}pane").expect("writing to a String cannot fail");
            } else {
                writeln!(document, "{indent}pane {}", attributes.join(" "))
                    .expect("writing to a String cannot fail");
            }
        }
        Layout::Split {
            direction,
            ratio,
            first,
            second,
        } => {
            // Only the first child is sized; the second takes the remainder so
            // the two percentages can never disagree after rounding.
            let first_size = Some(percent(*ratio));

            if is_tab_root {
                render_layout(
                    document,
                    first,
                    panes,
                    focused_pane,
                    depth,
                    first_size,
                    false,
                );
                render_layout(document, second, panes, focused_pane, depth, None, false);
                return;
            }

            let mut attributes = vec![format!(
                "split_direction={}",
                kdl_string(split_direction(*direction))
            )];
            if let Some(size) = size {
                attributes.push(format!("size={}", kdl_string(&size)));
            }
            writeln!(document, "{indent}pane {} {{", attributes.join(" "))
                .expect("writing to a String cannot fail");
            render_layout(
                document,
                first,
                panes,
                focused_pane,
                depth + 1,
                first_size,
                false,
            );
            render_layout(
                document,
                second,
                panes,
                focused_pane,
                depth + 1,
                None,
                false,
            );
            writeln!(document, "{indent}}}").expect("writing to a String cannot fail");
        }
    }
}

/// zellij names a split by the orientation of the divider, which is the
/// opposite word from the direction bootmux splits in: side-by-side panes are
/// separated by a `"vertical"` divider.
fn split_direction(direction: SplitDirection) -> &'static str {
    match direction {
        SplitDirection::Right => "vertical",
        SplitDirection::Down => "horizontal",
    }
}

/// zellij sizes panes in whole percent, so a ratio is rounded and clamped into
/// a range that always leaves room for the sibling pane.
fn percent(ratio: f64) -> String {
    let rounded = (ratio * 100.0).round().clamp(1.0, 99.0) as u32;
    format!("{rounded}%")
}

/// Quotes a value as a KDL escaped string. KDL spells unicode escapes
/// `\u{XXXX}` rather than JSON's `\uXXXX`.
pub fn kdl_string(value: &str) -> String {
    let mut quoted = String::with_capacity(value.len() + 2);
    quoted.push('"');
    for character in value.chars() {
        match character {
            '"' => quoted.push_str("\\\""),
            '\\' => quoted.push_str("\\\\"),
            '\u{8}' => quoted.push_str("\\b"),
            '\t' => quoted.push_str("\\t"),
            '\n' => quoted.push_str("\\n"),
            '\u{c}' => quoted.push_str("\\f"),
            '\r' => quoted.push_str("\\r"),
            control if control <= '\u{1f}' || control == '\u{7f}' => {
                write!(&mut quoted, "\\u{{{:X}}}", control as u32)
                    .expect("writing to a String cannot fail");
            }
            _ => quoted.push(character),
        }
    }
    quoted.push('"');
    quoted
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::path::PathBuf;

    use super::*;
    use crate::env::Env;
    use crate::project::LoadOptions;
    use crate::settings::Backend;

    fn spec(source: &str) -> ProjectSpec {
        ProjectSpec::load(
            "/work/demo.yml",
            source,
            &HashMap::new(),
            &[],
            LoadOptions::default(),
            &Env {
                home: "/home/test".into(),
                cwd: PathBuf::from("/work"),
                ..Env::default()
            },
            Backend::Zellij,
        )
        .unwrap()
    }

    #[test]
    fn a_single_pane_window_is_a_tab_with_one_pane() {
        let rendered = render_project(&spec(
            "name: solo\nroot: /work\nwindows:\n  - editor: vim\n",
        ))
        .unwrap();
        assert_eq!(
            rendered,
            "layout {\n\
             \x20   tab name=\"editor\" cwd=\"/work\" focus=true {\n\
             \x20       pane focus=true\n\
             \x20   }\n\
             }\n"
        );
    }

    #[test]
    fn a_root_split_is_declared_on_the_tab_and_only_the_first_child_is_sized() {
        let rendered = render_project(&spec(
            "name: chain\nroot: /work\nwindows:\n  - app:\n      panes:\n        \
             - editor:\n            command: vim\n        - shell:\n            \
             split: right\n            ratio: 0.65\n            command: bash\n",
        ))
        .unwrap();
        assert_eq!(
            rendered,
            "layout {\n\
             \x20   tab name=\"app\" cwd=\"/work\" focus=true split_direction=\"vertical\" {\n\
             \x20       pane name=\"editor\" size=\"65%\" focus=true\n\
             \x20       pane name=\"shell\"\n\
             \x20   }\n\
             }\n"
        );
    }

    #[test]
    fn nested_splits_become_nested_container_panes() {
        let rendered = render_project(&spec(
            "name: tiled\nroot: /work\nwindows:\n  - grid:\n      layout: tiled\n      \
             panes:\n        - a\n        - b\n        - c\n        - d\n",
        ))
        .unwrap();
        assert_eq!(
            rendered,
            "layout {\n\
             \x20   tab name=\"grid\" cwd=\"/work\" focus=true split_direction=\"horizontal\" {\n\
             \x20       pane split_direction=\"vertical\" size=\"50%\" {\n\
             \x20           pane size=\"50%\" focus=true\n\
             \x20           pane\n\
             \x20       }\n\
             \x20       pane split_direction=\"vertical\" {\n\
             \x20           pane size=\"50%\"\n\
             \x20           pane\n\
             \x20       }\n\
             \x20   }\n\
             }\n"
        );
    }

    #[test]
    fn startup_window_and_startup_pane_place_the_focus() {
        let rendered = render_project(&spec(
            "name: focus\nroot: /work\nstartup_window: server\nstartup_pane: logs\n\
             windows:\n  - editor:\n      focused_pane: two\n      panes:\n        \
             - one: echo one\n        - two: echo two\n  - server:\n      panes:\n        \
             - api: echo api\n        - logs: echo logs\n",
        ))
        .unwrap();
        // The non-startup tab keeps its own focused_pane; the startup tab is
        // overridden by startup_pane.
        assert!(rendered.contains("tab name=\"editor\" cwd=\"/work\""));
        assert!(!rendered.contains("tab name=\"editor\" cwd=\"/work\" focus=true"));
        assert!(rendered.contains("pane name=\"two\" focus=true"));
        assert!(rendered.contains("tab name=\"server\" cwd=\"/work\" focus=true"));
        assert!(rendered.contains("pane name=\"logs\" focus=true"));
    }

    #[test]
    fn render_window_emits_a_complete_single_tab_document() {
        let project =
            spec("name: two\nroot: /work\nwindows:\n  - first: echo one\n  - second: echo two\n");
        let rendered = render_window(&project, 1).unwrap();
        assert!(rendered.starts_with("layout {\n"));
        assert!(rendered.ends_with("}\n"));
        assert!(rendered.contains("tab name=\"second\""));
        assert!(!rendered.contains("tab name=\"first\""));
    }

    #[test]
    fn names_and_paths_are_escaped_as_kdl_strings() {
        assert_eq!(kdl_string(r#"a"b\c"#), r#""a\"b\\c""#);
        assert_eq!(kdl_string("tab\nname"), r#""tab\nname""#);
        assert_eq!(kdl_string("bell\u{7}"), r#""bell\u{7}""#);

        let rendered = render_project(&spec(
            "name: quoted\nroot: /work\nwindows:\n  - 'say \"hi\"': echo hi\n",
        ))
        .unwrap();
        assert!(rendered.contains(r#"tab name="say \"hi\"""#), "{rendered}");
    }

    #[test]
    fn ratios_are_rounded_into_a_range_that_leaves_room_for_the_sibling() {
        assert_eq!(percent(0.5), "50%");
        assert_eq!(percent(0.654), "65%");
        assert_eq!(percent(0.0), "1%");
        assert_eq!(percent(1.0), "99%");
    }
}
