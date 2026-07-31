use std::fmt;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::PathBuf;
use std::str::FromStr;
use std::sync::atomic::{AtomicU64, Ordering};

use anyhow::{bail, Context, Result};

use crate::env::Env;

pub const DEFAULT_BACKEND_KEY: &str = "default_backend";
pub const DEFAULT_BACKEND_CLI_KEY: &str = "default-backend";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Backend {
    Tmux,
    Herdr,
    Zellij,
}

impl Backend {
    /// Every backend in the order used by help text, completions, and
    /// ambiguity diagnostics.
    pub const ALL: [Self; 3] = [Self::Tmux, Self::Herdr, Self::Zellij];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Tmux => "tmux",
            Self::Herdr => "herdr",
            Self::Zellij => "zellij",
        }
    }

    /// The spelling used in prose, which is capitalized for Herdr only.
    pub const fn display_name(self) -> &'static str {
        match self {
            Self::Tmux => "tmux",
            Self::Herdr => "Herdr",
            Self::Zellij => "zellij",
        }
    }
}

impl fmt::Display for Backend {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BackendParseError {
    value: String,
}

impl fmt::Display for BackendParseError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let expected = Backend::ALL
            .iter()
            .map(|backend| format!("{:?}", backend.as_str()))
            .collect::<Vec<_>>()
            .join(", ");
        write!(
            formatter,
            "invalid backend {:?}; expected one of {expected}",
            self.value
        )
    }
}

impl std::error::Error for BackendParseError {}

impl FromStr for Backend {
    type Err = BackendParseError;

