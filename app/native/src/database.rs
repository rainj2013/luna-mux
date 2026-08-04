use std::{fs, path::Path, sync::Mutex};

use chrono::Utc;
use keyring::Entry;
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Serialize, de::DeserializeOwned};
use uuid::Uuid;

use crate::models::*;

const AI_API_KEY_ACCOUNT: &str = "__ai_api_key__";

pub struct Database {
    connection: Mutex<Connection>,
    credential_service: String,
}

impl Database {
    pub fn open(path: &Path, credential_service: &str) -> Result<Self, String> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        let connection = Connection::open(path).map_err(|e| e.to_string())?;
        let database = Self {
            connection: Mutex::new(connection),
            credential_service: credential_service.into(),
        };
        database.initialize()?;
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
              keepaliveCountMax INTEGER NOT NULL DEFAULT 3, sortOrder INTEGER NOT NULL DEFAULT 0,
              createdAt TEXT NOT NULL, updatedAt TEXT NOT NULL
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
            PRAGMA user_version = 6;
        "#))?;
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
        self.with_conn(|db| db.execute("UPDATE transfers SET status='interrupted', error='应用上次退出时传输未完成', updatedAt=? WHERE status IN ('queued','scanning','running','conflict')", [Utc::now().to_rfc3339()]).map(|_| ()))
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

    pub fn set_bookmark_groups(&self, groups: &[String]) -> Result<(), String> {
        self.set_setting("bookmarkGroups", &groups)
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

    pub fn save_ai_api_key(&self, api_key: &str) -> Result<(), String> {
        Entry::new(&self.credential_service, AI_API_KEY_ACCOUNT)
            .map_err(|e| e.to_string())?
            .set_password(api_key)
            .map_err(|e| e.to_string())
    }

    pub fn get_ai_api_key(&self) -> Option<String> {
        Entry::new(&self.credential_service, AI_API_KEY_ACCOUNT)
            .ok()?
            .get_password()
            .ok()
            .filter(|value| !value.is_empty())
    }

    pub fn delete_ai_api_key(&self) {
        if let Ok(entry) = Entry::new(&self.credential_service, AI_API_KEY_ACCOUNT) {
            let _ = entry.delete_credential();
        }
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

#[cfg(test)]
mod tests {
    use super::Database;
    use crate::models::{AuthType, BookmarkInput};
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
    fn persists_manual_bookmark_order() {
        let path = std::env::temp_dir().join(format!("ssh-client-{}.db", Uuid::new_v4()));
        let db = Database::open(&path, "ssh-client-order-test").expect("open test database");
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
    fn persists_and_manages_bookmark_groups() {
        let path = std::env::temp_dir().join(format!("ssh-client-groups-{}.db", Uuid::new_v4()));
        let db = Database::open(&path, "ssh-client-groups-test").expect("open test database");
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

        let db = Database::open(&path, "ssh-client-groups-test").expect("reopen test database");
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
        let path = std::env::temp_dir().join(format!("ssh-client-ungrouped-{}.db", Uuid::new_v4()));
        let db = Database::open(&path, "ssh-client-ungrouped-test").expect("open test database");
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
    fn migrates_bookmark_sort_order_from_version_five() {
        let path = std::env::temp_dir().join(format!("ssh-client-v5-{}.db", Uuid::new_v4()));
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

        let db = Database::open(&path, "ssh-client-migration-test").expect("migrate database");
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
