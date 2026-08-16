pub(crate) fn posix_shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\"'\"'"))
}

pub(crate) fn shell_quote(value: &str) -> String {
    posix_shell_quote(value)
}

pub(crate) fn powershell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}

pub(crate) fn shell_argument_quote(value: &str, target_id: &str) -> String {
    if crate::local_pty_backend::is_powershell_target(target_id) {
        powershell_quote(value)
    } else {
        posix_shell_quote(value)
    }
}

pub(crate) fn executable_command_quote(value: &str) -> String {
    format!("\"{}\"", value.replace('"', "\\\""))
}
