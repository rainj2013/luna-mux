use std::{
    fs,
    path::{Path, PathBuf},
};

use crate::shell_quoting::{posix_shell_quote, powershell_quote};

#[derive(Clone, Debug, Default)]
pub(crate) struct RuntimeEnvironment {
    pub hook_endpoint: Option<String>,
    pub hook_authorization: Option<String>,
    pub mcp_endpoint: Option<String>,
    pub mcp_authorization: Option<String>,
    pub browser_bridge_credentials: Option<String>,
    pub browser_cdp_port: Option<u16>,
}

const ENV_FILE_NAME_SH: &str = "agent-env.sh";
const ENV_FILE_NAME_PS: &str = "agent-env.ps1";
const RUNTIME_OWNER_FILE_NAME: &str = ".runtime-owner";
pub(crate) const HOOK_AUTH_DIR_NAME: &str = "hook-auth";

pub(crate) fn hook_auth_dir() -> PathBuf {
    std::env::temp_dir()
        .join("luna-mux")
        .join(HOOK_AUTH_DIR_NAME)
}

pub(crate) fn runtime_temp_dir(runtime_id: &str) -> Result<PathBuf, String> {
    if !is_valid_runtime_id(runtime_id) {
        return Err("Runtime ID 无效，无法写入持久环境文件".into());
    }
    Ok(std::env::temp_dir().join("luna-mux").join(runtime_id))
}

pub(crate) fn sh_env_file(runtime_id: &str) -> Result<PathBuf, String> {
    Ok(runtime_temp_dir(runtime_id)?.join(ENV_FILE_NAME_SH))
}

pub(crate) fn ps_env_file(runtime_id: &str) -> Result<PathBuf, String> {
    Ok(runtime_temp_dir(runtime_id)?.join(ENV_FILE_NAME_PS))
}

pub(crate) fn write_environment_for_target(
    runtime_id: &str,
    target_id: &str,
    environment: &RuntimeEnvironment,
) -> Result<PathBuf, String> {
    write_runtime_owner(runtime_id)?;
    let directory = runtime_temp_dir(runtime_id)?;
    fs::create_dir_all(&directory).map_err(|error| error.to_string())?;
    let path = if crate::local_pty_backend::is_powershell_target(target_id) {
        ps_env_file(runtime_id)?
    } else {
        sh_env_file(runtime_id)?
    };
    let contents = if crate::local_pty_backend::is_powershell_target(target_id) {
        powershell_environment_contents(environment)
    } else {
        posix_environment_contents(environment)
    };
    write_file(&path, contents.as_bytes())?;
    Ok(path)
}

/// Marks a Runtime directory as owned by this Luna Mux desktop process.
///
/// Startup cleanup only removes directories with this marker. This keeps
/// directories created by another still-running Luna Mux instance intact,
/// while allowing crashed instances to be cleaned up on the next start.
pub(crate) fn write_runtime_owner(runtime_id: &str) -> Result<(), String> {
    let directory = runtime_temp_dir(runtime_id)?;
    fs::create_dir_all(&directory).map_err(|error| error.to_string())?;
    write_file(
        &directory.join(RUNTIME_OWNER_FILE_NAME),
        format!("{}\n", std::process::id()).as_bytes(),
    )
}