    fn from_str(value: &str) -> std::result::Result<Self, Self::Err> {
        let normalized = value.trim().to_ascii_lowercase();
        Backend::ALL
            .into_iter()
            .find(|backend| backend.as_str() == normalized)
            .ok_or_else(|| BackendParseError {
                value: value.to_string(),
            })
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ActiveEnvironment {
    pub tmux: bool,
    pub herdr: bool,
    pub herdr_popup: bool,
    pub zellij: bool,
}

impl ActiveEnvironment {
    /// The backends whose environment markers are present, in [`Backend::ALL`]
    /// order.
    pub fn backends(self) -> Vec<Backend> {
        Backend::ALL
            .into_iter()
            .filter(|backend| match backend {
                Backend::Tmux => self.tmux,
                Backend::Herdr => self.herdr,
                Backend::Zellij => self.zellij,
            })
            .collect()
    }

    pub fn is_ambiguous(self) -> bool {
        !self.herdr_popup && self.backends().len() > 1
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Settings {
    pub default_backend: Option<Backend>,
}

/// Classifies which multiplexer owns the foreground process when nested
/// multiplexer environment variables alone cannot provide an answer.
pub trait ForegroundProcessClassifier {
    fn classify_foreground(&self, env: &Env) -> Result<Option<Backend>>;
}

impl<F> ForegroundProcessClassifier for F
where
    F: Fn(&Env) -> Result<Option<Backend>>,
{
    fn classify_foreground(&self, env: &Env) -> Result<Option<Backend>> {
        self(env)
    }
}

/// Returns the global bootmux settings path.
///
/// This follows `${XDG_CONFIG_HOME:-$HOME/.config}/bootmux/config.toml`.
pub fn path(env: &Env) -> Result<PathBuf> {
    let xdg_config_home = env
        .xdg_config_home
        .as_deref()
        .or_else(|| env.all.get("XDG_CONFIG_HOME").map(String::as_str))
        .filter(|value| !value.is_empty());

    let config_home = match xdg_config_home {
        Some(value) => PathBuf::from(value),
        None => {
            let home = if env.home.is_empty() {
                env.all.get("HOME").map(String::as_str).unwrap_or("")
            } else {
                env.home.as_str()
            };
            if home.is_empty() {
                bail!("cannot locate bootmux settings: HOME and XDG_CONFIG_HOME are both unset");
            }
            PathBuf::from(home).join(".config")
        }
    };

    Ok(config_home.join("bootmux").join("config.toml"))
}

pub fn load(env: &Env) -> Result<Settings> {
    let config_path = path(env)?;
    let document = match fs::read_to_string(&config_path) {
        Ok(document) => document,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Settings::default()),
        Err(error) => {
            return Err(error).with_context(|| format!("failed to read {}", config_path.display()));
        }
    };

    parse_settings(&document).with_context(|| format!("failed to parse {}", config_path.display()))
}

pub fn default_backend(env: &Env) -> Result<Option<Backend>> {
    Ok(load(env)?.default_backend)
}

/// Reads a setting using its command-line key name.
pub fn get(env: &Env, key: &str) -> Result<Option<String>> {
    match key {
        DEFAULT_BACKEND_KEY | DEFAULT_BACKEND_CLI_KEY => {
            Ok(default_backend(env)?.map(|backend| backend.to_string()))
        }
        _ => bail!(
            "unknown bootmux setting {key:?}; supported setting: {DEFAULT_BACKEND_CLI_KEY} \
             (TOML key {DEFAULT_BACKEND_KEY})"
        ),
    }
}

/// Writes a setting using its command-line key name.
pub fn set(env: &Env, key: &str, value: &str) -> Result<()> {
    match key {
        DEFAULT_BACKEND_KEY | DEFAULT_BACKEND_CLI_KEY => set_default_backend(env, value.parse()?),
        _ => bail!(
            "unknown bootmux setting {key:?}; supported setting: {DEFAULT_BACKEND_CLI_KEY} \
             (TOML key {DEFAULT_BACKEND_KEY})"
        ),
    }
}

pub fn set_default_backend(env: &Env, backend: Backend) -> Result<()> {
    let config_path = path(env)?;
    let existing = match fs::read_to_string(&config_path) {
        Ok(document) => document,
        Err(error) if error.kind() == io::ErrorKind::NotFound => String::new(),
        Err(error) => {
            return Err(error).with_context(|| format!("failed to read {}", config_path.display()));
        }
    };
    let updated = document_with_default_backend(&existing, backend)
        .with_context(|| format!("failed to update {}", config_path.display()))?;
    atomic_write(&config_path, updated.as_bytes())
}

pub fn active_environment(env: &Env) -> ActiveEnvironment {
    let populated = |key: &str| {
        env.all
            .get(key)
            .map(|value| !value.is_empty())
            .unwrap_or(false)
    };
    let populated_prefix = |prefix: &str| {
        env.all
            .iter()
            .any(|(key, value)| key.starts_with(prefix) && !value.is_empty())
    };

    let herdr_popup = populated_prefix("HERDR_ACTIVE_");
    let herdr = herdr_popup
        || [
            "HERDR_ENV",
            "HERDR_WORKSPACE_ID",
            "HERDR_TAB_ID",
            "HERDR_PANE_ID",
            "HERDR_SOCKET_PATH",
        ]
        .into_iter()
        .any(&populated);
    let tmux = env
        .tmux
        .as_deref()
        .map(|value| !value.is_empty())
        .unwrap_or(false)
        || populated("TMUX");
    // zellij sets ZELLIJ to "0" inside a session, so presence rather than
    // truthiness is the marker.
    let zellij = ["ZELLIJ", "ZELLIJ_SESSION_NAME", "ZELLIJ_PANE_ID"]
        .into_iter()
        .any(&populated);

    ActiveEnvironment {
        tmux,
        herdr,
        herdr_popup,
        zellij,
    }
}

pub fn is_herdr_popup(env: &Env) -> bool {
    active_environment(env).herdr_popup
}

/// Resolves a backend in this order: an explicit choice, the active
/// multiplexer environment, the global setting, then tmux.
pub fn resolve_backend(explicit: Option<Backend>, env: &Env) -> Result<Backend> {
    resolve_backend_with_classifier(explicit, env, None)
}

pub fn resolve_backend_with_classifier(
    explicit: Option<Backend>,
    env: &Env,
    classifier: Option<&dyn ForegroundProcessClassifier>,
) -> Result<Backend> {
    if let Some(backend) = explicit {
        return Ok(backend);
    }

    let active = active_environment(env);
    // A Herdr popup deliberately inherits the underlying pane's environment,
    // including TMUX when Herdr itself is nested in tmux. HERDR_ACTIVE_*
    // identifies the popup owner more precisely than those inherited markers.
    if active.herdr_popup {
        return Ok(Backend::Herdr);
    }
    let active_backends = active.backends();
    match active_backends.as_slice() {
        [] => {}
        [backend] => return Ok(*backend),
        several => {
            let classified = classifier
                .map(|classifier| classifier.classify_foreground(env))
                .transpose()?
                .flatten();
            if let Some(backend) = classified {
                return Ok(backend);
            }
            let names = several
                .iter()
                .map(|backend| backend.display_name())
                .collect::<Vec<_>>()
                .join(", ");
            bail!(
                "several multiplexer environments are active ({names}) and the \
                 foreground multiplexer is ambiguous; choose one explicitly with --backend"
            );
        }
    }

    Ok(default_backend(env)?.unwrap_or(Backend::Tmux))
}

fn parse_settings(document: &str) -> Result<Settings> {
    let mut settings = Settings::default();
    let mut in_top_level = true;

    for (line_index, original_line) in document.lines().enumerate() {
        let line = original_line.trim_start_matches('\u{feff}').trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if line.starts_with('[') {
            in_top_level = false;
            continue;
        }
        if !in_top_level {
            continue;
        }

        let Some((raw_key, raw_value)) = line.split_once('=') else {
            continue;
        };
        if raw_key.trim() != DEFAULT_BACKEND_KEY {
            continue;
        }
        if settings.default_backend.is_some() {
            bail!(
                "duplicate {DEFAULT_BACKEND_KEY} setting on line {}",
                line_index + 1
            );
        }

        let value = parse_toml_string(raw_value).with_context(|| {
            format!(
                "invalid {DEFAULT_BACKEND_KEY} value on line {}",
                line_index + 1
            )
        })?;
        settings.default_backend = Some(value.parse()?);
    }

    Ok(settings)
}

fn parse_toml_string(raw_value: &str) -> Result<String> {
    let value = strip_toml_comment(raw_value).trim();
    if value.len() < 2 {
        bail!("expected a quoted TOML string");
    }

    let quote = value.as_bytes()[0];
    if !matches!(quote, b'\'' | b'"') || value.as_bytes()[value.len() - 1] != quote {
        bail!("expected a quoted TOML string");
    }
    let inner = &value[1..value.len() - 1];
    if quote == b'\'' {
        return Ok(inner.to_string());
    }

    let mut parsed = String::with_capacity(inner.len());
    let mut chars = inner.chars();
    while let Some(character) = chars.next() {
        if character != '\\' {
            parsed.push(character);
            continue;
        }
        let escaped = chars
            .next()
            .ok_or_else(|| anyhow::anyhow!("unterminated TOML escape"))?;
        match escaped {
            '"' => parsed.push('"'),
            '\\' => parsed.push('\\'),
            'n' => parsed.push('\n'),
            'r' => parsed.push('\r'),
            't' => parsed.push('\t'),
            _ => bail!("unsupported TOML escape \\{escaped}"),
        }
    }
    Ok(parsed)
}

fn strip_toml_comment(value: &str) -> &str {
    let mut single_quoted = false;
    let mut double_quoted = false;
    let mut escaped = false;

    for (index, character) in value.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        match character {
            '\\' if double_quoted => escaped = true,
            '\'' if !double_quoted => single_quoted = !single_quoted,
            '"' if !single_quoted => double_quoted = !double_quoted,
            '#' if !single_quoted && !double_quoted => return &value[..index],
            _ => {}
        }
    }
    value
}

fn document_with_default_backend(document: &str, backend: Backend) -> Result<String> {
    // Validate the value currently on disk before preserving the rest of the
    // document. This also catches duplicate assignments.
    parse_settings(document)?;

    let assignment = format!("{DEFAULT_BACKEND_KEY} = \"{}\"", backend.as_str());
    let mut output = String::new();
    let mut replaced = false;
    let mut in_top_level = true;

    for original_line in document.lines() {
        let line = original_line.trim_start_matches('\u{feff}').trim();
        if in_top_level && line.starts_with('[') {
            if !replaced {
                output.push_str(&assignment);
                output.push('\n');
                replaced = true;
            }
            in_top_level = false;
        }

        let is_assignment = in_top_level
            && line
                .split_once('=')
                .map(|(key, _)| key.trim() == DEFAULT_BACKEND_KEY)
                .unwrap_or(false);
        if is_assignment {
            output.push_str(&assignment);
            replaced = true;
        } else {
            output.push_str(original_line);
        }
        output.push('\n');
    }

    if !replaced {
        output.push_str(&assignment);
        output.push('\n');
    }
    Ok(output)
}

fn atomic_write(config_path: &std::path::Path, contents: &[u8]) -> Result<()> {
    let parent = config_path
        .parent()
        .ok_or_else(|| anyhow::anyhow!("settings path has no parent directory"))?;
    fs::create_dir_all(parent).with_context(|| format!("failed to create {}", parent.display()))?;

    let (temp_path, mut temp_file) = create_temp_file(parent)?;
    let write_result = (|| -> Result<()> {
        temp_file
            .write_all(contents)
            .with_context(|| format!("failed to write {}", temp_path.display()))?;
        temp_file
            .sync_all()
            .with_context(|| format!("failed to sync {}", temp_path.display()))?;

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&temp_path, fs::Permissions::from_mode(0o600))
                .with_context(|| format!("failed to secure {}", temp_path.display()))?;
        }

        drop(temp_file);
        fs::rename(&temp_path, config_path).with_context(|| {
            format!(
                "failed to atomically replace {} with {}",
                config_path.display(),
                temp_path.display()
            )
        })?;

        // Best-effort directory sync makes the rename durable on filesystems
        // that support syncing directory handles.
        if let Ok(directory) = File::open(parent) {
            let _ = directory.sync_all();
        }
        Ok(())
    })();

