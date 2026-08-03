use std::time::Duration;

use futures::StreamExt;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use url::Url;

use crate::{database::Database, models::*};

const SETTINGS_KEY: &str = "aiSettings";
const DEFAULT_BASE_URL: &str = "https://api.openai.com/v1";
const MAX_RESPONSE_BYTES: usize = 1024 * 1024;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct StoredAiSettings {
    base_url: String,
    model: String,
    default_shell: AiShell,
}

impl Default for StoredAiSettings {
    fn default() -> Self {
        Self {
            base_url: DEFAULT_BASE_URL.into(),
            model: String::new(),
            default_shell: AiShell::Linux,
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ModelSuggestion {
    command: String,
    #[serde(default)]
    explanation: String,
    #[serde(default)]
    assumptions: Vec<String>,
    #[serde(default)]
    warnings: Vec<String>,
    #[serde(default)]
    risk_level: Option<String>,
}

pub fn get_settings(db: &Database) -> AiSettings {
    let stored = db.get_setting(SETTINGS_KEY, StoredAiSettings::default());
    AiSettings {
        base_url: stored.base_url,
        model: stored.model,
        default_shell: stored.default_shell,
        api_key_configured: db.get_ai_api_key().is_some(),
    }
}

pub fn save_settings(db: &Database, mut input: AiSettingsInput) -> Result<AiSettings, String> {
    input.base_url = input.base_url.trim().trim_end_matches('/').to_string();
    input.model = input.model.trim().to_string();
    validate_endpoint(&input.base_url)?;
    if let Some(api_key) = input.api_key.take() {
        let api_key = api_key.trim();
        if api_key.is_empty() {
            return Err("API Key 不能为空；如需删除请使用删除按钮".into());
        }
        db.save_ai_api_key(api_key)?;
    }
    db.set_setting(
        SETTINGS_KEY,
        &StoredAiSettings {
            base_url: input.base_url,
            model: input.model,
            default_shell: input.default_shell,
        },
    )?;
    Ok(get_settings(db))
}

pub fn delete_api_key(db: &Database) -> AiSettings {
    db.delete_ai_api_key();
    get_settings(db)
}

pub async fn test_settings(db: &Database, input: AiSettingsInput) -> Result<(), String> {
    let endpoint = validate_endpoint(input.base_url.trim())?;
    let model = input.model.trim();
    if model.is_empty() {
        return Err("模型名称不能为空".into());
    }
    let api_key = input
        .api_key
        .filter(|value| !value.trim().is_empty())
        .or_else(|| db.get_ai_api_key());
    let body = json!({
        "model": model,
        "messages": [
            { "role": "system", "content": "Reply with only OK." },
            { "role": "user", "content": "Connection test" }
        ]
    });
    let response = send_request(endpoint, api_key.as_deref(), body).await?;
    extract_content(&response).map(|_| ())
}

pub async fn generate_command(
    db: &Database,
    request: AiGenerateRequest,
) -> Result<AiCommandSuggestion, String> {
    let requirement = request.requirement.trim();
    if requirement.is_empty() {
        return Err("请先描述需要生成的命令".into());
    }
    if requirement.len() > 8000 {
        return Err("需求描述不能超过 8000 个字符".into());
    }
    let settings = get_settings(db);
    let endpoint = validate_endpoint(&settings.base_url)?;
    if settings.model.trim().is_empty() {
        return Err("请先在设置中配置 AI 模型".into());
    }
    let shell = shell_prompt(&request.shell);
    let system_prompt = format!(
        "You generate shell commands for an SSH client. Target shell: {shell}. Return exactly one JSON object with fields command, explanation, assumptions, warnings, riskLevel. command must be one executable line with no CR or LF. riskLevel must be low, medium, or high. Never wrap the JSON in prose. Explain in Simplified Chinese. Do not claim a command is safe merely because the user requested it."
    );
    let body = json!({
        "model": settings.model,
        "messages": [
            { "role": "system", "content": system_prompt },
            { "role": "user", "content": requirement }
        ]
    });
    let response = send_request(endpoint, db.get_ai_api_key().as_deref(), body).await?;
    let content = extract_content(&response)?;
    let parsed = parse_suggestion(content)?;
    let local = analyze_command(&parsed.command)?;
    let model_risk = parse_risk(parsed.risk_level.as_deref());
    let risk_level = max_risk(local.risk_level, model_risk);
    let mut warnings = parsed.warnings;
    for warning in local.warnings {
        if !warnings.contains(&warning) {
            warnings.push(warning);
        }
    }
    Ok(AiCommandSuggestion {
        command: parsed.command.trim().to_string(),
        explanation: parsed.explanation,
        assumptions: parsed.assumptions,
        warnings,
        risk_level,
    })
}

pub fn analyze_command(command: &str) -> Result<AiRiskAssessment, String> {
    validate_command(command)?;
    let normalized = command.to_lowercase();
    let mut warnings = Vec::new();
    let high_patterns = [
        "rm -rf",
        "rm -fr",
        "mkfs",
        "diskpart",
        "format ",
        "shutdown",
        "reboot",
        "poweroff",
        "halt ",
        "userdel",
        "drop database",
        "drop table",
        "truncate table",
        "del /s",
        "rd /s",
    ];
    if high_patterns
        .iter()
        .any(|pattern| normalized.contains(pattern))
        || (normalized.contains("dd ") && normalized.contains(" of="))
        || (normalized.contains("remove-item") && normalized.contains("-recurse"))
        || ((normalized.contains("curl ") || normalized.contains("wget "))
            && (normalized.contains("| sh") || normalized.contains("| bash")))
        || ((normalized.contains("chmod ") || normalized.contains("chown "))
            && normalized.contains(" -r"))
    {
        warnings.push("命令可能删除、覆盖或大范围修改系统数据".into());
        return Ok(AiRiskAssessment {
            risk_level: AiRiskLevel::High,
            warnings,
        });
    }

    let medium_patterns = [
        "sudo ",
        "rm ",
        "chmod ",
        "chown ",
        "kill ",
        "pkill ",
        "systemctl ",
        "service ",
        "apt install",
        "apt-get install",
        "yum install",
        "dnf install",
        "delete from",
        "update ",
        "insert into",
        " > ",
        ">/",
    ];
    if medium_patterns
        .iter()
        .any(|pattern| normalized.contains(pattern))
    {
        warnings.push("命令会修改文件、进程、软件包、权限或数据".into());
        return Ok(AiRiskAssessment {
            risk_level: AiRiskLevel::Medium,
            warnings,
        });
    }
    Ok(AiRiskAssessment {
        risk_level: AiRiskLevel::Low,
        warnings,
    })
}

fn validate_command(command: &str) -> Result<(), String> {
    let command = command.trim();
    if command.is_empty() {
        return Err("命令不能为空".into());
    }
    if command.len() > 16_000 {
        return Err("命令不能超过 16000 个字符".into());
    }
    if command.chars().any(char::is_control) {
        return Err("为避免意外执行，AI 助手只允许单行命令".into());
    }
    Ok(())
}

fn validate_endpoint(base_url: &str) -> Result<Url, String> {
    let base_url = base_url.trim().trim_end_matches('/');
    let endpoint = if base_url.ends_with("/chat/completions") {
        base_url.to_string()
    } else {
        format!("{base_url}/chat/completions")
    };
    let url = Url::parse(&endpoint).map_err(|_| "API 地址格式不正确".to_string())?;
    if !matches!(url.scheme(), "http" | "https") {
        return Err("API 地址只支持 HTTP 或 HTTPS".into());
    }
    if !url.username().is_empty() || url.password().is_some() || url.host_str().is_none() {
        return Err("API 地址不能包含用户名或密码".into());
    }
    Ok(url)
}

async fn send_request(endpoint: Url, api_key: Option<&str>, body: Value) -> Result<Value, String> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(60))
        .build()
        .map_err(|error| format!("无法初始化 AI 请求：{error}"))?;
    let mut request = client.post(endpoint).json(&body);
    if let Some(api_key) = api_key.filter(|value| !value.is_empty()) {
        request = request.bearer_auth(api_key);
    }
    let response = request.send().await.map_err(|error| {
        if error.is_timeout() {
            "AI 请求超时，请检查地址、网络或模型状态".to_string()
        } else {
            format!("AI 请求失败：{error}")
        }
    })?;
    let status = response.status();
    let mut stream = response.bytes_stream();
    let mut bytes = Vec::new();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|error| format!("读取 AI 响应失败：{error}"))?;
        if bytes.len() + chunk.len() > MAX_RESPONSE_BYTES {
            return Err("AI 响应超过 1 MB 限制".into());
        }
        bytes.extend_from_slice(&chunk);
    }
    let value: Value = serde_json::from_slice(&bytes).map_err(|_| {
        if status.is_success() {
            "AI 返回的内容不是有效 JSON".to_string()
        } else {
            format!("AI 服务返回 HTTP {status}")
        }
    })?;
    if !status.is_success() {
        let message = value
            .pointer("/error/message")
            .and_then(Value::as_str)
            .unwrap_or("请求被 AI 服务拒绝");
        return Err(format!(
            "AI 服务返回 HTTP {status}：{}",
            truncate(message, 400)
        ));
    }
    Ok(value)
}

