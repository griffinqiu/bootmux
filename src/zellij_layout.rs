//! Renders a project into the KDL layout documents zellij consumes.
//!
//! zellij is declarative where tmux is imperative: one layout document
//! expresses tab names, working directories, split geometry, pane titles, and
//! initial focus, so bootmux builds the whole topology in a single call
//! instead of a chain of split commands.
//!
//! Three constraints are load-bearing. Confirmed against zellij 0.44: the
//! document must be a complete `layout { … }` node (a bare tab body is
//! rejected), and every node needs its own line (semicolon-separated nodes on
//! one line fail to parse). Confirmed against zellij 0.45.0: a container is
//! merged into its parent whenever both split the same way, and the merged
//! children keep the percentages they were written with instead of being
//! rescaled into the container they came from. Nested same-direction
//! containers therefore render at the wrong sizes, so [`flatten_group`] emits
//! the merged form directly and sizes every child against its own container.

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
/// `zellij action new-tab --layout`.
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
    render_children(document, &layout, &panes, focused_pane, 2);
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

/// Emits everything the container rooted at `layout` holds, already flattened
/// into the child list zellij itself would produce.
fn render_children(
    document: &mut String,
    layout: &Layout,
    panes: &[crate::spec::PaneSpec],
    focused_pane: usize,
    depth: usize,
) {
    let Layout::Split { direction, .. } = layout else {
        render_node(document, layout, panes, focused_pane, depth, None);
        return;
    };

    let children = flatten_group(layout, *direction);
    let sizes = group_percentages(children.iter().map(|child| child.share));
    for (child, size) in children.iter().zip(sizes) {
        render_node(document, child.layout, panes, focused_pane, depth, size);
    }
}

/// Emits one child of a container: a leaf pane, or a nested container holding
/// the splits that run the other way.
fn render_node(
    document: &mut String,
    layout: &Layout,
    panes: &[crate::spec::PaneSpec],
    focused_pane: usize,
    depth: usize,
    size: Option<String>,
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
        Layout::Split { direction, .. } => {
            let mut attributes = vec![format!(
                "split_direction={}",
                kdl_string(split_direction(*direction))
            )];
            if let Some(size) = size {
                attributes.push(format!("size={}", kdl_string(&size)));
            }
            writeln!(document, "{indent}pane {} {{", attributes.join(" "))
                .expect("writing to a String cannot fail");
            render_children(document, layout, panes, focused_pane, depth + 1);
            writeln!(document, "{indent}}}").expect("writing to a String cannot fail");
        }
    }
}

/// One child of a flattened container, holding the share of that container's
/// own axis the child occupies.
struct GroupChild<'layout> {
    layout: &'layout Layout,
    share: f64,
}

/// Collects the run of splits sharing `direction` into one flat child list.
///
/// The binary layout model nests every extra split, but zellij merges a
/// same-direction container into its parent and then reads the merged
/// children's percentages as shares of that parent. Emitting the merged form
/// up front keeps bootmux's sizes and zellij's interpretation in agreement.
fn flatten_group(layout: &Layout, direction: SplitDirection) -> Vec<GroupChild<'_>> {
    fn visit<'layout>(
        layout: &'layout Layout,
        direction: SplitDirection,
        share: f64,
        children: &mut Vec<GroupChild<'layout>>,
    ) {
        match layout {
            Layout::Split {
                direction: split_direction,
                ratio,
                first,
                second,
            } if *split_direction == direction => {
                visit(first, direction, share * ratio, children);
                visit(second, direction, share * (1.0 - ratio), children);
            }
            _ => children.push(GroupChild { layout, share }),
        }
    }

    let mut children = Vec::new();
    visit(layout, direction, 1.0, &mut children);
    children
}

