use std::collections::HashMap;

use anyhow::{bail, Result};
use minijinja::{context, Environment, UndefinedBehavior};

use crate::env::Env;

const ERB_MIGRATION_HINT: &str = "this file contains ERB tags (`<% ... %>`), which tmuxinator \
supported but bootmux does not. bootmux uses MiniJinja templates instead: migrate \
`<%= @settings[\"key\"] %>` to `{{ settings.key }}`, `<%= @args[0] %>` to `{{ args[0] }}` \
and `<%= ENV[\"VAR\"] %>` to `{{ env.VAR }}`.";

pub fn render_config(
    source: &str,
    settings: &HashMap<String, String>,
    args: &[String],
    env: &Env,
) -> Result<String> {
    if source.contains("<%") {
        bail!("Failed to parse config file: {ERB_MIGRATION_HINT}");
    }

    let mut jinja = Environment::new();
    jinja.set_undefined_behavior(UndefinedBehavior::Lenient);
    match jinja.render_str(source, context! { settings, args, env => env.all }) {
        Ok(rendered) => Ok(rendered),
        Err(error) => bail!("Failed to parse config file: {error}"),
    }
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
    fn erb_syntax_is_rejected_with_migration_hint() {
        let err = render_config("name: <%= @args[0] %>", &HashMap::new(), &[], &test_env())
            .unwrap_err()
            .to_string();
        assert!(err.starts_with("Failed to parse config file:"));
        assert!(err.contains("MiniJinja"));
    }
}
