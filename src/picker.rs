use std::io::{self, Write};
use std::process::{Command, Stdio};

use anyhow::{bail, Context, Result};

pub const FZF_PROGRAM: &str = "fzf";

/// Runs the interactive project picker.
///
/// `fzf` is intentionally an optional runtime dependency: bootmux features
/// that do not use the picker work without it.
pub fn pick_project(project_names: &[String]) -> Result<Option<String>> {
    let mut command = Command::new(FZF_PROGRAM);
    pick_project_with_command(project_names, &mut command)
}

/// Command-injectable picker entry point for tests and custom fzf wrappers.
///
/// The supplied command's stdin and stdout are reserved for the picker.
pub fn pick_project_with_command(
    project_names: &[String],
    command: &mut Command,
) -> Result<Option<String>> {
    if let Some(name) = project_names
        .iter()
        .find(|name| name.contains('\n') || name.contains('\r'))
    {
        bail!("project name {name:?} contains a newline and cannot be sent to fzf");
    }

    let program = command.get_program().to_string_lossy().into_owned();
    command.stdin(Stdio::piped()).stdout(Stdio::piped());
    let mut child = match command.spawn() {
        Ok(child) => child,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            bail!(
                "optional dependency {program:?} was not found; install fzf to use `bootmux picker`"
            );
        }
        Err(error) => {
            return Err(error)
                .with_context(|| format!("failed to start project picker {program:?}"));
        }
    };

    let write_result = (|| -> io::Result<()> {
        let mut stdin = child
            .stdin
            .take()
            .ok_or_else(|| io::Error::new(io::ErrorKind::BrokenPipe, "picker stdin unavailable"))?;
        for project_name in project_names {
            stdin.write_all(project_name.as_bytes())?;
            stdin.write_all(b"\n")?;
        }
        Ok(())
    })();

    // Always reap the child, including when it closes stdin during
    // cancellation and causes the producer above to see BrokenPipe.
    let output = child
        .wait_with_output()
        .with_context(|| format!("failed while waiting for project picker {program:?}"))?;

    match output.status.code() {
        Some(1 | 130) => return Ok(None),
        _ if output.status.success() => {}
        Some(code) => bail!("project picker {program:?} exited with status {code}"),
        None => bail!("project picker {program:?} was terminated by a signal"),
    }

    write_result.with_context(|| format!("failed to send project names to {program:?}"))?;

    let selected = String::from_utf8(output.stdout)
        .with_context(|| format!("project picker {program:?} returned non-UTF-8 output"))?;
    let selected = selected.trim_end_matches(['\r', '\n']);
    if selected.is_empty() {
        Ok(None)
    } else {
        Ok(Some(selected.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::{Path, PathBuf};

    use tempfile::TempDir;

    use super::*;

    #[cfg(unix)]
    fn script(temp: &TempDir, name: &str, contents: &str) -> PathBuf {
        use std::os::unix::fs::PermissionsExt;

        let path = temp.path().join(name);
        fs::write(&path, contents).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
        path
    }

    #[cfg(unix)]
    #[test]
    fn sends_every_project_to_stdin_and_returns_the_selection() {
        let temp = TempDir::new().unwrap();
        let capture = temp.path().join("projects.txt");
        let executable = script(
            &temp,
            "fake-fzf",
            "#!/bin/sh\ncat > \"$CAPTURE_FILE\"\nprintf 'nested/project\\n'\n",
        );
        let mut command = Command::new(executable);
        command.env("CAPTURE_FILE", &capture);

        let projects = vec![
            "alpha".to_string(),
            "nested/project".to_string(),
            "zeta".to_string(),
        ];
        assert_eq!(
            pick_project_with_command(&projects, &mut command).unwrap(),
            Some("nested/project".to_string())
        );
        assert_eq!(
            fs::read_to_string(capture).unwrap(),
            "alpha\nnested/project\nzeta\n"
        );
    }

    #[cfg(unix)]
    #[test]
    fn exit_one_and_exit_130_are_normal_cancellation() {
        let temp = TempDir::new().unwrap();
        for status in [1, 130] {
            let executable = script(
                &temp,
                &format!("cancel-{status}"),
                &format!("#!/bin/sh\ncat >/dev/null\nexit {status}\n"),
            );
            let mut command = Command::new(executable);
            assert_eq!(
                pick_project_with_command(&["alpha".to_string()], &mut command).unwrap(),
                None
            );
        }
    }

    #[test]
    fn missing_fzf_has_an_optional_dependency_error() {
        let missing = Path::new("/definitely/not/a/real/bootmux-fzf");
        let mut command = Command::new(missing);
        let error = pick_project_with_command(&[], &mut command)
            .unwrap_err()
            .to_string();
        assert!(error.contains("optional dependency"));
        assert!(error.contains("install fzf"));
        assert!(error.contains("bootmux picker"));
    }

    #[cfg(unix)]
    #[test]
    fn unexpected_exit_status_is_an_error() {
        let temp = TempDir::new().unwrap();
        let executable = script(&temp, "broken-fzf", "#!/bin/sh\ncat >/dev/null\nexit 2\n");
        let mut command = Command::new(executable);
        let error = pick_project_with_command(&["alpha".to_string()], &mut command)
            .unwrap_err()
            .to_string();
        assert!(error.contains("status 2"));
    }

    #[test]
    fn rejects_project_names_that_cannot_be_line_delimited() {
        let mut command = Command::new(FZF_PROGRAM);
        let error = pick_project_with_command(&["bad\nname".to_string()], &mut command)
            .unwrap_err()
            .to_string();
        assert!(error.contains("newline"));
    }
}
