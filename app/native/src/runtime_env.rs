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
        ("LUNA_MUX_HOOK_ENDPOINT", environment.hook_endpoint.as_deref()),
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
            contents.push_str(&format!(
                "$env:{name} = {}\n",
                powershell_quote(value)
            ));
        }
    }
    if let Some(value) = environment.browser_cdp_port {
        contents.push_str(&format!("$env:LUNA_MUX_BROWSER_CDP_PORT = '{value}'\n"));
    }
    contents
}

pub(crate) fn cleanup_stale_runtime_dirs() {
    let root = std::env::temp_dir().join("luna-mux");
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
        let _ = fs::remove_dir_all(path);
    }
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
        assert!(contents.contains("export LUNA_MUX_HOOK_ENDPOINT='http://127.0.0.1:43127/v1/hooks'"));
        assert!(contents.contains("export LUNA_MUX_HOOK_AUTHORIZATION='lmxh_hook-secret'"));
        assert!(contents.contains("export LUNA_MUX_MCP_ENDPOINT='http://127.0.0.1:43128/mcp'"));
        assert!(contents.contains("export LUNA_MUX_MCP_AUTHORIZATION='lmx_control-secret'"));
        assert!(contents.contains("export LUNA_MUX_BROWSER_BRIDGE_CREDENTIALS='/tmp/browser-bridge.json'"));
        assert!(contents.contains("export LUNA_MUX_BROWSER_CDP_PORT=43129"));
    }

    #[test]
    fn runtime_directory_names_cannot_escape_temp_root() {
        assert!(is_valid_runtime_id("0198af43-f96e-7161-87a1-cf2f1c181294"));
        assert!(!is_valid_runtime_id("../runtime-1"));
        assert!(!is_valid_runtime_id("runtime/1"));
        assert!(!is_valid_runtime_id(""));
    }
}
