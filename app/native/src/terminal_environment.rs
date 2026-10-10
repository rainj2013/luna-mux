//! Color capabilities belong to terminal runtimes, not the desktop parent process.
pub(crate) const COLOR_ENVIRONMENT: [(&str, &str); 4] = [
    ("COLORTERM", "truecolor"),
    ("FORCE_COLOR", "3"),
    ("CLICOLOR", "1"),
    ("CLICOLOR_FORCE", "1"),
];

// WSL's shell configuration is still selected by the distribution/user, not Luna.
pub(crate) const WSL_COLOR_SHELL: &str = "unset NO_COLOR; export TERM=xterm-256color COLORTERM=truecolor FORCE_COLOR=3 CLICOLOR=1 CLICOLOR_FORCE=1; exec \"${SHELL:-/bin/sh}\" -l";

#[cfg(test)]
mod tests {
    #[test]
    fn wsl_color_settings_match_local_terminals() {
        for (name, value) in super::COLOR_ENVIRONMENT {
            assert!(super::WSL_COLOR_SHELL.contains(&format!("{name}={value}")));
        }
        assert!(super::WSL_COLOR_SHELL.starts_with("unset NO_COLOR;"));
    }
}
