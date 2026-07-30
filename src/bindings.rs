use std::fmt::Write as _;

use anyhow::{bail, Result};

use crate::settings::Backend;

pub const PICKER_COMMAND: &str = "bootmux picker";
pub const DEFAULT_TMUX_KEY: &str = "F";
pub const DEFAULT_HERDR_KEY: &str = "prefix+shift+f";
pub const DEFAULT_HERDR_POPUP_SIZE: &str = "80%";

pub fn snippet(backend: Backend) -> String {
    match backend {
        Backend::Tmux => tmux_snippet(),
        Backend::Herdr => herdr_snippet(),
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
    }
}