    if write_result.is_err() {
        let _ = fs::remove_file(&temp_path);
    }
    write_result
}

fn create_temp_file(parent: &std::path::Path) -> Result<(PathBuf, File)> {
    static NEXT_TEMP_ID: AtomicU64 = AtomicU64::new(0);

    for _ in 0..64 {
        let id = NEXT_TEMP_ID.fetch_add(1, Ordering::Relaxed);
        let temp_path = parent.join(format!(".config.toml.tmp.{}.{}", std::process::id(), id));
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }

        match options.open(&temp_path) {
            Ok(file) => return Ok((temp_path, file)),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => {
                return Err(error)
                    .with_context(|| format!("failed to create {}", temp_path.display()));
            }
        }
    }

    bail!(
        "failed to create a unique temporary settings file in {}",
        parent.display()
    )
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::fs;

    use tempfile::TempDir;

    use super::*;

    fn env_at(temp: &TempDir) -> Env {
        Env {
            home: temp.path().join("home").to_string_lossy().into_owned(),
            xdg_config_home: Some(temp.path().join("xdg").to_string_lossy().into_owned()),
            ..Env::default()
        }
    }

    fn env_with(vars: &[(&str, &str)]) -> Env {
        let all = vars
            .iter()
            .map(|(key, value)| (key.to_string(), value.to_string()))
            .collect::<HashMap<_, _>>();
        Env {
            tmux: all.get("TMUX").cloned(),
            all,
            ..Env::default()
        }
    }

    #[test]
    fn backend_parsing_is_case_insensitive_and_display_is_canonical() {
        assert_eq!("TMUX".parse::<Backend>().unwrap(), Backend::Tmux);
        assert_eq!(" Herdr ".parse::<Backend>().unwrap(), Backend::Herdr);
        assert_eq!(" ZELLIJ ".parse::<Backend>().unwrap(), Backend::Zellij);
        assert_eq!(Backend::Herdr.to_string(), "herdr");
        assert_eq!(Backend::Zellij.to_string(), "zellij");

        let error = "screen".parse::<Backend>().unwrap_err().to_string();
        for backend in Backend::ALL {
            assert!(error.contains(backend.as_str()), "{error}");
        }
    }

    #[test]
    fn path_prefers_nonempty_xdg_and_falls_back_to_home() {
        let mut env = Env {
            home: "/users/me".to_string(),
            xdg_config_home: Some("/custom/config".to_string()),
            ..Env::default()
        };
        assert_eq!(
            path(&env).unwrap(),
            PathBuf::from("/custom/config/bootmux/config.toml")
        );

        env.xdg_config_home = Some(String::new());
        assert_eq!(
            path(&env).unwrap(),
            PathBuf::from("/users/me/.config/bootmux/config.toml")
        );
    }

    #[test]
    fn missing_home_and_xdg_is_an_actionable_error() {
        let error = path(&Env::default()).unwrap_err().to_string();
        assert!(error.contains("HOME"));
        assert!(error.contains("XDG_CONFIG_HOME"));
    }

    #[test]
    fn load_accepts_comments_quotes_and_ignores_other_keys() {
        let temp = TempDir::new().unwrap();
        let env = env_at(&temp);
        let config_path = path(&env).unwrap();
        fs::create_dir_all(config_path.parent().unwrap()).unwrap();
        fs::write(
            &config_path,
            "# bootmux\nother = true\ndefault_backend = 'herdr' # selected\n",
        )
        .unwrap();

        assert_eq!(load(&env).unwrap().default_backend, Some(Backend::Herdr));
        assert_eq!(
            get(&env, DEFAULT_BACKEND_KEY).unwrap().as_deref(),
            Some("herdr")
        );
        assert_eq!(
            get(&env, DEFAULT_BACKEND_CLI_KEY).unwrap().as_deref(),
            Some("herdr")
        );
    }

    #[test]
    fn invalid_and_duplicate_backend_values_are_rejected() {
        let temp = TempDir::new().unwrap();
        let env = env_at(&temp);
        let config_path = path(&env).unwrap();
        fs::create_dir_all(config_path.parent().unwrap()).unwrap();
        fs::write(&config_path, "default_backend = \"screen\"\n").unwrap();
        assert!(load(&env).unwrap_err().to_string().contains("parse"));

        fs::write(
            &config_path,
            "default_backend = \"tmux\"\ndefault_backend = \"herdr\"\n",
        )
        .unwrap();
        let error = load(&env).unwrap_err();
        assert!(format!("{error:#}").contains("duplicate default_backend"));
    }

    #[test]
    fn set_is_atomic_secure_and_preserves_unrelated_content() {
        let temp = TempDir::new().unwrap();
        let env = env_at(&temp);
        let config_path = path(&env).unwrap();
        fs::create_dir_all(config_path.parent().unwrap()).unwrap();
        fs::write(
            &config_path,
            "# retained\nother = \"value\"\ndefault_backend = \"tmux\"\n",
        )
        .unwrap();

        set_default_backend(&env, Backend::Herdr).unwrap();

        let document = fs::read_to_string(&config_path).unwrap();
        assert!(document.contains("# retained"));
        assert!(document.contains("other = \"value\""));
        assert!(document.contains("default_backend = \"herdr\""));
        assert_eq!(default_backend(&env).unwrap(), Some(Backend::Herdr));
        assert!(fs::read_dir(config_path.parent().unwrap())
            .unwrap()
            .all(|entry| !entry
                .unwrap()
                .file_name()
                .to_string_lossy()
                .contains(".tmp.")));

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(&config_path).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
    }

    #[test]
    fn setting_is_inserted_before_toml_tables() {
        let document = "# heading\n[future]\nvalue = 1\n";
        let updated = document_with_default_backend(document, Backend::Tmux).unwrap();
        assert_eq!(
            updated,
            "# heading\ndefault_backend = \"tmux\"\n[future]\nvalue = 1\n"
        );
    }

    #[test]
    fn detects_tmux_ordinary_herdr_and_herdr_popup() {
        let tmux = active_environment(&env_with(&[("TMUX", "/tmp/tmux")]));
        assert_eq!(
            tmux,
            ActiveEnvironment {
                tmux: true,
                herdr: false,
                herdr_popup: false,
                zellij: false,
            }
        );

        let herdr = active_environment(&env_with(&[("HERDR_ENV", "1")]));
        assert!(herdr.herdr);
        assert!(!herdr.herdr_popup);

        for marker in [
            "HERDR_WORKSPACE_ID",
            "HERDR_TAB_ID",
            "HERDR_PANE_ID",
            "HERDR_SOCKET_PATH",
        ] {
            assert!(active_environment(&env_with(&[(marker, "present")])).herdr);
        }

        let popup = active_environment(&env_with(&[("HERDR_ACTIVE_PANE_ID", "4")]));
        assert!(popup.herdr);
        assert!(popup.herdr_popup);
    }

    #[test]
    fn detects_zellij_from_any_of_its_pane_markers() {
        // zellij sets ZELLIJ to the literal "0", which is not truthy.
        let zellij = active_environment(&env_with(&[("ZELLIJ", "0")]));
        assert_eq!(
            zellij,
            ActiveEnvironment {
                tmux: false,
                herdr: false,
                herdr_popup: false,
                zellij: true,
            }
        );
        assert_eq!(zellij.backends(), vec![Backend::Zellij]);
        assert!(!zellij.is_ambiguous());

        for marker in ["ZELLIJ_SESSION_NAME", "ZELLIJ_PANE_ID"] {
            assert!(active_environment(&env_with(&[(marker, "present")])).zellij);
        }
        assert!(!active_environment(&env_with(&[("ZELLIJ", "")])).zellij);
    }

    #[test]
    fn a_single_active_backend_wins_and_three_nested_ones_are_ambiguous() {
        let zellij = env_with(&[("ZELLIJ", "0")]);
        assert_eq!(resolve_backend(None, &zellij).unwrap(), Backend::Zellij);

        let nested = env_with(&[("TMUX", "/tmp/tmux"), ("ZELLIJ", "0"), ("HERDR_ENV", "1")]);
        assert!(active_environment(&nested).is_ambiguous());
        let error = resolve_backend(None, &nested).unwrap_err().to_string();
        assert!(error.contains("ambiguous"), "{error}");
        assert!(error.contains("--backend"), "{error}");
        for backend in Backend::ALL {
            assert!(error.contains(backend.display_name()), "{error}");
        }

        let classify_zellij = |_env: &Env| Ok(Some(Backend::Zellij));
        assert_eq!(
            resolve_backend_with_classifier(None, &nested, Some(&classify_zellij)).unwrap(),
            Backend::Zellij
        );
    }

    #[test]
    fn herdr_configuration_variables_alone_do_not_mean_inside_herdr() {
        let active = active_environment(&env_with(&[
            ("HERDR_CONFIG_PATH", "/tmp/herdr.toml"),
            ("HERDR_LOG", "debug"),
            ("HERDR_DISABLE_SOUND", "1"),
        ]));
        assert_eq!(active, ActiveEnvironment::default());
    }

    #[test]
    fn explicit_environment_config_and_fallback_precedence() {
        let temp = TempDir::new().unwrap();
        let mut env = env_at(&temp);
        set_default_backend(&env, Backend::Herdr).unwrap();

        assert_eq!(
            resolve_backend(Some(Backend::Tmux), &env).unwrap(),
            Backend::Tmux
        );

        env.tmux = Some("/tmp/tmux".to_string());
        env.all.insert("TMUX".to_string(), "/tmp/tmux".to_string());
        assert_eq!(resolve_backend(None, &env).unwrap(), Backend::Tmux);

        env.tmux = None;
        env.all.clear();
        assert_eq!(resolve_backend(None, &env).unwrap(), Backend::Herdr);

        fs::remove_file(path(&env).unwrap()).unwrap();
        assert_eq!(resolve_backend(None, &env).unwrap(), Backend::Tmux);
    }

    #[test]
    fn nested_multiplexers_require_or_use_a_classifier() {
        let env = env_with(&[("TMUX", "/tmp/tmux"), ("HERDR_ENV", "1")]);
        let error = resolve_backend(None, &env).unwrap_err().to_string();
        assert!(error.contains("ambiguous"));
        assert!(error.contains("--backend"));

        let classify_herdr = |_env: &Env| Ok(Some(Backend::Herdr));
        assert_eq!(
            resolve_backend_with_classifier(None, &env, Some(&classify_herdr)).unwrap(),
            Backend::Herdr
        );

        let cannot_classify = |_env: &Env| Ok(None);
        assert!(
            resolve_backend_with_classifier(None, &env, Some(&cannot_classify))
                .unwrap_err()
                .to_string()
                .contains("ambiguous")
        );
    }

    #[test]
    fn herdr_popup_beats_inherited_tmux_without_calling_classifier() {
        let env = env_with(&[("TMUX", "/tmp/tmux"), ("HERDR_ACTIVE_PANE_ID", "pane-4")]);
        let must_not_run = |_env: &Env| -> Result<Option<Backend>> {
            panic!("popup classification should not consult foreground processes")
        };

        assert_eq!(
            resolve_backend_with_classifier(None, &env, Some(&must_not_run)).unwrap(),
            Backend::Herdr
        );
        assert_eq!(resolve_backend(None, &env).unwrap(), Backend::Herdr);
    }
}