pub(crate) fn posix_environment_contents(environment: &RuntimeEnvironment) -> String {
    let mut contents = String::new();
    if let Some(value) = environment.hook_endpoint.as_deref() {
        contents.push_str(&format!(
            "export LUNA_MUX_HOOK_ENDPOINT={}\n",
            posix_shell_quote(value)
        ));
    }
    if let Some(value) = environment.hook_authorization.as_deref() {
        contents.push_str(&format!(
            "export LUNA_MUX_HOOK_AUTHORIZATION={}\n",
            posix_shell_quote(value)
        ));
    }
    if let Some(value) = environment.mcp_endpoint.as_deref() {
        contents.push_str(&format!(
            "export LUNA_MUX_MCP_ENDPOINT={}\n",
            posix_shell_quote(value)
        ));
    }
    if let Some(value) = environment.mcp_authorization.as_deref() {
        contents.push_str(&format!(
            "export LUNA_MUX_MCP_AUTHORIZATION={}\n",
            posix_shell_quote(value)
        ));
    }
    if let Some(value) = environment.browser_bridge_credentials.as_deref() {
        contents.push_str(&format!(
            "export LUNA_MUX_BROWSER_BRIDGE_CREDENTIALS={}\n",
            posix_shell_quote(value)
        ));
    }
    if let Some(value) = environment.browser_cdp_port {
        contents.push_str(&format!("export LUNA_MUX_BROWSER_CDP_PORT={value}\n"));
    }
    contents
}

pub(crate) fn powershell_environment_contents(environment: &RuntimeEnvironment) -> String {
    let mut contents = String::new();
    for (name, value) in [
        (
            "LUNA_MUX_HOOK_ENDPOINT",
            environment.hook_endpoint.as_deref(),
        ),
        (
            "LUNA_MUX_HOOK_AUTHORIZATION",
            environment.hook_authorization.as_deref(),
        ),
        ("LUNA_MUX_MCP_ENDPOINT", environment.mcp_endpoint.as_deref()),
        (
            "LUNA_MUX_MCP_AUTHORIZATION",
            environment.mcp_authorization.as_deref(),
        ),
        (
            "LUNA_MUX_BROWSER_BRIDGE_CREDENTIALS",
            environment.browser_bridge_credentials.as_deref(),
        ),
    ] {
        if let Some(value) = value {
            contents.push_str(&format!("$env:{name} = {}\n", powershell_quote(value)));
        }
    }
    if let Some(value) = environment.browser_cdp_port {
        contents.push_str(&format!("$env:LUNA_MUX_BROWSER_CDP_PORT = '{value}'\n"));
    }
    contents
}

pub(crate) fn cleanup_stale_runtime_dirs() {
    let root = std::env::temp_dir().join("luna-mux");
    cleanup_stale_runtime_dirs_at(&root, process_is_alive);
}

fn cleanup_stale_runtime_dirs_at(root: &Path, is_process_alive: impl Fn(u32) -> bool) {
    let Ok(entries) = fs::read_dir(&root) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            continue;
        };
        if name == "agent-browser" || name == HOOK_AUTH_DIR_NAME {
            continue;
        }
        let owner_path = path.join(RUNTIME_OWNER_FILE_NAME);
        let Ok(owner) = fs::read_to_string(owner_path) else {
            // Runtime directories created by older versions have no owner
            // metadata. Preserve them rather than risking a live Runtime.
            continue;
        };
        let Ok(pid) = owner.trim().parse::<u32>() else {
            continue;
        };
        if !is_process_alive(pid) {
            let _ = fs::remove_dir_all(path);
        }
    }
}

#[cfg(windows)]
fn process_is_alive(pid: u32) -> bool {
    use windows::Win32::{
        Foundation::{CloseHandle, ERROR_INVALID_PARAMETER, STILL_ACTIVE},
        System::Threading::{GetExitCodeProcess, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION},
    };
    use windows::core::HRESULT;

    if pid == 0 {
        return false;
    }
    // A process handle can still be opened after a process exits, so inspect
    // its exit code instead of treating OpenProcess success as liveness.
    let handle = match unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) } {
        Ok(handle) => handle,
        Err(error) => {
            // Access denied or another inconclusive error must not make one
            // Luna Mux instance delete files owned by another instance.
            return error.code() != HRESULT::from_win32(ERROR_INVALID_PARAMETER.0);
        }
    };
    let mut exit_code = 1_u32;
    let alive = unsafe { GetExitCodeProcess(handle, &mut exit_code).is_ok() }
        && exit_code == STILL_ACTIVE.0 as u32;
    let _ = unsafe { CloseHandle(handle) };
    alive
}

