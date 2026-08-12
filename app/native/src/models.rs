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

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BookmarkArchive {
    pub format: String,
    pub version: u32,
    pub exported_at: String,
    pub groups: Vec<String>,
    pub connections: Vec<BookmarkArchiveEntry>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum BookmarkArchiveSource {
    LunaMux,
    LunaRemote,
    Legacy,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BookmarkArchivePreview {
    pub preview_id: String,
    pub path: String,
    pub source: BookmarkArchiveSource,
    pub exported_at: String,
    pub groups: Vec<String>,
    pub connections: Vec<BookmarkArchiveEntry>,
    pub credentials_included: bool,
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
pub struct AiCommandHistoryEntry {
    pub id: String,
    pub created_at: String,
    pub requirement: String,
    pub shell: AiShell,
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
            font_family: "\"JetBrains Mono\", \"SFMono-Regular\", Consolas, monospace".into(),
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
pub struct NativeMenuLabels {
    pub settings: String,
    pub new_connection: String,
    pub import_open_ssh_config: String,
    pub new_session: String,
    pub close_tab: String,
    pub terminal: String,
    pub files: String,
    pub toggle_sidebar: String,
    pub help_item: String,
    pub about: String,
    pub services: String,
    pub hide: String,
    pub hide_others: String,
    pub show_all: String,
    pub quit: String,
    pub connection_menu: String,
    pub edit_menu: String,
    pub undo: String,
    pub redo: String,
    pub cut: String,
    pub copy: String,
    pub paste: String,
    pub select_all: String,
    pub view_menu: String,
    pub fullscreen: String,
    pub window_menu: String,
    pub minimize: String,
    pub zoom: String,
    pub bring_all_to_front: String,
    pub help_menu: String,
}

impl Default for NativeMenuLabels {
    fn default() -> Self {
        Self {
            settings: "设置...".into(),
            new_connection: "新增连接".into(),
            import_open_ssh_config: "导入 OpenSSH Config...".into(),
            new_session: "为当前机器新建会话".into(),
            close_tab: "关闭当前标签".into(),
            terminal: "终端".into(),
            files: "文件传输".into(),
            toggle_sidebar: "展开/折叠连接管理".into(),
            help_item: format!("{} 使用帮助", crate::product::DISPLAY_NAME),
            about: format!("关于 {}", crate::product::DISPLAY_NAME),
            services: "服务".into(),
            hide: format!("隐藏 {}", crate::product::DISPLAY_NAME),
            hide_others: "隐藏其他".into(),
            show_all: "全部显示".into(),
            quit: format!("退出 {}", crate::product::DISPLAY_NAME),
            connection_menu: "连接".into(),
            edit_menu: "编辑".into(),
            undo: "撤销".into(),
            redo: "重做".into(),
            cut: "剪切".into(),
            copy: "复制".into(),
            paste: "粘贴".into(),
            select_all: "全选".into(),
            view_menu: "视图".into(),
            fullscreen: "进入/退出全屏".into(),
            window_menu: "窗口".into(),
            minimize: "最小化".into(),
            zoom: "缩放".into(),
            bring_all_to_front: "前置全部窗口".into(),
            help_menu: "帮助".into(),
        }
    }
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
