// Parity port of Ruby's Shellwords: escape uses backslash style (`a\ b`),
// not single-quote style, because generated scripts are byte-compared
// against tmuxinator's golden snapshots.

pub fn escape(s: &str) -> String {
    if s.is_empty() {
        return "''".to_string();
    }

    let mut escaped = String::with_capacity(s.len() * 2);
    for ch in s.chars() {
        match ch {
            'A'..='Z' | 'a'..='z' | '0'..='9' | '_' | '-' | '.' | ',' | ':' | '+' | '/' | '@' => {
                escaped.push(ch);
            }
            '\n' => escaped.push_str("'\n'"),
            _ => {
                escaped.push('\\');
                escaped.push(ch);
            }
        }
    }
    escaped
}

pub fn split(s: &str) -> Vec<String> {
    shell_words::split(s).unwrap_or_default()
}

pub fn unescape(s: &str) -> String {
    split(s).join("")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escapes_like_ruby_shellwords() {
        assert_eq!(escape(""), "''");
        assert_eq!(escape("simple"), "simple");
        assert_eq!(escape("bundle exec vim"), "bundle\\ exec\\ vim");
        assert_eq!(escape("it's"), "it\\'s");
        assert_eq!(escape("a$b"), "a\\$b");
        assert_eq!(escape("path/to.file:x,y+z@h_-"), "path/to.file:x,y+z@h_-");
        assert_eq!(escape("🍩"), "\\🍩");
        assert_eq!(escape("a\nb"), "a'\n'b");
        assert_eq!(escape("echo \"hi\""), "echo\\ \\\"hi\\\"");
    }

    #[test]
    fn unescape_reverses_escape() {
        for original in ["bundle exec vim", "🍩", "it's", "home_arpa_lab"] {
            assert_eq!(unescape(&escape(original)), original);
        }
    }
}
