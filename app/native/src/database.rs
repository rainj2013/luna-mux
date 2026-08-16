use std::{
    collections::{HashMap, HashSet},
    fs,
    path::Path,
    sync::Mutex,
    time::SystemTime,
};

use chrono::Utc;
use keyring::Entry;
use rusqlite::{Connection, OpenFlags, OptionalExtension, params};
use serde::{Serialize, de::DeserializeOwned};
use uuid::Uuid;

use crate::models::*;

const AI_API_KEY_ACCOUNT: &str = "__ai_api_key__";
const AI_COMMAND_HISTORY_KEY: &str = "aiCommandHistory";
const AI_COMMAND_HISTORY_LIMIT: usize = 10;
const PORTABLE_SETTING_KEYS: &[&str] = &[
    "aiSettings",
    "collapsedBookmarkGroups",
    "language",
    "sidebarCollapsed",
    "sidebarWidth",
    "terminalSettings",
    "uiTheme",
];

fn prune_mux_layout(
    node: MuxSplitNode,
    removed_pane_ids: &HashSet<String>,
) -> Option<MuxSplitNode> {
    match node {
        MuxSplitNode::Pane { pane_id } => {
            (!removed_pane_ids.contains(&pane_id)).then_some(MuxSplitNode::Pane { pane_id })
        }
        MuxSplitNode::Split {
            direction,
            ratio,
            first,
            second,
        } => match (
            prune_mux_layout(*first, removed_pane_ids),
            prune_mux_layout(*second, removed_pane_ids),
        ) {
            (Some(first), Some(second)) => Some(MuxSplitNode::Split {
                direction,
                ratio,
                first: Box::new(first),
                second: Box::new(second),
            }),
            (Some(remaining), None) | (None, Some(remaining)) => Some(remaining),
            (None, None) => None,
        },
    }
}

pub struct Database {
    connection: Mutex<Connection>,
    credential_service: String,
    legacy_credential_service: Option<String>,
}