fn extract_content(response: &Value) -> Result<&str, String> {
    response
        .pointer("/choices/0/message/content")
        .and_then(Value::as_str)
        .filter(|content| !content.trim().is_empty())
        .ok_or_else(|| "AI 响应中没有可用内容".into())
}

fn parse_suggestion(content: &str) -> Result<ModelSuggestion, String> {
    let mut candidate = content.trim();
    if candidate.starts_with("```") {
        candidate = candidate
            .split_once('\n')
            .map(|(_, rest)| rest)
            .unwrap_or(candidate);
        candidate = candidate.strip_suffix("```").unwrap_or(candidate).trim();
    }
    if let (Some(start), Some(end)) = (candidate.find('{'), candidate.rfind('}')) {
        candidate = &candidate[start..=end];
    }
    let suggestion: ModelSuggestion = serde_json::from_str(candidate)
        .map_err(|_| "AI 未按要求返回结构化命令，请重试".to_string())?;
    validate_command(&suggestion.command)?;
    Ok(suggestion)
}

fn shell_prompt(shell: &AiShell) -> &'static str {
    match shell {
        AiShell::Linux => "Linux POSIX shell (prefer broadly compatible bash syntax)",
        AiShell::PowerShell => "Windows PowerShell",
        AiShell::Cmd => "Windows cmd.exe",
        AiShell::Macos => "macOS POSIX shell (zsh compatible)",
    }
}

