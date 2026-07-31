use std::fmt::Write as _;

use anyhow::{bail, Result};

use crate::settings::Backend;
use crate::zellij_layout::kdl_string;

pub const PICKER_COMMAND: &str = "bootmux picker";
pub const DEFAULT_TMUX_KEY: &str = "F";
pub const DEFAULT_HERDR_KEY: &str = "prefix+shift+f";
pub const DEFAULT_HERDR_POPUP_SIZE: &str = "80%";
/// zellij binds Ctrl g/h/n/o/p/q/s/t and Alt f/i/o/n/h/j/k/l by default, so
/// the picker claims one of the few unused control chords.
pub const DEFAULT_ZELLIJ_KEY: &str = "Ctrl y";

pub fn snippet(backend: Backend) -> String {
    match backend {
        Backend::Tmux => tmux_snippet(),
        Backend::Herdr => herdr_snippet(),
        Backend::Zellij => zellij_snippet(),
    }
}

/// Generates a tmux 2.6-compatible prefix binding.
///
/// A regular window is used because tmux's `display-popup` command was added
/// long after the minimum supported tmux version.
pub fn tmux_snippet() -> String {
    tmux_snippet_with(DEFAULT_TMUX_KEY, PICKER_COMMAND)
        .expect("the built-in tmux picker binding is valid")
}

pub fn tmux_binding() -> String {
    tmux_snippet()
}

pub fn tmux_snippet_with(key: &str, command: &str) -> Result<String> {
    validate_tmux_key(key)?;
    validate_command(command)?;
    Ok(format!(
        "bind-key {key} new-window {}\n",
        tmux_double_quoted(command)
    ))
}

/// Generates a Herdr popup command binding using the documented
/// `[[keys.command]]` schema.
pub fn herdr_snippet() -> String {
    herdr_snippet_with(
        DEFAULT_HERDR_KEY,
        PICKER_COMMAND,
        DEFAULT_HERDR_POPUP_SIZE,
        DEFAULT_HERDR_POPUP_SIZE,
    )
    .expect("the built-in Herdr picker binding is valid")
}

pub fn herdr_binding() -> String {
    herdr_snippet()
}

pub fn herdr_snippet_with(key: &str, command: &str, width: &str, height: &str) -> Result<String> {
    if key.trim().is_empty() {
        bail!("Herdr binding key cannot be empty");
    }
    validate_command(command)?;
    validate_popup_dimension("width", width)?;
    validate_popup_dimension("height", height)?;

    Ok(format!(
        "[[keys.command]]\n\
         key = {}\n\
         type = \"popup\"\n\
         command = {}\n\
         description = \"open bootmux project picker\"\n\
         width = {}\n\
         height = {}\n",
        toml_basic_string(key),
        toml_basic_string(command),
        toml_basic_string(width),
        toml_basic_string(height),
    ))
}

/// Generates a zellij `keybinds` block that opens the picker in a floating
/// pane.
pub fn zellij_snippet() -> String {
    zellij_snippet_with(DEFAULT_ZELLIJ_KEY, PICKER_COMMAND)
        .expect("the built-in zellij picker binding is valid")
}

pub fn zellij_binding() -> String {
    zellij_snippet()
}

pub fn zellij_snippet_with(key: &str, command: &str) -> Result<String> {
    if key.trim().is_empty() {
        bail!("zellij binding key cannot be empty");
    }
    if key.contains(['"', '\\', '{', '}', ';']) {
        bail!("zellij binding key {key:?} contains KDL syntax");
    }
    validate_command(command)?;

    let argv = crate::shellwords::split(command);
    if argv.is_empty() {
        bail!("picker command {command:?} does not parse into an executable and arguments");
    }
    let argv = argv
        .iter()
        .map(|word| kdl_string(word))
        .collect::<Vec<_>>()
        .join(" ");

    // `shared_except "locked"` mirrors zellij's own default bindings so the
    // picker stays reachable from every mode but the locked one.
    Ok(format!(
        "keybinds {{\n    \
             shared_except \"locked\" {{\n        \
                 bind {} {{\n            \
                     Run {argv} {{\n                \
                         floating true\n                \
                         close_on_exit true\n            \
                     }}\n        \
                 }}\n    \
             }}\n\
         }}\n",
        kdl_string(key),
    ))
}

fn validate_tmux_key(key: &str) -> Result<()> {
    if key.is_empty() {
        bail!("tmux binding key cannot be empty");
    }
    if key.chars().any(char::is_whitespace) {
        bail!("tmux binding key {key:?} cannot contain whitespace");
    }
    if key.contains(['"', '\'', '\\', ';', '#']) {
        bail!("tmux binding key {key:?} contains tmux configuration syntax");
    }
    Ok(())
}

fn validate_command(command: &str) -> Result<()> {
    if command.trim().is_empty() {
        bail!("picker command cannot be empty");
    }
    if command.contains(['\n', '\r', '\0']) {
        bail!("picker command cannot contain a newline or NUL byte");
    }
    Ok(())
}

