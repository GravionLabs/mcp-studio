//! GUI apps on macOS and Linux start with a minimal `PATH`, so `npx`, `uvx`, or `node` from the
//! user's shell setup are not found. We ask the login shell once and reuse its `PATH`.

use std::{
    process::{Command, Stdio},
    sync::{mpsc, OnceLock},
    time::Duration,
};

const START: &str = "__MCP_STUDIO_PATH_START__";
const END: &str = "__MCP_STUDIO_PATH_END__";

static LOGIN_PATH: OnceLock<Option<String>> = OnceLock::new();

/// `PATH` as seen by the user's login shell; `None` on Windows or when the shell does not answer.
pub fn login_shell_path() -> Option<&'static str> {
    LOGIN_PATH
        .get_or_init(|| query_login_shell(&default_shell(), Duration::from_secs(3)))
        .as_deref()
}

fn default_shell() -> String {
    std::env::var("SHELL")
        .ok()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "/bin/sh".to_owned())
}

/// Runs `<shell> -l -c 'printf ... $PATH ...'` with a timeout. Markers protect against banners and
/// other noise printed by shell startup files.
pub fn query_login_shell(shell: &str, timeout: Duration) -> Option<String> {
    if cfg!(windows) {
        return None;
    }
    let script = format!("printf '%s' '{START}'; printf '%s' \"$PATH\"; printf '%s' '{END}'");
    let (tx, rx) = mpsc::channel();
    let shell = shell.to_owned();
    std::thread::spawn(move || {
        let output = Command::new(shell)
            .args(["-l", "-c", &script])
            .stdin(Stdio::null())
            .stderr(Stdio::null())
            .output();
        let _ = tx.send(output);
    });
    let output = rx.recv_timeout(timeout).ok()?.ok()?;
    parse_marked(&String::from_utf8_lossy(&output.stdout))
}

fn parse_marked(text: &str) -> Option<String> {
    let start = text.find(START)? + START.len();
    let end = text[start..].find(END)? + start;
    let path = text[start..end].trim();
    (!path.is_empty()).then(|| path.to_owned())
}

/// Combines the login-shell `PATH` with the current one, keeping order and dropping duplicates.
pub fn merge_paths(login: &str, current: &str) -> String {
    let separator = if cfg!(windows) { ';' } else { ':' };
    let mut seen = std::collections::HashSet::new();
    login
        .split(separator)
        .chain(current.split(separator))
        .filter(|part| !part.is_empty() && seen.insert(*part))
        .collect::<Vec<_>>()
        .join(&separator.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_output_between_markers_ignoring_noise() {
        let text = format!("welcome banner\n{START}/usr/bin:/opt/bin{END}trailing");
        assert_eq!(parse_marked(&text).as_deref(), Some("/usr/bin:/opt/bin"));
    }

    #[test]
    fn rejects_missing_markers_and_empty_paths() {
        assert_eq!(parse_marked("/usr/bin"), None);
        assert_eq!(parse_marked(&format!("{START}{END}")), None);
    }

    #[cfg(unix)]
    #[test]
    fn asks_a_real_shell() {
        let path = query_login_shell("/bin/sh", Duration::from_secs(5)).expect("sh answers");
        assert!(path.contains("/bin"), "{path}");
    }

    #[cfg(unix)]
    #[test]
    fn missing_shell_or_timeout_yields_none() {
        assert_eq!(
            query_login_shell("/definitely/not/a/shell", Duration::from_secs(1)),
            None
        );
    }

    #[cfg(unix)]
    #[test]
    fn merge_keeps_login_entries_first_without_duplicates() {
        assert_eq!(merge_paths("/a:/b", "/b:/c"), "/a:/b:/c");
        assert_eq!(merge_paths("", "/x"), "/x");
    }
}
