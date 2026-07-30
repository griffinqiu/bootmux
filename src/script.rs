use crate::project::{
    Project, HOOK_ON_PROJECT_EXIT, HOOK_ON_PROJECT_FIRST_START, HOOK_ON_PROJECT_RESTART,
    HOOK_ON_PROJECT_START, HOOK_ON_PROJECT_STOP,
};

// Hand-written port of tmuxinator's template.erb / template-stop.erb.
// Whitespace is part of the contract: golden tests compare the output
// byte-for-byte against tmuxinator's own debug snapshots, so `slot` lines
// keep their indentation even when the value is empty, exactly like an
// ERB `<%= %>` tag whose expression is nil.
struct ScriptWriter {
    output: String,
}

impl ScriptWriter {
    fn new() -> Self {
        ScriptWriter {
            output: String::new(),
        }
    }

    fn line(&mut self, content: &str) {
        self.output.push_str(content);
        self.output.push('\n');
    }

    fn blank(&mut self) {
        self.output.push('\n');
    }

    fn slot(&mut self, indent: &str, content: Option<&str>) {
        self.output.push_str(indent);
        if let Some(content) = content {
            self.output.push_str(content);
        }
        self.output.push('\n');
    }

    fn finish(self) -> String {
        self.output
    }
}

pub fn render_start(project: &Project) -> String {
    let mut w = ScriptWriter::new();
    let tmux = project.tmux();
    let name = project.name().unwrap_or_default();

    w.line(&format!("#!{}", project.env.shell_or_default()));
    w.blank();
    w.blank();
    if !project.opts.append {
        w.line("  # Clear rbenv variables before starting tmux");
        w.line("  unset RBENV_VERSION");
        w.line("  unset RBENV_DIR");
        w.blank();
        w.line(&format!("  {tmux} start-server;"));
    }
    w.blank();
    w.line(&format!("cd {}", project.root().as_deref().unwrap_or(".")));
    w.blank();
    w.line("# Run on_project_start command.");
    w.slot("", project.hook(HOOK_ON_PROJECT_START).as_deref());
    w.blank();

    if project.opts.append || !project.has_session() {
        w.blank();
        // The top-level `pre` option is unsupported in bootmux, but its
        // comment and (empty) slot stay for snapshot parity.
        w.line("  # Run pre command.");
        w.slot("  ", None);
        w.blank();
        w.line("  # Run on_project_first_start command.");
        w.slot("  ", project.hook(HOOK_ON_PROJECT_FIRST_START).as_deref());
        w.blank();
        w.slot("  ", project.new_session_command().as_deref());
        w.blank();
        w.blank();
        if project.enable_pane_titles()
            && project.pane_title_position().is_some()
            && !project.pane_title_position_valid()
        {
            w.slot("  ", Some(&project.pane_title_position_warning()));
        }
        w.blank();
        w.line("  # Create windows.");
        for window in project.windows() {
            w.slot("  ", Some(&window.new_window_command(project)));
        }
        w.blank();

        for window in project.windows() {
            w.blank();
            w.line(&format!(
                "  # Window \"{}\"",
                window.name.as_deref().unwrap_or_default()
            ));
            if window.synchronize_before() {
                w.slot("  ", Some(&window.synchronize_command(project)));
            }
            w.blank();
            if project.enable_pane_titles() {
                w.slot(
                    "  ",
                    Some(&project.set_pane_title_position_command(&window.target(project))),
                );
                w.slot(
                    "  ",
                    Some(&project.set_pane_title_format_command(&window.target(project))),
                );
            }
            w.blank();
            if !window.has_panes() {
                if project.pre_window().is_some() {
                    w.slot("  ", window.pre_window_command(project).as_deref());
                }
                for command in window.commands(project) {
                    w.slot("  ", Some(&command));
                }
            } else {
                for pane in &window.panes {
                    w.slot("  ", pane.set_title_command(window, project).as_deref());
                    if project.pre_window().is_some() {
                        w.slot("  ", pane.pre_window_command(window, project).as_deref());
                    }
                    if window.pre().is_some() {
                        w.slot("  ", pane.pre_command(window, project).as_deref());
                    }
                    for command in &pane.commands {
                        w.slot(
                            "  ",
                            Some(&pane.main_command(command.as_deref(), window, project)),
                        );
                    }
                    w.blank();
                    if !pane.is_last(window) {
                        w.slot("  ", Some(&pane.split_command(window, project)));
                    }
                    if !window.is_pane_chain() {
                        w.slot("  ", Some(&window.tiled_layout_command(project)));
                    }
                }
                w.blank();
                if !window.is_pane_chain() {
                    w.slot("  ", Some(&window.layout_command(project)));
                }
                w.slot("  ", Some(&window.focus_pane_command(project)));
            }
            w.blank();
            if window.synchronize_after() {
                w.slot("  ", Some(&window.synchronize_command(project)));
            }
        }
        w.blank();
        w.line(&format!(
            "  {tmux} select-window -t {}",
            project.startup_window()
        ));
        w.line(&format!("  {}", project.startup_pane_command()));
    } else {
        w.line("  # Run on_project_restart command.");
        w.slot("  ", project.hook(HOOK_ON_PROJECT_RESTART).as_deref());
    }
    w.blank();

    if project.attach() && !project.opts.append {
        w.line("  if [ -z \"$TMUX\" ]; then");
        w.line(&format!("    {tmux} -u attach-session -t {name}"));
        w.line("  else");
        w.line(&format!("    {tmux} -u switch-client -t {name}"));
        w.line("  fi");
    }
    w.blank();

    // Slot for the unsupported top-level `post` option, kept for parity.
    w.slot("", None);
    w.blank();
    w.line("# Run on_project_exit command.");
    w.slot("", project.hook(HOOK_ON_PROJECT_EXIT).as_deref());

    w.finish()
}

pub fn render_stop(project: &Project) -> String {
    let mut w = ScriptWriter::new();

    w.line(&format!("#!{}", project.env.shell_or_default()));
    w.blank();
    if project.has_session() {
        w.line(&format!(
            "  cd {}",
            project.root().as_deref().unwrap_or(".")
        ));
        w.blank();
        w.line("  # Run on_project_stop command");
        w.slot("  ", project.hook(HOOK_ON_PROJECT_STOP).as_deref());
        w.blank();
        w.line(&format!("  {}", project.kill_session_command()));
    }

    w.finish()
}