impl Database {
    pub fn read_luna_remote_snapshot(path: &Path) -> Result<LunaRemoteSnapshot, String> {
        let metadata = fs::metadata(path).map_err(|error| error.to_string())?;
        if !metadata.is_file() {
            return Err("Luna Remote 数据源必须是普通数据库文件".into());
        }
        let source_modified_at =
            chrono::DateTime::<Utc>::from(metadata.modified().unwrap_or(SystemTime::UNIX_EPOCH))
                .to_rfc3339();
        let connection = Connection::open_with_flags(
            path,
            OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )
        .map_err(|error| format!("无法只读打开 Luna Remote 数据库：{error}"))?;
        connection
            .execute_batch("PRAGMA query_only = ON; BEGIN DEFERRED TRANSACTION;")
            .map_err(|error| error.to_string())?;

        let required_tables = ["bookmarks", "settings", "known_hosts"];
        for table in required_tables {
            let exists = connection
                .query_row(
                    "SELECT 1 FROM sqlite_master WHERE type='table' AND name=?",
                    [table],
                    |_| Ok(()),
                )
                .optional()
                .map_err(|error| error.to_string())?
                .is_some();
            if !exists {
                return Err(format!(
                    "所选文件不是兼容的 Luna Remote 数据库：缺少 {table} 表"
                ));
            }
        }
        let has_sort_order = {
            let mut statement = connection
                .prepare("PRAGMA table_info(bookmarks)")
                .map_err(|error| error.to_string())?;
            statement
                .query_map([], |row| row.get::<_, String>(1))
                .map_err(|error| error.to_string())?
                .collect::<rusqlite::Result<Vec<_>>>()
                .map_err(|error| error.to_string())?
                .iter()
                .any(|column| column == "sortOrder")
        };
        let order = if has_sort_order {
            "sortOrder ASC, createdAt ASC"
        } else {
            "createdAt ASC"
        };
        let connections = {
            let mut statement = connection
                .prepare(&format!(
                    "SELECT id,name,host,port,username,authType,privateKeyPath,jumpBookmarkId,groupName,favorite,keepaliveEnabled,keepaliveIntervalSeconds,keepaliveCountMax,note FROM bookmarks ORDER BY {order}"
                ))
                .map_err(|error| error.to_string())?;
            statement
                .query_map([], |row| {
                    Ok(BookmarkArchiveEntry {
                        id: row.get(0)?,
                        name: row.get(1)?,
                        host: row.get(2)?,
                        port: row.get(3)?,
                        username: row.get(4)?,
                        auth_type: AuthType::parse(&row.get::<_, String>(5)?),
                        private_key_path: row.get(6)?,
                        jump_bookmark_id: row.get(7)?,
                        group_name: row.get(8)?,
                        favorite: row.get::<_, i64>(9)? != 0,
                        keepalive_enabled: row.get::<_, i64>(10)? != 0,
                        keepalive_interval_seconds: row.get(11)?,
                        keepalive_count_max: row.get(12)?,
                        note: row.get(13)?,
                    })
                })
                .map_err(|error| error.to_string())?
                .collect::<rusqlite::Result<Vec<_>>>()
                .map_err(|error| error.to_string())?
        };
        if connections.len() > 10_000 {
            return Err("Luna Remote 数据库中的连接数量过多".into());
        }
        let connection_ids = connections
            .iter()
            .map(|entry| entry.id.as_str())
            .collect::<HashSet<_>>();
        if connection_ids.len() != connections.len()
            || connections.iter().any(|entry| {
                entry.id.trim().is_empty()
                    || entry.name.trim().is_empty()
                    || entry.host.trim().is_empty()
                    || entry.username.trim().is_empty()
                    || entry.port == 0
                    || entry.jump_bookmark_id == entry.id
                    || (!entry.jump_bookmark_id.is_empty()
                        && !connection_ids.contains(entry.jump_bookmark_id.as_str()))
            })
        {
            return Err("Luna Remote 数据库包含无效连接或跳板机引用".into());
        }
        let connections_by_id = connections
            .iter()
            .map(|entry| (entry.id.as_str(), entry))
            .collect::<HashMap<_, _>>();
        if connections.iter().any(|entry| {
            !entry.jump_bookmark_id.is_empty()
                && connections_by_id
                    .get(entry.jump_bookmark_id.as_str())
                    .is_some_and(|jump| !jump.jump_bookmark_id.is_empty())
        }) {
            return Err("Luna Remote 数据库包含多层跳板机，当前仅支持一层跳板机".into());
        }

        let raw_settings = {
            let mut statement = connection
                .prepare("SELECT key,value FROM settings")
                .map_err(|error| error.to_string())?;
            statement
                .query_map([], |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
                })
                .map_err(|error| error.to_string())?
                .collect::<rusqlite::Result<HashMap<_, _>>>()
                .map_err(|error| error.to_string())?
        };
        let mut groups = raw_settings
            .get("bookmarkGroups")
            .and_then(|value| serde_json::from_str::<Vec<String>>(value).ok())
            .unwrap_or_default();
        for group in connections.iter().map(|entry| entry.group_name.trim()) {
            if !group.is_empty() && !groups.iter().any(|existing| existing == group) {
                groups.push(group.to_string());
            }
        }
        groups.retain(|group| !group.trim().is_empty());
        groups.truncate(1_000);
        let settings = PORTABLE_SETTING_KEYS
            .iter()
            .filter_map(|key| {
                raw_settings.get(*key).and_then(|value| {
                    if *key == "terminalSettings" {
                        let mut settings = serde_json::from_str::<TerminalSettings>(value).ok()?;
                        settings.background_image_path.clear();
                        return Some(PortableSettingEntry {
                            key: (*key).to_string(),
                            value: serde_json::to_string(&settings).ok()?,
                        });
                    }
                    serde_json::from_str::<serde_json::Value>(value).ok()?;
                    Some(PortableSettingEntry {
                        key: (*key).to_string(),
                        value: value.clone(),
                    })
                })
            })
            .collect();
        let forwarding_profiles = raw_settings
            .get("portForwardProfiles")
            .and_then(|value| serde_json::from_str::<Vec<PortForwardProfile>>(value).ok())
            .unwrap_or_default()
            .into_iter()
            .filter(|profile| connection_ids.contains(profile.bookmark_id.as_str()))
            .take(10_000)
            .collect();
        let known_hosts = {
            let mut statement = connection
                .prepare("SELECT host,port,fingerprint FROM known_hosts ORDER BY host,port")
                .map_err(|error| error.to_string())?;
            statement
                .query_map([], |row| {
                    Ok(KnownHostImportEntry {
                        host: row.get(0)?,
                        port: row.get(1)?,
                        fingerprint: row.get(2)?,
                    })
                })
                .map_err(|error| error.to_string())?
                .collect::<rusqlite::Result<Vec<_>>>()
                .map_err(|error| error.to_string())?
        };
        if known_hosts.len() > 100_000 {
            return Err("Luna Remote 数据库中的主机指纹数量过多".into());
        }
        let credential_connection_ids = {
            let has_table = connection
                .query_row(
                    "SELECT 1 FROM sqlite_master WHERE type='table' AND name='credential_refs'",
                    [],
                    |_| Ok(()),
                )
                .optional()
                .map_err(|error| error.to_string())?
                .is_some();
            if has_table {
                let mut statement = connection
                    .prepare("SELECT bookmarkId FROM credential_refs ORDER BY bookmarkId")
                    .map_err(|error| error.to_string())?;
                statement
                    .query_map([], |row| row.get::<_, String>(0))
                    .map_err(|error| error.to_string())?
                    .collect::<rusqlite::Result<Vec<_>>>()
                    .map_err(|error| error.to_string())?
                    .into_iter()
                    .filter(|id| connection_ids.contains(id.as_str()))
                    .collect()
            } else {
                Vec::new()
            }
        };
        connection
            .execute_batch("COMMIT")
            .map_err(|error| error.to_string())?;
        Ok(LunaRemoteSnapshot {
            path: path.to_string_lossy().into_owned(),
            source_modified_at,
            groups,
            connections,
            known_hosts,
            settings,
            forwarding_profiles,
            credential_connection_ids,
        })
    }

    pub fn open(path: &Path, credential_service: &str) -> Result<Self, String> {
        Self::open_with_legacy(path, credential_service, None)
    }

    pub fn open_with_legacy(
        path: &Path,
        credential_service: &str,
        legacy_credential_service: Option<&str>,
    ) -> Result<Self, String> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        let connection = Connection::open(path).map_err(|e| e.to_string())?;
        let database = Self {
            connection: Mutex::new(connection),
            credential_service: credential_service.into(),
            legacy_credential_service: legacy_credential_service.map(str::to_string),
        };
        database.initialize()?;
        Ok(database)
    }

    fn initialize(&self) -> Result<(), String> {
        self.with_conn(|db| db.execute_batch(r#"
            PRAGMA journal_mode = WAL;
            PRAGMA foreign_keys = ON;
            CREATE TABLE IF NOT EXISTS bookmarks (
              id TEXT PRIMARY KEY, name TEXT NOT NULL, host TEXT NOT NULL, port INTEGER NOT NULL,
              username TEXT NOT NULL, authType TEXT NOT NULL, privateKeyPath TEXT NOT NULL DEFAULT '',
              jumpBookmarkId TEXT NOT NULL DEFAULT '', note TEXT NOT NULL DEFAULT '', groupName TEXT NOT NULL DEFAULT '',
              favorite INTEGER NOT NULL DEFAULT 0, lastConnectedAt TEXT NOT NULL DEFAULT '',
              keepaliveEnabled INTEGER NOT NULL DEFAULT 1, keepaliveIntervalSeconds INTEGER NOT NULL DEFAULT 15,
              keepaliveCountMax INTEGER NOT NULL DEFAULT 3, sortOrder INTEGER NOT NULL DEFAULT 0,
              createdAt TEXT NOT NULL, updatedAt TEXT NOT NULL
            );
            CREATE TABLE IF NOT EXISTS credential_refs (bookmarkId TEXT PRIMARY KEY, account TEXT NOT NULL);
            CREATE TABLE IF NOT EXISTS known_hosts (host TEXT NOT NULL, port INTEGER NOT NULL, fingerprint TEXT NOT NULL, PRIMARY KEY(host, port));
            CREATE TABLE IF NOT EXISTS settings (key TEXT PRIMARY KEY, value TEXT NOT NULL);
            CREATE TABLE IF NOT EXISTS mux_sessions (
              id TEXT PRIMARY KEY, name TEXT NOT NULL, rootPath TEXT NOT NULL DEFAULT '',
              layoutJson TEXT NOT NULL DEFAULT '', sortOrder INTEGER NOT NULL DEFAULT 0,
              createdAt TEXT NOT NULL, updatedAt TEXT NOT NULL
            );
            CREATE TABLE IF NOT EXISTS transfers (
              id TEXT PRIMARY KEY, sessionId TEXT NOT NULL, bookmarkId TEXT NOT NULL DEFAULT '', direction TEXT NOT NULL,
              sourcePath TEXT NOT NULL, destinationPath TEXT NOT NULL, displayName TEXT NOT NULL, status TEXT NOT NULL,
              bytesTotal INTEGER NOT NULL, bytesTransferred INTEGER NOT NULL, speed REAL NOT NULL, error TEXT,
              createdAt TEXT NOT NULL, updatedAt TEXT NOT NULL
            );
            CREATE TABLE IF NOT EXISTS agent_control_grants (
              id TEXT PRIMARY KEY, sourcePaneId TEXT NOT NULL,
              targetResourceKind TEXT NOT NULL, targetResourceId TEXT NOT NULL,
              access TEXT NOT NULL, createdAt TEXT NOT NULL, updatedAt TEXT NOT NULL,
              UNIQUE(sourcePaneId, targetResourceKind, targetResourceId),
              FOREIGN KEY(sourcePaneId) REFERENCES mux_panes(id) ON DELETE CASCADE
            );
            CREATE TABLE IF NOT EXISTS control_audit (
              id TEXT PRIMARY KEY, timestamp TEXT NOT NULL, callerId TEXT NOT NULL,
              callerKind TEXT NOT NULL, operation TEXT NOT NULL, resourceKind TEXT NOT NULL DEFAULT '',
              resourceId TEXT NOT NULL DEFAULT '', argumentsJson TEXT NOT NULL,
              result TEXT NOT NULL, errorCode TEXT NOT NULL DEFAULT ''
            );
            CREATE INDEX IF NOT EXISTS control_audit_timestamp ON control_audit(timestamp);
        "#))?;
        let has_layout_json = self.with_conn(|db| {
            let mut statement = db.prepare("PRAGMA table_info(mux_sessions)")?;
            let columns = statement
                .query_map([], |row| row.get::<_, String>(1))?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            Ok(columns.iter().any(|column| column == "layoutJson"))
        })?;
        let has_launch_profile_id = self.with_conn(|db| {
            let mut statement = db.prepare("PRAGMA table_info(mux_panes)")?;
            let columns = statement
                .query_map([], |row| row.get::<_, String>(1))?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            Ok(columns.iter().any(|column| column == "launchProfileId"))
        })?;
        let has_resource_grants = self.with_conn(|db| {
            let mut statement = db.prepare("PRAGMA table_info(agent_control_grants)")?;
            let columns = statement
                .query_map([], |row| row.get::<_, String>(1))?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            Ok(columns.iter().any(|column| column == "targetResourceKind"))
        })?;
        self.with_conn(|db| {
            let transaction = db.transaction()?;
            if !has_layout_json {
                transaction.execute(
                    "ALTER TABLE mux_sessions ADD COLUMN layoutJson TEXT NOT NULL DEFAULT ''",
                    [],
                )?;
            }
            transaction.execute_batch(
                r#"
                CREATE TABLE IF NOT EXISTS mux_panes (
                  id TEXT PRIMARY KEY, muxSessionId TEXT NOT NULL, kind TEXT NOT NULL,
                  title TEXT NOT NULL, targetId TEXT NOT NULL DEFAULT '', bookmarkId TEXT NOT NULL DEFAULT '',
                  cwd TEXT NOT NULL DEFAULT '', command TEXT NOT NULL DEFAULT '', sortOrder INTEGER NOT NULL DEFAULT 0,
                  createdAt TEXT NOT NULL, updatedAt TEXT NOT NULL,
                  FOREIGN KEY(muxSessionId) REFERENCES mux_sessions(id) ON DELETE CASCADE
                );
                CREATE TABLE IF NOT EXISTS browser_resources (
                  id TEXT PRIMARY KEY, muxSessionId TEXT NOT NULL, name TEXT NOT NULL,
                  sourcePaneId TEXT NOT NULL DEFAULT '', bookmarkId TEXT NOT NULL DEFAULT '',
                  url TEXT NOT NULL DEFAULT 'about:blank', temporaryProfile INTEGER NOT NULL DEFAULT 0,
                  sortOrder INTEGER NOT NULL DEFAULT 0, createdAt TEXT NOT NULL, updatedAt TEXT NOT NULL,
                  FOREIGN KEY(muxSessionId) REFERENCES mux_sessions(id) ON DELETE CASCADE
                );
                "#,
            )?;
            if !has_launch_profile_id {
                transaction.execute(
                    "ALTER TABLE mux_panes ADD COLUMN launchProfileId TEXT NOT NULL DEFAULT ''",
                    [],
                )?;
            }
            if !has_resource_grants {
                let browser_pane_ids = {
                    let mut statement = transaction
                        .prepare("SELECT id FROM mux_panes WHERE kind='browser'")?;
                    statement
                        .query_map([], |row| row.get::<_, String>(0))?
                        .collect::<rusqlite::Result<HashSet<_>>>()?
                };
                transaction.execute_batch(
                    r#"
                    INSERT OR IGNORE INTO browser_resources(
                      id,muxSessionId,name,sourcePaneId,bookmarkId,url,temporaryProfile,
                      sortOrder,createdAt,updatedAt
                    )
                    SELECT id,muxSessionId,title,targetId,bookmarkId,
                      CASE WHEN trim(command)='' THEN 'about:blank' ELSE command END,
                      0,sortOrder,createdAt,updatedAt
                    FROM mux_panes WHERE kind='browser';
                    CREATE TABLE agent_control_grants_v10 (
                      id TEXT PRIMARY KEY, sourcePaneId TEXT NOT NULL,
                      targetResourceKind TEXT NOT NULL, targetResourceId TEXT NOT NULL,
                      access TEXT NOT NULL, createdAt TEXT NOT NULL, updatedAt TEXT NOT NULL,
                      UNIQUE(sourcePaneId, targetResourceKind, targetResourceId),
                      FOREIGN KEY(sourcePaneId) REFERENCES mux_panes(id) ON DELETE CASCADE
                    );
                    INSERT INTO agent_control_grants_v10(
                      id,sourcePaneId,targetResourceKind,targetResourceId,access,createdAt,updatedAt
                    )
                    SELECT id,sourcePaneId,
                      CASE WHEN targetPaneId IN (SELECT id FROM mux_panes WHERE kind='browser')
                        THEN 'browser' ELSE 'pane' END,
                      targetPaneId,access,createdAt,updatedAt
                    FROM agent_control_grants;
                    DROP TABLE agent_control_grants;
                    ALTER TABLE agent_control_grants_v10 RENAME TO agent_control_grants;
                    "#,
                )?;
                if !browser_pane_ids.is_empty() {
                    let sessions = {
                        let mut statement = transaction
                            .prepare("SELECT id,layoutJson FROM mux_sessions")?;
                        statement
                            .query_map([], |row| {
                                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
                            })?
                            .collect::<rusqlite::Result<Vec<_>>>()?
                    };
                    for (session_id, layout_json) in sessions {
                        let layout = serde_json::from_str::<MuxSplitNode>(&layout_json).ok();
                        let pruned = layout.and_then(|node| {
                            prune_mux_layout(node, &browser_pane_ids)
                        });
                        let next_json = pruned
                            .as_ref()
                            .map(serde_json::to_string)
                            .transpose()
                            .map_err(|error| {
                                rusqlite::Error::ToSqlConversionFailure(Box::new(error))
                            })?
                            .unwrap_or_default();
                        transaction.execute(
                            "UPDATE mux_sessions SET layoutJson=? WHERE id=?",
                            params![next_json, session_id],
                        )?;
                    }
                    transaction.execute("DELETE FROM mux_panes WHERE kind='browser'", [])?;
                }
            }
            // Grant rows were migration input for the abandoned per-Pane model.
            transaction.execute_batch(
                "DROP TABLE IF EXISTS agent_control_grants; PRAGMA user_version = 10;",
            )?;
            transaction.commit()
        })?;
        let has_sort_order = self.with_conn(|db| {
            let mut statement = db.prepare("PRAGMA table_info(bookmarks)")?;
            let columns = statement
                .query_map([], |row| row.get::<_, String>(1))?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            Ok(columns.iter().any(|column| column == "sortOrder"))
        })?;
        if !has_sort_order {
            self.with_conn(|db| {
                let transaction = db.transaction()?;
                transaction.execute(
                    "ALTER TABLE bookmarks ADD COLUMN sortOrder INTEGER NOT NULL DEFAULT 0",
                    [],
                )?;
                let ids = {
                    let mut statement = transaction.prepare(
                        "SELECT id FROM bookmarks ORDER BY favorite DESC, CASE WHEN lastConnectedAt='' THEN updatedAt ELSE lastConnectedAt END DESC",
                    )?;
                    statement
                        .query_map([], |row| row.get::<_, String>(0))?
                        .collect::<rusqlite::Result<Vec<_>>>()?
                };
                for (index, id) in ids.iter().enumerate() {
                    transaction.execute(
                        "UPDATE bookmarks SET sortOrder=? WHERE id=?",
                        params![index as i64, id],
                    )?;
                }
                transaction.commit()
            })?;
        }
        self.with_conn(|db| db.execute("UPDATE transfers SET status='interrupted', error='应用上次退出时传输未完成', updatedAt=? WHERE status IN ('queued','scanning','running','conflict')", [Utc::now().to_rfc3339()]).map(|_| ()))?;
        for session in self.list_mux_sessions()? {
            self.ensure_session_browser_resource(&session.id)?;
        }
        self.prune_control_audit(30)?;
        Ok(())
    }

    fn with_conn<T>(
        &self,
        operation: impl FnOnce(&mut Connection) -> rusqlite::Result<T>,
    ) -> Result<T, String> {
        let mut connection = self
            .connection
            .lock()
            .map_err(|_| "数据库锁已损坏".to_string())?;
        operation(&mut connection).map_err(|e| e.to_string())
    }

    pub fn list_bookmarks(&self) -> Result<Vec<Bookmark>, String> {
        self.with_conn(|db| {
            let mut statement = db.prepare(r#"SELECT b.id,b.name,b.host,b.port,b.username,b.authType,b.privateKeyPath,b.jumpBookmarkId,b.groupName,b.favorite,b.lastConnectedAt,b.keepaliveEnabled,b.keepaliveIntervalSeconds,b.keepaliveCountMax,b.note,
                CASE WHEN c.bookmarkId IS NULL THEN 0 ELSE 1 END,b.sortOrder,b.createdAt,b.updatedAt
                FROM bookmarks b LEFT JOIN credential_refs c ON c.bookmarkId=b.id
                ORDER BY b.sortOrder ASC, b.createdAt ASC"#)?;
            statement.query_map([], |row| Ok(Bookmark {
                id: row.get(0)?, name: row.get(1)?, host: row.get(2)?, port: row.get(3)?, username: row.get(4)?,
                auth_type: AuthType::parse(&row.get::<_, String>(5)?), private_key_path: row.get(6)?, jump_bookmark_id: row.get(7)?,
                group_name: row.get(8)?, favorite: row.get::<_, i64>(9)? != 0, last_connected_at: row.get(10)?,
                keepalive_enabled: row.get::<_, i64>(11)? != 0, keepalive_interval_seconds: row.get(12)?, keepalive_count_max: row.get(13)?,
                note: row.get(14)?, has_saved_credential: row.get::<_, i64>(15)? != 0, sort_order: row.get(16)?, created_at: row.get(17)?, updated_at: row.get(18)?,
            }))?.collect()
        })
    }

    pub fn list_mux_sessions(&self) -> Result<Vec<MuxSession>, String> {
        self.with_conn(|db| {
            let mut statement = db.prepare("SELECT id,name,rootPath,layoutJson,sortOrder,createdAt,updatedAt FROM mux_sessions ORDER BY sortOrder ASC, createdAt ASC")?;
            statement.query_map([], |row| {
                let layout_json = row.get::<_, String>(3)?;
                Ok(MuxSession {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    root_path: row.get(2)?,
                    layout: if layout_json.trim().is_empty() {
                        None
                    } else {
                        Some(serde_json::from_str(&layout_json).map_err(|error| {
                            rusqlite::Error::FromSqlConversionFailure(
                                3,
                                rusqlite::types::Type::Text,
                                Box::new(error),
                            )
                        })?)
                    },
                    sort_order: row.get(4)?,
                    created_at: row.get(5)?,
                    updated_at: row.get(6)?,
                })
            })?.collect()
        })
    }

    pub fn save_mux_session(&self, input: MuxSessionInput) -> Result<MuxSession, String> {
        let name = input.name.trim();
        if name.is_empty() {
            return Err("Session 名称不能为空".into());
        }
        let now = Utc::now().to_rfc3339();
        let id = input
            .id
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(|| Uuid::new_v4().to_string());
        let existing = self
            .list_mux_sessions()?
            .into_iter()
            .find(|item| item.id == id);
        let sort_order = if let Some(existing) = &existing {
            existing.sort_order
        } else {
            self.with_conn(|db| {
                db.query_row(
                    "SELECT COALESCE(MAX(sortOrder), -1) + 1 FROM mux_sessions",
                    [],
                    |row| row.get(0),
                )
            })?
        };
        let created_at = existing
            .as_ref()
            .map(|item| item.created_at.clone())
            .unwrap_or_else(|| now.clone());
        let session = MuxSession {
            id,
            name: name.into(),
            root_path: input.root_path.trim().into(),
            layout: input.layout,
            sort_order,
            created_at,
            updated_at: now,
        };
        let layout_json = session
            .layout
            .as_ref()
            .map(serde_json::to_string)
            .transpose()
            .map_err(|error| error.to_string())?
            .unwrap_or_default();
        self.with_conn(|db| db.execute("INSERT INTO mux_sessions(id,name,rootPath,layoutJson,sortOrder,createdAt,updatedAt) VALUES(?,?,?,?,?,?,?) ON CONFLICT(id) DO UPDATE SET name=excluded.name,rootPath=excluded.rootPath,layoutJson=excluded.layoutJson,updatedAt=excluded.updatedAt", params![session.id,session.name,session.root_path,layout_json,session.sort_order,session.created_at,session.updated_at]).map(|_| ()))?;
        Ok(session)
    }

    pub fn delete_mux_session(&self, id: &str) -> Result<(), String> {
        self.with_conn(|db| {
            let transaction = db.transaction()?;
            transaction.execute("DELETE FROM mux_panes WHERE muxSessionId=?", [id])?;
            transaction.execute("DELETE FROM mux_sessions WHERE id=?", [id])?;
            transaction.commit()
        })
    }

    pub fn list_mux_panes(&self, mux_session_id: Option<&str>) -> Result<Vec<MuxPane>, String> {
        self.with_conn(|db| {
            let sql = if mux_session_id.is_some() {
                "SELECT id,muxSessionId,kind,title,targetId,bookmarkId,cwd,command,launchProfileId,sortOrder,createdAt,updatedAt FROM mux_panes WHERE muxSessionId=? ORDER BY sortOrder ASC, createdAt ASC"
            } else {
                "SELECT id,muxSessionId,kind,title,targetId,bookmarkId,cwd,command,launchProfileId,sortOrder,createdAt,updatedAt FROM mux_panes ORDER BY muxSessionId ASC, sortOrder ASC, createdAt ASC"
            };
            let mut statement = db.prepare(sql)?;
            let read = |row: &rusqlite::Row<'_>| Ok(MuxPane {
                id: row.get(0)?, mux_session_id: row.get(1)?, kind: MuxPaneKind::parse(&row.get::<_, String>(2)?),
                title: row.get(3)?, target_id: row.get(4)?, bookmark_id: row.get(5)?, cwd: row.get(6)?,
                command: row.get(7)?, launch_profile_id: row.get(8)?, sort_order: row.get(9)?, created_at: row.get(10)?, updated_at: row.get(11)?,
            });
            if let Some(id) = mux_session_id {
                statement.query_map([id], read)?.collect()
            } else {
                statement.query_map([], read)?.collect()
            }
        })
    }

    pub fn list_browser_resources(
        &self,
        mux_session_id: Option<&str>,
    ) -> Result<Vec<BrowserResource>, String> {
        self.with_conn(|db| {
            let sql = if mux_session_id.is_some() {
                "SELECT id,muxSessionId,name,sourcePaneId,bookmarkId,url,temporaryProfile,sortOrder,createdAt,updatedAt FROM browser_resources WHERE muxSessionId=? ORDER BY sortOrder ASC,createdAt ASC"
            } else {
                "SELECT id,muxSessionId,name,sourcePaneId,bookmarkId,url,temporaryProfile,sortOrder,createdAt,updatedAt FROM browser_resources ORDER BY muxSessionId ASC,sortOrder ASC,createdAt ASC"
            };
            let mut statement = db.prepare(sql)?;
            let read = |row: &rusqlite::Row<'_>| {
                Ok(BrowserResource {
                    id: row.get(0)?,
                    mux_session_id: row.get(1)?,
                    name: row.get(2)?,
                    source_pane_id: row.get(3)?,
                    bookmark_id: row.get(4)?,
                    url: row.get(5)?,
                    temporary_profile: row.get::<_, i64>(6)? != 0,
                    sort_order: row.get(7)?,
                    created_at: row.get(8)?,
                    updated_at: row.get(9)?,
                })
            };
            if let Some(id) = mux_session_id {
                statement.query_map([id], read)?.collect()
            } else {
                statement.query_map([], read)?.collect()
            }
        })
    }

    pub fn ensure_session_browser_resource(
        &self,
        mux_session_id: &str,
    ) -> Result<BrowserResource, String> {
        if let Some(resource) = self
            .list_browser_resources(Some(mux_session_id))?
            .into_iter()
            .next()
        {
            return Ok(resource);
        }
        self.save_browser_resource(BrowserResourceInput {
            id: None,
            mux_session_id: mux_session_id.into(),
            name: "Browser".into(),
            source_pane_id: String::new(),
            bookmark_id: String::new(),
            url: String::new(),
            temporary_profile: false,
        })
    }

    pub fn save_browser_resource(
        &self,
        input: BrowserResourceInput,
    ) -> Result<BrowserResource, String> {
        let mux_session_id = input.mux_session_id.trim();
        let name = input.name.trim();
        let url = input.url.trim();
        if mux_session_id.is_empty() || name.is_empty() {
            return Err("浏览器资源必须属于一个会话，且名称不能为空".into());
        }
        let session_exists = self.with_conn(|db| {
            db.query_row(
                "SELECT EXISTS(SELECT 1 FROM mux_sessions WHERE id=?)",
                [mux_session_id],
                |row| row.get::<_, bool>(0),
            )
        })?;
        if !session_exists {
            return Err("Mux Session 不存在".into());
        }
        let source_pane_id = input.source_pane_id.trim();
        let bookmark_id = input.bookmark_id.trim();
        if !source_pane_id.is_empty() {
            let valid_source = self
                .list_mux_panes(Some(mux_session_id))?
                .into_iter()
                .any(|pane| {
                    pane.id == source_pane_id
                        && !pane.bookmark_id.is_empty()
                        && pane.bookmark_id == bookmark_id
                });
            if !valid_source {
                return Err("浏览器资源引用的 SSH 窗格与当前会话或目标不匹配".into());
            }
        }
        let id = input
            .id
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(|| Uuid::new_v4().to_string());
        let existing = self
            .list_browser_resources(None)?
            .into_iter()
            .collect::<Vec<_>>();
        if existing
            .iter()
            .any(|resource| resource.mux_session_id == mux_session_id && resource.id != id)
        {
            return Err("每个 Mux Session 只能关联一个浏览器资源".into());
        }
        let existing = existing.into_iter().find(|resource| resource.id == id);
        let sort_order = if let Some(existing) = &existing {
            existing.sort_order
        } else {
            self.with_conn(|db| {
                db.query_row(
                    "SELECT COALESCE(MAX(sortOrder),-1)+1 FROM browser_resources WHERE muxSessionId=?",
                    [mux_session_id],
                    |row| row.get(0),
                )
            })?
        };
        let now = Utc::now().to_rfc3339();
        let resource = BrowserResource {
            id,
            mux_session_id: mux_session_id.into(),
            name: name.into(),
            source_pane_id: source_pane_id.into(),
            bookmark_id: bookmark_id.into(),
            url: url.into(),
            temporary_profile: input.temporary_profile,
            sort_order,
            created_at: existing
                .as_ref()
                .map(|resource| resource.created_at.clone())
                .unwrap_or_else(|| now.clone()),
            updated_at: now,
        };
        self.with_conn(|db| db.execute(
            "INSERT INTO browser_resources(id,muxSessionId,name,sourcePaneId,bookmarkId,url,temporaryProfile,sortOrder,createdAt,updatedAt) VALUES(?,?,?,?,?,?,?,?,?,?) ON CONFLICT(id) DO UPDATE SET muxSessionId=excluded.muxSessionId,name=excluded.name,sourcePaneId=excluded.sourcePaneId,bookmarkId=excluded.bookmarkId,url=excluded.url,temporaryProfile=excluded.temporaryProfile,updatedAt=excluded.updatedAt",
            params![resource.id,resource.mux_session_id,resource.name,resource.source_pane_id,resource.bookmark_id,resource.url,resource.temporary_profile as i64,resource.sort_order,resource.created_at,resource.updated_at],
        ).map(|_| ()))?;
        Ok(resource)
    }

    pub fn delete_browser_resource(&self, id: &str) -> Result<(), String> {
        self.with_conn(|db| {
            db.execute("DELETE FROM browser_resources WHERE id=?", [id])?;
            Ok(())
        })
    }

    pub fn append_control_audit(&self, record: &ControlAuditRecord) -> Result<(), String> {
        let arguments_json =
            serde_json::to_string(&record.arguments).map_err(|error| error.to_string())?;
        self.with_conn(|db| db.execute(
            "INSERT INTO control_audit(id,timestamp,callerId,callerKind,operation,resourceKind,resourceId,argumentsJson,result,errorCode) VALUES(?,?,?,?,?,?,?,?,?,?)",
            params![record.id,record.timestamp,record.caller_id,record.caller_kind,record.operation,record.resource_kind,record.resource_id,arguments_json,record.result,record.error_code],
        ).map(|_| ()))
    }

    pub fn list_control_audit(&self, limit: usize) -> Result<Vec<ControlAuditRecord>, String> {
        self.with_conn(|db| {
            let mut statement = db.prepare("SELECT id,timestamp,callerId,callerKind,operation,resourceKind,resourceId,argumentsJson,result,errorCode FROM control_audit ORDER BY timestamp DESC LIMIT ?")?;
            statement.query_map([limit.clamp(1, 10_000) as i64], |row| {
                let arguments_json = row.get::<_, String>(7)?;
                Ok(ControlAuditRecord {
                    id: row.get(0)?, timestamp: row.get(1)?, caller_id: row.get(2)?, caller_kind: row.get(3)?,
                    operation: row.get(4)?, resource_kind: row.get(5)?, resource_id: row.get(6)?,
                    arguments: serde_json::from_str(&arguments_json).unwrap_or_else(|_| serde_json::json!({})),
                    result: row.get(8)?, error_code: row.get(9)?,
                })
            })?.collect()
        })
    }

    pub fn clear_control_audit(&self) -> Result<usize, String> {
        self.with_conn(|db| db.execute("DELETE FROM control_audit", []))
    }

    pub fn prune_control_audit(&self, retention_days: i64) -> Result<usize, String> {
        let cutoff = (Utc::now() - chrono::Duration::days(retention_days.max(1))).to_rfc3339();
        self.with_conn(|db| db.execute("DELETE FROM control_audit WHERE timestamp < ?", [cutoff]))
    }

    pub fn save_mux_pane(&self, input: MuxPaneInput) -> Result<MuxPane, String> {
        let mux_session_id = input.mux_session_id.trim();
        let title = input.title.trim();
        if mux_session_id.is_empty() || title.is_empty() {
            return Err("Pane 必须属于一个 Session，且名称不能为空".into());
        }
        if input.kind == MuxPaneKind::Browser {
            return Err("浏览器是会话级资源，不能保存为 Pane".into());
        }
        if input.kind == MuxPaneKind::Terminal && input.target_id.trim().is_empty() {
            return Err("Terminal Pane 必须指定 targetId".into());
        }
        let session_exists = self.with_conn(|db| {
            db.query_row(
                "SELECT EXISTS(SELECT 1 FROM mux_sessions WHERE id=?)",
                [mux_session_id],
                |row| row.get::<_, bool>(0),
            )
        })?;
        if !session_exists {
            return Err("Mux Session 不存在".into());
        }
        let now = Utc::now().to_rfc3339();
        let id = input
            .id
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(|| Uuid::new_v4().to_string());
        let existing = self
            .list_mux_panes(None)?
            .into_iter()
            .find(|pane| pane.id == id);
        let sort_order = if let Some(existing) = &existing {
            existing.sort_order
        } else {
            self.with_conn(|db| {
                db.query_row(
                    "SELECT COALESCE(MAX(sortOrder), -1) + 1 FROM mux_panes WHERE muxSessionId=?",
                    [mux_session_id],
                    |row| row.get(0),
                )
            })?
        };
        let pane = MuxPane {
            id,
            mux_session_id: mux_session_id.into(),
            kind: input.kind,
            title: title.into(),
            target_id: input.target_id.trim().into(),
            bookmark_id: input.bookmark_id.trim().into(),
            cwd: input.cwd.trim().into(),
            command: input.command.trim().into(),
            launch_profile_id: input.launch_profile_id.trim().into(),
            sort_order,
            created_at: existing
                .as_ref()
                .map(|pane| pane.created_at.clone())
                .unwrap_or_else(|| now.clone()),
            updated_at: now,
        };
        self.with_conn(|db| db.execute("INSERT INTO mux_panes(id,muxSessionId,kind,title,targetId,bookmarkId,cwd,command,launchProfileId,sortOrder,createdAt,updatedAt) VALUES(?,?,?,?,?,?,?,?,?,?,?,?) ON CONFLICT(id) DO UPDATE SET muxSessionId=excluded.muxSessionId,kind=excluded.kind,title=excluded.title,targetId=excluded.targetId,bookmarkId=excluded.bookmarkId,cwd=excluded.cwd,command=excluded.command,launchProfileId=excluded.launchProfileId,updatedAt=excluded.updatedAt", params![pane.id,pane.mux_session_id,pane.kind.as_str(),pane.title,pane.target_id,pane.bookmark_id,pane.cwd,pane.command,pane.launch_profile_id,pane.sort_order,pane.created_at,pane.updated_at]).map(|_| ()))?;
        Ok(pane)
    }

    pub fn delete_mux_pane(&self, id: &str) -> Result<(), String> {
        self.with_conn(|db| {
            db.execute("DELETE FROM mux_panes WHERE id=?", [id])?;
            Ok(())
        })
    }

    pub fn get_bookmark(&self, id: &str) -> Result<Option<Bookmark>, String> {
        Ok(self
            .list_bookmarks()?
            .into_iter()
            .find(|bookmark| bookmark.id == id))
    }

    pub fn save_bookmark(&self, mut input: BookmarkInput) -> Result<Bookmark, String> {
        input.name = input.name.trim().to_string();
        input.host = input.host.trim().to_string();
        input.username = input.username.trim().to_string();
        input.private_key_path = input.private_key_path.trim().to_string();
        input.jump_bookmark_id = input.jump_bookmark_id.trim().to_string();
        input.group_name = input.group_name.trim().to_string();
        input.note = input.note.trim().to_string();
        if input.name.is_empty() || input.host.is_empty() || input.username.is_empty() {
            return Err("连接名称、主机和用户名不能为空".into());
        }
        if let Some(id) = &input.id {
            if input.jump_bookmark_id == *id {
                return Err("连接不能将自身设置为跳板机".into());
            }
        }
        if !input.jump_bookmark_id.is_empty()
            && self.get_bookmark(&input.jump_bookmark_id)?.is_none()
        {
            return Err("选择的跳板机连接不存在".into());
        }
        let now = Utc::now().to_rfc3339();
        let existing = input
            .id
            .as_deref()
            .map(|id| self.get_bookmark(id))
            .transpose()?
            .flatten();
        let next_sort_order = if existing.is_none() {
            self.with_conn(|db| {
                db.query_row(
                    "SELECT COALESCE(MAX(sortOrder), -1) + 1 FROM bookmarks",
                    [],
                    |row| row.get(0),
                )
            })?
        } else {
            0
        };
        if existing
            .as_ref()
            .is_some_and(|item| item.auth_type != input.auth_type)
        {
            self.forget_credential(existing.as_ref().unwrap().id.as_str())?;
        }
        let bookmark = Bookmark {
            id: input.id.unwrap_or_else(|| Uuid::new_v4().to_string()),
            name: input.name,
            host: input.host,
            port: input.port,
            username: input.username,
            auth_type: input.auth_type,
            private_key_path: input.private_key_path,
            jump_bookmark_id: input.jump_bookmark_id,
            group_name: input.group_name,
            favorite: input.favorite,
            last_connected_at: existing
                .as_ref()
                .map(|item| item.last_connected_at.clone())
                .unwrap_or_default(),
            keepalive_enabled: input.keepalive_enabled,
            keepalive_interval_seconds: input.keepalive_interval_seconds.clamp(5, 300),
            keepalive_count_max: input.keepalive_count_max.clamp(1, 10),
            note: input.note,
            has_saved_credential: existing
                .as_ref()
                .is_some_and(|item| item.has_saved_credential),
            sort_order: existing
                .as_ref()
                .map(|item| item.sort_order)
                .unwrap_or(next_sort_order),
            created_at: existing
                .map(|item| item.created_at)
                .unwrap_or_else(|| now.clone()),
            updated_at: now,
        };
        self.with_conn(|db| db.execute(r#"INSERT INTO bookmarks(id,name,host,port,username,authType,privateKeyPath,jumpBookmarkId,note,groupName,favorite,lastConnectedAt,keepaliveEnabled,keepaliveIntervalSeconds,keepaliveCountMax,sortOrder,createdAt,updatedAt)
            VALUES(?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?) ON CONFLICT(id) DO UPDATE SET name=excluded.name,host=excluded.host,port=excluded.port,username=excluded.username,authType=excluded.authType,privateKeyPath=excluded.privateKeyPath,jumpBookmarkId=excluded.jumpBookmarkId,note=excluded.note,groupName=excluded.groupName,favorite=excluded.favorite,keepaliveEnabled=excluded.keepaliveEnabled,keepaliveIntervalSeconds=excluded.keepaliveIntervalSeconds,keepaliveCountMax=excluded.keepaliveCountMax,updatedAt=excluded.updatedAt"#,
            params![bookmark.id,bookmark.name,bookmark.host,bookmark.port,bookmark.username,bookmark.auth_type.as_str(),bookmark.private_key_path,bookmark.jump_bookmark_id,bookmark.note,bookmark.group_name,bookmark.favorite as i64,bookmark.last_connected_at,bookmark.keepalive_enabled as i64,bookmark.keepalive_interval_seconds,bookmark.keepalive_count_max,bookmark.sort_order,bookmark.created_at,bookmark.updated_at]).map(|_| ()))?;
        Ok(bookmark)
    }

    pub fn import_bookmark_archive(
        &self,
        entries: &[BookmarkArchiveEntry],
        selected_groups: &[String],
    ) -> Result<BookmarkArchiveImportResult, String> {
        let existing_groups = self.list_bookmark_groups()?;
        let existing_named_groups = existing_groups
            .iter()
            .filter(|group| !group.is_empty())
            .cloned()
            .collect::<std::collections::HashSet<_>>();
        let mut groups = existing_groups;
        for group in selected_groups
            .iter()
            .chain(entries.iter().map(|entry| &entry.group_name))
        {
            let group = group.trim();
            if !group.is_empty() && !groups.iter().any(|existing| existing == group) {
                groups.push(group.to_string());
            }
        }
        let imported_groups = groups
            .iter()
            .filter(|group| !group.is_empty() && !existing_named_groups.contains(*group))
            .count();
        let group_json = serde_json::to_string(&groups).map_err(|error| error.to_string())?;
        let id_map = entries
            .iter()
            .map(|entry| (entry.id.clone(), Uuid::new_v4().to_string()))
            .collect::<std::collections::HashMap<_, _>>();
        let now = Utc::now().to_rfc3339();

        self.with_conn(|db| {
            let transaction = db.transaction()?;
            let first_sort_order = transaction.query_row(
                "SELECT COALESCE(MAX(sortOrder), -1) + 1 FROM bookmarks",
                [],
                |row| row.get::<_, i64>(0),
            )?;
            for (index, entry) in entries.iter().enumerate() {
                let id = id_map.get(&entry.id).expect("archive ID was mapped");
                let jump_bookmark_id = id_map
                    .get(&entry.jump_bookmark_id)
                    .cloned()
                    .unwrap_or_default();
                transaction.execute(
                    r#"INSERT INTO bookmarks(id,name,host,port,username,authType,privateKeyPath,jumpBookmarkId,note,groupName,favorite,lastConnectedAt,keepaliveEnabled,keepaliveIntervalSeconds,keepaliveCountMax,sortOrder,createdAt,updatedAt)
                    VALUES(?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?)"#,
                    params![
                        id,
                        entry.name.trim(),
                        entry.host.trim(),
                        entry.port,
                        entry.username.trim(),
                        entry.auth_type.as_str(),
                        entry.private_key_path.trim(),
                        jump_bookmark_id,
                        entry.note.trim(),
                        entry.group_name.trim(),
                        entry.favorite as i64,
                        "",
                        entry.keepalive_enabled as i64,
                        entry.keepalive_interval_seconds.clamp(5, 300),
                        entry.keepalive_count_max.clamp(1, 10),
                        first_sort_order + index as i64,
                        now,
                        now,
                    ],
                )?;
            }
            transaction.execute(
                "INSERT OR REPLACE INTO settings(key,value) VALUES('bookmarkGroups',?)",
                [group_json],
            )?;
            transaction.commit()
        })?;

        Ok(BookmarkArchiveImportResult {
            imported_connections: entries.len(),
            imported_groups,
        })
    }

    pub fn import_luna_remote_snapshot(
        &self,
        snapshot: &LunaRemoteSnapshot,
        entries: &[BookmarkArchiveEntry],
        selected_groups: &[String],
        import_host_keys: bool,
        import_settings: bool,
        import_forwarding_profiles: bool,
        import_credentials: bool,
        source_credential_services: &[&str],
    ) -> Result<LunaRemoteImportResult, String> {
        let existing_groups = self.list_bookmark_groups()?;
        let existing_named_groups = existing_groups
            .iter()
            .filter(|group| !group.is_empty())
            .cloned()
            .collect::<HashSet<_>>();
        let mut groups = existing_groups;
        for group in selected_groups
            .iter()
            .chain(entries.iter().map(|entry| &entry.group_name))
        {
            let group = group.trim();
            if !group.is_empty() && !groups.iter().any(|existing| existing == group) {
                groups.push(group.to_string());
            }
        }
        let imported_groups = groups
            .iter()
            .filter(|group| !group.is_empty() && !existing_named_groups.contains(*group))
            .count();
        let group_json = serde_json::to_string(&groups).map_err(|error| error.to_string())?;
        let connection_id_map = entries
            .iter()
            .map(|entry| (entry.id.clone(), Uuid::new_v4().to_string()))
            .collect::<HashMap<_, _>>();
        let selected_ids = connection_id_map.keys().cloned().collect::<HashSet<_>>();
        let source_credential_ids = snapshot
            .credential_connection_ids
            .iter()
            .map(String::as_str)
            .collect::<HashSet<_>>();
        let mut credential_values = HashMap::new();
        let mut unavailable_credentials = 0;
        if import_credentials {
            for entry in entries {
                if !source_credential_ids.contains(entry.id.as_str()) {
                    continue;
                }
                let secret = source_credential_services.iter().find_map(|service| {
                    Entry::new(service, &entry.id)
                        .ok()
                        .and_then(|item| item.get_password().ok())
                        .filter(|value| !value.is_empty())
                });
                if let Some(secret) = secret {
                    credential_values.insert(entry.id.clone(), secret);
                } else {
                    unavailable_credentials += 1;
                }
            }
        }
        let forwarding_profiles = if import_forwarding_profiles {
            snapshot
                .forwarding_profiles
                .iter()
                .filter(|profile| selected_ids.contains(&profile.bookmark_id))
                .map(|profile| PortForwardProfile {
                    id: Uuid::new_v4().to_string(),
                    bookmark_id: connection_id_map
                        .get(&profile.bookmark_id)
                        .expect("selected forwarding profile connection was mapped")
                        .clone(),
                    name: profile.name.clone(),
                    forward_type: profile.forward_type.clone(),
                    bind_address: profile.bind_address.clone(),
                    bind_port: profile.bind_port,
                    target_host: profile.target_host.clone(),
                    target_port: profile.target_port,
                })
                .collect::<Vec<_>>()
        } else {
            Vec::new()
        };

        let mut created_credential_accounts: Vec<String> = Vec::new();
        for (source_id, secret) in &credential_values {
            let target_id = connection_id_map
                .get(source_id)
                .expect("credential connection was mapped");
            let write_result = Entry::new(&self.credential_service, target_id)
                .map_err(|error| error.to_string())
                .and_then(|entry| {
                    entry
                        .set_password(secret)
                        .map_err(|error| error.to_string())
                });
            if let Err(error) = write_result {
                for account in created_credential_accounts {
                    if let Ok(entry) = Entry::new(&self.credential_service, &account) {
                        let _ = entry.delete_credential();
                    }
                }
                return Err(error);
            }
            created_credential_accounts.push(target_id.clone());
        }

        let now = Utc::now().to_rfc3339();
        let database_result = self.with_conn(|db| {
            let transaction = db.transaction()?;
            let first_sort_order = transaction.query_row(
                "SELECT COALESCE(MAX(sortOrder), -1) + 1 FROM bookmarks",
                [],
                |row| row.get::<_, i64>(0),
            )?;
            for (index, entry) in entries.iter().enumerate() {
                let id = connection_id_map
                    .get(&entry.id)
                    .expect("snapshot connection ID was mapped");
                let jump_bookmark_id = connection_id_map
                    .get(&entry.jump_bookmark_id)
                    .cloned()
                    .unwrap_or_default();
                transaction.execute(
                    r#"INSERT INTO bookmarks(id,name,host,port,username,authType,privateKeyPath,jumpBookmarkId,note,groupName,favorite,lastConnectedAt,keepaliveEnabled,keepaliveIntervalSeconds,keepaliveCountMax,sortOrder,createdAt,updatedAt)
                    VALUES(?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?)"#,
                    params![
                        id,
                        entry.name.trim(),
                        entry.host.trim(),
                        entry.port,
                        entry.username.trim(),
                        entry.auth_type.as_str(),
                        entry.private_key_path.trim(),
                        jump_bookmark_id,
                        entry.note.trim(),
                        entry.group_name.trim(),
                        entry.favorite as i64,
                        "",
                        entry.keepalive_enabled as i64,
                        entry.keepalive_interval_seconds.clamp(5, 300),
                        entry.keepalive_count_max.clamp(1, 10),
                        first_sort_order + index as i64,
                        now,
                        now,
                    ],
                )?;
                if credential_values.contains_key(&entry.id) {
                    transaction.execute(
                        "INSERT INTO credential_refs(bookmarkId,account) VALUES(?,?)",
                        params![id, id],
                    )?;
                }
            }
            transaction.execute(
                "INSERT OR REPLACE INTO settings(key,value) VALUES('bookmarkGroups',?)",
                [&group_json],
            )?;
            if import_host_keys {
                for entry in &snapshot.known_hosts {
                    transaction.execute(
                        "INSERT OR REPLACE INTO known_hosts(host,port,fingerprint) VALUES(?,?,?)",
                        params![entry.host.trim(), entry.port, entry.fingerprint.trim()],
                    )?;
                }
            }
            if import_settings {
                for entry in &snapshot.settings {
                    transaction.execute(
                        "INSERT OR REPLACE INTO settings(key,value) VALUES(?,?)",
                        params![entry.key, entry.value],
                    )?;
                }
            }
            if import_forwarding_profiles && !forwarding_profiles.is_empty() {
                let current = transaction
                    .query_row(
                        "SELECT value FROM settings WHERE key='portForwardProfiles'",
                        [],
                        |row| row.get::<_, String>(0),
                    )
                    .optional()?
                    .and_then(|value| serde_json::from_str::<Vec<PortForwardProfile>>(&value).ok())
                    .unwrap_or_default();
                let merged = forwarding_profiles
                    .iter()
                    .cloned()
                    .chain(current)
                    .collect::<Vec<_>>();
                transaction.execute(
                    "INSERT OR REPLACE INTO settings(key,value) VALUES('portForwardProfiles',?)",
                    [serde_json::to_string(&merged)
                        .map_err(|error| rusqlite::Error::ToSqlConversionFailure(error.into()))?],
                )?;
            }
            transaction.commit()
        });
        if let Err(error) = database_result {
            for account in created_credential_accounts {
                if let Ok(entry) = Entry::new(&self.credential_service, &account) {
                    let _ = entry.delete_credential();
                }
            }
            return Err(error);
        }

        Ok(LunaRemoteImportResult {
            imported_connections: entries.len(),
            imported_groups,
            imported_host_keys: if import_host_keys {
                snapshot.known_hosts.len()
            } else {
                0
            },
            imported_settings: if import_settings {
                snapshot.settings.len()
            } else {
                0
            },
            imported_forwarding_profiles: forwarding_profiles.len(),
            imported_credentials: credential_values.len(),
            unavailable_credentials,
        })
    }

    pub fn reorder_bookmarks(&self, ids: &[String]) -> Result<Vec<Bookmark>, String> {
        let existing = self.list_bookmarks()?;
        let expected = existing
            .iter()
            .map(|bookmark| bookmark.id.as_str())
            .collect::<std::collections::HashSet<_>>();
        let supplied = ids
            .iter()
            .map(String::as_str)
            .collect::<std::collections::HashSet<_>>();
        if ids.len() != existing.len() || supplied != expected {
            return Err("连接排序数据已过期，请刷新后重试".into());
        }
        self.with_conn(|db| {
            let transaction = db.transaction()?;
            for (index, id) in ids.iter().enumerate() {
                transaction.execute(
                    "UPDATE bookmarks SET sortOrder=? WHERE id=?",
                    params![index as i64, id],
                )?;
            }
            transaction.commit()
        })?;
        self.list_bookmarks()
    }

    pub fn list_bookmark_groups(&self) -> Result<Vec<String>, String> {
        let stored = self.get_setting::<Vec<String>>("bookmarkGroups", vec![]);
        let mut groups = Vec::new();
        for group in stored.into_iter().chain(
            self.list_bookmarks()?
                .into_iter()
                .map(|bookmark| bookmark.group_name),
        ) {
            let group = group.trim().to_string();
            if !groups.contains(&group) {
                groups.push(group);
            }
        }
        Ok(groups)
    }

    pub fn create_bookmark_group(&self, name: &str) -> Result<Vec<String>, String> {
        let name = name.trim();
        if name.is_empty() || name == "未分组" {
            return Err("分组名称不能为空或“未分组”".into());
        }
        let mut groups = self.list_bookmark_groups()?;
        if groups.iter().any(|group| group == name) {
            return Err("同名分组已存在".into());
        }
        groups.push(name.to_string());
        self.set_setting("bookmarkGroups", &groups)?;
        Ok(groups)
    }

    pub fn rename_bookmark_group(
        &self,
        old_name: &str,
        new_name: &str,
    ) -> Result<Vec<String>, String> {
        let old_name = old_name.trim();
        let new_name = new_name.trim();
        if new_name.is_empty() || new_name == "未分组" {
            return Err("分组名称不能为空或“未分组”".into());
        }
        let mut groups = self.list_bookmark_groups()?;
        if !groups.iter().any(|group| group == old_name) {
            return Err("要重命名的分组不存在".into());
        }
        if old_name != new_name && groups.iter().any(|group| group == new_name) {
            return Err("同名分组已存在".into());
        }
        for group in &mut groups {
            if group == old_name {
                *group = new_name.to_string();
            }
        }
        let json = serde_json::to_string(&groups).map_err(|e| e.to_string())?;
        self.with_conn(|db| {
            let transaction = db.transaction()?;
            transaction.execute(
                "UPDATE bookmarks SET groupName=?, updatedAt=? WHERE groupName=?",
                params![new_name, Utc::now().to_rfc3339(), old_name],
            )?;
            transaction.execute(
                "INSERT OR REPLACE INTO settings(key,value) VALUES('bookmarkGroups',?)",
                [json],
            )?;
            transaction.commit()
        })?;
        Ok(groups)
    }

    pub fn delete_bookmark_group(&self, name: &str) -> Result<BookmarkGroupDeleteResult, String> {
        let name = name.trim();
        let mut groups = self.list_bookmark_groups()?;
        if !groups.iter().any(|group| group == name) {
            return Err("要删除的分组不存在".into());
        }
        let bookmarks = self.list_bookmarks()?;
        let deleted_bookmark_ids = bookmarks
            .iter()
            .filter(|bookmark| bookmark.group_name == name)
            .map(|bookmark| bookmark.id.clone())
            .collect::<Vec<_>>();
        let deleted = deleted_bookmark_ids
            .iter()
            .map(String::as_str)
            .collect::<std::collections::HashSet<_>>();
        if let Some(dependent) = bookmarks.iter().find(|bookmark| {
            !deleted.contains(bookmark.id.as_str())
                && deleted.contains(bookmark.jump_bookmark_id.as_str())
        }) {
            return Err(format!(
                "连接“{}”正在使用该分组中的跳板机，请先修改该连接",
                dependent.name
            ));
        }
        groups.retain(|group| group != name);
        let json = serde_json::to_string(&groups).map_err(|e| e.to_string())?;
        self.with_conn(|db| {
            let transaction = db.transaction()?;
            transaction.execute(
                "DELETE FROM credential_refs WHERE bookmarkId IN (SELECT id FROM bookmarks WHERE groupName=?)",
                [name],
            )?;
            transaction.execute("DELETE FROM bookmarks WHERE groupName=?", [name])?;
            transaction.execute(
                "INSERT OR REPLACE INTO settings(key,value) VALUES('bookmarkGroups',?)",
                [json],
            )?;
            transaction.commit()
        })?;
        for id in &deleted_bookmark_ids {
            if let Ok(entry) = Entry::new(&self.credential_service, id) {
                let _ = entry.delete_credential();
            }
        }
        Ok(BookmarkGroupDeleteResult {
            groups,
            deleted_bookmark_ids,
        })
    }

    pub fn reorder_bookmark_groups(&self, groups: &[String]) -> Result<Vec<String>, String> {
        let existing = self.list_bookmark_groups()?;
        let normalized = groups
            .iter()
            .map(|group| group.trim().to_string())
            .collect::<Vec<_>>();
        let expected = existing
            .iter()
            .map(String::as_str)
            .collect::<std::collections::HashSet<_>>();
        let supplied = normalized
            .iter()
            .map(String::as_str)
            .collect::<std::collections::HashSet<_>>();
        if normalized.len() != existing.len() || supplied != expected {
            return Err("分组排序数据已过期，请刷新后重试".into());
        }
        self.set_setting("bookmarkGroups", &normalized)?;
        Ok(normalized)
    }

    pub fn move_bookmark_to_group(
        &self,
        id: &str,
        group_name: &str,
    ) -> Result<Vec<Bookmark>, String> {
        let group_name = group_name.trim();
        if !self
            .list_bookmark_groups()?
            .iter()
            .any(|group| group == group_name)
        {
            return Err("目标分组不存在".into());
        }
        let changed = self.with_conn(|db| {
            db.execute(
                "UPDATE bookmarks SET groupName=?, updatedAt=? WHERE id=?",
                params![group_name, Utc::now().to_rfc3339(), id],
            )
        })?;
        if changed == 0 {
            return Err("要移动的连接不存在".into());
        }
        self.list_bookmarks()
    }

    pub fn remove_bookmark(&self, id: &str) -> Result<(), String> {
        self.forget_credential(id)?;
        self.with_conn(|db| {
            db.execute("DELETE FROM bookmarks WHERE id=?", [id])
                .map(|_| ())
        })
    }

    pub fn save_credential(&self, bookmark_id: &str, secret: &str) -> Result<(), String> {
        Entry::new(&self.credential_service, bookmark_id)
            .map_err(|e| e.to_string())?
            .set_password(secret)
            .map_err(|e| e.to_string())?;
        self.with_conn(|db| {
            db.execute(
                "INSERT OR REPLACE INTO credential_refs(bookmarkId,account) VALUES(?,?)",
                params![bookmark_id, bookmark_id],
            )
            .map(|_| ())
        })
    }

    pub fn get_credential(&self, bookmark_id: &str) -> Option<String> {
        let exists = self
            .with_conn(|db| {
                db.query_row(
                    "SELECT 1 FROM credential_refs WHERE bookmarkId=?",
                    [bookmark_id],
                    |_| Ok(()),
                )
                .optional()
            })
            .ok()
            .flatten()
            .is_some();
        if !exists {
            return None;
        }
        self.get_password(bookmark_id)
    }

    pub fn forget_credential(&self, bookmark_id: &str) -> Result<(), String> {
        for service in self.credential_services() {
            if let Ok(entry) = Entry::new(service, bookmark_id) {
                let _ = entry.delete_credential();
            }
        }
        self.with_conn(|db| {
            db.execute(
                "DELETE FROM credential_refs WHERE bookmarkId=?",
                [bookmark_id],
            )
            .map(|_| ())
        })
    }

    pub fn save_ai_api_key(&self, api_key: &str) -> Result<(), String> {
        Entry::new(&self.credential_service, AI_API_KEY_ACCOUNT)
            .map_err(|e| e.to_string())?
            .set_password(api_key)
            .map_err(|e| e.to_string())
    }

    pub fn get_ai_api_key(&self) -> Option<String> {
        self.get_password(AI_API_KEY_ACCOUNT)
    }

    pub fn delete_ai_api_key(&self) {
        for service in self.credential_services() {
            if let Ok(entry) = Entry::new(service, AI_API_KEY_ACCOUNT) {
                let _ = entry.delete_credential();
            }
        }
    }

    fn credential_services(&self) -> impl Iterator<Item = &str> {
        std::iter::once(self.credential_service.as_str())
            .chain(self.legacy_credential_service.as_deref())
    }

    fn get_password(&self, account: &str) -> Option<String> {
        if let Some(value) = Entry::new(&self.credential_service, account)
            .ok()
            .and_then(|entry| entry.get_password().ok())
            .filter(|value| !value.is_empty())
        {
            return Some(value);
        }
        let legacy_service = self.legacy_credential_service.as_deref()?;
        let value = Entry::new(legacy_service, account)
            .ok()?
            .get_password()
            .ok()
            .filter(|value| !value.is_empty())?;
        if let Ok(entry) = Entry::new(&self.credential_service, account) {
            let _ = entry.set_password(&value);
        }
        Some(value)
    }

    pub fn mark_connected(&self, id: &str) -> Result<(), String> {
        self.with_conn(|db| {
            db.execute(
                "UPDATE bookmarks SET lastConnectedAt=? WHERE id=?",
                params![Utc::now().to_rfc3339(), id],
            )
            .map(|_| ())
        })
    }
    pub fn known_host(&self, host: &str, port: u16) -> Result<Option<String>, String> {
        self.with_conn(|db| {
            db.query_row(
                "SELECT fingerprint FROM known_hosts WHERE host=? AND port=?",
                params![host, port],
                |row| row.get(0),
            )
            .optional()
        })
    }
    pub fn trust_host(&self, host: &str, port: u16, fingerprint: &str) -> Result<(), String> {
        self.with_conn(|db| {
            db.execute(
                "INSERT OR REPLACE INTO known_hosts(host,port,fingerprint) VALUES(?,?,?)",
                params![host, port, fingerprint],
            )
            .map(|_| ())
        })
    }

    pub fn get_setting<T: DeserializeOwned>(&self, key: &str, fallback: T) -> T {
        self.with_conn(|db| {
            db.query_row("SELECT value FROM settings WHERE key=?", [key], |row| {
                row.get::<_, String>(0)
            })
            .optional()
        })
        .ok()
        .flatten()
        .and_then(|value| serde_json::from_str(&value).ok())
        .unwrap_or(fallback)
    }
    pub fn set_setting<T: Serialize>(&self, key: &str, value: &T) -> Result<(), String> {
        let json = serde_json::to_string(value).map_err(|e| e.to_string())?;
        self.with_conn(|db| {
            db.execute(
                "INSERT OR REPLACE INTO settings(key,value) VALUES(?,?)",
                params![key, json],
            )
            .map(|_| ())
        })
    }

    pub fn list_ai_command_history(&self) -> Vec<AiCommandHistoryEntry> {
        self.get_setting(AI_COMMAND_HISTORY_KEY, Vec::new())
    }

    pub fn add_ai_command_history(&self, entry: AiCommandHistoryEntry) -> Result<(), String> {
        let mut history = self.list_ai_command_history();
        history.insert(0, entry);
        history.truncate(AI_COMMAND_HISTORY_LIMIT);
        self.set_setting(AI_COMMAND_HISTORY_KEY, &history)
    }

    pub fn clear_ai_command_history(&self) -> Result<(), String> {
        self.set_setting(AI_COMMAND_HISTORY_KEY, &Vec::<AiCommandHistoryEntry>::new())
    }

    pub fn list_transfers(&self) -> Result<Vec<TransferTask>, String> {
        self.with_conn(|db| {
            let mut statement = db.prepare("SELECT id,sessionId,bookmarkId,direction,sourcePath,destinationPath,displayName,status,bytesTotal,bytesTransferred,speed,error,createdAt,updatedAt FROM transfers ORDER BY createdAt DESC LIMIT 500")?;
            statement.query_map([], |row| Ok(TransferTask {
                id: row.get(0)?, session_id: row.get(1)?, bookmark_id: row.get(2)?, direction: if row.get::<_, String>(3)? == "download" { TransferDirection::Download } else { TransferDirection::Upload },
                source_path: row.get(4)?, destination_path: row.get(5)?, display_name: row.get(6)?, status: match row.get::<_, String>(7)?.as_str() { "scanning" => TransferStatus::Scanning, "running" => TransferStatus::Running, "conflict" => TransferStatus::Conflict, "completed" => TransferStatus::Completed, "failed" => TransferStatus::Failed, "cancelled" => TransferStatus::Cancelled, "interrupted" => TransferStatus::Interrupted, _ => TransferStatus::Queued },
                bytes_total: row.get(8)?, bytes_transferred: row.get(9)?, speed: row.get(10)?, error: row.get(11)?, created_at: row.get(12)?, updated_at: row.get(13)?,
            }))?.collect()
        })
    }

    pub fn save_transfer(&self, task: &TransferTask) -> Result<(), String> {
        self.with_conn(|db| db.execute(r#"INSERT INTO transfers(id,sessionId,bookmarkId,direction,sourcePath,destinationPath,displayName,status,bytesTotal,bytesTransferred,speed,error,createdAt,updatedAt) VALUES(?,?,?,?,?,?,?,?,?,?,?,?,?,?)
            ON CONFLICT(id) DO UPDATE SET sessionId=excluded.sessionId,bookmarkId=excluded.bookmarkId,destinationPath=excluded.destinationPath,status=excluded.status,bytesTotal=excluded.bytesTotal,bytesTransferred=excluded.bytesTransferred,speed=excluded.speed,error=excluded.error,updatedAt=excluded.updatedAt"#,
            params![task.id,task.session_id,task.bookmark_id,if task.direction==TransferDirection::Download{"download"}else{"upload"},task.source_path,task.destination_path,task.display_name,task.status.as_str(),task.bytes_total,task.bytes_transferred,task.speed,task.error,task.created_at,task.updated_at]).map(|_| ()))
    }
    pub fn clear_completed(&self) -> Result<(), String> {
        self.with_conn(|db| {
            db.execute(
                "DELETE FROM transfers WHERE status IN ('completed','cancelled')",
                [],
            )
            .map(|_| ())
        })
    }
}