fn validate_popup_dimension(name: &str, dimension: &str) -> Result<()> {
    let valid_cells = dimension
        .parse::<u16>()
        .map(|cells| cells > 0)
        .unwrap_or(false);
    let valid_percent = dimension
        .strip_suffix('%')
        .and_then(|percent| percent.parse::<u8>().ok())
        .map(|percent| (1..=100).contains(&percent))
        .unwrap_or(false);
    if !valid_cells && !valid_percent {
        bail!(
            "Herdr popup {name} {dimension:?} must be positive terminal cells or a percentage from 1% to 100%"
        );
    }
    Ok(())
}

fn tmux_double_quoted(value: &str) -> String {
    let mut quoted = String::with_capacity(value.len() + 2);
    quoted.push('"');
    for character in value.chars() {
        match character {
            '\\' | '"' | '$' | '`' => {
                quoted.push('\\');
                quoted.push(character);
            }
            _ => quoted.push(character),
        }
    }
    quoted.push('"');
    quoted
}

fn toml_basic_string(value: &str) -> String {
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
                write!(&mut quoted, "\\u{:04X}", control as u32)
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
    use super::*;

    #[test]
    fn tmux_default_is_a_26_compatible_new_window_binding() {
        assert_eq!(tmux_snippet(), "bind-key F new-window \"bootmux picker\"\n");
        assert!(!tmux_snippet().contains("display-popup"));
    }

    #[test]
    fn tmux_custom_command_is_quoted_as_one_argument() {
        assert_eq!(
            tmux_snippet_with("C-f", r#"bootmux "$PROJECT" `which fzf`"#).unwrap(),
            "bind-key C-f new-window \"bootmux \\\"\\$PROJECT\\\" \\`which fzf\\`\"\n"
        );
    }

    #[test]
    fn tmux_rejects_config_injection_in_keys_and_commands() {
        assert!(tmux_snippet_with("F\nrun-shell evil", PICKER_COMMAND).is_err());
        assert!(tmux_snippet_with("F", "bootmux picker\nrun evil").is_err());
        assert!(tmux_snippet_with("F;run-shell", PICKER_COMMAND).is_err());
    }

    #[test]
    fn herdr_default_uses_popup_schema_and_eighty_percent_geometry() {
        assert_eq!(
            herdr_snippet(),
            "[[keys.command]]\n\
             key = \"prefix+shift+f\"\n\
             type = \"popup\"\n\
             command = \"bootmux picker\"\n\
             description = \"open bootmux project picker\"\n\
             width = \"80%\"\n\
             height = \"80%\"\n"
        );
    }

    #[test]
    fn herdr_values_are_valid_toml_basic_strings() {
        let generated = herdr_snippet_with(
            "prefix+alt+g",
            r#"bootmux picker --prompt "go""#,
            "120",
            "75%",
        )
        .unwrap();
        assert!(generated.contains(r#"command = "bootmux picker --prompt \"go\"""#));
        assert!(generated.contains("width = \"120\""));
        assert!(generated.contains("height = \"75%\""));
    }

    #[test]
    fn herdr_rejects_invalid_geometry() {
        assert!(herdr_snippet_with(DEFAULT_HERDR_KEY, PICKER_COMMAND, "0", "80%").is_err());
        assert!(herdr_snippet_with(DEFAULT_HERDR_KEY, PICKER_COMMAND, "101%", "80%").is_err());
        assert!(herdr_snippet_with(DEFAULT_HERDR_KEY, PICKER_COMMAND, "wide", "80%").is_err());
    }

    #[test]
    fn backend_dispatches_to_the_matching_format() {
        assert_eq!(snippet(Backend::Tmux), tmux_snippet());
        assert_eq!(snippet(Backend::Herdr), herdr_snippet());
        assert_eq!(snippet(Backend::Zellij), zellij_snippet());
    }

    #[test]
    fn zellij_default_is_a_floating_run_binding_outside_locked_mode() {
        assert_eq!(
            zellij_snippet(),
            "keybinds {\n    \
                 shared_except \"locked\" {\n        \
                     bind \"Ctrl y\" {\n            \
                         Run \"bootmux\" \"picker\" {\n                \
                             floating true\n                \
                             close_on_exit true\n            \
                         }\n        \
                     }\n    \
                 }\n\
             }\n"
        );
    }

    #[test]
    fn zellij_splits_the_picker_command_into_argv_and_escapes_kdl() {
        let generated = zellij_snippet_with("Alt g", r#"bootmux picker --prompt "go ""#).unwrap();
        assert!(generated.contains("bind \"Alt g\""));
        assert!(
            generated.contains(r#"Run "bootmux" "picker" "--prompt" "go ""#),
            "{generated}"
        );
    }

    #[test]
    fn zellij_rejects_kdl_injection_and_unusable_commands() {
        assert!(zellij_snippet_with("", PICKER_COMMAND).is_err());
        assert!(zellij_snippet_with("Ctrl y\" }; bind \"x", PICKER_COMMAND).is_err());
        assert!(zellij_snippet_with(DEFAULT_ZELLIJ_KEY, "bootmux picker\nRun evil").is_err());
        // An unterminated quote parses to no words at all.
        assert!(zellij_snippet_with(DEFAULT_ZELLIJ_KEY, "\"bootmux").is_err());
    }
}