/// zellij sizes panes in whole percent, so shares become cumulative
/// boundaries: every child keeps its proportion, rounding never accumulates
/// across a long run, and the last child stays unsized so zellij gives it the
/// exact remainder.
fn group_percentages(shares: impl ExactSizeIterator<Item = f64>) -> Vec<Option<String>> {
    let count = shares.len();
    let mut percentages = Vec::with_capacity(count);
    let mut cumulative = 0.0;
    let mut assigned = 0i64;
    for (index, share) in shares.enumerate() {
        if index + 1 == count {
            percentages.push(None);
            break;
        }
        cumulative += share;
        // Every child still to come needs at least one percent of its own.
        let floor = assigned + 1;
        let ceiling = (100 - (count - index - 1) as i64).max(floor);
        let boundary = ((cumulative * 100.0).round() as i64).clamp(floor, ceiling);
        percentages.push(Some(format!("{}%", boundary - assigned)));
        assigned = boundary;
    }
    percentages
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

    fn sizes(shares: &[f64]) -> Vec<Option<String>> {
        group_percentages(shares.iter().copied())
    }

    #[test]
    fn ratios_are_rounded_into_a_range_that_leaves_room_for_every_sibling() {
        assert_eq!(sizes(&[0.5, 0.5]), [Some("50%".into()), None]);
        assert_eq!(sizes(&[0.654, 0.346]), [Some("65%".into()), None]);
        assert_eq!(sizes(&[0.0, 1.0]), [Some("1%".into()), None]);
        assert_eq!(sizes(&[1.0, 0.0]), [Some("99%".into()), None]);
    }

    #[test]
    fn a_run_of_shares_is_sized_from_cumulative_boundaries() {
        let third = 1.0 / 3.0;
        assert_eq!(
            sizes(&[third, third, third]),
            [Some("33%".into()), Some("34%".into()), None]
        );
        assert_eq!(
            sizes(&[0.2, 0.2, 0.2, 0.2, 0.2]),
            [
                Some("20%".into()),
                Some("20%".into()),
                Some("20%".into()),
                Some("20%".into()),
                None
            ]
        );

        // Every leading child keeps at least one percent, and the remainder
        // left for the last child never falls below one percent either.
        let crowded = vec![1.0 / 150.0; 150];
        let crowded = sizes(&crowded);
        assert!(crowded
            .iter()
            .all(|size| size.is_none() || size.as_deref() == Some("1%")));
        assert_eq!(crowded.last(), Some(&None));
    }

    #[test]
    fn a_run_of_same_direction_splits_is_flattened_into_one_container() {
        // zellij merges same-direction containers, so a three-pane row has to
        // be written flat with each pane's own share of the tab.
        let rendered = render_project(&spec(
            "name: row\nroot: /work\nwindows:\n  - grid:\n      layout: even-horizontal\n      \
             panes:\n        - a: echo a\n        - b: echo b\n        - c: echo c\n",
        ))
        .unwrap();
        assert_eq!(
            rendered,
            "layout {\n\
             \x20   tab name=\"grid\" cwd=\"/work\" focus=true split_direction=\"vertical\" {\n\
             \x20       pane name=\"a\" size=\"33%\" focus=true\n\
             \x20       pane name=\"b\" size=\"34%\"\n\
             \x20       pane name=\"c\"\n\
             \x20   }\n\
             }\n"
        );
    }

    #[test]
    fn tiled_rows_stay_nested_while_their_panes_are_flattened() {
        let rendered = render_project(&spec(
            "name: grid\nroot: /work\nwindows:\n  - grid:\n      layout: tiled\n      \
             panes:\n        - a: echo a\n        - b: echo b\n        - c: echo c\n        \
             - d: echo d\n        - e: echo e\n",
        ))
        .unwrap();
        assert_eq!(
            rendered,
            "layout {\n\
             \x20   tab name=\"grid\" cwd=\"/work\" focus=true split_direction=\"horizontal\" {\n\
             \x20       pane split_direction=\"vertical\" size=\"50%\" {\n\
             \x20           pane name=\"a\" size=\"33%\" focus=true\n\
             \x20           pane name=\"b\" size=\"34%\"\n\
             \x20           pane name=\"c\"\n\
             \x20       }\n\
             \x20       pane split_direction=\"vertical\" {\n\
             \x20           pane name=\"d\" size=\"50%\"\n\
             \x20           pane name=\"e\"\n\
             \x20       }\n\
             \x20   }\n\
             }\n"
        );
    }
}
