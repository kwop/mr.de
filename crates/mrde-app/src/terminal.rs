//! Native Ghostty surfaces for interactive Claude sessions in Threads.
mrde_ghostty_adapter::bind_gpui!(gpui);

/// Plan mode keeps questions read-only: Claude explores and answers without editing.
const READ_ONLY: &str = "--permission-mode plan";

pub fn resume_command(executable: &str, session: &str, read_only: bool) -> String {
    let mut command = format!("{} --resume {}", quote(executable), quote(session));
    if read_only {
        command = format!("{command} {READ_ONLY}");
    }
    command
}

/// Starts a questions session with a known ID so later openings can resume it.
pub fn ask_command(
    executable: &str,
    session: &str,
    context_dir: &std::path::Path,
    prompt: &str,
) -> String {
    format!(
        "{} --session-id {} {READ_ONLY} --add-dir {} {}",
        quote(executable),
        quote(session),
        quote(&context_dir.to_string_lossy()),
        quote(prompt)
    )
}

// Tests record terminal commands without spawning native surfaces.
#[cfg_attr(test, allow(dead_code))]
pub fn configuration() -> mrde_ghostty_adapter::TerminalConfiguration {
    use crate::theme::*;
    use mrde_ghostty_adapter::{TerminalColor, TerminalConfiguration, TerminalTheme};
    let color = |rgb: u32| TerminalColor::new((rgb >> 16) as u8, (rgb >> 8) as u8, rgb as u8);
    TerminalConfiguration::UserDefaultWithOverride(TerminalTheme::new(
        color(PAGE),
        color(INK),
        [
            PAGE, ERROR, SUCCESS, WARNING, INFO, 0xc5a3ff, 0x82d9d0, INK, MUTED, ERROR, SUCCESS,
            WARNING, INFO, 0xc5a3ff, 0x82d9d0, HEADING,
        ]
        .map(color),
    ))
}

fn quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}
pub fn preview_command() -> String {
    format!(
        "/bin/sh -c {}",
        quote(
            "printf 'Embedded Ghostty terminal\\nType a response, then press Enter.\\n> '; read answer; printf '\\nReceived: %s\\n' \"$answer\"; exec /bin/sh"
        )
    )
}

#[cfg(test)]
mod tests {
    #[test]
    fn command_quotes_paths_and_session_arguments() {
        assert_eq!(
            super::resume_command("/a b/cla'ude", "session", false),
            "'/a b/cla'\\''ude' --resume 'session'"
        );
        assert_eq!(
            super::resume_command("claude", "session", true),
            "'claude' --resume 'session' --permission-mode plan"
        );
        assert_eq!(
            super::ask_command(
                "claude",
                "session",
                std::path::Path::new("/tmp/it's"),
                "Read $HOME; then wait"
            ),
            "'claude' --session-id 'session' --permission-mode plan --add-dir '/tmp/it'\\''s' 'Read $HOME; then wait'"
        );
    }
}
