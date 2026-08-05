use std::{
    collections::{BTreeSet, HashMap},
    env,
    path::{Path, PathBuf},
};

use crate::models::SshConfigImportEntry;

#[derive(Default)]
struct HostBlock {
    patterns: Vec<String>,
    options: HashMap<String, Vec<String>>,
}

fn expand_home(value: &str, home: &Path) -> String {
    let home = home.to_string_lossy();
    if value == "~" {
        return home.into_owned();
    }
    let Some(tail) = value
        .strip_prefix("~/")
        .or_else(|| value.strip_prefix("~\\"))
    else {
        return value.to_string();
    };
    let separator = if home.contains('\\') && !home.contains('/') {
        '\\'
    } else {
        '/'
    };
    let base = home.trim_end_matches(['/', '\\']);
    let tail = if separator == '\\' {
        tail.replace('/', "\\")
    } else {
        tail.replace('\\', "/")
    };
    if base.is_empty() {
        format!("{separator}{tail}")
    } else {
        format!("{base}{separator}{tail}")
    }
}

fn proxy_alias(value: &str) -> String {
    let hop = value.split(',').next().unwrap_or("").trim();
    if hop.is_empty() || hop.eq_ignore_ascii_case("none") {
        return String::new();
    }
    let host = hop.rsplit_once('@').map(|(_, host)| host).unwrap_or(hop);
    host.rsplit_once(':')
        .filter(|(_, port)| port.parse::<u16>().is_ok())
        .map(|(host, _)| host)
        .unwrap_or(host)
        .to_string()
}

fn matches(pattern: &str, alias: &str) -> bool {
    if pattern.contains(['[', ']']) {
        return false;
    }
    let pattern = pattern.to_ascii_lowercase().into_bytes();
    let value = alias.to_ascii_lowercase().into_bytes();
    let (mut p, mut v, mut star, mut matched) = (0usize, 0usize, None, 0usize);
    while v < value.len() {
        if p < pattern.len() && (pattern[p] == b'?' || pattern[p] == value[v]) {
            p += 1;
            v += 1;
        } else if p < pattern.len() && pattern[p] == b'*' {
            star = Some(p);
            p += 1;
            matched = v;
        } else if let Some(index) = star {
            p = index + 1;
            matched += 1;
            v = matched;
        } else {
            return false;
        }
    }
    while p < pattern.len() && pattern[p] == b'*' {
        p += 1;
    }
    p == pattern.len()
}

fn block_matches(patterns: &[String], alias: &str) -> bool {
    let mut positive = false;
    for pattern in patterns {
        if let Some(pattern) = pattern.strip_prefix('!') {
            if matches(pattern, alias) {
                return false;
            }
        } else if matches(pattern, alias) {
            positive = true;
        }
    }
    positive
}

pub fn parse(text: &str, home: &Path) -> Vec<SshConfigImportEntry> {
    let mut blocks = vec![HostBlock {
        patterns: vec!["*".into()],
        options: HashMap::new(),
    }];
    for raw in text.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let (key, value) = line
            .split_once(char::is_whitespace)
            .map(|(key, value)| (key, value.trim()))
            .unwrap_or((line, ""));
        if key.eq_ignore_ascii_case("host") {
            blocks.push(HostBlock {
                patterns: value.split_whitespace().map(str::to_string).collect(),
                options: HashMap::new(),
            });
        } else {
            blocks
                .last_mut()
                .unwrap()
                .options
                .entry(key.to_ascii_lowercase())
                .or_default()
                .push(value.trim_matches('"').to_string());
        }
    }
    let aliases: BTreeSet<String> = blocks
        .iter()
        .flat_map(|block| block.patterns.iter())
        .filter(|value| !value.starts_with('!') && !value.contains(['*', '?', '[', ']']))
        .cloned()
        .collect();
    aliases
        .into_iter()
        .filter_map(|alias| {
            let mut values = HashMap::<String, String>::new();
            for block in &blocks {
                if !block_matches(&block.patterns, &alias) {
                    continue;
                }
                for (key, options) in &block.options {
                    if let Some(value) = options.first() {
                        values.entry(key.clone()).or_insert_with(|| value.clone());
                    }
                }
            }
            let host = values
                .get("hostname")
                .cloned()
                .unwrap_or_else(|| alias.clone());
            let username = values
                .get("user")
                .cloned()
                .or_else(|| env::var("USER").ok())
                .or_else(|| env::var("USERNAME").ok())
                .unwrap_or_default();
            let port = values
                .get("port")
                .and_then(|value| value.parse::<u16>().ok())
                .unwrap_or(22);
            if host.is_empty() || username.is_empty() || port == 0 {
                return None;
            }
            let private_key_path = values
                .get("identityfile")
                .map(|value| expand_home(value, home))
                .unwrap_or_default();
            let proxy_jump_alias = values
                .get("proxyjump")
                .map(|value| proxy_alias(value))
                .unwrap_or_default();
            Some(SshConfigImportEntry {
                alias: alias.clone(),
                name: alias,
                host,
                port,
                username,
                private_key_path,
                proxy_jump_alias,
            })
        })
        .collect()
}

pub fn default_path() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_default()
        .join(".ssh")
        .join("config")
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parses_defaults_keys_and_jump_hosts() {
        let entries = parse(
            "Host *\n User deploy\n Port 2200\nHost bastion\n HostName jump.example.com\nHost app\n HostName 10.0.0.5\n IdentityFile ~/.ssh/id_ed25519\n ProxyJump deploy@bastion:22\n",
            Path::new("/home/test"),
        );
        let app = entries.iter().find(|entry| entry.alias == "app").unwrap();
        assert_eq!(app.username, "deploy");
        assert_eq!(app.port, 2200);
        assert_eq!(app.private_key_path, "/home/test/.ssh/id_ed25519");
        assert_eq!(app.proxy_jump_alias, "bastion");
    }
    #[test]
    fn ignores_wildcard_only_hosts() {
        assert!(parse("Host *.example.com\n User test\n", Path::new("/tmp")).is_empty());
    }

    #[test]
    fn expands_home_using_the_home_paths_separator_style() {
        assert_eq!(
            expand_home("~/.ssh/id_ed25519", Path::new("/home/test")),
            "/home/test/.ssh/id_ed25519"
        );
        assert_eq!(
            expand_home("~/.ssh/id_ed25519", Path::new(r"C:\Users\test")),
            r"C:\Users\test\.ssh\id_ed25519"
        );
        assert_eq!(expand_home("~", Path::new("/home/test")), "/home/test");
        assert_eq!(
            expand_home("/opt/keys/id_ed25519", Path::new("/home/test")),
            "/opt/keys/id_ed25519"
        );
    }
}