#[cfg(test)]
mod tests {
    use super::Database;
    use crate::models::{
        AiCommandHistoryEntry, AiRiskLevel, AiShell, AuthType, BookmarkArchiveEntry, BookmarkInput,
        BrowserResourceInput, ControlAuditRecord, MuxPaneInput, MuxPaneKind, MuxSessionInput,
        MuxSplitDirection, MuxSplitNode, PortForwardProfile,
    };
    use chrono::Utc;
    use rusqlite::OptionalExtension;
    use uuid::Uuid;

    fn input(name: &str, group_name: &str) -> BookmarkInput {
        BookmarkInput {
            id: None,
            name: name.into(),
            host: format!("{}.example.com", name.to_lowercase()),
            port: 22,
            username: "test".into(),
            auth_type: AuthType::Agent,
            private_key_path: String::new(),
            jump_bookmark_id: String::new(),
            group_name: group_name.into(),
            favorite: false,
            keepalive_enabled: true,
            keepalive_interval_seconds: 15,
            keepalive_count_max: 3,
            note: String::new(),
        }
    }

    #[test]
    fn persists_mux_sessions_panes_and_layout() {
        let path = std::env::temp_dir().join(format!(
            "{}-mux-session-{}.db",
            crate::product::PRODUCT_KEY,
            Uuid::new_v4()
        ));
        let credential_service = format!("{}.mux-session-test", crate::product::CREDENTIAL_SERVICE);
        let db = Database::open(&path, &credential_service).expect("open test database");
        let mut session = db
            .save_mux_session(MuxSessionInput {
                id: None,
                name: "Project Alpha".into(),
                root_path: "D:/code/alpha".into(),
                layout: None,
            })
            .expect("save mux session");
        let local = db
            .save_mux_pane(MuxPaneInput {
                id: None,
                mux_session_id: session.id.clone(),
                kind: MuxPaneKind::Terminal,
                title: "PowerShell".into(),
                target_id: "local:powershell".into(),
                bookmark_id: String::new(),
                cwd: session.root_path.clone(),
                command: String::new(),
                launch_profile_id: String::new(),
            })
            .expect("save local pane");
        let ssh = db
            .save_mux_pane(MuxPaneInput {
                id: None,
                mux_session_id: session.id.clone(),
                kind: MuxPaneKind::Terminal,
                title: "Build host".into(),
                target_id: "ssh:bookmark-1".into(),
                bookmark_id: "bookmark-1".into(),
                cwd: String::new(),
                command: "codex".into(),
                launch_profile_id: "codex.default".into(),
            })
            .expect("save ssh pane");
        session.layout = Some(MuxSplitNode::Split {
            direction: MuxSplitDirection::Horizontal,
            ratio: 0.5,
            first: Box::new(MuxSplitNode::Pane {
                pane_id: local.id.clone(),
            }),
            second: Box::new(MuxSplitNode::Pane {
                pane_id: ssh.id.clone(),
            }),
        });
        db.save_mux_session(MuxSessionInput {
            id: Some(session.id.clone()),
            name: session.name.clone(),
            root_path: session.root_path.clone(),
            layout: session.layout.clone(),
        })
        .expect("save session layout");

        drop(db);
        let db = Database::open(&path, &credential_service).expect("reopen test database");
        let sessions = db.list_mux_sessions().expect("list mux sessions");
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].layout, session.layout);
        let panes = db
            .list_mux_panes(Some(&session.id))
            .expect("list mux panes");
        assert_eq!(panes.len(), 2);
        assert_eq!(panes[0].id, local.id);
        assert_eq!(panes[1].id, ssh.id);
        let browser = db
            .list_browser_resources(Some(&session.id))
            .expect("list implicit browser resources");
        assert_eq!(browser.len(), 1);
        assert_eq!(browser[0].name, "Browser");
        assert_eq!(
            db.ensure_session_browser_resource(&session.id)
                .expect("reuse implicit browser resource")
                .id,
            browser[0].id
        );

        db.delete_mux_session(&session.id)
            .expect("delete mux session");
        assert!(db.list_mux_panes(None).expect("list panes").is_empty());

        drop(db);
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(path.with_extension("db-wal"));
        let _ = std::fs::remove_file(path.with_extension("db-shm"));
    }

    #[test]
    fn persists_browser_resource_updates_and_enforces_one_per_session() {
        let path = std::env::temp_dir().join(format!(
            "{}-browser-resource-{}.db",
            crate::product::PRODUCT_KEY,
            Uuid::new_v4()
        ));
        let database = Database::open(
            &path,
            &format!(
                "{}.browser-resource-test",
                crate::product::CREDENTIAL_SERVICE
            ),
        )
        .unwrap();
        let session = database
            .save_mux_session(MuxSessionInput {
                id: None,
                name: "Browser Session".into(),
                root_path: String::new(),
                layout: None,
            })
            .unwrap();
        let browser = database
            .save_browser_resource(BrowserResourceInput {
                id: None,
                mux_session_id: session.id.clone(),
                name: "Browser".into(),
                source_pane_id: String::new(),
                bookmark_id: String::new(),
                url: String::new(),
                temporary_profile: false,
            })
            .unwrap();
        assert!(browser.url.is_empty());
        assert!(
            database
                .save_browser_resource(BrowserResourceInput {
                    id: None,
                    mux_session_id: session.id.clone(),
                    name: "Second Browser".into(),
                    source_pane_id: String::new(),
                    bookmark_id: String::new(),
                    url: String::new(),
                    temporary_profile: false,
                })
                .unwrap_err()
                .contains("只能关联一个浏览器资源")
        );
        let renamed = database
            .save_browser_resource(BrowserResourceInput {
                id: Some(browser.id.clone()),
                mux_session_id: session.id.clone(),
                name: "Renamed Browser".into(),
                source_pane_id: String::new(),
                bookmark_id: String::new(),
                url: String::new(),
                temporary_profile: false,
            })
            .unwrap();
        assert_eq!(renamed.id, browser.id);
        assert_eq!(renamed.name, "Renamed Browser");
        assert_eq!(
            database.list_browser_resources(Some(&session.id)).unwrap(),
            vec![renamed]
        );
        drop(database);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn prunes_control_audit_older_than_retention_window() {
        let path = std::env::temp_dir().join(format!(
            "{}-audit-retention-{}.db",
            crate::product::PRODUCT_KEY,
            Uuid::new_v4()
        ));
        let database = Database::open(
            &path,
            &format!("{}.audit-test", crate::product::CREDENTIAL_SERVICE),
        )
        .unwrap();
        let record = |id: &str, timestamp: String| ControlAuditRecord {
            id: id.into(),
            timestamp,
            caller_id: "agent".into(),
            caller_kind: "agent".into(),
            operation: "agents.list".into(),
            resource_kind: "agent".into(),
            resource_id: String::new(),
            arguments: serde_json::json!({}),
            result: "success".into(),
            error_code: String::new(),
        };
        database
            .append_control_audit(&record(
                "old",
                (Utc::now() - chrono::Duration::days(31)).to_rfc3339(),
            ))
            .unwrap();
        database
            .append_control_audit(&record("current", Utc::now().to_rfc3339()))
            .unwrap();
        assert_eq!(database.prune_control_audit(30).unwrap(), 1);
        assert_eq!(database.list_control_audit(10).unwrap()[0].id, "current");
        drop(database);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn persists_manual_bookmark_order() {
        let path = std::env::temp_dir().join(format!(
            "{}-{}.db",
            crate::product::PRODUCT_KEY,
            Uuid::new_v4()
        ));
        let credential_service = format!("{}.order-test", crate::product::CREDENTIAL_SERVICE);
        let db = Database::open(&path, &credential_service).expect("open test database");
        let first = db.save_bookmark(input("First", "A")).expect("save first");
        let second = db.save_bookmark(input("Second", "A")).expect("save second");
        let third = db.save_bookmark(input("Third", "B")).expect("save third");

        let reordered = db
            .reorder_bookmarks(&[third.id.clone(), first.id.clone(), second.id.clone()])
            .expect("reorder bookmarks");
        assert_eq!(
            reordered
                .into_iter()
                .map(|item| item.id)
                .collect::<Vec<_>>(),
            [third.id, first.id, second.id]
        );

        drop(db);
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(path.with_extension("db-wal"));
        let _ = std::fs::remove_file(path.with_extension("db-shm"));
    }

    #[test]
    fn keeps_only_ten_most_recent_ai_commands() {
        let path = std::env::temp_dir().join(format!(
            "{}-{}.db",
            crate::product::PRODUCT_KEY,
            Uuid::new_v4()
        ));
        let credential_service = format!("{}.ai-history-test", crate::product::CREDENTIAL_SERVICE);
        let db = Database::open(&path, &credential_service).expect("open test database");
        for index in 0..12 {
            db.add_ai_command_history(AiCommandHistoryEntry {
                id: index.to_string(),
                created_at: format!("2026-08-05T00:00:{index:02}Z"),
                requirement: format!("requirement {index}"),
                shell: AiShell::Linux,
                command: format!("command-{index}"),
                explanation: String::new(),
                assumptions: vec![],
                warnings: vec![],
                risk_level: AiRiskLevel::Low,
            })
            .expect("save history entry");
        }

        let history = db.list_ai_command_history();
        assert_eq!(history.len(), 10);
        assert_eq!(
            history.first().map(|entry| entry.command.as_str()),
            Some("command-11")
        );
        assert_eq!(
            history.last().map(|entry| entry.command.as_str()),
            Some("command-2")
        );

        db.clear_ai_command_history().expect("clear history");
        assert!(db.list_ai_command_history().is_empty());

        drop(db);
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(path.with_extension("db-wal"));
        let _ = std::fs::remove_file(path.with_extension("db-shm"));
    }

    #[test]
    fn persists_and_manages_bookmark_groups() {
        let path = std::env::temp_dir().join(format!(
            "{}-groups-{}.db",
            crate::product::PRODUCT_KEY,
            Uuid::new_v4()
        ));
        let credential_service = format!("{}.groups-test", crate::product::CREDENTIAL_SERVICE);
        let db = Database::open(&path, &credential_service).expect("open test database");
        let ungrouped = db
            .save_bookmark(input("Ungrouped", ""))
            .expect("save ungrouped");
        db.save_bookmark(input("Alpha", "A")).expect("save grouped");

        assert_eq!(db.list_bookmark_groups().expect("list groups"), ["", "A"]);
        assert_eq!(
            db.create_bookmark_group("Empty")
                .expect("create empty group"),
            ["", "A", "Empty"]
        );
        drop(db);

        let db = Database::open(&path, &credential_service).expect("reopen test database");
        assert_eq!(
            db.list_bookmark_groups().expect("list persisted groups"),
            ["", "A", "Empty"]
        );
        assert_eq!(
            db.reorder_bookmark_groups(&["Empty".into(), "".into(), "A".into()])
                .expect("reorder groups"),
            ["Empty", "", "A"]
        );
        db.move_bookmark_to_group(&ungrouped.id, "Empty")
            .expect("move bookmark to group");
        assert_eq!(
            db.get_bookmark(&ungrouped.id)
                .expect("get moved bookmark")
                .expect("moved bookmark exists")
                .group_name,
            "Empty"
        );
        assert_eq!(
            db.rename_bookmark_group("Empty", "Production")
                .expect("rename group"),
            ["Production", "", "A"]
        );
        assert_eq!(
            db.get_bookmark(&ungrouped.id)
                .expect("get renamed bookmark")
                .expect("renamed bookmark exists")
                .group_name,
            "Production"
        );
        let deleted = db
            .delete_bookmark_group("Production")
            .expect("delete group with bookmarks");
        assert_eq!(deleted.deleted_bookmark_ids, [ungrouped.id.clone()]);
        assert_eq!(deleted.groups, ["", "A"]);
        assert!(
            db.get_bookmark(&ungrouped.id)
                .expect("get deleted bookmark")
                .is_none()
        );

        drop(db);
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(path.with_extension("db-wal"));
        let _ = std::fs::remove_file(path.with_extension("db-shm"));
    }

    #[test]
    fn allows_renaming_and_deleting_ungrouped() {
        let path = std::env::temp_dir().join(format!(
            "{}-ungrouped-{}.db",
            crate::product::PRODUCT_KEY,
            Uuid::new_v4()
        ));
        let credential_service = format!("{}.ungrouped-test", crate::product::CREDENTIAL_SERVICE);
        let db = Database::open(&path, &credential_service).expect("open test database");
        let first = db.save_bookmark(input("First", "")).expect("save first");
        assert_eq!(
            db.rename_bookmark_group("", "Default")
                .expect("rename ungrouped"),
            ["Default"]
        );
        assert_eq!(
            db.get_bookmark(&first.id)
                .expect("get renamed bookmark")
                .expect("renamed bookmark exists")
                .group_name,
            "Default"
        );

        let second = db.save_bookmark(input("Second", "")).expect("save second");
        let deleted = db.delete_bookmark_group("").expect("delete ungrouped");
        assert_eq!(deleted.deleted_bookmark_ids, [second.id]);
        assert_eq!(deleted.groups, ["Default"]);

        drop(db);
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(path.with_extension("db-wal"));
        let _ = std::fs::remove_file(path.with_extension("db-shm"));
    }

    #[test]
    fn imports_archive_transaction_with_new_ids_and_jump_relationships() {
        let path = std::env::temp_dir().join(format!(
            "{}-archive-import-{}.db",
            crate::product::PRODUCT_KEY,
            Uuid::new_v4()
        ));
        let credential_service = format!("{}.archive-test", crate::product::CREDENTIAL_SERVICE);
        let db = Database::open(&path, &credential_service).expect("open test database");
        db.create_bookmark_group("Existing")
            .expect("create existing group");
        let entries = vec![
            BookmarkArchiveEntry {
                id: "jump-source-id".into(),
                name: "Jump".into(),
                host: "jump.example.com".into(),
                port: 22,
                username: "jump-user".into(),
                auth_type: AuthType::Agent,
                private_key_path: String::new(),
                jump_bookmark_id: String::new(),
                group_name: "Imported".into(),
                favorite: false,
                keepalive_enabled: true,
                keepalive_interval_seconds: 15,
                keepalive_count_max: 3,
                note: String::new(),
            },
            BookmarkArchiveEntry {
                id: "target-source-id".into(),
                name: "Target".into(),
                host: "target.example.com".into(),
                port: 2202,
                username: "target-user".into(),
                auth_type: AuthType::Password,
                private_key_path: String::new(),
                jump_bookmark_id: "jump-source-id".into(),
                group_name: "Imported".into(),
                favorite: true,
                keepalive_enabled: true,
                keepalive_interval_seconds: 30,
                keepalive_count_max: 4,
                note: "from archive".into(),
            },
        ];

        let result = db
            .import_bookmark_archive(&entries, &["Empty imported".into()])
            .expect("import archive");
        assert_eq!(result.imported_connections, 2);
        assert_eq!(result.imported_groups, 2);

        let bookmarks = db.list_bookmarks().expect("list imported bookmarks");
        let jump = bookmarks
            .iter()
            .find(|bookmark| bookmark.name == "Jump")
            .expect("jump imported");
        let target = bookmarks
            .iter()
            .find(|bookmark| bookmark.name == "Target")
            .expect("target imported");
        assert_ne!(jump.id, "jump-source-id");
        assert_ne!(target.id, "target-source-id");
        assert_eq!(target.jump_bookmark_id, jump.id);
        assert!(!jump.has_saved_credential);
        assert!(!target.has_saved_credential);
        assert_eq!(
            db.list_bookmark_groups().expect("list groups"),
            ["Existing", "Empty imported", "Imported"]
        );

        drop(db);
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(path.with_extension("db-wal"));
        let _ = std::fs::remove_file(path.with_extension("db-shm"));
    }

    #[test]
    fn reads_and_imports_luna_remote_snapshot_without_mutating_source() {
        let source_path = std::env::temp_dir().join(format!(
            "{}-remote-source-{}.db",
            crate::product::PRODUCT_KEY,
            Uuid::new_v4()
        ));
        let source = rusqlite::Connection::open(&source_path).expect("create source database");
        source
            .execute_batch(
                r#"
                CREATE TABLE bookmarks (
                  id TEXT PRIMARY KEY, name TEXT NOT NULL, host TEXT NOT NULL, port INTEGER NOT NULL,
                  username TEXT NOT NULL, authType TEXT NOT NULL, privateKeyPath TEXT NOT NULL DEFAULT '',
                  jumpBookmarkId TEXT NOT NULL DEFAULT '', note TEXT NOT NULL DEFAULT '', groupName TEXT NOT NULL DEFAULT '',
                  favorite INTEGER NOT NULL DEFAULT 0, lastConnectedAt TEXT NOT NULL DEFAULT '',
                  keepaliveEnabled INTEGER NOT NULL DEFAULT 1, keepaliveIntervalSeconds INTEGER NOT NULL DEFAULT 15,
                  keepaliveCountMax INTEGER NOT NULL DEFAULT 3, sortOrder INTEGER NOT NULL DEFAULT 0,
                  createdAt TEXT NOT NULL, updatedAt TEXT NOT NULL
                );
                CREATE TABLE credential_refs (bookmarkId TEXT PRIMARY KEY, account TEXT NOT NULL);
                CREATE TABLE known_hosts (host TEXT NOT NULL, port INTEGER NOT NULL, fingerprint TEXT NOT NULL, PRIMARY KEY(host, port));
                CREATE TABLE settings (key TEXT PRIMARY KEY, value TEXT NOT NULL);
                INSERT INTO bookmarks VALUES
                  ('jump','Jump','jump.example.com',22,'jump-user','agent','','','','Imported',0,'',1,15,3,0,'2026-01-01','2026-01-01'),
                  ('target','Target','target.example.com',2202,'target-user','password','','jump','note','Imported',1,'',1,30,4,1,'2026-01-01','2026-01-01');
                INSERT INTO credential_refs VALUES ('target','target');
                INSERT INTO known_hosts VALUES ('target.example.com',2202,'SHA256:test');
                INSERT INTO settings VALUES ('bookmarkGroups','["Imported","Empty"]');
                INSERT INTO settings VALUES ('uiTheme','"dark"');
                INSERT INTO settings VALUES ('portForwardProfiles','[{"id":"forward-source","bookmarkId":"target","name":"Web","type":"local","bindAddress":"127.0.0.1","bindPort":8080,"targetHost":"127.0.0.1","targetPort":80}]');
                "#,
            )
            .expect("seed source database");
        drop(source);
        let before = std::fs::read(&source_path).expect("read source before preview");

        let snapshot = Database::read_luna_remote_snapshot(&source_path).expect("read snapshot");
        assert_eq!(snapshot.connections.len(), 2);
        assert_eq!(snapshot.groups, ["Imported", "Empty"]);
        assert_eq!(snapshot.known_hosts.len(), 1);
        assert_eq!(snapshot.settings.len(), 1);
        assert_eq!(snapshot.forwarding_profiles.len(), 1);
        assert_eq!(snapshot.credential_connection_ids, ["target"]);
        assert_eq!(
            std::fs::read(&source_path).expect("read source after preview"),
            before
        );

        let target_path = std::env::temp_dir().join(format!(
            "{}-remote-target-{}.db",
            crate::product::PRODUCT_KEY,
            Uuid::new_v4()
        ));
        let credential_service =
            format!("{}.remote-import-test", crate::product::CREDENTIAL_SERVICE);
        let db = Database::open(&target_path, &credential_service).expect("open target database");
        let result = db
            .import_luna_remote_snapshot(
                &snapshot,
                &snapshot.connections,
                &["Empty".into()],
                true,
                true,
                true,
                false,
                &[],
            )
            .expect("import snapshot");
        assert_eq!(result.imported_connections, 2);
        assert_eq!(result.imported_groups, 2);
        assert_eq!(result.imported_host_keys, 1);
        assert_eq!(result.imported_settings, 1);
        assert_eq!(result.imported_forwarding_profiles, 1);
        assert_eq!(result.imported_credentials, 0);
        assert_eq!(result.unavailable_credentials, 0);

        let bookmarks = db.list_bookmarks().expect("list imported bookmarks");
        let jump = bookmarks
            .iter()
            .find(|bookmark| bookmark.name == "Jump")
            .expect("jump imported");
        let target = bookmarks
            .iter()
            .find(|bookmark| bookmark.name == "Target")
            .expect("target imported");
        assert_ne!(jump.id, "jump");
        assert_ne!(target.id, "target");
        assert_eq!(target.jump_bookmark_id, jump.id);
        assert_eq!(
            db.known_host("target.example.com", 2202)
                .expect("read host key"),
            Some("SHA256:test".into())
        );
        assert_eq!(db.get_setting::<String>("uiTheme", String::new()), "dark");
        let profiles = db.get_setting::<Vec<PortForwardProfile>>("portForwardProfiles", vec![]);
        assert_eq!(profiles.len(), 1);
        assert_ne!(profiles[0].id, "forward-source");
        assert_eq!(profiles[0].bookmark_id, target.id);
        assert_eq!(
            std::fs::read(&source_path).expect("read source after import"),
            before
        );

        drop(db);
        for path in [&source_path, &target_path] {
            let _ = std::fs::remove_file(path);
            let _ = std::fs::remove_file(path.with_extension("db-wal"));
            let _ = std::fs::remove_file(path.with_extension("db-shm"));
        }
    }

    #[test]
    fn migrates_version_nine_browser_panes_to_session_resources() {
        let path = std::env::temp_dir().join(format!(
            "{}-v9-browser-resource-{}.db",
            crate::product::PRODUCT_KEY,
            Uuid::new_v4()
        ));
        let connection = rusqlite::Connection::open(&path).expect("create version nine database");
        connection
            .execute_batch(
                r#"
                CREATE TABLE mux_sessions (
                  id TEXT PRIMARY KEY, name TEXT NOT NULL, rootPath TEXT NOT NULL DEFAULT '',
                  layoutJson TEXT NOT NULL DEFAULT '', sortOrder INTEGER NOT NULL DEFAULT 0,
                  createdAt TEXT NOT NULL, updatedAt TEXT NOT NULL
                );
                CREATE TABLE mux_panes (
                  id TEXT PRIMARY KEY, muxSessionId TEXT NOT NULL, kind TEXT NOT NULL,
                  title TEXT NOT NULL, targetId TEXT NOT NULL DEFAULT '', bookmarkId TEXT NOT NULL DEFAULT '',
                  cwd TEXT NOT NULL DEFAULT '', command TEXT NOT NULL DEFAULT '',
                  launchProfileId TEXT NOT NULL DEFAULT '', sortOrder INTEGER NOT NULL DEFAULT 0,
                  createdAt TEXT NOT NULL, updatedAt TEXT NOT NULL,
                  FOREIGN KEY(muxSessionId) REFERENCES mux_sessions(id) ON DELETE CASCADE
                );
                CREATE TABLE agent_control_grants (
                  id TEXT PRIMARY KEY, sourcePaneId TEXT NOT NULL, targetPaneId TEXT NOT NULL,
                  access TEXT NOT NULL, createdAt TEXT NOT NULL, updatedAt TEXT NOT NULL,
                  UNIQUE(sourcePaneId, targetPaneId),
                  FOREIGN KEY(sourcePaneId) REFERENCES mux_panes(id) ON DELETE CASCADE,
                  FOREIGN KEY(targetPaneId) REFERENCES mux_panes(id) ON DELETE CASCADE
                );
                INSERT INTO mux_sessions VALUES (
                  'session-v9','Legacy project','D:/code/legacy',
                  '{"type":"split","direction":"horizontal","ratio":0.4,"first":{"type":"pane","paneId":"agent-pane"},"second":{"type":"split","direction":"vertical","ratio":0.5,"first":{"type":"pane","paneId":"browser-pane"},"second":{"type":"pane","paneId":"terminal-pane"}}}',
                  0,'2026-08-01T00:00:00Z','2026-08-02T00:00:00Z'
                );
                INSERT INTO mux_panes VALUES
                  ('agent-pane','session-v9','terminal','Codex','local:powershell','','D:/code/legacy','codex','codex.default',0,'2026-08-01T00:00:00Z','2026-08-02T00:00:00Z'),
                  ('browser-pane','session-v9','browser','Preview','ssh-runtime','bookmark-1','','http://localhost:4173','',1,'2026-08-01T01:00:00Z','2026-08-02T01:00:00Z'),
                  ('terminal-pane','session-v9','terminal','Shell','local:powershell','','D:/code/legacy','','',2,'2026-08-01T02:00:00Z','2026-08-02T02:00:00Z');
                INSERT INTO agent_control_grants VALUES
                  ('grant-browser','agent-pane','browser-pane','write','2026-08-02T00:00:00Z','2026-08-02T00:00:00Z'),
                  ('grant-pane','agent-pane','terminal-pane','read','2026-08-02T00:00:00Z','2026-08-02T00:00:00Z');
                PRAGMA user_version = 9;
                "#,
            )
            .expect("create version nine schema");
        drop(connection);

        let credential_service =
            format!("{}.v9-migration-test", crate::product::CREDENTIAL_SERVICE);
        let database =
            Database::open(&path, &credential_service).expect("migrate version nine database");

        let panes = database
            .list_mux_panes(Some("session-v9"))
            .expect("list migrated panes");
        assert_eq!(
            panes
                .iter()
                .map(|pane| pane.id.as_str())
                .collect::<Vec<_>>(),
            ["agent-pane", "terminal-pane"]
        );
        let resources = database
            .list_browser_resources(Some("session-v9"))
            .expect("list migrated browser resources");
        assert_eq!(resources.len(), 1);
        assert_eq!(resources[0].id, "browser-pane");
        assert_eq!(resources[0].name, "Preview");
        assert_eq!(resources[0].source_pane_id, "ssh-runtime");
        assert_eq!(resources[0].bookmark_id, "bookmark-1");
        assert_eq!(resources[0].url, "http://localhost:4173");
        assert_eq!(resources[0].created_at, "2026-08-01T01:00:00Z");

        let session = database
            .list_mux_sessions()
            .expect("list migrated sessions")
            .into_iter()
            .find(|session| session.id == "session-v9")
            .expect("migrated session");
        assert_eq!(
            session.layout,
            Some(MuxSplitNode::Split {
                direction: MuxSplitDirection::Horizontal,
                ratio: 0.4,
                first: Box::new(MuxSplitNode::Pane {
                    pane_id: "agent-pane".into(),
                }),
                second: Box::new(MuxSplitNode::Pane {
                    pane_id: "terminal-pane".into(),
                }),
            })
        );

        let legacy_grant_table_exists = database
            .with_conn(|connection| {
                connection
                    .query_row(
                        "SELECT 1 FROM sqlite_master WHERE type='table' AND name='agent_control_grants'",
                        [],
                        |_| Ok(true),
                    )
                    .optional()
                    .map(|value| value.unwrap_or(false))
            })
            .expect("check legacy grant table");
        assert!(!legacy_grant_table_exists);

        drop(database);
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(path.with_extension("db-wal"));
        let _ = std::fs::remove_file(path.with_extension("db-shm"));
    }

    #[test]
    fn migrates_bookmark_sort_order_from_version_five() {
        let path = std::env::temp_dir().join(format!(
            "{}-v5-{}.db",
            crate::product::PRODUCT_KEY,
            Uuid::new_v4()
        ));
        let connection = rusqlite::Connection::open(&path).expect("create old database");
        connection
            .execute_batch(
                r#"
                CREATE TABLE bookmarks (
                  id TEXT PRIMARY KEY, name TEXT NOT NULL, host TEXT NOT NULL, port INTEGER NOT NULL,
                  username TEXT NOT NULL, authType TEXT NOT NULL, privateKeyPath TEXT NOT NULL DEFAULT '',
                  jumpBookmarkId TEXT NOT NULL DEFAULT '', note TEXT NOT NULL DEFAULT '', groupName TEXT NOT NULL DEFAULT '',
                  favorite INTEGER NOT NULL DEFAULT 0, lastConnectedAt TEXT NOT NULL DEFAULT '',
                  keepaliveEnabled INTEGER NOT NULL DEFAULT 1, keepaliveIntervalSeconds INTEGER NOT NULL DEFAULT 15,
                  keepaliveCountMax INTEGER NOT NULL DEFAULT 3, createdAt TEXT NOT NULL, updatedAt TEXT NOT NULL
                );
                INSERT INTO bookmarks VALUES
                  ('recent','Recent','recent.example.com',22,'test','agent','','','','',1,'2026-01-02',1,15,3,'2026-01-01','2026-01-02'),
                  ('older','Older','older.example.com',22,'test','agent','','','','',0,'',1,15,3,'2026-01-01','2026-01-01');
                PRAGMA user_version = 5;
                "#,
            )
            .expect("create version five schema");
        drop(connection);

        let credential_service = format!("{}.migration-test", crate::product::CREDENTIAL_SERVICE);
        let db = Database::open(&path, &credential_service).expect("migrate database");
        let bookmarks = db.list_bookmarks().expect("list migrated bookmarks");
        assert_eq!(
            bookmarks
                .into_iter()
                .map(|item| item.id)
                .collect::<Vec<_>>(),
            ["recent", "older"]
        );

        drop(db);
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(path.with_extension("db-wal"));
        let _ = std::fs::remove_file(path.with_extension("db-shm"));
    }
}