fn parse_risk(value: Option<&str>) -> AiRiskLevel {
    match value.unwrap_or_default().to_lowercase().as_str() {
        "high" => AiRiskLevel::High,
        "medium" => AiRiskLevel::Medium,
        _ => AiRiskLevel::Low,
    }
}

fn max_risk(left: AiRiskLevel, right: AiRiskLevel) -> AiRiskLevel {
    match (left, right) {
        (AiRiskLevel::High, _) | (_, AiRiskLevel::High) => AiRiskLevel::High,
        (AiRiskLevel::Medium, _) | (_, AiRiskLevel::Medium) => AiRiskLevel::Medium,
        _ => AiRiskLevel::Low,
    }
}

fn truncate(value: &str, max: usize) -> String {
    value.chars().take(max).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_api_endpoints() {
        assert_eq!(
            validate_endpoint("https://example.com/v1")
                .unwrap()
                .as_str(),
            "https://example.com/v1/chat/completions"
        );
        assert_eq!(
            validate_endpoint("http://127.0.0.1:11434/v1/chat/completions")
                .unwrap()
                .as_str(),
            "http://127.0.0.1:11434/v1/chat/completions"
        );
        assert!(validate_endpoint("file:///tmp/model").is_err());
    }

    #[test]
    fn parses_plain_and_fenced_suggestions() {
        let plain = parse_suggestion(
            r#"{"command":"grep 'updated [1-9]' app.log","explanation":"筛选日志","riskLevel":"low"}"#,
        )
        .unwrap();
        assert_eq!(plain.command, "grep 'updated [1-9]' app.log");
        let fenced = parse_suggestion(
            "```json\n{\"command\":\"Get-Content app.log\",\"riskLevel\":\"low\"}\n```",
        )
        .unwrap();
        assert_eq!(fenced.command, "Get-Content app.log");
    }

    #[test]
    fn rejects_multiline_commands() {
        assert!(analyze_command("echo ok\nrm -rf /").is_err());
        assert!(analyze_command("echo\tok").is_err());
    }

    #[test]
    fn classifies_command_risk() {
        assert_eq!(
            analyze_command("grep -E '更新([1-9][0-9]*)条' app.log")
                .unwrap()
                .risk_level,
            AiRiskLevel::Low
        );
        assert_eq!(
            analyze_command("sudo systemctl restart nginx")
                .unwrap()
                .risk_level,
            AiRiskLevel::Medium
        );
        assert_eq!(
            analyze_command("rm -rf /var/lib/app").unwrap().risk_level,
            AiRiskLevel::High
        );
    }
}
