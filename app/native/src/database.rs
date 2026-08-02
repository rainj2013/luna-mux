use std::{
    fs,
    path::{Path, PathBuf},
    sync::Mutex,
};

use chrono::Utc;
use keyring::Entry;
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Serialize, de::DeserializeOwned};
use uuid::Uuid;

use crate::models::*;

pub struct Database {
    connection: Mutex<Connection>,
    credential_service: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create_legacy(path: &Path, modern: bool, name: &str) {
        let db = Connection::open(path).unwrap();
        let bookmark_columns = if modern {
            "jumpBookmarkId TEXT NOT NULL,note TEXT NOT NULL,groupName TEXT NOT NULL,favorite INTEGER NOT NULL,lastConnectedAt TEXT NOT NULL,keepaliveEnabled INTEGER NOT NULL,keepaliveIntervalSeconds INTEGER NOT NULL,keepaliveCountMax INTEGER NOT NULL,"
        } else {
            "note TEXT NOT NULL,"
        };
        db.execute_batch(&format!("CREATE TABLE bookmarks(id TEXT PRIMARY KEY,name TEXT NOT NULL,host TEXT NOT NULL,port INTEGER NOT NULL,username TEXT NOT NULL,authType TEXT NOT NULL,privateKeyPath TEXT NOT NULL,{bookmark_columns}createdAt TEXT NOT NULL,updatedAt TEXT NOT NULL);CREATE TABLE credentials(bookmarkId TEXT PRIMARY KEY,encrypted TEXT NOT NULL);CREATE TABLE known_hosts(host TEXT,port INTEGER,fingerprint TEXT);CREATE TABLE settings(key TEXT,value TEXT);CREATE TABLE transfers(id TEXT PRIMARY KEY,sessionId TEXT NOT NULL,{}direction TEXT NOT NULL,sourcePath TEXT NOT NULL,destinationPath TEXT NOT NULL,displayName TEXT NOT NULL,status TEXT NOT NULL,bytesTotal INTEGER NOT NULL,bytesTransferred INTEGER NOT NULL,speed REAL NOT NULL,error TEXT,createdAt TEXT NOT NULL,updatedAt TEXT NOT NULL);",if modern{"bookmarkId TEXT NOT NULL,"}else{""})).unwrap();
        let now = "2026-01-01T00:00:00Z";
        if modern {
            db.execute(
                "INSERT INTO bookmarks VALUES(?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?)",
                params![
                    "b1", name, "host", 22, "user", "password", "", "", "note", "group", 1, "", 1,
                    15, 3, now, now
                ],
            )
            .unwrap();
            db.execute(
                "INSERT INTO transfers VALUES(?,?,?,?,?,?,?,?,?,?,?,?,?,?)",
                params![
                    "t1",
                    "s1",
                    "b1",
                    "upload",
                    "/a",
                    "/b",
                    "a",
                    "running",
                    1,
                    0,
                    0.0,
                    Option::<String>::None,
                    now,
                    now
                ],
            )
            .unwrap();
        } else {
            db.execute(
                "INSERT INTO bookmarks VALUES(?,?,?,?,?,?,?,?,?,?)",
                params![
                    "b1", name, "host", 22, "user", "password", "", "note", now, now
                ],
            )
            .unwrap();
            db.execute(
                "INSERT INTO transfers VALUES(?,?,?,?,?,?,?,?,?,?,?,?,?)",
                params![
                    "t1",
                    "s1",
                    "upload",
                    "/a",
                    "/b",
                    "a",
                    "running",
                    1,
                    0,
                    0.0,
                    Option::<String>::None,
                    now,
                    now
                ],
            )
            .unwrap();
        }
        db.execute("INSERT INTO credentials VALUES('b1','secret')", [])
            .unwrap();
    }

    #[test]
    fn imports_v0_without_secrets_and_applies_defaults() {
        let temp = tempfile::tempdir().unwrap();
        let legacy = temp.path().join("legacy.db");
        create_legacy(&legacy, false, "old");
        let db = Database::open(
            &temp.path().join("new.db"),
            &[legacy],
            "test.credentials.v0",
        )
        .unwrap();
        let bookmark = db.list_bookmarks().unwrap().remove(0);
        assert_eq!(bookmark.name, "old");
        assert_eq!(bookmark.keepalive_interval_seconds, 15);
        assert!(!bookmark.has_saved_credential);
        let transfer = db.list_transfers().unwrap().remove(0);
        assert_eq!(transfer.status, TransferStatus::Interrupted);
        assert!(transfer.bookmark_id.is_empty());
        assert_eq!(
            db.get_setting::<String>("tauriMigrationSource", String::new()),
            "electron"
        );
    }

    #[test]
    fn prefers_first_migration_source() {
        let temp = tempfile::tempdir().unwrap();
        let preview = temp.path().join("tauri-preview.db");
        let electron = temp.path().join("electron.db");
        create_legacy(&preview, true, "preview");
        create_legacy(&electron, true, "electron");
        let db = Database::open(
            &temp.path().join("new.db"),
            &[preview, electron],
            "test.credentials.priority",
        )
        .unwrap();
        assert_eq!(db.list_bookmarks().unwrap().remove(0).name, "preview");
        assert_eq!(
            db.get_setting::<String>("tauriMigrationSource", String::new()),
            "tauri-preview"
        );
    }
}

