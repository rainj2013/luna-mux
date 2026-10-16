/// Chrome accepts one proxy endpoint here. Keep credentials and arbitrary
/// Chromium proxy rules out of persisted resource metadata and process arguments.
pub fn normalize_proxy_server(value: &str) -> Result<String, String> {
    let value = value.trim();
    if value.is_empty() {
        return Ok(String::new());
    }
    let invalid = || {
        "代理地址必须是 http、https、socks4 或 socks5 的主机和端口，且不能包含账号密码、路径或参数"
            .to_string()
    };
    if value.len() > 2048
        || value
            .chars()
            .any(|ch| ch.is_whitespace() || ch.is_control())
    {
        return Err(invalid());
    }
    let url = url::Url::parse(value).map_err(|_| invalid())?;
    let valid_host = match url.host() {
        Some(url::Host::Domain(host)) => {
            !host.is_empty()
                && host
                    .bytes()
                    .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, b'.' | b'-' | b'_'))
        }
        Some(url::Host::Ipv4(_) | url::Host::Ipv6(_)) => true,
        None => false,
    };
    if !valid_host
        || !matches!(url.scheme(), "http" | "https" | "socks4" | "socks5")
        || url.host_str().is_none_or(str::is_empty)
        || url.port_or_known_default().is_none_or(|port| port == 0)
        || !url.username().is_empty()
        || url.password().is_some()
        || !matches!(url.path(), "" | "/")
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(invalid());
    }
    Ok(url.as_str().trim_end_matches('/').to_string())
}

pub fn proxy_launch_arg(value: &str) -> Result<Option<String>, String> {
    let proxy = normalize_proxy_server(value)?;
    Ok((!proxy.is_empty()).then(|| format!("--proxy-server={proxy}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn proxy_endpoint_becomes_one_chrome_argument() {
        assert_eq!(proxy_launch_arg("  ").unwrap(), None);
        for endpoint in [
            "http://127.0.0.1:7890",
            "https://proxy.example:8443",
            "socks4://localhost:1080",
            "socks5://[::1]:1080",
        ] {
            assert_eq!(
                proxy_launch_arg(endpoint).unwrap(),
                Some(format!("--proxy-server={endpoint}"))
            );
        }
        assert_eq!(
            normalize_proxy_server(" http://localhost:7890/ ").unwrap(),
            "http://localhost:7890"
        );
    }

    #[test]
    fn rejects_credentials_and_non_endpoint_input() {
        for endpoint in [
            "socks5://localhost;http=evil:1080",
            "socks5://localhost,evil:1080",
            "localhost:7890",
            "ftp://localhost:7890",
            "http://user:secret@localhost:7890",
            "http://localhost:7890/path",
            "http://localhost:7890?x=y",
            "http://localhost:7890#x",
            "socks5://localhost",
            "http://localhost:0",
            "http://localhost:7890 --no-sandbox",
        ] {
            let error = proxy_launch_arg(endpoint).unwrap_err();
            assert!(!error.contains(endpoint));
        }
    }
}
