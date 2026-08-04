use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Bookmark {
    pub id: String,
    pub name: String,
    pub host: String,
    pub port: u16,
    pub username: String,
    pub auth_type: AuthType,
    pub private_key_path: String,
    pub jump_bookmark_id: String,
    pub group_name: String,
    pub favorite: bool,
    pub last_connected_at: String,
    pub keepalive_enabled: bool,
    pub keepalive_interval_seconds: u32,
    pub keepalive_count_max: u32,
    pub note: String,
    pub has_saved_credential: bool,
    pub sort_order: i64,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct BookmarkInput {
    #[serde(default)]
    pub id: Option<String>,
    pub name: String,
    pub host: String,
    pub port: u16,
    pub username: String,
    pub auth_type: AuthType,
    pub private_key_path: String,
    pub jump_bookmark_id: String,
    pub group_name: String,
    pub favorite: bool,
    pub keepalive_enabled: bool,
    pub keepalive_interval_seconds: u32,
    pub keepalive_count_max: u32,
    pub note: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BookmarkGroupDeleteResult {
    pub groups: Vec<String>,
    pub deleted_bookmark_ids: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BookmarkArchiveEntry {
    pub id: String,
    pub name: String,
    pub host: String,
    pub port: u16,
    pub username: String,
    pub auth_type: AuthType,
    pub private_key_path: String,
    pub jump_bookmark_id: String,
    pub group_name: String,
    pub favorite: bool,
    pub keepalive_enabled: bool,
    pub keepalive_interval_seconds: u32,
    pub keepalive_count_max: u32,
    pub note: String,
}

impl BookmarkArchiveEntry {
    pub fn to_input(&self, id: Option<String>, jump_bookmark_id: String) -> BookmarkInput {
        BookmarkInput {
            id,
            name: self.name.clone(),
            host: self.host.clone(),
            port: self.port,
            username: self.username.clone(),
            auth_type: self.auth_type.clone(),
            private_key_path: self.private_key_path.clone(),
            jump_bookmark_id,
            group_name: self.group_name.trim().to_string(),
            favorite: self.favorite,
            keepalive_enabled: self.keepalive_enabled,
            keepalive_interval_seconds: self.keepalive_interval_seconds,
            keepalive_count_max: self.keepalive_count_max,
            note: self.note.clone(),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BookmarkArchive {
    pub format: String,
    pub version: u32,
    pub exported_at: String,
    pub groups: Vec<String>,
    pub connections: Vec<BookmarkArchiveEntry>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BookmarkArchiveImportResult {
    pub imported_connections: usize,
    pub imported_groups: usize,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum AuthType {
    Password,
    PrivateKey,
    Agent,
}

impl AuthType {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Password => "password",
            Self::PrivateKey => "privateKey",
            Self::Agent => "agent",
        }
    }
    pub fn parse(value: &str) -> Self {
        match value {
            "privateKey" => Self::PrivateKey,
            "agent" => Self::Agent,
            _ => Self::Password,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConnectInput {
    pub bookmark_id: String,
    #[serde(default)]
    pub new_session: bool,
    pub credential: Option<String>,
    #[serde(default)]
    pub remember_credential: bool,
    pub jump_credential: Option<String>,
    #[serde(default)]
    pub remember_jump_credential: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionSummary {
    pub id: String,
    pub bookmark_id: String,
    pub title: String,
    pub status: SessionStatus,
    pub error: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum SessionStatus {
    Connecting,
    Connected,
    Disconnected,
    Error,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum AiShell {
    Linux,
    PowerShell,
    Cmd,
    Macos,
}

impl Default for AiShell {
    fn default() -> Self {
        Self::Linux
    }
}

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum AiProvider {
    #[default]
    Auto,
    OpenAi,
    Anthropic,
    Qwen,
    DeepSeek,
    Kimi,
    Glm,
    MiniMax,
    Grok,
    Gemini,
}

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum AiThinkingMode {
    #[default]
    Default,
    Disabled,
    Enabled,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AiSettings {
    pub base_url: String,
    pub model: String,
    pub default_shell: AiShell,
    pub provider: AiProvider,
    pub thinking_mode: AiThinkingMode,
    pub api_key_configured: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AiSettingsInput {
    pub base_url: String,
    pub model: String,
    pub default_shell: AiShell,
    #[serde(default)]
    pub provider: AiProvider,
    #[serde(default)]
    pub thinking_mode: AiThinkingMode,
    #[serde(default)]
    pub api_key: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AiGenerateRequest {
    pub requirement: String,
    pub shell: AiShell,
    #[serde(default)]
    pub terminal_context: Option<String>,
    #[serde(default)]
    pub redact_terminal_context: bool,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum AiRiskLevel {
    Low,
    Medium,
    High,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AiRiskAssessment {
    pub risk_level: AiRiskLevel,
    pub warnings: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AiCommandSuggestion {
    pub command: String,
    pub explanation: String,
    pub assumptions: Vec<String>,
    pub warnings: Vec<String>,
    pub risk_level: AiRiskLevel,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AiRawExchange {
    pub occurred_at: String,
    pub endpoint: String,
    pub request_headers: String,
    pub request_body: String,
    pub response_status: Option<u16>,
    pub response_headers: String,
    pub response_body: String,
    pub error: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HostKeyPrompt {
    pub session_id: String,
    pub host: String,
    pub port: u16,
    pub fingerprint: String,
    pub status: HostKeyStatus,
    pub previous_fingerprint: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum HostKeyStatus {
    Unknown,
    Changed,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DirectoryEntry {
    pub name: String,
    pub path: String,
    pub kind: EntryKind,
    pub size: Option<u64>,
    pub modified_at: Option<i64>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum EntryKind {
    File,
    Directory,
    Symlink,
    Other,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FilePreview {
    pub content: String,
    pub size: u64,
    pub truncated: bool,
    pub position: PreviewPosition,
    pub binary: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PreviewPosition {
    Start,
    End,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SshConfigImportEntry {
    pub alias: String,
    pub name: String,
    pub host: String,
    pub port: u16,
    pub username: String,
    pub private_key_path: String,
    pub proxy_jump_alias: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SshConfigPreview {
    pub path: String,
    pub entries: Vec<SshConfigImportEntry>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FavoritePaths {
    pub local: Vec<String>,
    pub remote: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TerminalSettings {
    pub font_family: String,
    pub font_size: u16,
    pub foreground_color: String,
    pub background_color: String,
    pub background_opacity: f64,
    pub background_image_path: String,
    pub background_image_fit: TerminalBackgroundFit,
}

impl Default for TerminalSettings {
    fn default() -> Self {
        Self {
            font_family: "Cascadia Mono, SFMono-Regular, Consolas, monospace".into(),
            font_size: 14,
            foreground_color: "#e8eaed".into(),
            background_color: "#111315".into(),
            background_opacity: 1.0,
            background_image_path: String::new(),
            background_image_fit: TerminalBackgroundFit::Cover,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TerminalBackgroundFit {
    Cover,
    Contain,
    Stretch,
    Tile,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum UiTheme {
    System,
    Light,
    Dark,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum AppIconId {
    Ssh,
    Classic,
    Neon,
    Light,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppIconOption {
    pub id: AppIconId,
    pub name: String,
    pub data_url: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppIconSettings {
    pub selected: AppIconId,
    pub options: Vec<AppIconOption>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TransferRequest {
    pub session_id: String,
    pub direction: TransferDirection,
    pub source_paths: Vec<String>,
    pub destination_directory: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum TransferDirection {
    Upload,
    Download,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum TransferStatus {
    Queued,
    Scanning,
    Running,
    Conflict,
    Completed,
    Failed,
    Cancelled,
    Interrupted,
}

impl TransferStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Queued => "queued",
            Self::Scanning => "scanning",
            Self::Running => "running",
            Self::Conflict => "conflict",
            Self::Completed => "completed",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
            Self::Interrupted => "interrupted",
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TransferTask {
    pub id: String,
    pub session_id: String,
    pub bookmark_id: String,
    pub direction: TransferDirection,
    pub source_path: String,
    pub destination_path: String,
    pub display_name: String,
    pub status: TransferStatus,
    pub bytes_total: u64,
    pub bytes_transferred: u64,
    pub speed: f64,
    pub error: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ConflictResolution {
    Overwrite,
    Skip,
    Rename,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeploymentProfile {
    pub id: String,
    pub bookmark_id: String,
    pub name: String,
    pub local_directory: String,
    pub remote_directory: String,
    pub delete_extraneous: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeploymentDiffEntry {
    pub relative_path: String,
    pub local_path: Option<String>,
    pub remote_path: String,
    pub status: DeploymentDiffStatus,
    pub size: Option<u64>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum DeploymentDiffStatus {
    New,
    Changed,
    Same,
    RemoteOnly,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum PortForwardType {
    Local,
    Remote,
    Dynamic,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PortForwardProfile {
    pub id: String,
    pub bookmark_id: String,
    pub name: String,
    #[serde(rename = "type")]
    pub forward_type: PortForwardType,
    pub bind_address: String,
    pub bind_port: u16,
    pub target_host: String,
    pub target_port: u16,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TunnelStatus {
    Starting,
    Running,
    Error,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TunnelSummary {
    pub id: String,
    pub profile_id: String,
    pub session_id: String,
    pub name: String,
    #[serde(rename = "type")]
    pub forward_type: PortForwardType,
    pub bind_address: String,
    pub bind_port: u16,
    pub target_host: String,
    pub target_port: u16,
    pub status: TunnelStatus,
    pub error: Option<String>,
    #[serde(default)]
    pub removed: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "type", content = "payload", rename_all = "kebab-case")]
pub enum AppEvent {
    Session(SessionSummary),
    TerminalData(TerminalData),
    HostKey(HostKeyPrompt),
    Transfer(TransferTask),
    Tunnel(TunnelSummary),
    TransferConflict(TransferConflict),
    MenuCommand(MenuCommand),
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TerminalData {
    pub session_id: String,
    pub data: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TransferConflict {
    pub task_id: String,
    pub source_path: String,
    pub destination_path: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum MenuCommand {
    NewConnection,
    ImportConnections,
    NewSession,
    CloseTab,
    Settings,
    Help,
    ShowTerminal,
    ShowFiles,
    ToggleSidebar,
}
