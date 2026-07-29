use crate::project::Project;
use crate::window::Window;

pub struct Pane {
    pub index: usize,
    // Shell-escaped at construction, like Ruby's Pane#initialize; the
    // escaped form is also what focused_pane title matching compares.
    pub title: Option<String>,
    pub commands: Vec<Option<String>>,
}

impl Pane {
    pub fn target(&self, window: &Window, project: &Project) -> String {
        format!(
            "{}:{}.{}",
            project.name().unwrap_or_default(),
            window.index as i64 + project.base_index(),
            self.index as i64 + project.pane_base_index()
        )
    }

    pub fn set_title_command(&self, window: &Window, project: &Project) -> Option<String> {
        self.title.as_ref().map(|title| {
            format!(
                "{} select-pane -t {} -T {}",
                project.tmux(),
                self.target(window, project),
                title
            )
        })
    }

    pub fn pre_window_command(&self, window: &Window, project: &Project) -> Option<String> {
        project.pre_window().map(|pre_window| {
            self.send_keys(window, project, &crate::shellwords::escape(&pre_window))
        })
    }

    pub fn pre_command(&self, window: &Window, project: &Project) -> Option<String> {
        window
            .pre()
            .map(|pre| self.send_keys(window, project, &crate::shellwords::escape(&pre)))
    }

    // A nil command renders as an empty (indentation-only) script line.
    pub fn main_command(
        &self,
        command: Option<&str>,
        window: &Window,
        project: &Project,
    ) -> String {
        match command {
            Some(cmd) => self.send_keys(window, project, &crate::shellwords::escape(cmd)),
            None => String::new(),
        }
    }

    pub fn split_command(&self, window: &Window, project: &Project) -> String {
        let path = window
            .root(project)
            .map(|root| format!("-c {root}"))
            .unwrap_or_default();
        format!(
            "{} splitw {} -t {}",
            project.tmux(),
            path,
            window.target(project)
        )
    }

    pub fn is_last(&self, window: &Window) -> bool {
        self.index == window.panes.len() - 1
    }

    fn send_keys(&self, window: &Window, project: &Project, keys: &str) -> String {
        format!(
            "{} send-keys -t {} {} C-m",
            project.tmux(),
            self.target(window, project),
            keys
        )
    }
}