impl Database {
    pub fn open(
        path: &Path,
        legacy_paths: &[PathBuf],
        credential_service: &str,
    ) -> Result<Self, String> {
        let is_new = !path.exists();
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        let connection = Connection::open(path).map_err(|e| e.to_string())?;
        let database = Self {
            connection: Mutex::new(connection),
            credential_service: credential_service.into(),
        };
        database.initialize()?;
        if is_new {
            if let Some(source) = legacy_paths.iter().find(|candidate| candidate.is_file()) {
                database.import_legacy(source)?;
            }
        }
        Ok(database)
    }

    fn initialize(&self) -> Result<(), String> {
        self.with_conn(|db| db.execute_batch(r#"
            PRAGMA journal_mode = WAL;
            CREATE TABLE IF NOT EXISTS bookmarks (
              id TEXT PRIMARY KEY, name TEXT NOT NULL, host TEXT NOT NULL, port INTEGER NOT NULL,
              username TEXT NOT NULL, authType TEXT NOT NULL, privateKeyPath TEXT NOT NULL DEFAULT '',
              jumpBookmarkId TEXT NOT NULL DEFAULT '', note TEXT NOT NULL DEFAULT '', groupName TEXT NOT NULL DEFAULT '',
              favorite INTEGER NOT NULL DEFAULT 0, lastConnectedAt TEXT NOT NULL DEFAULT '',
              keepaliveEnabled INTEGER NOT NULL DEFAULT 1, keepaliveIntervalSeconds INTEGER NOT NULL DEFAULT 15,
              keepaliveCountMax INTEGER NOT NULL DEFAULT 3, createdAt TEXT NOT NULL, updatedAt TEXT NOT NULL
            );
            CREATE TABLE IF NOT EXISTS credential_refs (bookmarkId TEXT PRIMARY KEY, account TEXT NOT NULL);
            CREATE TABLE IF NOT EXISTS known_hosts (host TEXT NOT NULL, port INTEGER NOT NULL, fingerprint TEXT NOT NULL, PRIMARY KEY(host, port));
            CREATE TABLE IF NOT EXISTS settings (key TEXT PRIMARY KEY, value TEXT NOT NULL);
            CREATE TABLE IF NOT EXISTS transfers (
              id TEXT PRIMARY KEY, sessionId TEXT NOT NULL, bookmarkId TEXT NOT NULL DEFAULT '', direction TEXT NOT NULL,
              sourcePath TEXT NOT NULL, destinationPath TEXT NOT NULL, displayName TEXT NOT NULL, status TEXT NOT NULL,
              bytesTotal INTEGER NOT NULL, bytesTransferred INTEGER NOT NULL, speed REAL NOT NULL, error TEXT,
              createdAt TEXT NOT NULL, updatedAt TEXT NOT NULL
            );
            PRAGMA user_version = 5;
        "#))?;
        self.with_conn(|db| db.execute("UPDATE transfers SET status='interrupted', error='应用上次退出时传输未完成', updatedAt=? WHERE status IN ('queued','scanning','running','conflict')", [Utc::now().to_rfc3339()]).map(|_| ()))
    }

    fn import_legacy(&self, source: &Path) -> Result<(), String> {
        let source = source.to_string_lossy().to_string();
        self.with_conn(|db| {
            db.execute("ATTACH DATABASE ? AS legacy", [&source])?;
            let table_exists = |db: &Connection, table: &str| -> rusqlite::Result<bool> { db.query_row("SELECT EXISTS(SELECT 1 FROM legacy.sqlite_master WHERE type='table' AND name=?)", [table], |row| row.get(0)) };
            let columns = |db: &Connection, table: &str| -> rusqlite::Result<Vec<String>> { let mut statement=db.prepare(&format!("PRAGMA legacy.table_info({table})"))?;statement.query_map([],|row|row.get(1))?.collect() };
            let mut statements=vec!["BEGIN".to_string()];
            if table_exists(db,"bookmarks")? { let c=columns(db,"bookmarks")?;let has=|name:&str|c.iter().any(|item|item==name);let value=|name:&str,fallback:&str|if has(name){name.to_string()}else{fallback.to_string()};statements.push(format!("INSERT OR REPLACE INTO bookmarks(id,name,host,port,username,authType,privateKeyPath,jumpBookmarkId,note,groupName,favorite,lastConnectedAt,keepaliveEnabled,keepaliveIntervalSeconds,keepaliveCountMax,createdAt,updatedAt) SELECT id,name,host,port,username,authType,privateKeyPath,{},{},{},{},{},{},{},{},createdAt,updatedAt FROM legacy.bookmarks",value("jumpBookmarkId","''"),value("note","''"),value("groupName","''"),value("favorite","0"),value("lastConnectedAt","''"),value("keepaliveEnabled","1"),value("keepaliveIntervalSeconds","15"),value("keepaliveCountMax","3"))); }
            if table_exists(db,"known_hosts")? { statements.push("INSERT OR REPLACE INTO known_hosts SELECT host,port,fingerprint FROM legacy.known_hosts".into()); }
            if table_exists(db,"settings")? { statements.push("INSERT OR REPLACE INTO settings SELECT key,value FROM legacy.settings".into()); }
            if table_exists(db,"transfers")? { let c=columns(db,"transfers")?;let bookmark=if c.iter().any(|item|item=="bookmarkId"){"bookmarkId"}else{"''"};statements.push(format!("INSERT OR REPLACE INTO transfers(id,sessionId,bookmarkId,direction,sourcePath,destinationPath,displayName,status,bytesTotal,bytesTransferred,speed,error,createdAt,updatedAt) SELECT id,sessionId,{bookmark},direction,sourcePath,destinationPath,displayName,CASE WHEN status IN ('queued','scanning','running','conflict') THEN 'interrupted' ELSE status END,bytesTotal,bytesTransferred,speed,CASE WHEN status IN ('queued','scanning','running','conflict') THEN '迁移时任务未完成' ELSE error END,createdAt,updatedAt FROM legacy.transfers")); }
            statements.push("COMMIT".into());
            let result = db.execute_batch(&statements.join(";"));
            if result.is_err() { let _ = db.execute_batch("ROLLBACK;"); }
            let _ = db.execute_batch("DETACH DATABASE legacy;");
            result
        })?;
        let origin = if source.contains("tauri-preview") {
            "tauri-preview"
        } else {
            "electron"
        };
        self.set_setting("tauriMigrationSource", &origin)
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
                CASE WHEN c.bookmarkId IS NULL THEN 0 ELSE 1 END,b.createdAt,b.updatedAt
                FROM bookmarks b LEFT JOIN credential_refs c ON c.bookmarkId=b.id
                ORDER BY b.favorite DESC, CASE WHEN b.lastConnectedAt='' THEN b.updatedAt ELSE b.lastConnectedAt END DESC"#)?;
            statement.query_map([], |row| Ok(Bookmark {
                id: row.get(0)?, name: row.get(1)?, host: row.get(2)?, port: row.get(3)?, username: row.get(4)?,
                auth_type: AuthType::parse(&row.get::<_, String>(5)?), private_key_path: row.get(6)?, jump_bookmark_id: row.get(7)?,
                group_name: row.get(8)?, favorite: row.get::<_, i64>(9)? != 0, last_connected_at: row.get(10)?,
                keepalive_enabled: row.get::<_, i64>(11)? != 0, keepalive_interval_seconds: row.get(12)?, keepalive_count_max: row.get(13)?,
                note: row.get(14)?, has_saved_credential: row.get::<_, i64>(15)? != 0, created_at: row.get(16)?, updated_at: row.get(17)?,
            }))?.collect()
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
            created_at: existing
                .map(|item| item.created_at)
                .unwrap_or_else(|| now.clone()),
            updated_at: now,
        };
        self.with_conn(|db| db.execute(r#"INSERT INTO bookmarks(id,name,host,port,username,authType,privateKeyPath,jumpBookmarkId,note,groupName,favorite,lastConnectedAt,keepaliveEnabled,keepaliveIntervalSeconds,keepaliveCountMax,createdAt,updatedAt)
            VALUES(?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?) ON CONFLICT(id) DO UPDATE SET name=excluded.name,host=excluded.host,port=excluded.port,username=excluded.username,authType=excluded.authType,privateKeyPath=excluded.privateKeyPath,jumpBookmarkId=excluded.jumpBookmarkId,note=excluded.note,groupName=excluded.groupName,favorite=excluded.favorite,keepaliveEnabled=excluded.keepaliveEnabled,keepaliveIntervalSeconds=excluded.keepaliveIntervalSeconds,keepaliveCountMax=excluded.keepaliveCountMax,updatedAt=excluded.updatedAt"#,
            params![bookmark.id,bookmark.name,bookmark.host,bookmark.port,bookmark.username,bookmark.auth_type.as_str(),bookmark.private_key_path,bookmark.jump_bookmark_id,bookmark.note,bookmark.group_name,bookmark.favorite as i64,bookmark.last_connected_at,bookmark.keepalive_enabled as i64,bookmark.keepalive_interval_seconds,bookmark.keepalive_count_max,bookmark.created_at,bookmark.updated_at]).map(|_| ()))?;
        Ok(bookmark)
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
        Entry::new(&self.credential_service, bookmark_id)
            .ok()?
            .get_password()
            .ok()
    }

    pub fn forget_credential(&self, bookmark_id: &str) -> Result<(), String> {
        if let Ok(entry) = Entry::new(&self.credential_service, bookmark_id) {
            let _ = entry.delete_credential();
        }
        self.with_conn(|db| {
            db.execute(
                "DELETE FROM credential_refs WHERE bookmarkId=?",
                [bookmark_id],
            )
            .map(|_| ())
        })
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
