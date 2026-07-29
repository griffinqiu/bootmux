use std::io::IsTerminal;
use std::path::Path;

// Lexical port of Ruby's File.expand_path: tilde expansion, absolutize
// against a base directory, and `.`/`..` normalization without touching
// the filesystem.
pub fn expand_path(path: &str, base: &str, home: &str) -> String {
    let joined = if path == "~" {
        home.to_string()
    } else if let Some(rest) = path.strip_prefix("~/") {
        format!("{}/{}", home.trim_end_matches('/'), rest)
    } else if path.starts_with('/') {
        path.to_string()
    } else {
        format!("{}/{}", base.trim_end_matches('/'), path)
    };

    normalize_lexically(&joined)
}

fn normalize_lexically(path: &str) -> String {
    let mut parts: Vec<&str> = Vec::new();
    for part in path.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            other => parts.push(other),
        }
    }
    format!("/{}", parts.join("/"))
}

pub fn blank(s: Option<&str>) -> bool {
    s.map(|v| v.is_empty()).unwrap_or(true)
}

pub fn exit_with_message(msg: &str) -> ! {
    println!("{msg}");
    std::process::exit(1);
}

pub enum Color {
    Red,
    Green,
    Yellow,
}

pub fn say_colored(msg: &str, color: Color) {
    if std::io::stdout().is_terminal() {
        let code = match color {
            Color::Red => "31",
            Color::Green => "32",
            Color::Yellow => "33",
        };
        println!("\x1b[{code}m{msg}\x1b[0m");
    } else {
        println!("{msg}");
    }
}

pub fn yes_no(condition: bool) {
    if condition {
        say_colored("Yes", Color::Green);
    } else {
        say_colored("No", Color::Red);
    }
}

pub fn ask_yes(question: &str) -> bool {
    use std::io::Write;
    print!("{question} ");
    std::io::stdout().flush().ok();
    let mut answer = String::new();
    if std::io::stdin().read_line(&mut answer).is_err() {
        return false;
    }
    let answer = answer.trim().to_lowercase();
    answer == "y" || answer == "yes"
}

pub fn press_enter_to_continue() {
    use std::io::Read;
    println!();
    print!("Press ENTER to continue.");
    use std::io::Write;
    std::io::stdout().flush().ok();
    let mut buf = [0u8; 1];
    std::io::stdin().read_exact(&mut buf).ok();
}

pub fn file_stem_matches(path: &Path, name: &str) -> bool {
    path.file_stem().map(|stem| stem == name).unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expands_tilde_and_relative_paths() {
        assert_eq!(expand_path("~", "/cwd", "/home/u"), "/home/u");
        assert_eq!(expand_path("~/test", "/cwd", "/home/u"), "/home/u/test");
        assert_eq!(expand_path("/abs/path", "/cwd", "/home/u"), "/abs/path");
        assert_eq!(expand_path("rel", "/cwd", "/home/u"), "/cwd/rel");
        assert_eq!(
            expand_path(".", "/workspace/basic", "/h"),
            "/workspace/basic"
        );
        assert_eq!(
            expand_path("app", "/workspace/basic", "/h"),
            "/workspace/basic/app"
        );
        assert_eq!(expand_path("../x", "/a/b", "/h"), "/a/x");
        assert_eq!(expand_path("a/./b//c", "/", "/h"), "/a/b/c");
    }
}