#[cfg(unix)]
fn process_is_alive(pid: u32) -> bool {
    if pid == 0 {
        return false;
    }
    // The desktop process and the cleanup process run as the same user, so a
    // successful zero-signal probe is sufficient and avoids spawning a shell.
    if unsafe { libc::kill(pid as libc::pid_t, 0) } == 0 {
        return true;
    }
    std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
}

#[cfg(not(any(unix, windows)))]
fn process_is_alive(_pid: u32) -> bool {
    false
}

fn write_file(path: &Path, contents: &[u8]) -> Result<(), String> {
    fs::write(path, contents).map_err(|error| error.to_string())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(path, fs::Permissions::from_mode(0o600));
    }
    Ok(())
}

fn is_valid_runtime_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_root(name: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!("luna-mux-runtime-env-{name}"));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        root
    }

    #[test]
    fn posix_environment_quotes_all_supported_values() {
        let environment = RuntimeEnvironment {
            hook_endpoint: Some("http://127.0.0.1:43127/v1/hooks".into()),
            hook_authorization: Some("lmxh_hook-secret".into()),
            mcp_endpoint: Some("http://127.0.0.1:43128/mcp".into()),
            mcp_authorization: Some("lmx_control-secret".into()),
            browser_bridge_credentials: Some("/tmp/browser-bridge.json".into()),
            browser_cdp_port: Some(43129),
        };
        let contents = posix_environment_contents(&environment);
        assert!(
            contents.contains("export LUNA_MUX_HOOK_ENDPOINT='http://127.0.0.1:43127/v1/hooks'")
        );
        assert!(contents.contains("export LUNA_MUX_HOOK_AUTHORIZATION='lmxh_hook-secret'"));
        assert!(contents.contains("export LUNA_MUX_MCP_ENDPOINT='http://127.0.0.1:43128/mcp'"));
        assert!(contents.contains("export LUNA_MUX_MCP_AUTHORIZATION='lmx_control-secret'"));
        assert!(
            contents
                .contains("export LUNA_MUX_BROWSER_BRIDGE_CREDENTIALS='/tmp/browser-bridge.json'")
        );
        assert!(contents.contains("export LUNA_MUX_BROWSER_CDP_PORT=43129"));
    }

    #[test]
    fn runtime_directory_names_cannot_escape_temp_root() {
        assert!(is_valid_runtime_id("0198af43-f96e-7161-87a1-cf2f1c181294"));
        assert!(!is_valid_runtime_id("../runtime-1"));
        assert!(!is_valid_runtime_id("runtime/1"));
        assert!(!is_valid_runtime_id(""));
    }

    #[test]
    fn stale_cleanup_preserves_live_and_unowned_runtime_directories() {
        let root = test_root("preserve");
        let live = root.join("live");
        fs::create_dir_all(&live).unwrap();
        fs::write(live.join(RUNTIME_OWNER_FILE_NAME), "42\n").unwrap();
        let unowned = root.join("unowned");
        fs::create_dir_all(&unowned).unwrap();
        cleanup_stale_runtime_dirs_at(&root, |pid| pid == 42);
        assert!(live.is_dir());
        assert!(unowned.is_dir());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn stale_cleanup_removes_runtime_owned_by_exited_process() {
        let root = test_root("remove");
        let stale = root.join("stale");
        fs::create_dir_all(&stale).unwrap();
        fs::write(stale.join(RUNTIME_OWNER_FILE_NAME), "42\n").unwrap();
        cleanup_stale_runtime_dirs_at(&root, |_| false);
        assert!(!stale.exists());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn process_probe_recognizes_current_process() {
        assert!(process_is_alive(std::process::id()));
        assert!(!process_is_alive(0));
    }
}
