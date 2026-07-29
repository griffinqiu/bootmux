use std::collections::HashMap;
use std::path::PathBuf;

#[derive(Clone, Debug, Default)]
pub struct Env {
    pub shell: Option<String>,
    pub editor: Option<String>,
    pub home: String,
    pub xdg_config_home: Option<String>,
    pub tmuxinator_config: Option<String>,
    pub tmux: Option<String>,
    pub cwd: PathBuf,
    pub all: HashMap<String, String>,
}

impl Env {
    pub fn from_process() -> Self {
        let all: HashMap<String, String> = std::env::vars().collect();
        Env {
            shell: all.get("SHELL").cloned(),
            editor: all.get("EDITOR").cloned(),
            home: all.get("HOME").cloned().unwrap_or_default(),
            xdg_config_home: all.get("XDG_CONFIG_HOME").cloned(),
            tmuxinator_config: all.get("TMUXINATOR_CONFIG").cloned(),
            tmux: all.get("TMUX").cloned(),
            cwd: std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
            all,
        }
    }

    pub fn shell_or_default(&self) -> &str {
        self.shell
            .as_deref()
            .filter(|s| !s.is_empty())
            .unwrap_or("/bin/bash")
    }

    pub fn inside_tmux(&self) -> bool {
        self.tmux.is_some()
    }
}
