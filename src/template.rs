use std::collections::HashMap;

use anyhow::{bail, Result};
use minijinja::{context, Environment, UndefinedBehavior};

use crate::env::Env;

const ERB_MIGRATION_HINT: &str = "this file contains ERB tags (`<% ... %>`), which tmuxinator \
supported but bootmux only accepts in mux's restricted `<%= @settings[\"key\"] %>` form. \
For other templates, migrate to MiniJinja: `<%= @args[0] %>` becomes `{{ args[0] }}` and \
`<%= ENV[\"VAR\"] %>` to `{{ env.VAR }}`.";

pub fn render_config(
    source: &str,
    settings: &HashMap<String, String>,
    args: &[String],
    env: &Env,
) -> Result<String> {
    let mut jinja = Environment::new();
    jinja.set_undefined_behavior(UndefinedBehavior::Lenient);
    let rendered = match jinja.render_str(source, context! { settings, args, env => env.all }) {
        Ok(rendered) => rendered,
        Err(error) => bail!("Failed to parse config file: {error}"),
    };

    // mux intentionally implements a tiny, non-executable subset of ERB for
    // key=value settings. Mirror that dialect without evaluating Ruby.
    let rendered = match substitute_mux_settings(&rendered, settings) {
        Some(rendered) => rendered,
        None => {
            bail!("Failed to parse config file: {ERB_MIGRATION_HINT}");
        }
    };
    Ok(rendered)
}

fn substitute_mux_settings(source: &str, settings: &HashMap<String, String>) -> Option<String> {
    const OPEN: &str = "<%";
    const EXPRESSION_OPEN: &str = "<%=";
    const CLOSE: &str = "%>";
    const PREFIX: &str = "@settings[\"";
    const SUFFIX: &str = "\"]";

    let mut output = String::with_capacity(source.len());
    let mut rest = source;
    while let Some(open) = rest.find(OPEN) {
        output.push_str(&rest[..open]);
        let tag = &rest[open..];
        if !tag.starts_with(EXPRESSION_OPEN) {
            return None;
        }
        let close = tag.find(CLOSE)?;
        let end = close + CLOSE.len();
        let expression = tag[EXPRESSION_OPEN.len()..close].trim_matches(' ');
        let key = expression
            .strip_prefix(PREFIX)
            .and_then(|value| value.strip_suffix(SUFFIX))
            .filter(|key| !key.is_empty())?;
        if let Some(value) = settings.get(key) {
            output.push_str(value);
        }
        rest = &tag[end..];
    }
    output.push_str(rest);
    Some(output)
}

pub fn render_sample(source: &str, name: &str, path: &str) -> Result<String> {
    let mut jinja = Environment::new();
    jinja.set_undefined_behavior(UndefinedBehavior::Lenient);
    Ok(jinja.render_str(source, context! { name, path })?)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_env() -> Env {
        let mut env = Env::default();
        env.all.insert("MY_VAR".to_string(), "hello".to_string());
        env
    }

    #[test]
    fn renders_settings_args_and_env() {
        let mut settings = HashMap::new();
        settings.insert("workspace".to_string(), "/code".to_string());
        let args = vec!["extra".to_string()];

        let out = render_config(
            "root: {{ settings.workspace }}\nfirst: {{ args[0] }}\nvar: {{ env.MY_VAR }}",
            &settings,
            &args,
            &test_env(),
        )
        .unwrap();
        assert_eq!(out, "root: /code\nfirst: extra\nvar: hello");
    }

    #[test]
    fn undefined_variables_render_empty() {
        let out = render_config(
            "x: {{ settings.missing }}!",
            &HashMap::new(),
            &[],
            &test_env(),
        )
        .unwrap();
        assert_eq!(out, "x: !");
    }

    #[test]
    fn renders_mux_settings_erb_without_executing_ruby() {
        let settings = HashMap::from([
            ("root".to_owned(), "/tmp/{{ untouched }}".to_owned()),
            ("host".to_owned(), "localhost".to_owned()),
        ]);
        let out = render_config(
            "root: <%=  @settings[\"root\"]  %>\nhost: <%= @settings[\"host\"] %>\nmissing: <%= @settings[\"missing\"] %>",
            &settings,
            &[],
            &test_env(),
        )
        .unwrap();
        assert_eq!(
            out,
            "root: /tmp/{{ untouched }}\nhost: localhost\nmissing: "
        );
    }

    #[test]
    fn other_erb_syntax_is_rejected_with_migration_hint() {
        let err = render_config("name: <%= @args[0] %>", &HashMap::new(), &[], &test_env())
            .unwrap_err()
            .to_string();
        assert!(err.starts_with("Failed to parse config file:"));
        assert!(err.contains("MiniJinja"));
    }

    #[test]
    fn missing_mux_settings_render_empty_for_lifecycle_commands() {
        let out = render_config(
            "name: static\nroot: <%= @settings[\"root\"] %>",
            &HashMap::new(),
            &[],
            &test_env(),
        )
        .unwrap();
        assert_eq!(out, "name: static\nroot: ");
    }
}
