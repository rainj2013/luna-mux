//! Database driver abstraction and lifecycle management for Database Panes.
//!
//! SQLite is embedded through rusqlite. Network drivers are represented by the
//! same abstraction and can be enabled without changing the Pane contract.

use std::{
    collections::HashMap,
    fs::File,
    io::{BufWriter, Write},
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::Duration,
};

use rusqlite::{Connection, OpenFlags, types::ValueRef};
use mysql_async::prelude::Queryable;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum DatabaseDriver { Sqlite, Mysql, Mariadb, #[serde(rename = "postgresql", alias = "postgres")] Postgres }

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DatabaseConnectionConfig {
    pub profile_id: Option<String>,
    pub driver: DatabaseDriver,
    pub host: Option<String>,
    pub port: Option<u16>,
    pub database: Option<String>,
    pub username: Option<String>,
    #[serde(default, skip_serializing)] pub password: Option<String>,
    #[serde(default)] pub read_only: bool,
    #[serde(default)] pub ssl_enabled: bool,
    pub sqlite_path: Option<PathBuf>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum DatabaseRuntimeStatus { Connecting, Ready, Disconnected, Error }

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DatabaseRuntimeSummary {
    pub id: String,
    pub driver: DatabaseDriver,
    pub status: DatabaseRuntimeStatus,
    pub error: Option<String>,
}

enum DatabaseHandle { Sqlite(Connection), Network(DatabaseConnectionConfig) }

struct RuntimeRecord { summary: DatabaseRuntimeSummary, handle: Option<DatabaseHandle> }

pub struct DatabaseRuntimeManager { runtimes: Mutex<HashMap<String, RuntimeRecord>> }

impl Default for DatabaseRuntimeManager { fn default() -> Self { Self::new() } }

impl DatabaseRuntimeManager {
    pub fn import_rows(&self, id: &str, table: &str, columns: Vec<String>, rows: Vec<Vec<serde_json::Value>>) -> Result<u64, String> {
        if table.is_empty() || columns.is_empty() || !table.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') || columns.iter().any(|c| c.is_empty() || !c.chars().all(|x| x.is_ascii_alphanumeric() || x == '_')) { return Err("非法表名或字段名".into()); }
        let mut map = self.runtimes.lock().map_err(|_| "database runtime lock poisoned".to_string())?;
        let record = map.get_mut(id).ok_or_else(|| "database runtime not found".to_string())?;
        let Some(DatabaseHandle::Sqlite(connection)) = record.handle.as_mut() else { return Err("database is disconnected".into()); };
        let tx = connection.transaction().map_err(|e| e.to_string())?;
        let placeholders = (0..columns.len()).map(|_| "?").collect::<Vec<_>>().join(",");
        let sql = format!("INSERT INTO \"{}\" ({}) VALUES ({})", table, columns.iter().map(|c| format!("\"{}\"", c)).collect::<Vec<_>>().join(","), placeholders);
        let mut count = 0; for row in rows { let params = row.iter().map(|v| match v { serde_json::Value::Null => rusqlite::types::Value::Null, serde_json::Value::Bool(b) => rusqlite::types::Value::Integer(*b as i64), serde_json::Value::Number(n) => n.as_i64().map(rusqlite::types::Value::Integer).or_else(|| n.as_f64().map(rusqlite::types::Value::Real)).unwrap_or(rusqlite::types::Value::Text(n.to_string())), serde_json::Value::String(s) => rusqlite::types::Value::Text(s.clone()), _ => rusqlite::types::Value::Text(v.to_string()) }).collect::<Vec<_>>(); tx.execute(&sql, rusqlite::params_from_iter(params)).map_err(|e| e.to_string())?; count += 1; }
        tx.commit().map_err(|e| e.to_string())?; Ok(count)
    }
    /// Network runtimes keep their connection settings instead of a live handle, so a dump or a
    /// script runs on a short-lived connection owned by that call.
    fn network_config(&self, id: &str) -> Result<Option<DatabaseConnectionConfig>, String> {
        let map = self.runtimes.lock().map_err(|_| "database runtime lock poisoned".to_string())?;
        let record = map.get(id).ok_or_else(|| "database runtime not found".to_string())?;
        Ok(match record.handle.as_ref() { Some(DatabaseHandle::Network(config)) => Some(config.clone()), _ => None })
    }
    pub fn import_sql(&self, id: &str, script: &str) -> Result<(), String> {
        if script.trim().is_empty() { return Err("SQL 脚本不能为空".into()); }
        if let Some(config) = self.network_config(id)? { return tauri::async_runtime::block_on(import_network_sql(config, script)); }
        let mut map = self.runtimes.lock().map_err(|_| "database runtime lock poisoned".to_string())?;
        let record = map.get_mut(id).ok_or_else(|| "database runtime not found".to_string())?;
        let Some(DatabaseHandle::Sqlite(connection)) = record.handle.as_mut() else { return Err("database is disconnected".into()) };
        let transaction = connection.transaction().map_err(|e| e.to_string())?;
        transaction.execute_batch(script).map_err(|e| {
            let message = e.to_string();
            // The whole script runs in one transaction here, so a script that opens its own
            // transaction cannot be rolled back as a unit.
            if message.contains("transaction within a transaction") { "脚本自带 BEGIN/COMMIT，无法整体执行：请移除脚本里的事务语句后重试".into() } else { message }
        })?;
        transaction.commit().map_err(|e| e.to_string())
    }
    /// Same as `import_sql`, but read from disk so a whole-database dump never has to travel
    /// through the webview and the IPC layer as one JavaScript string.
    pub fn import_sql_file(&self, id: &str, path: &Path) -> Result<(), String> {
        let script = std::fs::read_to_string(path).map_err(|e| format!("无法读取 SQL 文件：{e}"))?;
        self.import_sql(id, &script)
    }
    /// Whole-database dump written straight to `path`: schema first, then row data as batched
    /// `INSERT` statements that name their table, then indexes, triggers and views. Streaming
    /// through a buffered writer keeps memory flat no matter how large the database is.
    /// Writes a SQL dump for the whole database, or for one table/view when `table` is set.
    #[allow(dead_code)]
    pub fn export_sql(&self, id: &str, path: &Path, table: Option<&str>) -> Result<DatabaseSqlExport, String> {
        self.export_sql_with_options(id, path, table, false)
    }
    #[allow(dead_code)]
    pub fn export_sql_with_options(&self, id: &str, path: &Path, table: Option<&str>, schema_only: bool) -> Result<DatabaseSqlExport, String> {
        self.export_sql_with_progress(id, path, table, schema_only, None)
    }
    pub fn export_sql_with_progress(&self, id: &str, path: &Path, table: Option<&str>, schema_only: bool, progress: Option<DatabaseExportProgressSink>) -> Result<DatabaseSqlExport, String> {
        if let Some(config) = self.network_config(id)? { return tauri::async_runtime::block_on(export_network_sql(config, path, table, schema_only, progress)); }
        let mut map = self.runtimes.lock().map_err(|_| "database runtime lock poisoned".to_string())?;
        let record = map.get_mut(id).ok_or_else(|| "database runtime not found".to_string())?;
        let Some(DatabaseHandle::Sqlite(connection)) = record.handle.as_mut() else { return Err("database is disconnected".into()) };
        let mut out = BufWriter::with_capacity(1 << 20, File::create(path).map_err(|e| format!("无法写入导出文件：{e}"))?);
        // One read transaction keeps every table on the same snapshot while the dump streams out.
        let transaction = connection.transaction().map_err(|e| e.to_string())?;
        let summary = write_sql_dump(&transaction, &mut out, table, schema_only, progress.as_ref())?;
        transaction.commit().map_err(|e| e.to_string())?;
        let bytes = finish_dump(out)?;
        Ok(DatabaseSqlExport { tables: summary.tables, rows: summary.rows, bytes })
    }
    pub fn export_json(&self, id: &str, sql: &str) -> Result<String, String> {
        let result = self.execute(id, sql, 10000)?;
        let rows = result.rows.into_iter().map(|row| {
            let mut object = serde_json::Map::new();
            for (index, value) in row.into_iter().enumerate() { if let Some(column) = result.columns.get(index) { object.insert(column.clone(), value); } }
            serde_json::Value::Object(object)
        }).collect::<Vec<_>>();
        serde_json::to_string_pretty(&rows).map_err(|e| e.to_string())
    }
    pub fn add_column(&self, id: &str, table: &str, column: &str, data_type: &str) -> Result<u64, String> {
        for value in [table, column, data_type] { if value.is_empty() || !value.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '(' || c == ')' || c == ' ') { return Err("非法结构名称或类型".into()); } }
        let mut map = self.runtimes.lock().map_err(|_| "database runtime lock poisoned".to_string())?;
        let record = map.get_mut(id).ok_or_else(|| "database runtime not found".to_string())?;
        let Some(DatabaseHandle::Sqlite(connection)) = record.handle.as_mut() else { return Err("database is disconnected".into()); };
        connection.execute(&format!("ALTER TABLE \"{}\" ADD COLUMN \"{}\" {}", table, column, data_type), []).map(|n| n as u64).map_err(|e| e.to_string())
    }
    pub fn drop_column(&self, id: &str, table: &str, column: &str) -> Result<u64, String> {
        for value in [table, column] { if value.is_empty() || !value.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') { return Err("非法表名或字段名".into()); } }
        let mut map = self.runtimes.lock().map_err(|_| "database runtime lock poisoned".to_string())?;
        let record = map.get_mut(id).ok_or_else(|| "database runtime not found".to_string())?;
        let Some(DatabaseHandle::Sqlite(connection)) = record.handle.as_mut() else { return Err("database is disconnected".into()); };
        connection.execute(&format!("ALTER TABLE \"{}\" DROP COLUMN \"{}\"", table, column), []).map(|n| n as u64).map_err(|e| e.to_string())
    }
    pub fn export_csv(&self, id: &str, sql: &str) -> Result<String, String> {
        let result = self.execute(id, sql, 10000)?;
        fn quote(value: &serde_json::Value) -> String { let s = match value { serde_json::Value::Null => String::new(), serde_json::Value::String(s) => s.clone(), other => other.to_string() }; format!("\"{}\"", s.replace('"', "\"\"")) }
        let mut out = result.columns.iter().map(|c| quote(&serde_json::Value::String(c.clone()))).collect::<Vec<_>>().join(","); out.push('\n');
        for row in result.rows { out.push_str(&row.iter().map(quote).collect::<Vec<_>>().join(",")); out.push('\n'); }
        Ok(out)
    }
    pub fn list_tables(&self, id: &str) -> Result<Vec<String>, String> {
        let mut map = self.runtimes.lock().map_err(|_| "database runtime lock poisoned".to_string())?;
        let record = map.get_mut(id).ok_or_else(|| "database runtime not found".to_string())?;
        if let Some(DatabaseHandle::Network(config)) = record.handle.as_ref() {
            let config = config.clone();
            drop(map);
            let sql = match config.driver { DatabaseDriver::Postgres => "SELECT table_name FROM information_schema.tables WHERE table_schema = 'public' ORDER BY table_name", _ => "SELECT table_name FROM information_schema.tables WHERE table_schema = DATABASE() ORDER BY table_name" };
            return tauri::async_runtime::block_on(execute_network(config, sql, 10000)).map(|result| result.rows.into_iter().filter_map(|row| row.first().and_then(|v| v.as_str().map(str::to_string))).collect());
        }
        let Some(DatabaseHandle::Sqlite(connection)) = record.handle.as_mut() else { return Err("database is disconnected".into()); };
        let mut statement = connection.prepare("SELECT name FROM sqlite_master WHERE type IN ('table','view') ORDER BY name").map_err(|e| e.to_string())?;
        statement.query_map([], |row| row.get::<_, String>(0)).map_err(|e| e.to_string())?.collect::<rusqlite::Result<Vec<_>>>().map_err(|e| e.to_string())
    }

    pub fn new() -> Self { Self { runtimes: Mutex::new(HashMap::new()) } }

    pub fn connect(&self, config: DatabaseConnectionConfig) -> Result<DatabaseRuntimeSummary, String> {
        let id = Uuid::new_v4().to_string();
        let mut summary = DatabaseRuntimeSummary { id: id.clone(), driver: config.driver.clone(), status: DatabaseRuntimeStatus::Connecting, error: None };
        let handle = match config.driver {
            DatabaseDriver::Sqlite => {
                let path = config.sqlite_path.ok_or_else(|| "SQLite connection requires sqlitePath".to_string())?;
                let flags = if config.read_only { OpenFlags::SQLITE_OPEN_READ_ONLY } else { OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_CREATE };
                Connection::open_with_flags(path, flags).map(DatabaseHandle::Sqlite).map_err(|e| e.to_string())
            }
            DatabaseDriver::Mysql | DatabaseDriver::Mariadb | DatabaseDriver::Postgres => {
                tauri::async_runtime::block_on(validate_network_config(config.clone())).map(|_| DatabaseHandle::Network(config.clone()))
            },
        };
        let handle = Some(handle?);
        summary.status = DatabaseRuntimeStatus::Ready;
        let result = summary.clone();
        self.runtimes.lock().map_err(|_| "database runtime lock poisoned".to_string())?.insert(id, RuntimeRecord { summary, handle });
        Ok(result)
    }

    pub fn list(&self) -> Result<Vec<DatabaseRuntimeSummary>, String> {
        Ok(self.runtimes.lock().map_err(|_| "database runtime lock poisoned".to_string())?.values().map(|r| r.summary.clone()).collect())
    }

    pub fn disconnect(&self, id: &str) -> Result<(), String> {
        let mut map = self.runtimes.lock().map_err(|_| "database runtime lock poisoned".to_string())?;
        let record = map.get_mut(id).ok_or_else(|| "database runtime not found".to_string())?;
        record.handle = None; record.summary.status = DatabaseRuntimeStatus::Disconnected; Ok(())
    }

    pub fn disconnect_all(&self) {
        if let Ok(mut map) = self.runtimes.lock() {
            for record in map.values_mut() {
                record.handle = None;
                record.summary.status = DatabaseRuntimeStatus::Disconnected;
            }
        }
    }
}


#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DatabaseQueryResult { pub columns: Vec<String>, pub rows: Vec<Vec<serde_json::Value>>, pub affected_rows: u64, pub truncated: bool, #[serde(default, skip_serializing_if = "Option::is_none")] pub has_more: Option<bool> }

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DatabaseSqlExport { pub tables: usize, pub rows: u64, pub bytes: u64 }

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DatabaseSqlExportProgress { pub operation_id: String, pub current: usize, pub total: usize, pub table: Option<String> }

pub type DatabaseExportProgressSink = Arc<dyn Fn(usize, usize, Option<&str>) + Send + Sync>;

/// Write a query result script to the path selected by the user. The SQL is already rendered
/// in the webview because it represents the visible result snapshot rather than a database dump.
#[allow(dead_code)]
pub fn write_query_sql_file(path: &Path, sql: &str, rows: u64) -> Result<DatabaseSqlExport, String> {
    write_query_sql_file_with_progress(path, sql, rows, None)
}

pub fn write_query_sql_file_with_progress(path: &Path, sql: &str, rows: u64, progress: Option<DatabaseExportProgressSink>) -> Result<DatabaseSqlExport, String> {
    if sql.trim().is_empty() { return Err("查询结果为空，无法导出 SQL 文件".into()); }
    if let Some(report) = progress.as_ref() { report(0, 1, None); }
    let mut out = create_dump_writer(path)?;
    out.write_all(sql.as_bytes()).map_err(|e| format!("无法写入导出文件：{e}"))?;
    if !sql.ends_with('\n') { out.write_all(b"\n").map_err(|e| format!("无法写入导出文件：{e}"))?; }
    let bytes = finish_dump(out)?;
    if let Some(report) = progress.as_ref() { report(1, 1, None); }
    Ok(DatabaseSqlExport { tables: if rows > 0 { 1 } else { 0 }, rows, bytes })
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DatabaseColumnInfo { pub name: String, pub data_type: String, pub not_null: bool, pub primary_key: bool, pub default_value: Option<String>, #[serde(default)] pub comment: Option<String> }
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DatabaseIndexInfo { pub name: String, pub unique: bool }
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DatabaseForeignKeyInfo { pub column: String, pub references_table: String, pub references_column: String, pub on_update: String, pub on_delete: String }

impl DatabaseRuntimeManager {
    pub fn list_foreign_keys(&self, id: &str, table: &str) -> Result<Vec<DatabaseForeignKeyInfo>, String> {
        if !table.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') { return Err("非法表名".into()); }
        let mut map = self.runtimes.lock().map_err(|_| "database runtime lock poisoned".to_string())?; let record = map.get_mut(id).ok_or_else(|| "database runtime not found".to_string())?; let Some(DatabaseHandle::Sqlite(connection)) = record.handle.as_mut() else { return Err("database is disconnected".into()); };
        let mut s = connection.prepare(&format!("PRAGMA foreign_key_list(\"{}\")", table)).map_err(|e| e.to_string())?;
        s.query_map([], |r| Ok(DatabaseForeignKeyInfo { references_table: r.get(2)?, column: r.get(3)?, references_column: r.get(4)?, on_update: r.get(5)?, on_delete: r.get(6)? })).map_err(|e| e.to_string())?.collect::<rusqlite::Result<Vec<_>>>().map_err(|e| e.to_string())
    }
    pub fn list_indexes(&self, id: &str, table: &str) -> Result<Vec<DatabaseIndexInfo>, String> {
        if !table.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') { return Err("非法表名".into()); }
        let mut map = self.runtimes.lock().map_err(|_| "database runtime lock poisoned".to_string())?; let record = map.get_mut(id).ok_or_else(|| "database runtime not found".to_string())?;
        if let Some(DatabaseHandle::Network(config)) = record.handle.as_ref() { let config = config.clone(); drop(map); let sql = format!("SELECT index_name, CASE WHEN non_unique=0 THEN 1 ELSE 0 END FROM information_schema.statistics WHERE table_schema=DATABASE() AND table_name='{}' GROUP BY index_name,non_unique ORDER BY index_name", table); return tauri::async_runtime::block_on(execute_network(config, &sql, 10000)).map(|r| r.rows.into_iter().filter_map(|row| Some(DatabaseIndexInfo { name: row.first()?.as_str()?.into(), unique: row.get(1).and_then(|v| v.as_str()) == Some("1") })).collect()); }
        let Some(DatabaseHandle::Sqlite(connection)) = record.handle.as_mut() else { return Err("database is disconnected".into()); };
        let mut s = connection.prepare(&format!("PRAGMA index_list(\"{}\")", table)).map_err(|e| e.to_string())?; s.query_map([], |r| Ok(DatabaseIndexInfo { name: r.get(1)?, unique: r.get::<_, i64>(2)? != 0 })).map_err(|e| e.to_string())?.collect::<rusqlite::Result<Vec<_>>>().map_err(|e| e.to_string())
    }
    pub fn create_index(&self, id: &str, table: &str, index: &str, column: &str, unique: bool) -> Result<u64, String> { for value in [table,index,column] { if value.is_empty() || !value.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') { return Err("非法索引名称".into()); } } let mut map=self.runtimes.lock().map_err(|_| "database runtime lock poisoned".to_string())?; let record=map.get_mut(id).ok_or_else(|| "database runtime not found".to_string())?; let Some(DatabaseHandle::Sqlite(connection))=record.handle.as_mut() else{return Err("database is disconnected".into())}; connection.execute(&format!("CREATE {} INDEX \"{}\" ON \"{}\" (\"{}\")",if unique{"UNIQUE"}else{""},index,table,column),[]).map(|n|n as u64).map_err(|e|e.to_string()) }
    pub fn drop_index(&self, id: &str, index: &str) -> Result<u64, String> { if index.is_empty() || !index.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') { return Err("非法索引名称".into()); } let mut map=self.runtimes.lock().map_err(|_| "database runtime lock poisoned".to_string())?; let record=map.get_mut(id).ok_or_else(|| "database runtime not found".to_string())?; let Some(DatabaseHandle::Sqlite(connection))=record.handle.as_mut() else{return Err("database is disconnected".into())}; connection.execute(&format!("DROP INDEX \"{}\"",index),[]).map(|n|n as u64).map_err(|e|e.to_string()) }
    pub fn describe_table(&self, id: &str, table: &str) -> Result<Vec<DatabaseColumnInfo>, String> {
        if !table.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') { return Err("非法表名".into()); }
        let mut map = self.runtimes.lock().map_err(|_| "database runtime lock poisoned".to_string())?;
        let record = map.get_mut(id).ok_or_else(|| "database runtime not found".to_string())?;
        if let Some(DatabaseHandle::Network(config)) = record.handle.as_ref() {
            let config = config.clone(); drop(map);
            let sql = match config.driver {
                DatabaseDriver::Postgres => format!("SELECT c.column_name,c.data_type,c.is_nullable,c.column_default,pg_catalog.col_description(cl.oid,attr.attnum) FROM information_schema.columns c JOIN pg_catalog.pg_namespace ns ON ns.nspname=c.table_schema JOIN pg_catalog.pg_class cl ON cl.relnamespace=ns.oid AND cl.relname=c.table_name JOIN pg_catalog.pg_attribute attr ON attr.attrelid=cl.oid AND attr.attname=c.column_name WHERE c.table_schema='public' AND c.table_name='{}' ORDER BY c.ordinal_position", table),
                _ => format!("SELECT column_name,data_type,is_nullable,column_default,column_comment FROM information_schema.columns WHERE table_schema=DATABASE() AND table_name='{}' ORDER BY ordinal_position", table),
            };
            return tauri::async_runtime::block_on(execute_network(config, &sql, 10000)).map(|result| result.rows.into_iter().map(|row| DatabaseColumnInfo { name: row.first().and_then(|v| v.as_str()).unwrap_or_default().into(), data_type: row.get(1).and_then(|v| v.as_str()).unwrap_or_default().into(), not_null: row.get(2).and_then(|v| v.as_str()) == Some("NO"), primary_key: false, default_value: row.get(3).and_then(|v| v.as_str()).map(str::to_string), comment: row.get(4).and_then(|v| v.as_str()).filter(|v| !v.is_empty()).map(str::to_string) }).collect());
        }
        let Some(DatabaseHandle::Sqlite(connection)) = record.handle.as_mut() else { return Err("database is disconnected".into()); };
        let mut s = connection.prepare(&format!("PRAGMA table_info(\"{}\")", table)).map_err(|e| e.to_string())?;
        s.query_map([], |r| Ok(DatabaseColumnInfo { name: r.get(1)?, data_type: r.get(2)?, not_null: r.get::<_, i64>(3)? != 0, primary_key: r.get::<_, i64>(5)? != 0, default_value: r.get(4)?, comment: None })).map_err(|e| e.to_string())?.collect::<rusqlite::Result<Vec<_>>>().map_err(|e| e.to_string())
    }
    pub fn execute_page(&self, id: &str, sql: &str, page_offset: usize, page_size: usize) -> Result<DatabaseQueryResult, String> {
        let driver = {
            let records = self.runtimes.lock().map_err(|_| "database runtime lock poisoned".to_string())?;
            records.get(id).ok_or_else(|| "database runtime not found".to_string())?.summary.driver.clone()
        };
        let page_size = page_size.clamp(1, 10_000);
        let (paged_sql, pageable) = prepare_query(sql, &driver, page_size, Some(page_offset));
        if page_offset > 0 && paged_sql == sql && !pageable { return Err("此 SQL 不支持自动翻页，请修改 SQL 后执行".into()); }
        let mut result = self.execute(id, &paged_sql, page_size)?;
        result.has_more = Some(pageable && result.rows.len() == page_size);
        Ok(result)
    }

    pub fn execute(&self, id: &str, sql: &str, max_rows: usize) -> Result<DatabaseQueryResult, String> {
        let mut map = self.runtimes.lock().map_err(|_| "database runtime lock poisoned".to_string())?;
        let record = map.get_mut(id).ok_or_else(|| "database runtime not found".to_string())?;
        let Some(handle) = record.handle.as_mut() else { return Err("database is disconnected".into()); };
        if let DatabaseHandle::Network(config) = handle {
            let config = config.clone();
            drop(map);
            return tauri::async_runtime::block_on(execute_network(config, sql, max_rows));
        }
        let DatabaseHandle::Sqlite(connection) = handle else { return Err("database is disconnected".into()); };
        let bounded_sql = bounded_query_sql(sql, &DatabaseDriver::Sqlite, max_rows.clamp(1, 10000));
        let mut statement = connection.prepare(&bounded_sql).map_err(|e| e.to_string())?;
        let columns = statement.column_names().iter().map(|v| v.to_string()).collect::<Vec<_>>();
        if columns.is_empty() {
            let affected = statement.execute([]).map_err(|e| e.to_string())?;
            return Ok(DatabaseQueryResult { columns, rows: vec![], affected_rows: affected as u64, truncated: false, has_more: None });
        }
        let count = columns.len();
        let mut cursor = statement.query([]).map_err(|e| e.to_string())?;
        let mut rows = Vec::new();
        let limit = max_rows.clamp(1, 10000);
        let mut truncated = false;
        while let Some(row) = cursor.next().map_err(|e| e.to_string())? {
            if rows.len() == limit { truncated = true; break; }
            let mut values = Vec::with_capacity(count);
            for index in 0..count {
                use rusqlite::types::ValueRef;
                let value = match row.get_ref(index).map_err(|e| e.to_string())? {
                    ValueRef::Null => serde_json::Value::Null,
                    ValueRef::Integer(v) => serde_json::json!(v.to_string()),
                    ValueRef::Real(v) => serde_json::json!(v),
                    ValueRef::Text(v) => serde_json::json!(String::from_utf8_lossy(&v[..v.len().min(65536)])),
                    ValueRef::Blob(v) => serde_json::json!({"bytes": v.len()}),
                };
                values.push(value);
            }
            rows.push(values);
        }
        Ok(DatabaseQueryResult { columns, truncated: truncated || rows.len() == limit, rows, affected_rows: 0, has_more: None })
    }
}

const CONNECTION_TIMEOUT: Duration = Duration::from_secs(8);
const QUERY_TIMEOUT: Duration = Duration::from_secs(30);

// mysql_async's default Drop tries to drain pending results. On a stalled query
// that cleanup can wait forever, so cancellation explicitly closes the connection.
struct MysqlConnection(Option<mysql_async::Conn>);
impl Drop for MysqlConnection {
    fn drop(&mut self) {
        if let Some(connection) = self.0.take() {
            tokio::spawn(async move {
                let _ = tokio::time::timeout(Duration::from_secs(1), connection.disconnect()).await;
            });
        }
    }
}

// Dropping a timed-out PostgreSQL query must also stop its connection driver.
struct ConnectionTask(tokio::task::JoinHandle<()>);
impl Drop for ConnectionTask {
    fn drop(&mut self) { self.0.abort(); }
}

async fn execute_network(config: DatabaseConnectionConfig, sql: &str, max_rows: usize) -> Result<DatabaseQueryResult, String> {
    tokio::time::timeout(QUERY_TIMEOUT, execute_network_inner(config, sql, max_rows))
        .await.map_err(|_| "数据库查询超时（30 秒）；执行结果可能未知，请刷新数据后再决定是否重试".to_string())?
}

async fn execute_network_inner(config: DatabaseConnectionConfig, sql: &str, max_rows: usize) -> Result<DatabaseQueryResult, String> {
    let host = config.host.clone().ok_or_else(|| "数据库主机不能为空".to_string())?;
    let port = config.port.unwrap_or(if matches!(config.driver, DatabaseDriver::Postgres) { 5432 } else { 3306 });
    // Cloned so the PostgreSQL branch can hand the whole config to its own connect helper.
    let user = config.username.clone().unwrap_or_default();
    let database = config.database.clone().unwrap_or_default();
    let limit = max_rows.clamp(1, 10000);
    let bounded_sql = bounded_query_sql(sql, &config.driver, limit);
    let sql = bounded_sql.as_str();
    match config.driver {
        DatabaseDriver::Mysql | DatabaseDriver::Mariadb => {
            let mut builder = mysql_async::OptsBuilder::default().ip_or_hostname(host).tcp_port(port).prefer_socket(false).user(Some(user)).pass(config.password).db_name((!database.is_empty()).then_some(database));
            if config.ssl_enabled { builder = builder.ssl_opts(Some(mysql_async::SslOpts::default())); }
            let conn = tokio::time::timeout(CONNECTION_TIMEOUT, mysql_async::Conn::new(builder))
                .await.map_err(|_| "数据库连接超时（8 秒）".to_string())?.map_err(|e| e.to_string())?;
            let mut connection = MysqlConnection(Some(conn));
            let conn = connection.0.as_mut().expect("connection is owned until cleanup");
            let mut result = conn.query_iter(sql).await.map_err(|e| e.to_string())?;
            let columns = result.columns().map(|cols| cols.as_ref().iter().map(|c| c.name_str().to_string()).collect::<Vec<_>>()).unwrap_or_default();
            let mut rows = Vec::new(); let mut truncated = false;
            while let Some(row) = result.next().await.map_err(|e| e.to_string())? {
                if rows.len() == limit { truncated = true; break; }
                rows.push(row.unwrap().into_iter().map(mysql_value).collect());
            }
            truncated |= rows.len() == limit;
            let affected_rows = result.affected_rows();
            // Finish the protocol even after the display budget, so writes/late errors aren't hidden.
            result.drop_result().await.map_err(|e| e.to_string())?;
            connection.0.take().expect("connection is owned until cleanup").disconnect().await.map_err(|e| e.to_string())?;
            Ok(DatabaseQueryResult { columns, rows, affected_rows, truncated, has_more: None })
        }
        DatabaseDriver::Postgres => {
            let (client, _connection_task) = postgres_client(&config).await?;
            read_postgres_result(&client, sql, limit).await
        }
        DatabaseDriver::Sqlite => Err("SQLite 使用本地连接".into()),
    }
}

// Bound ordinary SELECTs at the database, not just in the UI. Complex/write syntax
// stays untouched and uses bounded result storage while waiting for completion.
fn bounded_query_sql(sql: &str, driver: &DatabaseDriver, cap: usize) -> String {
    prepare_query(sql, driver, cap, None).0
}

fn prepare_query(sql: &str, driver: &DatabaseDriver, cap: usize, page_offset: Option<usize>) -> (String, bool) {
    use sqlparser::{ast::{Expr, LimitClause, Query, SetExpr, Statement, Value}, dialect::{Dialect, MySqlDialect, PostgreSqlDialect, SQLiteDialect}, parser::Parser};
    fn read_query(query: &Query) -> bool {
        query.locks.is_empty() && read_body(&query.body) && query.with.as_ref().is_none_or(|w| w.cte_tables.iter().all(|c| read_query(&c.query)))
    }
    fn read_body(body: &SetExpr) -> bool {
        match body {
            SetExpr::Select(s) => s.into.is_none(),
            SetExpr::Query(q) => read_query(q),
            SetExpr::SetOperation { left, right, .. } => read_body(left) && read_body(right),
            SetExpr::Values(_) | SetExpr::Table(_) => true,
            _ => false,
        }
    }
    fn clamp_literal(expr: &mut Expr, cap: usize) -> bool {
        match expr {
            Expr::Value(value) => match &value.value {
                Value::Number(number, _) => match number.parse::<u64>() {
                    Ok(number) if number <= cap as u64 => false,
                    Ok(_) => { *expr = Expr::Value(Value::Number(cap.to_string(), false).into()); true }
                    Err(_) => false,
                },
                Value::Null => { *expr = Expr::Value(Value::Number(cap.to_string(), false).into()); true }
                _ => false,
            },
            // SQLite LIMIT -1 is unlimited.
            Expr::UnaryOp { op: sqlparser::ast::UnaryOperator::Minus, expr: inner } if matches!(inner.as_ref(), Expr::Value(v) if matches!(&v.value, Value::Number(n, _) if n == "1")) => { *expr = Expr::Value(Value::Number(cap.to_string(), false).into()); true }
            _ => false,
        }
    }
    let dialect: Box<dyn Dialect> = match driver { DatabaseDriver::Sqlite => Box::new(SQLiteDialect {}), DatabaseDriver::Postgres => Box::new(PostgreSqlDialect {}), _ => Box::new(MySqlDialect {}) };
    let Ok(mut statements) = Parser::parse_sql(dialect.as_ref(), sql) else { return (sql.into(), false); };
    if statements.len() != 1 { return (sql.into(), false); }
    let Statement::Query(query) = &mut statements[0] else { return (sql.into(), false); };
    if !read_query(query) { return (sql.into(), false); }
    fn sqlite_accepts_limit(body: &SetExpr) -> bool {
        match body { SetExpr::Select(_) => true, SetExpr::SetOperation { right, .. } => sqlite_accepts_limit(right), _ => false }
    }
    if matches!(driver, DatabaseDriver::Sqlite) && !sqlite_accepts_limit(&query.body) { return (sql.into(), false); }
    if let Some(page_offset) = page_offset {
        // Explicit limits remain the total budget; offset advances within that range.
        if query.fetch.is_some() { return (sql.into(), false); }
        fn number(expr: &Expr) -> Option<usize> {
            match expr { Expr::Value(v) => match &v.value { Value::Number(n, _) => n.parse().ok(), _ => None }, _ => None }
        }
        let (total, base_offset) = match &query.limit_clause {
            None => (None, 0),
            Some(LimitClause::LimitOffset { limit, offset, .. }) => {
                let total = match limit { None => None, Some(expr) => match number(expr) { Some(n) => Some(n), None => return (sql.into(), false) } };
                let offset = match offset { None => 0, Some(offset) => match number(&offset.value) { Some(n) => n, None => return (sql.into(), false) } };
                (total, offset)
            }
            Some(LimitClause::OffsetCommaLimit { offset, limit }) => match (number(limit), number(offset)) { (Some(limit), Some(offset)) => (Some(limit), offset), _ => return (sql.into(), false) },
        };
        let count = total.map(|total| total.saturating_sub(page_offset).min(cap)).unwrap_or(cap);
        let Some(offset) = base_offset.checked_add(page_offset) else { return (sql.into(), false); };
        let literal = |n: usize| Expr::Value(Value::Number(n.to_string(), false).into());
        query.limit_clause = Some(match driver {
            DatabaseDriver::Mysql | DatabaseDriver::Mariadb => LimitClause::OffsetCommaLimit { offset: literal(offset), limit: literal(count) },
            _ => LimitClause::LimitOffset { limit: Some(literal(count)), offset: Some(sqlparser::ast::Offset { value: literal(offset), rows: sqlparser::ast::OffsetRows::None }), limit_by: vec![] },
        });
        return (statements[0].to_string(), total.is_none_or(|n| n.saturating_sub(page_offset) > cap));
    }
    let changed = if let Some(fetch) = &mut query.fetch {
        if fetch.with_ties || fetch.percent { false } else { fetch.quantity.as_mut().is_some_and(|expr| clamp_literal(expr, cap)) }
    } else {
        match &mut query.limit_clause {
            Some(LimitClause::LimitOffset { limit, .. }) => {
                if let Some(expr) = limit { clamp_literal(expr, cap) } else { *limit = Some(Expr::Value(Value::Number(cap.to_string(), false).into())); true }
            }
            Some(LimitClause::OffsetCommaLimit { limit, .. }) => clamp_literal(limit, cap),
            None => { query.limit_clause = Some(LimitClause::LimitOffset { limit: Some(Expr::Value(Value::Number(cap.to_string(), false).into())), offset: None, limit_by: vec![] }); true }
        }
    };
    (if changed { statements[0].to_string() } else { sql.into() }, false)
}

// Stream rows instead of collecting the entire query before applying its budget.
async fn read_postgres_result(client: &tokio_postgres::Client, sql: &str, limit: usize) -> Result<DatabaseQueryResult, String> {
    use futures::TryStreamExt;
    let messages = client.simple_query_raw(sql).await.map_err(|e| e.as_db_error().map(|db| db.message().to_owned()).unwrap_or_else(|| e.to_string()))?;
    futures::pin_mut!(messages);
    let mut columns = Vec::new();
    let mut rows = Vec::new();
    let mut affected_rows = 0;
    let mut truncated = false;
    while let Some(message) = messages.try_next().await.map_err(|e| e.as_db_error().map(|db| db.message().to_owned()).unwrap_or_else(|| e.to_string()))? {
        match message {
            tokio_postgres::SimpleQueryMessage::RowDescription(description) => { if columns.is_empty() { columns = description.iter().map(|c| c.name().to_string()).collect(); } }
            tokio_postgres::SimpleQueryMessage::Row(row) => {
                if rows.len() < limit { rows.push((0..row.len()).map(|i| row.get(i).map(|v| serde_json::Value::String(v.to_string())).unwrap_or(serde_json::Value::Null)).collect()); }
                else { truncated = true; }
            }
            tokio_postgres::SimpleQueryMessage::CommandComplete(count) => affected_rows = count,
            _ => {}
        }
    }
    Ok(DatabaseQueryResult { columns, truncated: truncated || rows.len() == limit, rows, affected_rows, has_more: None })
}

async fn validate_network_config(config: DatabaseConnectionConfig) -> Result<(), String> {
    tokio::time::timeout(CONNECTION_TIMEOUT, execute_network_inner(config, "SELECT 1", 1))
        .await.map_err(|_| "数据库连接测试超时（8 秒）".to_string())?.map(|_| ())
}

/// Test authentication and a read without saving a Runtime or creating a SQLite file.
/// Called only from a blocking worker, like the synchronous Runtime manager.
pub fn test_connection(config: DatabaseConnectionConfig) -> Result<(), String> {
    if matches!(config.driver, DatabaseDriver::Sqlite) {
        let path = config.sqlite_path.ok_or_else(|| "SQLite connection requires sqlitePath".to_string())?;
        let connection = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY).map_err(|e| e.to_string())?;
        connection.query_row("SELECT count(*) FROM sqlite_master", [], |row| row.get::<_, i64>(0))
            .map(|_| ()).map_err(|e| e.to_string())
    } else {
        tauri::async_runtime::block_on(validate_network_config(config))
    }
}

fn mysql_value(value: mysql_async::Value) -> serde_json::Value { match value { mysql_async::Value::NULL => serde_json::Value::Null, mysql_async::Value::Bytes(v) => serde_json::Value::String(String::from_utf8_lossy(&v).into_owned()), mysql_async::Value::Int(v) => serde_json::json!(v), mysql_async::Value::UInt(v) => serde_json::json!(v), mysql_async::Value::Float(v) => serde_json::json!(v), mysql_async::Value::Double(v) => serde_json::json!(v), mysql_async::Value::Date(y,m,d,h,mi,s,ms) => serde_json::json!(format!("{y:04}-{m:02}-{d:02} {h:02}:{mi:02}:{s:02}.{ms:03}")), mysql_async::Value::Time(..) => serde_json::json!(format!("{value:?}")) } }

/// Batching rows per `INSERT` cuts the statement count, and with it the parse and transaction
/// overhead, by two orders of magnitude while staying inside every SQLite build's limits.
const SQL_DUMP_ROWS_PER_STATEMENT: usize = 200;

struct SqlDumpSummary { tables: usize, rows: u64 }

/// The DDL of every dumpable object, in `sqlite_master` order. With `table` set it returns that
/// one object plus the indexes and triggers that belong to it.
fn sqlite_dump_objects(connection: &Connection, table: Option<&str>) -> Result<Vec<(String, String, String)>, String> {
    const BASE: &str = "SELECT type, name, sql FROM sqlite_master WHERE sql IS NOT NULL AND name NOT LIKE 'sqlite\\_%' ESCAPE '\\'";
    let filter = " AND (name = ?1 OR (tbl_name = ?1 AND type IN ('index', 'trigger'))) ORDER BY rowid";
    let mut statement = connection.prepare(&match table { Some(_) => format!("{BASE}{filter}"), None => format!("{BASE} ORDER BY rowid") }).map_err(|e| e.to_string())?;
    let project = |row: &rusqlite::Row<'_>| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, String>(2)?));
    let rows = match table { Some(name) => statement.query_map([name], project), None => statement.query_map([], project) }.map_err(|e| e.to_string())?;
    rows.collect::<rusqlite::Result<Vec<_>>>().map_err(|e| e.to_string())
}

fn write_sql_dump(connection: &Connection, out: &mut impl Write, table: Option<&str>, schema_only: bool, progress: Option<&DatabaseExportProgressSink>) -> Result<SqlDumpSummary, String> {
    let objects = sqlite_dump_objects(connection, table)?;
    if objects.is_empty() && table.is_some() { return Err("数据库中找不到该表或视图".into()); }
    // Tables are created before any data lands, and indexes/triggers/views only afterwards so a
    // restore does not pay index maintenance on every inserted row.
    let tables = objects.iter().filter(|(kind, _, _)| kind == "table").cloned().collect::<Vec<_>>();
    let tail = objects.iter().filter(|(kind, _, _)| kind != "table").cloned().collect::<Vec<_>>();
    match table {
        Some(name) => out.write_all(format!("-- Luna Mux dump: {name}\n\n").as_bytes()),
        None => out.write_all(b"-- Luna Mux database dump\n-- Schema first, then data, then indexes/triggers/views.\n\n"),
    }.map_err(|e| e.to_string())?;
    let total_tables = tables.len();
    if total_tables > 0 { if let Some(report) = progress { report(0, total_tables, None); } }
    for (index, (_, name, sql)) in tables.iter().enumerate() {
        writeln!(out, "{};", sql.trim_end_matches(';')).map_err(|e| e.to_string())?;
        if schema_only { if let Some(report) = progress { report(index + 1, total_tables, Some(name)); } }
    }
    if !tables.is_empty() { out.write_all(b"\n").map_err(|e| e.to_string())?; }
    let mut rows_written = 0u64;
    if !schema_only {
        for (index, (_, name, _)) in tables.iter().enumerate() {
            rows_written += write_sql_table_data(connection, out, name)?;
            if let Some(report) = progress { report(index + 1, total_tables, Some(name)); }
        }
    }
    if !tail.is_empty() { out.write_all(b"\n").map_err(|e| e.to_string())?; }
    for (_, _, sql) in &tail { writeln!(out, "{};", sql.trim_end_matches(';')).map_err(|e| e.to_string())?; }
    Ok(SqlDumpSummary { tables: tables.len(), rows: rows_written })
}

fn write_sql_table_data(connection: &Connection, out: &mut impl Write, table: &str) -> Result<u64, String> {
    let mut statement = connection.prepare(&format!("SELECT * FROM {}", sql_dump_identifier(table))).map_err(|e| e.to_string())?;
    let columns = statement.column_names().iter().map(|name| sql_dump_identifier(name)).collect::<Vec<_>>();
    let mut cursor = statement.query([]).map_err(|e| e.to_string())?;
    let mut written = 0u64;
    let mut batch = 0usize;
    while let Some(row) = cursor.next().map_err(|e| e.to_string())? {
        if batch == 0 {
            write!(out, "INSERT INTO {} ({}) VALUES\n  ", sql_dump_identifier(table), columns.join(", ")).map_err(|e| e.to_string())?;
        } else {
            out.write_all(b",\n  ").map_err(|e| e.to_string())?;
        }
        out.write_all(b"(").map_err(|e| e.to_string())?;
        for index in 0..columns.len() {
            if index > 0 { out.write_all(b", ").map_err(|e| e.to_string())?; }
            write_sql_value(out, row.get_ref(index).map_err(|e| e.to_string())?)?;
        }
        out.write_all(b")").map_err(|e| e.to_string())?;
        batch += 1;
        written += 1;
        if batch == SQL_DUMP_ROWS_PER_STATEMENT { out.write_all(b";\n").map_err(|e| e.to_string())?; batch = 0; }
    }
    if batch > 0 { out.write_all(b";\n").map_err(|e| e.to_string())?; }
    Ok(written)
}

fn sql_dump_identifier(name: &str) -> String { format!("\"{}\"", name.replace('"', "\"\"")) }

// Values are written byte for byte: TEXT may hold bytes that are not valid UTF-8, and a BLOB
// keeps its content here even though the JSON result path can only report its size.
fn write_sql_value(out: &mut impl Write, value: ValueRef<'_>) -> Result<(), String> {
    match value {
        ValueRef::Null => out.write_all(b"NULL").map_err(|e| e.to_string()),
        ValueRef::Integer(number) => write!(out, "{number}").map_err(|e| e.to_string()),
        // `{:?}` prints the shortest representation that reads back as the same f64; the SQL
        // literal path cannot express infinities or NaN, so those become NULL like BLOBs elsewhere.
        ValueRef::Real(number) => if number.is_finite() { write!(out, "{number:?}").map_err(|e| e.to_string()) } else { out.write_all(b"NULL").map_err(|e| e.to_string()) },
        // A TEXT value may hold bytes that are not valid UTF-8; those are dumped as a hex cast so
        // the dump file itself stays valid UTF-8 and the exact bytes still round trip.
        ValueRef::Text(bytes) if std::str::from_utf8(bytes).is_err() => {
            out.write_all(b"CAST(").map_err(|e| e.to_string())?;
            write_sql_hex(out, bytes)?;
            out.write_all(b" AS TEXT)").map_err(|e| e.to_string())
        }
        ValueRef::Text(bytes) => {
            out.write_all(b"'").map_err(|e| e.to_string())?;
            let mut start = 0;
            for (index, byte) in bytes.iter().enumerate() {
                if *byte != b'\'' { continue; }
                out.write_all(&bytes[start..index]).map_err(|e| e.to_string())?;
                out.write_all(b"''").map_err(|e| e.to_string())?;
                start = index + 1;
            }
            out.write_all(&bytes[start..]).map_err(|e| e.to_string())?;
            out.write_all(b"'").map_err(|e| e.to_string())
        }
        ValueRef::Blob(bytes) => write_sql_hex(out, bytes),
    }
}

fn write_sql_hex(out: &mut impl Write, bytes: &[u8]) -> Result<(), String> {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    out.write_all(b"X'").map_err(|e| e.to_string())?;
    let mut pair = [0u8; 2];
    for byte in bytes {
        pair[0] = HEX[(byte >> 4) as usize];
        pair[1] = HEX[(byte & 0x0f) as usize];
        out.write_all(&pair).map_err(|e| e.to_string())?;
    }
    out.write_all(b"'").map_err(|e| e.to_string())
}

// ---------------------------------------------------------------------------------------------
// Network dumps and restores (MySQL / MariaDB / PostgreSQL)
// ---------------------------------------------------------------------------------------------

/// Dumps stream through a one-megabyte buffer and are never bounded by the webview or by the
/// interactive 30 second query budget: a large table is expected to take longer than a query.
const SQL_DUMP_BUFFER_BYTES: usize = 1 << 20;

/// MySQL reports the binary charset for values with no text form (BLOB, BINARY, VARBINARY, BIT,
/// GEOMETRY, and every numeric or temporal column).
const MYSQL_BINARY_CHARSET: u16 = 63;

fn create_dump_writer(path: &Path) -> Result<BufWriter<File>, String> {
    let file = File::create(path).map_err(|e| format!("无法写入导出文件：{e}"))?;
    Ok(BufWriter::with_capacity(SQL_DUMP_BUFFER_BYTES, file))
}

/// The byte count comes from the finished file, so the reported size always matches the disk.
fn finish_dump(mut out: BufWriter<File>) -> Result<u64, String> {
    out.flush().map_err(|e| format!("无法写入导出文件：{e}"))?;
    let file = out.into_inner().map_err(|e| format!("无法写入导出文件：{}", e.into_error()))?;
    file.metadata().map(|meta| meta.len()).map_err(|e| e.to_string())
}

fn sql_identifier(quote: char, name: &str) -> String { format!("{quote}{}{quote}", name.replace(quote, &format!("{quote}{quote}"))) }

fn sql_string_literal(value: &str) -> String { format!("'{}'", value.replace('\'', "''")) }

async fn export_network_sql(config: DatabaseConnectionConfig, path: &Path, table: Option<&str>, schema_only: bool, progress: Option<DatabaseExportProgressSink>) -> Result<DatabaseSqlExport, String> {
    match config.driver {
        DatabaseDriver::Sqlite => Err("SQLite 使用本地连接".into()),
        DatabaseDriver::Postgres => postgres_export_sql_with_progress(config, path, table, schema_only, progress).await,
        DatabaseDriver::Mysql | DatabaseDriver::Mariadb => mysql_export_sql_with_progress(config, path, table, schema_only, progress).await,
    }
}

/// Network scripts are executed by the server in a single request: MySQL through
/// `CLIENT_MULTI_STATEMENTS` (always negotiated by the driver) and PostgreSQL through the
/// simple-query protocol, so semicolons inside strings and comments stay the server's problem.
async fn import_network_sql(config: DatabaseConnectionConfig, script: &str) -> Result<(), String> {
    match config.driver {
        DatabaseDriver::Sqlite => Err("SQLite 使用本地连接".into()),
        DatabaseDriver::Postgres => {
            let (client, _connection_task) = postgres_client(&config).await?;
            client.batch_execute(&format!("SET client_encoding = 'UTF8';\n{script}")).await.map_err(postgres_error)
        }
        DatabaseDriver::Mysql | DatabaseDriver::Mariadb => {
            let mut connection = mysql_client(&config).await?;
            let outcome = connection.query_drop(script).await.map_err(|e| e.to_string());
            let _ = connection.disconnect().await;
            outcome
        }
    }
}

async fn mysql_client(config: &DatabaseConnectionConfig) -> Result<mysql_async::Conn, String> {
    let host = config.host.clone().ok_or_else(|| "数据库主机不能为空".to_string())?;
    let port = config.port.unwrap_or(3306);
    let user = config.username.clone().unwrap_or_default();
    let database = config.database.clone().unwrap_or_default();
    let mut builder = mysql_async::OptsBuilder::default().ip_or_hostname(host).tcp_port(port).prefer_socket(false).user(Some(user)).pass(config.password.clone()).db_name((!database.is_empty()).then_some(database));
    if config.ssl_enabled { builder = builder.ssl_opts(Some(mysql_async::SslOpts::default())); }
    tokio::time::timeout(CONNECTION_TIMEOUT, mysql_async::Conn::new(builder)).await.map_err(|_| "数据库连接超时（8 秒）".to_string())?.map_err(|e| e.to_string())
}

/// How a value has to be spelled in the dump: numbers stay bare literals, values with no text
/// form become hex literals, and everything else is a quoted string.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum MysqlValueKind { Number, Binary, Text }

fn mysql_value_kind(column: &mysql_async::Column) -> MysqlValueKind {
    use mysql_async::consts::ColumnType::*;
    // Temporal columns also report the binary charset but do have a text form.
    match column.column_type() {
        MYSQL_TYPE_DATE | MYSQL_TYPE_NEWDATE | MYSQL_TYPE_DATETIME | MYSQL_TYPE_DATETIME2
        | MYSQL_TYPE_TIMESTAMP | MYSQL_TYPE_TIMESTAMP2 | MYSQL_TYPE_TIME | MYSQL_TYPE_TIME2 => MysqlValueKind::Text,
        kind if kind.is_numeric_type() => MysqlValueKind::Number,
        _ if column.character_set() == MYSQL_BINARY_CHARSET => MysqlValueKind::Binary,
        _ => MysqlValueKind::Text,
    }
}

struct MysqlDumpObject { name: String, is_view: bool }

async fn mysql_dump_objects(connection: &mut mysql_async::Conn, table: Option<&str>) -> Result<Vec<MysqlDumpObject>, String> {
    let mut sql = String::from("SELECT table_name, table_type FROM information_schema.tables WHERE table_schema = DATABASE()");
    if let Some(name) = table { sql.push_str(" AND table_name = "); sql.push_str(&sql_string_literal(name)); }
    sql.push_str(" ORDER BY table_type, table_name");
    let rows = connection.query::<(String, String), _>(sql).await.map_err(|e| e.to_string())?;
    Ok(rows.into_iter().map(|(name, kind)| MysqlDumpObject { name, is_view: kind.eq_ignore_ascii_case("VIEW") }).collect())
}

async fn mysql_object_ddl(connection: &mut mysql_async::Conn, object: &MysqlDumpObject) -> Result<String, String> {
    let statement = if object.is_view { "SHOW CREATE VIEW" } else { "SHOW CREATE TABLE" };
    let sql = format!("{statement} {}", sql_identifier('`', &object.name));
    let missing = || format!("无法读取 {} 的建表语句", object.name);
    let row = connection.query_first::<mysql_async::Row, _>(sql).await.map_err(|e| e.to_string())?.ok_or_else(missing)?;
    let ddl: Option<String> = row.get(1);
    Ok(ddl.ok_or_else(missing)?.trim_end_matches(';').to_string())
}

async fn mysql_write_table_data(connection: &mut mysql_async::Conn, out: &mut impl Write, table: &str) -> Result<u64, String> {
    let mut result = connection.query_iter(format!("SELECT * FROM {}", sql_identifier('`', table))).await.map_err(|e| e.to_string())?;
    let columns = result.columns_ref().to_vec();
    let names = columns.iter().map(|column| sql_identifier('`', &column.name_str())).collect::<Vec<_>>().join(", ");
    let mut written = 0u64;
    let mut batch = 0usize;
    while let Some(row) = result.next().await.map_err(|e| e.to_string())? {
        if batch == 0 {
            write!(out, "INSERT INTO {} ({names}) VALUES\n  ", sql_identifier('`', table)).map_err(|e| e.to_string())?;
        } else {
            out.write_all(b",\n  ").map_err(|e| e.to_string())?;
        }
        out.write_all(b"(").map_err(|e| e.to_string())?;
        let values = row.unwrap();
        for (index, value) in values.iter().enumerate() {
            if index > 0 { out.write_all(b", ").map_err(|e| e.to_string())?; }
            let kind = columns.get(index).map(mysql_value_kind).unwrap_or(MysqlValueKind::Text);
            mysql_dump_value(out, value, kind)?;
        }
        out.write_all(b")").map_err(|e| e.to_string())?;
        batch += 1;
        written += 1;
        if batch == SQL_DUMP_ROWS_PER_STATEMENT { out.write_all(b";\n").map_err(|e| e.to_string())?; batch = 0; }
    }
    if batch > 0 { out.write_all(b";\n").map_err(|e| e.to_string())?; }
    Ok(written)
}

fn mysql_dump_value(out: &mut impl Write, value: &mysql_async::Value, kind: MysqlValueKind) -> Result<(), String> {
    use mysql_async::Value;
    match value {
        Value::NULL => out.write_all(b"NULL").map_err(|e| e.to_string()),
        Value::Bytes(bytes) => match kind {
            // The text protocol returns canonical numbers; anything else would be a malformed
            // literal, so it falls back to the quoted form and fails loudly at import time.
            MysqlValueKind::Number if !bytes.is_empty() && bytes.iter().all(|byte| byte.is_ascii_digit() || matches!(byte, b'+' | b'-' | b'.' | b'e' | b'E')) => out.write_all(bytes).map_err(|e| e.to_string()),
            MysqlValueKind::Number | MysqlValueKind::Text => mysql_dump_string(out, bytes),
            MysqlValueKind::Binary => write_sql_hex(out, bytes),
        },
        Value::Int(number) => write!(out, "{number}").map_err(|e| e.to_string()),
        Value::UInt(number) => write!(out, "{number}").map_err(|e| e.to_string()),
        Value::Float(number) => write!(out, "{number:?}").map_err(|e| e.to_string()),
        Value::Double(number) => write!(out, "{number:?}").map_err(|e| e.to_string()),
        Value::Date(year, month, day, hour, minute, second, micros) => mysql_dump_string(out, mysql_datetime_text(*year, *month, *day, *hour, *minute, *second, *micros).as_bytes()),
        Value::Time(negative, days, hours, minutes, seconds, micros) => mysql_dump_string(out, mysql_time_text(*negative, *days, *hours, *minutes, *seconds, *micros).as_bytes()),
    }
}

/// Escapes only what MySQL string literals need, and hands bytes that have no printable text form
/// (including anything with a backslash, which `NO_BACKSLASH_ESCAPES` would read differently) to
/// the hex form so the dump restores byte for byte under either server mode.
fn mysql_dump_string(out: &mut impl Write, bytes: &[u8]) -> Result<(), String> {
    if bytes.iter().any(|byte| *byte < 0x20 || *byte == 0x7f || *byte == b'\\') || std::str::from_utf8(bytes).is_err() {
        return write_sql_hex(out, bytes);
    }
    out.write_all(b"'").map_err(|e| e.to_string())?;
    let mut start = 0;
    for (index, byte) in bytes.iter().enumerate() {
        if *byte != b'\'' { continue; }
        out.write_all(&bytes[start..index]).map_err(|e| e.to_string())?;
        out.write_all(b"''").map_err(|e| e.to_string())?;
        start = index + 1;
    }
    out.write_all(&bytes[start..]).map_err(|e| e.to_string())?;
    out.write_all(b"'").map_err(|e| e.to_string())
}

fn mysql_datetime_text(year: u16, month: u8, day: u8, hour: u8, minute: u8, second: u8, micros: u32) -> String {
    let base = format!("{year:04}-{month:02}-{day:02} {hour:02}:{minute:02}:{second:02}");
    if micros == 0 { base } else { format!("{base}.{micros:06}") }
}

fn mysql_time_text(negative: bool, days: u32, hours: u8, minutes: u8, seconds: u8, micros: u32) -> String {
    let sign = if negative { "-" } else { "" };
    let base = format!("{sign}{:02}:{minutes:02}:{seconds:02}", days * 24 + u32::from(hours));
    if micros == 0 { base } else { format!("{base}.{micros:06}") }
}

#[allow(dead_code)]
async fn mysql_export_sql(config: DatabaseConnectionConfig, path: &Path, table: Option<&str>, schema_only: bool) -> Result<DatabaseSqlExport, String> {
    mysql_export_sql_with_progress(config, path, table, schema_only, None).await
}

async fn mysql_export_sql_with_progress(config: DatabaseConnectionConfig, path: &Path, table: Option<&str>, schema_only: bool, progress: Option<DatabaseExportProgressSink>) -> Result<DatabaseSqlExport, String> {
    let mut connection = mysql_client(&config).await?;
    let outcome = async {
        let objects = mysql_dump_objects(&mut connection, table).await?;
        if objects.is_empty() && table.is_some() { return Err("数据库中找不到该表或视图".into()); }
        let (tables, views) = (objects.iter().filter(|object| !object.is_view).collect::<Vec<_>>(), objects.iter().filter(|object| object.is_view).collect::<Vec<_>>());
        let total_tables = tables.len();
        if total_tables > 0 { if let Some(report) = progress.as_ref() { report(0, total_tables, None); } }
        let mut out = create_dump_writer(path)?;
        match table {
            Some(name) => out.write_all(format!("-- Luna Mux dump: {name}\n-- Triggers, routines and events are not included.\n\n").as_bytes()),
            None => out.write_all(b"-- Luna Mux database dump\n-- Schema first, then data. Triggers, routines and events are not included.\n\n"),
        }.map_err(|e| e.to_string())?;
        out.write_all(b"SET NAMES utf8mb4;\n\n").map_err(|e| e.to_string())?;
        for (index, object) in tables.iter().enumerate() {
            writeln!(out, "DROP TABLE IF EXISTS {};\n{};", sql_identifier('`', &object.name), mysql_object_ddl(&mut connection, object).await?).map_err(|e| e.to_string())?;
            if schema_only { if let Some(report) = progress.as_ref() { report(index + 1, total_tables, Some(&object.name)); } }
        }
        if !tables.is_empty() { out.write_all(b"\n").map_err(|e| e.to_string())?; }
        let mut rows = 0u64;
        if !schema_only {
            for (index, object) in tables.iter().enumerate() {
                rows += mysql_write_table_data(&mut connection, &mut out, &object.name).await?;
                if let Some(report) = progress.as_ref() { report(index + 1, total_tables, Some(&object.name)); }
            }
        }
        // Views come last: they may select from any table in the dump.
        if !views.is_empty() { out.write_all(b"\n").map_err(|e| e.to_string())?; }
        for object in &views {
            writeln!(out, "DROP VIEW IF EXISTS {};\n{};", sql_identifier('`', &object.name), mysql_object_ddl(&mut connection, object).await?).map_err(|e| e.to_string())?;
        }
        Ok(DatabaseSqlExport { tables: tables.len(), rows, bytes: finish_dump(out)? })
    }.await;
    let _ = connection.disconnect().await;
    outcome
}

async fn postgres_client(config: &DatabaseConnectionConfig) -> Result<(tokio_postgres::Client, ConnectionTask), String> {
    let host = config.host.clone().ok_or_else(|| "数据库主机不能为空".to_string())?;
    let port = config.port.unwrap_or(5432);
    let user = config.username.clone().unwrap_or_default();
    let database = config.database.clone().unwrap_or_default();
    let mut settings = tokio_postgres::Config::new();
    settings.host(&host).port(port).user(&user);
    if !database.is_empty() { settings.dbname(&database); }
    if let Some(password) = config.password.as_deref() { settings.password(password); }
    if config.ssl_enabled { settings.ssl_mode(tokio_postgres::config::SslMode::Require); }
    let connect = async {
        if config.ssl_enabled {
            let (tls, _) = tokio_postgres_rustls::MakeRustlsConnect::with_native_certs().map_err(|_| "系统证书加载失败".to_string())?;
            let (client, connection) = settings.connect(tls).await.map_err(|e| e.to_string())?;
            Ok::<_, String>((client, ConnectionTask(tokio::spawn(async move { let _ = connection.await; }))))
        } else {
            let (client, connection) = settings.connect(tokio_postgres::NoTls).await.map_err(|e| e.to_string())?;
            Ok((client, ConnectionTask(tokio::spawn(async move { let _ = connection.await; }))))
        }
    };
    tokio::time::timeout(CONNECTION_TIMEOUT, connect).await.map_err(|_| "数据库连接超时（8 秒）".to_string())?
}

fn postgres_error(error: tokio_postgres::Error) -> String {
    error.as_db_error().map(|db| db.message().to_owned()).unwrap_or_else(|| error.to_string())
}

fn postgres_quote_ident(name: &str) -> String { sql_identifier('"', name) }

/// `regclass` parses its input like an identifier, so the quoted name keeps the exact case.
fn postgres_relclass(name: &str) -> String { format!("{}::regclass", sql_string_literal(&postgres_quote_ident(name))) }

fn postgres_text_rows(messages: Vec<tokio_postgres::SimpleQueryMessage>) -> Vec<Vec<Option<String>>> {
    messages.into_iter().filter_map(|message| match message {
        tokio_postgres::SimpleQueryMessage::Row(row) => Some((0..row.len()).map(|index| row.get(index).map(str::to_string)).collect()),
        _ => None,
    }).collect()
}

/// A `NULL` and a missing column both read as "no value", so callers can treat the dump as text.
fn text_at(row: &[Option<String>], index: usize) -> Option<String> { row.get(index).cloned().flatten() }

async fn postgres_query_rows(client: &tokio_postgres::Client, sql: &str) -> Result<Vec<Vec<Option<String>>>, String> {
    client.simple_query(sql).await.map(postgres_text_rows).map_err(postgres_error)
}

struct PostgresDumpObject { name: String, is_view: bool }

/// One column of a dumped table. `serial` marks a column whose default came from a sequence, and
/// `identity` marks a generated column; both are recreated as the matching pseudo-type and are
/// re-seeded after the data is loaded.
struct PostgresColumn { name: String, type_sql: String, not_null: bool, default: Option<String>, serial: Option<&'static str>, identity: bool }

impl PostgresColumn {
    fn is_bytea(&self) -> bool { self.type_sql == "bytea" }
    /// A `GENERATED ALWAYS AS IDENTITY` column rejects the explicit values a dump has to insert,
    /// so the dump recreates identity columns as `BY DEFAULT`, which accepts them.
    fn definition(&self) -> String {
        let mut definition = format!("  {} {}", postgres_quote_ident(&self.name), self.serial.unwrap_or(&self.type_sql));
        if self.identity { definition.push_str(" GENERATED BY DEFAULT AS IDENTITY"); }
        if self.not_null { definition.push_str(" NOT NULL"); }
        if self.serial.is_none() && !self.identity { if let Some(default) = &self.default { definition.push_str(&format!(" DEFAULT {default}")); } }
        definition
    }
    fn needs_sequence_reset(&self) -> bool { self.identity || self.serial.is_some() }
}

async fn postgres_dump_objects(client: &tokio_postgres::Client, table: Option<&str>) -> Result<Vec<PostgresDumpObject>, String> {
    let mut sql = String::from("SELECT table_name, table_type FROM information_schema.tables WHERE table_schema = current_schema()");
    if let Some(name) = table { sql.push_str(" AND table_name = "); sql.push_str(&sql_string_literal(name)); }
    sql.push_str(" ORDER BY table_type, table_name");
    Ok(postgres_query_rows(client, &sql).await?.into_iter().map(|row| PostgresDumpObject {
        name: text_at(&row, 0).unwrap_or_default(),
        is_view: text_at(&row, 1).is_some_and(|kind| kind.eq_ignore_ascii_case("VIEW")),
    }).collect())
}

async fn postgres_table_columns(client: &tokio_postgres::Client, table: &str) -> Result<Vec<PostgresColumn>, String> {
    let sql = format!("SELECT a.attname, format_type(a.atttypid, a.atttypmod), a.attnotnull, a.attidentity, pg_get_expr(d.adbin, d.adrelid) \
        FROM pg_attribute a LEFT JOIN pg_attrdef d ON d.adrelid = a.attrelid AND d.adnum = a.attnum \
        WHERE a.attrelid = {} AND a.attnum > 0 AND NOT a.attisdropped ORDER BY a.attnum", postgres_relclass(table));
    Ok(postgres_query_rows(client, &sql).await?.into_iter().map(|row| {
        let type_sql = text_at(&row, 1).unwrap_or_default();
        let default = text_at(&row, 4);
        PostgresColumn {
            name: text_at(&row, 0).unwrap_or_default(),
            serial: postgres_serial_type(&type_sql, default.as_deref()),
            type_sql,
            not_null: text_at(&row, 2).is_some_and(|value| value == "t"),
            default,
            identity: text_at(&row, 3).is_some_and(|value| value == "a" || value == "d"),
        }
    }).collect())
}

/// A `nextval(...)` default means the column was declared `serial`; the dump recreates the
/// pseudo-type instead, which also recreates the sequence it owns.
fn postgres_serial_type(type_sql: &str, default: Option<&str>) -> Option<&'static str> {
    if !default.is_some_and(|default| default.starts_with("nextval(")) { return None; }
    match type_sql { "smallint" => Some("smallserial"), "integer" => Some("serial"), "bigint" => Some("bigserial"), _ => None }
}

async fn postgres_table_indexes(client: &tokio_postgres::Client, table: &str) -> Result<Vec<String>, String> {
    // Constraint-backed indexes are recreated with their constraint, so they are skipped here.
    let sql = format!("SELECT pg_get_indexdef(i.indexrelid) FROM pg_index i WHERE i.indrelid = {} AND NOT i.indisprimary \
        AND NOT EXISTS (SELECT 1 FROM pg_constraint c WHERE c.conindid = i.indexrelid) ORDER BY i.indexrelid", postgres_relclass(table));
    Ok(postgres_query_rows(client, &sql).await?.into_iter().filter_map(|row| text_at(&row, 0)).collect())
}

async fn postgres_table_constraints(client: &tokio_postgres::Client, table: &str) -> Result<Vec<(String, String, String)>, String> {
    let sql = format!("SELECT conname, contype::text, pg_get_constraintdef(oid) FROM pg_constraint WHERE conrelid = {} \
        AND contype IN ('p','u','c','f') ORDER BY contype, conname", postgres_relclass(table));
    Ok(postgres_query_rows(client, &sql).await?.into_iter().filter_map(|row| match (text_at(&row, 0), text_at(&row, 1), text_at(&row, 2)) {
        (Some(name), Some(kind), Some(definition)) => Some((name, kind, definition)),
        _ => None,
    }).collect())
}

async fn postgres_view_definition(client: &tokio_postgres::Client, view: &str) -> Result<String, String> {
    let sql = format!("SELECT pg_get_viewdef({}, true)", postgres_relclass(view));
    postgres_query_rows(client, &sql).await?.into_iter().next().and_then(|row| text_at(&row, 0))
        .ok_or_else(|| format!("无法读取视图 {view} 的定义"))
}

async fn postgres_write_table_data(client: &tokio_postgres::Client, out: &mut impl Write, table: &str, columns: &[PostgresColumn]) -> Result<u64, String> {
    use futures::TryStreamExt;
    let messages = client.simple_query_raw(&format!("SELECT * FROM {}", postgres_quote_ident(table))).await.map_err(postgres_error)?;
    futures::pin_mut!(messages);
    let mut names: Option<Vec<String>> = None;
    let mut written = 0u64;
    let mut batch = 0usize;
    while let Some(message) = messages.try_next().await.map_err(postgres_error)? {
        let row = match message {
            tokio_postgres::SimpleQueryMessage::RowDescription(description) => { names = Some(description.iter().map(|column| column.name().to_string()).collect()); continue; }
            tokio_postgres::SimpleQueryMessage::Row(row) => row,
            _ => continue,
        };
        let names = names.as_ref().ok_or_else(|| "查询结果缺少列信息".to_string())?;
        if batch == 0 {
            write!(out, "INSERT INTO {} ({}) VALUES\n  ", postgres_quote_ident(table), names.iter().map(|name| postgres_quote_ident(name)).collect::<Vec<_>>().join(", ")).map_err(|e| e.to_string())?;
        } else {
            out.write_all(b",\n  ").map_err(|e| e.to_string())?;
        }
        out.write_all(b"(").map_err(|e| e.to_string())?;
        for (index, name) in names.iter().enumerate() {
            if index > 0 { out.write_all(b", ").map_err(|e| e.to_string())?; }
            let bytea = columns.iter().find(|column| &column.name == name).is_some_and(PostgresColumn::is_bytea);
            postgres_dump_value(out, row.get(index), bytea)?;
        }
        out.write_all(b")").map_err(|e| e.to_string())?;
        batch += 1;
        written += 1;
        if batch == SQL_DUMP_ROWS_PER_STATEMENT { out.write_all(b";\n").map_err(|e| e.to_string())?; batch = 0; }
    }
    if batch > 0 { out.write_all(b";\n").map_err(|e| e.to_string())?; }
    Ok(written)
}

/// PostgreSQL's simple-query rows always arrive as text, so every value is a quoted literal and
/// the column type coerces it back. `bytea` and any value holding a backslash use the `E'...'`
/// form, which means the same thing with or without `standard_conforming_strings`.
fn postgres_dump_value(out: &mut impl Write, value: Option<&str>, bytea: bool) -> Result<(), String> {
    let Some(text) = value else { return out.write_all(b"NULL").map_err(|e| e.to_string()) };
    let escaped = bytea || text.contains('\\');
    out.write_all(if escaped { b"E'" } else { b"'" }).map_err(|e| e.to_string())?;
    let mut start = 0;
    for (index, byte) in text.bytes().enumerate() {
        let replacement: &[u8] = match byte { b'\'' => b"''", b'\\' if escaped => b"\\\\", _ => continue };
        out.write_all(&text.as_bytes()[start..index]).map_err(|e| e.to_string())?;
        out.write_all(replacement).map_err(|e| e.to_string())?;
        start = index + 1;
    }
    out.write_all(&text.as_bytes()[start..]).map_err(|e| e.to_string())?;
    out.write_all(b"'").map_err(|e| e.to_string())
}

/// Re-seeds the sequences behind serial and identity columns so the next insert does not collide
/// with the rows the dump just restored.
fn postgres_sequence_reset(table: &str, column: &PostgresColumn) -> String {
    let name = postgres_quote_ident(&column.name);
    format!("SELECT setval(pg_get_serial_sequence({}, {}), COALESCE(MAX({name}), 1), COUNT({name}) > 0) FROM {};",
        sql_string_literal(&postgres_quote_ident(table)), sql_string_literal(&column.name), postgres_quote_ident(table))
}

#[allow(dead_code)]
async fn postgres_export_sql(config: DatabaseConnectionConfig, path: &Path, table: Option<&str>, schema_only: bool) -> Result<DatabaseSqlExport, String> {
    postgres_export_sql_with_progress(config, path, table, schema_only, None).await
}

async fn postgres_export_sql_with_progress(config: DatabaseConnectionConfig, path: &Path, table: Option<&str>, schema_only: bool, progress: Option<DatabaseExportProgressSink>) -> Result<DatabaseSqlExport, String> {
    let (client, _connection_task) = postgres_client(&config).await?;
    let objects = postgres_dump_objects(&client, table).await?;
    if objects.is_empty() && table.is_some() { return Err("数据库中找不到该表或视图".into()); }
    let (tables, views) = (objects.iter().filter(|object| !object.is_view).collect::<Vec<_>>(), objects.iter().filter(|object| object.is_view).collect::<Vec<_>>());
    let total_tables = tables.len();
    if total_tables > 0 { if let Some(report) = progress.as_ref() { report(0, total_tables, None); } }
    let mut out = create_dump_writer(path)?;
    match table {
        Some(name) => out.write_all(format!("-- Luna Mux dump: {name}\n-- Triggers, functions, ownership and grants are not included.\n\n").as_bytes()),
        None => out.write_all(b"-- Luna Mux database dump\n-- Tables first, then data, then indexes, constraints and views.\n-- Triggers, functions, ownership and grants are not included.\n\n"),
    }.map_err(|e| e.to_string())?;
    out.write_all(b"SET client_encoding = 'UTF8';\n\n").map_err(|e| e.to_string())?;
    let mut dumped = Vec::new();
    for (index, object) in tables.iter().enumerate() {
        let columns = postgres_table_columns(&client, &object.name).await?;
        writeln!(out, "DROP TABLE IF EXISTS {};", postgres_quote_ident(&object.name)).map_err(|e| e.to_string())?;
        writeln!(out, "CREATE TABLE {} (", postgres_quote_ident(&object.name)).map_err(|e| e.to_string())?;
        writeln!(out, "{}", columns.iter().map(PostgresColumn::definition).collect::<Vec<_>>().join(",\n")).map_err(|e| e.to_string())?;
        out.write_all(b");\n\n").map_err(|e| e.to_string())?;
        if schema_only { if let Some(report) = progress.as_ref() { report(index + 1, total_tables, Some(&object.name)); } }
        dumped.push((object.name.clone(), columns));
    }
    let mut rows = 0u64;
    if !schema_only {
        for (index, (name, columns)) in dumped.iter().enumerate() {
            rows += postgres_write_table_data(&client, &mut out, name, columns).await?;
            if let Some(report) = progress.as_ref() { report(index + 1, total_tables, Some(name)); }
        }
    }
    if !dumped.is_empty() { out.write_all(b"\n").map_err(|e| e.to_string())?; }
    for (name, columns) in &dumped {
        for index in postgres_table_indexes(&client, name).await? { writeln!(out, "{index};").map_err(|e| e.to_string())?; }
        for (constraint, _, definition) in postgres_table_constraints(&client, name).await?.iter().filter(|(_, kind, _)| kind != "f") {
            writeln!(out, "ALTER TABLE {} ADD CONSTRAINT {} {definition};", postgres_quote_ident(name), postgres_quote_ident(constraint)).map_err(|e| e.to_string())?;
        }
        if !schema_only {
            for column in columns.iter().filter(|column| column.needs_sequence_reset()) { writeln!(out, "{}", postgres_sequence_reset(name, column)).map_err(|e| e.to_string())?; }
        }
    }
    // Foreign keys land after the data so a restore does not pay a per-row check, and so tables
    // can be created in any order.
    for (name, _) in &dumped {
        for (constraint, _, definition) in postgres_table_constraints(&client, name).await?.iter().filter(|(_, kind, _)| kind == "f") {
            writeln!(out, "ALTER TABLE {} ADD CONSTRAINT {} {definition};", postgres_quote_ident(name), postgres_quote_ident(constraint)).map_err(|e| e.to_string())?;
        }
    }
    if !views.is_empty() { out.write_all(b"\n").map_err(|e| e.to_string())?; }
    for object in &views {
        writeln!(out, "DROP VIEW IF EXISTS {};\nCREATE VIEW {} AS {};", postgres_quote_ident(&object.name), postgres_quote_ident(&object.name), postgres_view_definition(&client, &object.name).await?.trim_end_matches(';')).map_err(|e| e.to_string())?;
    }
    Ok(DatabaseSqlExport { tables: tables.len(), rows, bytes: finish_dump(out)? })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};
    fn temp_db() -> PathBuf { std::env::temp_dir().join(format!("luna-mux-db-{}.sqlite", SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos())) }
    fn network_config(driver: DatabaseDriver, port: u16) -> DatabaseConnectionConfig {
        DatabaseConnectionConfig { profile_id: None, driver, host: Some("127.0.0.1".into()), port: Some(port), database: None, username: Some("fixture".into()), password: None, read_only: true, ssl_enabled: false, sqlite_path: None }
    }

    async fn packet(stream: &mut tokio::net::TcpStream, sequence: u8, payload: &[u8]) {
        use tokio::io::AsyncWriteExt;
        let length = payload.len();
        stream.write_all(&[length as u8, (length >> 8) as u8, (length >> 16) as u8, sequence]).await.unwrap();
        stream.write_all(payload).await.unwrap();
    }

    async fn read_packet(stream: &mut tokio::net::TcpStream) -> std::io::Result<Vec<u8>> {
        use tokio::io::AsyncReadExt;
        let mut header = [0; 4]; stream.read_exact(&mut header).await?;
        let length = header[0] as usize | ((header[1] as usize) << 8) | ((header[2] as usize) << 16);
        assert!(length < 4096);
        let mut data = vec![0; length]; stream.read_exact(&mut data).await?; Ok(data)
    }

    // Minimal wire-level server exercises the actual mysql_async driver, including COM_QUIT.
    async fn mysql_server(listener: tokio::net::TcpListener, stall_query: bool) -> bool {
        mysql_rows_server(listener, stall_query, None, false).await
    }
    async fn mysql_rows_server(listener: tokio::net::TcpListener, stall_query: bool, capped_rows: Option<usize>, late_error: bool) -> bool {
        let (mut stream, _) = listener.accept().await.unwrap();
        let caps: u32 = 1 | 4 | 512 | 8192 | 32768 | 524288;
        let mut greeting = vec![10]; greeting.extend_from_slice(b"8.0.36-fixture\0");
        greeting.extend_from_slice(&1u32.to_le_bytes()); greeting.extend_from_slice(b"12345678\0");
        greeting.extend_from_slice(&(caps as u16).to_le_bytes()); greeting.push(45);
        greeting.extend_from_slice(&2u16.to_le_bytes()); greeting.extend_from_slice(&((caps >> 16) as u16).to_le_bytes());
        greeting.push(21); greeting.extend_from_slice(&[0; 10]); greeting.extend_from_slice(b"abcdefghijkl\0mysql_native_password\0");
        packet(&mut stream, 0, &greeting).await;
        read_packet(&mut stream).await.unwrap();
        packet(&mut stream, 2, &[0, 0, 0, 2, 0, 0, 0]).await;
        while let Ok(data) = read_packet(&mut stream).await {
            if data.first() == Some(&1) { return true; }
            assert_eq!(data.first(), Some(&3));
            let sql = String::from_utf8_lossy(&data[1..]);
            let values = if sql.contains("@@max_allowed_packet") { vec!["67108864", "28800"] } else if sql.contains("column_comment") { vec!["id", "int", "NO", "0", "用户编号"] } else {
                assert!(sql.starts_with("SELECT 1"));
                if stall_query {
                    // Never return a query result; only observe client cleanup.
                    return read_packet(&mut stream).await.map(|data| data.first() == Some(&1)).unwrap_or(true);
                }
                vec!["1"]
            };
            packet(&mut stream, 1, &[values.len() as u8]).await;
            let mut sequence = 2;
            for _ in &values {
                // ColumnDefinition41: catalog/schema/table/org_table/name/org_name + fixed fields.
                let mut column = vec![3, b'd', b'e', b'f', 0, 0, 0, 1, b'1', 0, 12, 45, 0];
                column.extend_from_slice(&20u32.to_le_bytes()); column.extend_from_slice(&[3, 0, 0, 0, 0, 0]);
                packet(&mut stream, sequence, &column).await; sequence += 1;
            }
            packet(&mut stream, sequence, &[254, 0, 0, 2, 0]).await; sequence += 1;
            let mut row = Vec::new(); for value in values { row.push(value.len() as u8); row.extend_from_slice(value.as_bytes()); }
            for _ in 0..if sql.starts_with("SELECT 1") { capped_rows.unwrap_or(1) } else { 1 } {
                packet(&mut stream, sequence, &row).await; sequence = sequence.wrapping_add(1);
            }
            if sql.starts_with("SELECT 1") && late_error {
                let mut error = vec![255, 85, 5]; error.extend_from_slice(b"#22012late database error");
                packet(&mut stream, sequence, &error).await;
                return read_packet(&mut stream).await.map(|data| data.first() == Some(&1)).unwrap_or(true);
            }
            if sql.starts_with("SELECT 1") && capped_rows.is_some() { assert!(sql.contains("LIMIT 500")); }
            packet(&mut stream, sequence, &[254, 0, 0, 2, 0]).await;
        }
        false
    }

    #[tokio::test]
    async fn mysql_row_budget_is_applied_on_server_and_completed() {
        for driver in [DatabaseDriver::Mysql, DatabaseDriver::Mariadb] {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let config = network_config(driver, listener.local_addr().unwrap().port());
            let server = tokio::spawn(mysql_rows_server(listener, false, Some(500), false));
            let result = tokio::time::timeout(Duration::from_secs(3), execute_network(config, "SELECT 1", 500)).await.expect("bounded SELECT must finish").unwrap();
            assert_eq!(result.rows.len(), 500); assert!(result.truncated);
            assert!(tokio::time::timeout(Duration::from_secs(2), server).await.unwrap().unwrap());
        }
    }

    #[tokio::test]
    async fn mysql_column_comments_survive_native_metadata_path() {
        for driver in [DatabaseDriver::Mysql, DatabaseDriver::Mariadb] {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let config = network_config(driver.clone(), listener.local_addr().unwrap().port());
            let server = tokio::spawn(mysql_server(listener, false));
            let manager = DatabaseRuntimeManager::new();
            manager.runtimes.lock().unwrap().insert("fixture".into(), RuntimeRecord { summary: DatabaseRuntimeSummary { id: "fixture".into(), driver, status: DatabaseRuntimeStatus::Ready, error: None }, handle: Some(DatabaseHandle::Network(config)) });
            let columns = tokio::task::spawn_blocking(move || manager.describe_table("fixture", "users")).await.unwrap().unwrap();
            assert_eq!(columns[0].comment.as_deref(), Some("用户编号")); assert!(columns[0].not_null);
            assert!(server.await.unwrap());
        }
    }

    #[test]
    fn query_budget_preserves_write_batches_and_existing_limits() {
        for driver in [DatabaseDriver::Sqlite, DatabaseDriver::Mysql, DatabaseDriver::Mariadb, DatabaseDriver::Postgres] {
            assert!(bounded_query_sql("SELECT * FROM users; -- trailing", &driver, 500).ends_with("LIMIT 500"));
            assert!(bounded_query_sql("WITH x AS (SELECT * FROM users) SELECT * FROM x", &driver, 500).ends_with("LIMIT 500"));
            assert!(bounded_query_sql("SELECT * FROM users LIMIT 900 OFFSET 12", &driver, 500).ends_with("LIMIT 500 OFFSET 12"));
            for sql in ["SELECT * FROM users LIMIT 10", "UPDATE users SET id=2", "SELECT 1; SELECT 1/0", "not valid sql", "SELECT * FROM users FOR UPDATE"] { assert_eq!(bounded_query_sql(sql, &driver, 500), sql); }
        }
        for sql in ["SELECT * INTO copied FROM users", "WITH d AS (DELETE FROM users RETURNING *) SELECT * FROM d", "INSERT INTO users SELECT * FROM archive RETURNING *", "SELECT * FROM users LIMIT $1"] { assert_eq!(bounded_query_sql(sql, &DatabaseDriver::Postgres, 500), sql); }
        assert!(bounded_query_sql("SELECT * FROM users LIMIT 20, 900", &DatabaseDriver::Mysql, 500).ends_with("LIMIT 20, 500"));
        assert!(bounded_query_sql("SELECT * FROM users LIMIT -1", &DatabaseDriver::Sqlite, 500).ends_with("LIMIT 500"));
        assert!(bounded_query_sql("SELECT * FROM users FETCH FIRST 900 ROWS ONLY", &DatabaseDriver::Postgres, 500).ends_with("FETCH FIRST 500 ROWS ONLY"));
    }

    #[tokio::test]
    async fn mysql_late_errors_after_row_budget_are_not_reported_as_success() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let config = network_config(DatabaseDriver::Mysql, listener.local_addr().unwrap().port());
        let server = tokio::spawn(mysql_rows_server(listener, false, Some(501), true));
        let error = execute_network(config, "SELECT 1; SELECT 1/0", 500).await.unwrap_err(); assert!(error.contains("late database error"));
        tokio::time::timeout(Duration::from_secs(2), server).await.unwrap().unwrap();
    }

    #[tokio::test]
    async fn postgres_late_errors_and_write_batches_are_consumed_to_completion() {
        for ending in ["error", "complete"] {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let config = network_config(DatabaseDriver::Postgres, listener.local_addr().unwrap().port());
            let server = tokio::spawn(postgres_rows_server(listener, vec!["1"], 700, ending));
            let sql = "BEGIN; UPDATE users SET id=1; SELECT id FROM users; COMMIT";
            let result = execute_network(config, sql, 500).await;
            if ending == "error" { assert!(result.unwrap_err().contains("late database error")); }
            else { let result = result.unwrap(); assert_eq!(result.rows.len(), 500); assert!(result.truncated); }
            let sent = server.await.unwrap(); assert_eq!(sent.trim_end_matches('\0'), sql);
        }
    }

    #[test]
    fn mysql_and_postgres_pages_use_dialect_limits_and_preserve_explicit_budget() {
        assert_eq!(prepare_query("select * from users", &DatabaseDriver::Mysql, 50, Some(0)), ("SELECT * FROM users LIMIT 0, 50".into(), true));
        assert_eq!(prepare_query("select * from users", &DatabaseDriver::Postgres, 100, Some(200)), ("SELECT * FROM users LIMIT 100 OFFSET 200".into(), true));
        assert_eq!(prepare_query("select * from users", &DatabaseDriver::Mysql, 500, Some(0)), ("SELECT * FROM users LIMIT 0, 500".into(), true));
        assert_eq!(prepare_query("select * from users", &DatabaseDriver::Mysql, 500, Some(500)), ("SELECT * FROM users LIMIT 500, 500".into(), true));
        assert_eq!(prepare_query("select * from users", &DatabaseDriver::Postgres, 500, Some(500)), ("SELECT * FROM users LIMIT 500 OFFSET 500".into(), true));
        assert_eq!(prepare_query("select * from users LIMIT 900 OFFSET 20", &DatabaseDriver::Postgres, 500, Some(500)), ("SELECT * FROM users LIMIT 400 OFFSET 520".into(), false));
        for sql in ["DELETE FROM users RETURNING *", "SELECT 1; UPDATE users SET id=2", "SELECT * INTO copied FROM users"] { assert_eq!(prepare_query(sql, &DatabaseDriver::Postgres, 500, Some(0)), (sql.into(), false)); }
    }

    #[test]
    fn sqlite_page_budgets_and_optional_column_comments() {
        let path = temp_db(); let manager = DatabaseRuntimeManager::new();
        let runtime = manager.connect(DatabaseConnectionConfig { sqlite_path: Some(path.clone()), read_only: false, ..network_config(DatabaseDriver::Sqlite, 0) }).unwrap();
        manager.execute(&runtime.id, "CREATE TABLE items(id INTEGER, name TEXT)", 500).unwrap();
        manager.import_sql(&runtime.id, "WITH RECURSIVE n(x) AS (SELECT 1 UNION ALL SELECT x+1 FROM n WHERE x<1500) INSERT INTO items SELECT x,'value' FROM n").unwrap();
        let result = manager.execute(&runtime.id, "SELECT * FROM items", 500).unwrap();
        assert_eq!(result.rows.len(), 500); assert!(result.truncated);
        let result = manager.execute(&runtime.id, "SELECT * FROM items LIMIT 500", 500).unwrap(); assert_eq!(result.rows.len(), 500);
        for sql in ["VALUES (1), (2)", "SELECT 1 UNION ALL VALUES (2)"] {
            assert_eq!(prepare_query(sql, &DatabaseDriver::Sqlite, 500, Some(0)), (sql.into(), false));
            let values = manager.execute_page(&runtime.id, sql, 0, 50).unwrap(); assert_eq!(values.rows.len(), 2); assert_eq!(values.has_more, Some(false));
        }
        let compact = manager.execute_page(&runtime.id, "SELECT * FROM items ORDER BY id", 0, 50).unwrap();
        assert_eq!(compact.rows.len(), 50); assert_eq!(compact.has_more, Some(true));
        let compact_second = manager.execute_page(&runtime.id, "SELECT * FROM items ORDER BY id", 50, 50).unwrap();
        assert_eq!(compact_second.rows.len(), 50); assert_eq!(compact_second.rows[0][0], "51");
        let first = manager.execute_page(&runtime.id, "SELECT * FROM items ORDER BY id", 0, 500).unwrap();
        let second = manager.execute_page(&runtime.id, "SELECT * FROM items ORDER BY id", 500, 500).unwrap();
        assert_eq!(first.rows.len(), 500); assert_eq!(second.rows.len(), 500); assert_eq!(second.rows[0][0], "501"); assert_eq!(second.has_more, Some(true));
        let end = manager.execute_page(&runtime.id, "SELECT * FROM items ORDER BY id", 1500, 500).unwrap(); assert!(end.rows.is_empty()); assert_eq!(end.has_more, Some(false));
        assert!(manager.execute_page(&runtime.id, "INSERT INTO items VALUES(9999,'bad')", 500, 500).is_err());
        let count = manager.execute(&runtime.id, "SELECT COUNT(*) FROM items", 500).unwrap(); assert_eq!(count.rows[0][0], "1500");
        let columns = manager.describe_table(&runtime.id, "items").unwrap(); assert!(columns.iter().all(|c| c.comment.is_none()));
        let mut value = serde_json::to_value(&columns[0]).unwrap(); value.as_object_mut().unwrap().remove("comment");
        let legacy: DatabaseColumnInfo = serde_json::from_value(value).unwrap(); assert!(legacy.comment.is_none());
        let column = DatabaseColumnInfo { comment: Some("用户编号".into()), ..legacy };
        assert_eq!(serde_json::to_value(column).unwrap()["comment"], "用户编号");
        manager.disconnect(&runtime.id).unwrap(); std::fs::remove_file(path).unwrap();
    }

    #[tokio::test]
    async fn mysql_successful_test_and_query_close_the_connection() {
        for driver in [DatabaseDriver::Mysql, DatabaseDriver::Mariadb] {
            for test_only in [true, false] {
                let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
                let config = network_config(driver.clone(), listener.local_addr().unwrap().port());
                let server = tokio::spawn(mysql_server(listener, false));
                let operation = async {
                    if test_only { validate_network_config(config).await.unwrap(); }
                    else { let result = execute_network(config, "SELECT 1", 1).await.unwrap(); assert_eq!(result.rows, vec![vec![serde_json::json!("1")]]); }
                };
                tokio::time::timeout(Duration::from_secs(3), operation).await.expect("successful operation must not hang during cleanup");
                assert!(tokio::time::timeout(Duration::from_secs(1), server).await.unwrap().unwrap());
            }
        }
    }

    #[tokio::test]
    async fn stalled_handshakes_timeout_without_registering_a_runtime() {
        for driver in [DatabaseDriver::Mysql, DatabaseDriver::Postgres] {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let config = network_config(driver, listener.local_addr().unwrap().port());
            let server = tokio::spawn(async move { let (_stream, _) = listener.accept().await.unwrap(); std::future::pending::<()>().await; });
            let manager = Arc::new(DatabaseRuntimeManager::new()); let worker = manager.clone();
            let result = tokio::time::timeout(Duration::from_secs(10), tokio::task::spawn_blocking(move || worker.connect(config))).await.unwrap().unwrap();
            assert!(result.unwrap_err().contains("超时")); assert!(manager.list().unwrap().is_empty()); server.abort();
        }
    }

    #[tokio::test]
    async fn stalled_query_times_out_and_does_not_lock_runtime_directory() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let config = network_config(DatabaseDriver::Mysql, listener.local_addr().unwrap().port());
        let manager = Arc::new(DatabaseRuntimeManager::new());
        manager.runtimes.lock().unwrap().insert("fixture".into(), RuntimeRecord {
            summary: DatabaseRuntimeSummary { id: "fixture".into(), driver: DatabaseDriver::Mysql, status: DatabaseRuntimeStatus::Ready, error: None },
            handle: Some(DatabaseHandle::Network(config)),
        });
        let (accepted_tx, accepted_rx) = tokio::sync::oneshot::channel();
        // Accept indicates execute has passed the Runtime lookup and is now waiting on network.
        let proxy_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let proxy_port = proxy_listener.local_addr().unwrap().port();
        let server = tokio::spawn(mysql_server(proxy_listener, true));
        let proxy = tokio::spawn(async move {
            let (mut incoming, _) = listener.accept().await.unwrap(); let _ = accepted_tx.send(());
            let mut outgoing = tokio::net::TcpStream::connect(("127.0.0.1", proxy_port)).await.unwrap();
            let _ = tokio::io::copy_bidirectional(&mut incoming, &mut outgoing).await;
        });
        let worker = manager.clone();
        let query = tokio::task::spawn_blocking(move || worker.execute("fixture", "SELECT 1", 1));
        accepted_rx.await.unwrap();
        let worker = manager.clone();
        tokio::time::timeout(Duration::from_secs(1), tokio::task::spawn_blocking(move || {
            assert_eq!(worker.list().unwrap().len(), 1); worker.disconnect("fixture").unwrap();
        })).await.expect("directory and disconnect must remain responsive").unwrap();
        let error = tokio::time::timeout(Duration::from_secs(33), query).await.unwrap().unwrap().unwrap_err();
        assert!(error.contains("查询超时"));
        assert!(tokio::time::timeout(Duration::from_secs(2), server).await.expect("timed-out query must close its socket").unwrap());
        proxy.abort();
    }

    #[test]
    fn sqlite_test_does_not_create_files_and_rejects_invalid_databases() {
        let path = temp_db();
        let config = DatabaseConnectionConfig { sqlite_path: Some(path.clone()), ..network_config(DatabaseDriver::Sqlite, 0) };
        assert!(test_connection(config.clone()).is_err()); assert!(!path.exists());
        std::fs::write(&path, b"not a database").unwrap();
        assert!(test_connection(config.clone()).is_err()); std::fs::remove_file(&path).unwrap();
        Connection::open(&path).unwrap().execute_batch("CREATE TABLE fixture (id INTEGER)").unwrap();
        let before = std::fs::read(&path).unwrap(); test_connection(config).unwrap();
        assert_eq!(before, std::fs::read(&path).unwrap()); std::fs::remove_file(path).unwrap();
    }

    async fn postgres_rows_server(listener: tokio::net::TcpListener, values: Vec<&'static str>, count: usize, ending: &'static str) -> String {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let (mut stream, _) = listener.accept().await.unwrap();
        let length = stream.read_u32().await.unwrap();
        let mut startup = vec![0; length as usize - 4]; stream.read_exact(&mut startup).await.unwrap();
        stream.write_all(b"R\0\0\0\x08\0\0\0\0Z\0\0\0\x05I").await.unwrap();
        assert_eq!(stream.read_u8().await.unwrap(), b'Q');
        let length = stream.read_u32().await.unwrap();
        let mut query = vec![0; length as usize - 4]; stream.read_exact(&mut query).await.unwrap();
        async fn message(stream: &mut tokio::net::TcpStream, kind: u8, payload: &[u8]) {
            stream.write_u8(kind).await.unwrap(); stream.write_u32(payload.len() as u32 + 4).await.unwrap(); stream.write_all(payload).await.unwrap();
        }
        let mut description = (values.len() as u16).to_be_bytes().to_vec();
        for i in 0..values.len() {
            description.extend_from_slice(format!("col{i}\0").as_bytes()); description.extend_from_slice(&0u32.to_be_bytes()); description.extend_from_slice(&0u16.to_be_bytes()); description.extend_from_slice(&25u32.to_be_bytes()); description.extend_from_slice(&(-1i16).to_be_bytes()); description.extend_from_slice(&(-1i32).to_be_bytes()); description.extend_from_slice(&0u16.to_be_bytes());
        }
        message(&mut stream, b'T', &description).await;
        for _ in 0..count {
            let mut row = (values.len() as u16).to_be_bytes().to_vec();
            for value in &values { row.extend_from_slice(&(value.len() as u32).to_be_bytes()); row.extend_from_slice(value.as_bytes()); }
            message(&mut stream, b'D', &row).await;
        }
        if ending == "complete" { message(&mut stream, b'C', format!("SELECT {count}\0").as_bytes()).await; message(&mut stream, b'Z', b"I").await; }
        if ending == "error" { message(&mut stream, b'E', b"SERROR\0C22012\0Mlate database error\0\0").await; message(&mut stream, b'Z', b"I").await; }
        let mut byte = [0]; let _ = stream.read(&mut byte).await;
        String::from_utf8(query).unwrap()
    }

    #[tokio::test]
    async fn postgres_row_budget_is_applied_on_server_and_completed() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let config = network_config(DatabaseDriver::Postgres, listener.local_addr().unwrap().port());
        let server = tokio::spawn(postgres_rows_server(listener, vec!["1"], 500, "complete"));
        let result = tokio::time::timeout(Duration::from_secs(3), execute_network(config, "SELECT 1", 500)).await.expect("must stop without collecting full query").unwrap();
        assert_eq!(result.rows.len(), 500); assert!(result.truncated); assert_eq!(result.columns, vec!["col0"]);
        let sql = tokio::time::timeout(Duration::from_secs(2), server).await.expect("capped query must close socket").unwrap(); assert!(sql.contains("LIMIT 500"));
    }

    #[tokio::test]
    async fn postgres_column_comment_survives_native_metadata_path() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let config = network_config(DatabaseDriver::Postgres, listener.local_addr().unwrap().port());
        let server = tokio::spawn(postgres_rows_server(listener, vec!["id", "integer", "NO", "0", "用户编号"], 1, "complete"));
        let manager = DatabaseRuntimeManager::new();
        manager.runtimes.lock().unwrap().insert("fixture".into(), RuntimeRecord { summary: DatabaseRuntimeSummary { id: "fixture".into(), driver: DatabaseDriver::Postgres, status: DatabaseRuntimeStatus::Ready, error: None }, handle: Some(DatabaseHandle::Network(config)) });
        let columns = tokio::task::spawn_blocking(move || manager.describe_table("fixture", "users")).await.unwrap().unwrap();
        assert_eq!(columns[0].comment.as_deref(), Some("用户编号")); assert!(columns[0].not_null);
        let sql = server.await.unwrap(); assert!(sql.contains("col_description")); assert!(sql.contains("attr.attnum"));
    }

    #[tokio::test]
    async fn postgres_stalled_query_aborts_connection_driver_and_closes_socket() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let config = network_config(DatabaseDriver::Postgres, listener.local_addr().unwrap().port());
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let length = stream.read_u32().await.unwrap();
            assert!((8..4096).contains(&length));
            let mut startup = vec![0; length as usize - 4]; stream.read_exact(&mut startup).await.unwrap();
            // AuthenticationOk followed by ReadyForQuery.
            stream.write_all(b"R\0\0\0\x08\0\0\0\0Z\0\0\0\x05I").await.unwrap();
            assert_eq!(stream.read_u8().await.unwrap(), b'Q');
            let length = stream.read_u32().await.unwrap();
            let mut query = vec![0; length as usize - 4]; stream.read_exact(&mut query).await.unwrap();
            assert_eq!(query, b"SELECT 1 LIMIT 1\0");
            let mut remaining = Vec::new(); stream.read_to_end(&mut remaining).await.unwrap();
        });
        let error = tokio::time::timeout(Duration::from_secs(33), execute_network(config, "SELECT 1", 1))
            .await.unwrap().unwrap_err();
        assert!(error.contains("查询超时"));
        tokio::time::timeout(Duration::from_secs(2), server).await.expect("PostgreSQL connection must close after timeout").unwrap();
    }

    #[test]
    fn postgres_config_accepts_frontend_and_legacy_driver_names_without_serializing_passwords() {
        for name in ["postgres", "postgresql"] {
            let config: DatabaseConnectionConfig = serde_json::from_value(serde_json::json!({ "driver": name, "password": "synthetic-fixture" })).unwrap();
            assert_eq!(config.driver, DatabaseDriver::Postgres);
            assert!(serde_json::to_value(config).unwrap().get("password").is_none());
        }
    }

    #[test]
    fn sqlite_query_is_bounded_and_import_rolls_back() {
        let path = temp_db(); let manager = DatabaseRuntimeManager::new();
        let runtime = manager.connect(DatabaseConnectionConfig { profile_id: None, driver: DatabaseDriver::Sqlite, host: None, port: None, database: None, username: None, password: None, read_only: false, ssl_enabled: false, sqlite_path: Some(path.clone()) }).unwrap();
        manager.execute(&runtime.id, "create table items(id integer primary key, name text)", 10).unwrap();
        manager.import_sql(&runtime.id, "insert into items(name) values ('a'); insert into missing(name) values ('b');").unwrap_err();
        let result = manager.execute(&runtime.id, "select * from items", 1).unwrap(); assert!(result.rows.is_empty());
        manager.import_sql(&runtime.id, "insert into items(name) values ('a'); insert into items(name) values ('b');").unwrap();
        let result = manager.execute(&runtime.id, "select * from items", 1).unwrap(); assert_eq!(result.rows.len(), 1); assert!(result.truncated);
        manager.disconnect(&runtime.id).unwrap(); let _ = std::fs::remove_file(path);
    }

    #[test]
    fn sqlite_row_import_is_transactional() {
        let path = temp_db(); let manager = DatabaseRuntimeManager::new();
        let runtime = manager.connect(DatabaseConnectionConfig { profile_id: None, driver: DatabaseDriver::Sqlite, host: None, port: None, database: None, username: None, password: None, read_only: false, ssl_enabled: false, sqlite_path: Some(path.clone()) }).unwrap();
        manager.execute(&runtime.id, "create table items(id integer, name text)", 10).unwrap();
        let count = manager.import_rows(&runtime.id, "items", vec!["id".into(), "name".into()], vec![vec![serde_json::json!(1), serde_json::json!("a")]]).unwrap();
        assert_eq!(count, 1);
        assert!(manager.import_rows(&runtime.id, "items", vec!["id".into(), "bad-name".into()], vec![vec![serde_json::json!(2), serde_json::json!("b")]]).is_err());
        let result = manager.execute(&runtime.id, "select count(*) from items", 10).unwrap(); assert_eq!(result.rows[0][0], serde_json::json!("1"));
        let _ = std::fs::remove_file(path);
    }

    fn sqlite_runtime(manager: &DatabaseRuntimeManager) -> (DatabaseRuntimeSummary, PathBuf) {
        let path = temp_db();
        let runtime = manager.connect(DatabaseConnectionConfig { sqlite_path: Some(path.clone()), read_only: false, ..network_config(DatabaseDriver::Sqlite, 0) }).unwrap();
        (runtime, path)
    }

    #[test]
    fn query_sql_file_reports_rows_and_actual_bytes() {
        let path = temp_db().with_extension("query.sql");
        let sql = "INSERT INTO \"items\" (\"id\") VALUES\n  (1);";
        let summary = write_query_sql_file(&path, sql, 1).unwrap();
        assert_eq!(summary.tables, 1);
        assert_eq!(summary.rows, 1);
        let text = std::fs::read_to_string(&path).unwrap();
        assert_eq!(text, format!("{sql}\n"));
        assert_eq!(summary.bytes, std::fs::metadata(&path).unwrap().len());
        assert!(write_query_sql_file(&path, "  \n", 0).is_err());
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn exports_report_table_progress_and_query_progress() {
        let manager = DatabaseRuntimeManager::new();
        let (runtime, source_path) = sqlite_runtime(&manager);
        manager.execute(&runtime.id, "create table first(id integer)", 10).unwrap();
        manager.execute(&runtime.id, "create table second(id integer)", 10).unwrap();
        let dump_path = temp_db().with_extension("progress.sql");
        let dump_events = Arc::new(Mutex::new(Vec::<(usize, usize, Option<String>)>::new()));
        let dump_capture = dump_events.clone();
        let dump_sink: DatabaseExportProgressSink = Arc::new(move |current, total, table| dump_capture.lock().unwrap().push((current, total, table.map(str::to_owned))));
        manager.export_sql_with_progress(&runtime.id, &dump_path, None, true, Some(dump_sink)).unwrap();
        assert_eq!(dump_events.lock().unwrap().as_slice(), &[(0, 2, None), (1, 2, Some("first".into())), (2, 2, Some("second".into()))]);
        let query_path = temp_db().with_extension("query-progress.sql");
        let query_events = Arc::new(Mutex::new(Vec::<(usize, usize, Option<String>)>::new()));
        let query_capture = query_events.clone();
        let query_sink: DatabaseExportProgressSink = Arc::new(move |current, total, table| query_capture.lock().unwrap().push((current, total, table.map(str::to_owned))));
        write_query_sql_file_with_progress(&query_path, "INSERT INTO first(id) VALUES (1);", 1, Some(query_sink)).unwrap();
        assert_eq!(query_events.lock().unwrap().as_slice(), &[(0, 1, None), (1, 1, None)]);
        let _ = std::fs::remove_file(source_path); let _ = std::fs::remove_file(dump_path); let _ = std::fs::remove_file(query_path);
    }

    #[test]
    fn sqlite_dump_round_trips_schema_and_values_into_a_new_database() {
        let manager = DatabaseRuntimeManager::new();
        let (source, source_path) = sqlite_runtime(&manager);
        manager.execute(&source.id, "create table items(id integer primary key, name text, ratio real, payload blob, note text)", 10).unwrap();
        manager.execute(&source.id, "create table other(id integer)", 10).unwrap();
        manager.execute(&source.id, "create index items_name on items(name)", 10).unwrap();
        manager.execute(&source.id, "insert into items values (1, 'quote''s\nline', 0.1, X'00FF41', NULL)", 10).unwrap();
        // A TEXT value may hold bytes that are not valid UTF-8; the dump must not lose them.
        manager.execute(&source.id, "insert into items values (2, CAST(X'FF41' AS TEXT), -2.5, NULL, 'ok')", 10).unwrap();
        manager.execute(&source.id, "insert into other values (7)", 10).unwrap();
        let dump = temp_db().with_extension("sql");
        let summary = manager.export_sql(&source.id, &dump, None).unwrap();
        assert_eq!((summary.tables, summary.rows), (2, 3));
        assert_eq!(summary.bytes, std::fs::metadata(&dump).unwrap().len());
        let text = std::fs::read_to_string(&dump).unwrap();
        // Every statement names its own table, and raw bytes stay byte exact.
        assert!(text.contains("INSERT INTO \"items\" (\"id\", \"name\", \"ratio\", \"payload\", \"note\") VALUES"));
        assert!(text.contains("INSERT INTO \"other\" (\"id\") VALUES"));
        assert!(text.contains("CAST(X'FF41' AS TEXT)"));
        assert!(text.contains("X'00FF41'"));
        assert!(text.contains("CREATE INDEX items_name"));
        // Restoring into a different file proves nothing is reused from the source connection.
        let (target, target_path) = sqlite_runtime(&manager);
        manager.import_sql_file(&target.id, &dump).unwrap();
        let projection = "select id, name, ratio, hex(payload), note, typeof(name) from items order by id";
        let source_rows = manager.execute(&source.id, projection, 10).unwrap();
        let target_rows = manager.execute(&target.id, projection, 10).unwrap();
        assert_eq!(source_rows.rows, target_rows.rows);
        assert_eq!(manager.list_tables(&target.id).unwrap().len(), 2);
        for file in [source_path, target_path, dump] { let _ = std::fs::remove_file(file); }
    }

    #[test]
    fn sqlite_table_dump_keeps_only_the_requested_table_and_its_indexes() {
        let manager = DatabaseRuntimeManager::new();
        let (runtime, path) = sqlite_runtime(&manager);
        manager.execute(&runtime.id, "create table items(id integer primary key, name text)", 10).unwrap();
        manager.execute(&runtime.id, "create table other(id integer)", 10).unwrap();
        manager.execute(&runtime.id, "create index items_name on items(name)", 10).unwrap();
        manager.execute(&runtime.id, "insert into items values (1, 'a')", 10).unwrap();
        manager.execute(&runtime.id, "insert into other values (2)", 10).unwrap();
        let dump = temp_db().with_extension("sql");
        let summary = manager.export_sql(&runtime.id, &dump, Some("items")).unwrap();
        assert_eq!((summary.tables, summary.rows), (1, 1));
        let text = std::fs::read_to_string(&dump).unwrap();
        assert!(text.starts_with("-- Luna Mux dump: items"));
        assert!(text.contains("CREATE TABLE items"));
        assert!(text.contains("CREATE INDEX items_name"));
        assert!(text.contains("INSERT INTO \"items\""));
        assert!(!text.contains("other"));
        // A name that is not in sqlite_master is a user error, not an empty dump.
        assert!(manager.export_sql(&runtime.id, &dump, Some("missing")).unwrap_err().contains("找不到"));
        for file in [path, dump] { let _ = std::fs::remove_file(file); }
    }

    #[test]
    fn sqlite_schema_only_dump_omits_rows_for_database_and_table_exports() {
        let manager = DatabaseRuntimeManager::new();
        let (runtime, path) = sqlite_runtime(&manager);
        manager.execute(&runtime.id, "create table items(id integer primary key, name text)", 10).unwrap();
        manager.execute(&runtime.id, "create table other(id integer)", 10).unwrap();
        manager.execute(&runtime.id, "create index items_name on items(name)", 10).unwrap();
        manager.execute(&runtime.id, "insert into items values (1, 'a')", 10).unwrap();
        manager.execute(&runtime.id, "insert into other values (2)", 10).unwrap();

        let database_dump = temp_db().with_extension("schema.sql");
        let summary = manager.export_sql_with_options(&runtime.id, &database_dump, None, true).unwrap();
        assert_eq!((summary.tables, summary.rows), (2, 0));
        let text = std::fs::read_to_string(&database_dump).unwrap();
        assert!(text.contains("CREATE TABLE items"));
        assert!(text.contains("CREATE TABLE other"));
        assert!(text.contains("CREATE INDEX items_name"));
        assert!(!text.contains("INSERT INTO"));

        let table_dump = temp_db().with_extension("table-schema.sql");
        let summary = manager.export_sql_with_options(&runtime.id, &table_dump, Some("items"), true).unwrap();
        assert_eq!((summary.tables, summary.rows), (1, 0));
        let text = std::fs::read_to_string(&table_dump).unwrap();
        assert!(text.contains("CREATE TABLE items"));
        assert!(text.contains("CREATE INDEX items_name"));
        assert!(!text.contains("INSERT INTO"));
        assert!(!text.contains("other"));
        for file in [path, database_dump, table_dump] { let _ = std::fs::remove_file(file); }
    }

    #[test]
    fn sqlite_dump_of_an_empty_database_restores_without_error() {
        let manager = DatabaseRuntimeManager::new();
        let (runtime, path) = sqlite_runtime(&manager);
        let dump = temp_db().with_extension("sql");
        let summary = manager.export_sql(&runtime.id, &dump, None).unwrap();
        assert_eq!((summary.tables, summary.rows), (0, 0));
        assert!(std::fs::read_to_string(&dump).unwrap().starts_with("-- Luna Mux database dump"));
        let (target, target_path) = sqlite_runtime(&manager);
        manager.import_sql_file(&target.id, &dump).unwrap();
        for file in [path, target_path, dump] { let _ = std::fs::remove_file(file); }
    }

    #[test]
    fn sqlite_import_from_file_is_all_or_nothing() {
        let manager = DatabaseRuntimeManager::new();
        let (runtime, path) = sqlite_runtime(&manager);
        manager.execute(&runtime.id, "create table items(id integer)", 10).unwrap();
        let script = temp_db().with_extension("sql");
        std::fs::write(&script, "insert into items values (1);\ninsert into items values (2);\ninsert into missing values (3);\n").unwrap();
        assert!(manager.import_sql_file(&runtime.id, &script).is_err());
        let count = manager.execute(&runtime.id, "select count(*) from items", 10).unwrap();
        assert_eq!(count.rows[0][0], serde_json::json!("0"));
        // A script that opens its own transaction cannot be rolled back as one unit, so the error
        // explains the fix instead of leaking the driver wording.
        std::fs::write(&script, "BEGIN;\ninsert into items values (9);\nCOMMIT;\n").unwrap();
        assert!(manager.import_sql_file(&runtime.id, &script).unwrap_err().contains("BEGIN/COMMIT"));
        // A path that does not exist reports the file, not a runtime failure.
        assert!(manager.import_sql_file(&runtime.id, &temp_db().with_extension("missing.sql")).unwrap_err().contains("无法读取 SQL 文件"));
        let _ = std::fs::remove_file(path);
        let _ = std::fs::remove_file(script);
    }

    // --- Network dumps ------------------------------------------------------------------------

    fn dump_path(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!("luna-mux-dump-{}-{name}.sql", SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos()))
    }

    #[test]
    fn mysql_dump_values_follow_the_column_kind() {
        use mysql_async::consts::ColumnType;
        let column = |kind: ColumnType, charset: u16| mysql_async::Column::new(kind).with_character_set(charset);
        assert_eq!(mysql_value_kind(&column(ColumnType::MYSQL_TYPE_LONG, MYSQL_BINARY_CHARSET)), MysqlValueKind::Number);
        assert_eq!(mysql_value_kind(&column(ColumnType::MYSQL_TYPE_DATETIME, MYSQL_BINARY_CHARSET)), MysqlValueKind::Text);
        assert_eq!(mysql_value_kind(&column(ColumnType::MYSQL_TYPE_BLOB, MYSQL_BINARY_CHARSET)), MysqlValueKind::Binary);
        assert_eq!(mysql_value_kind(&column(ColumnType::MYSQL_TYPE_VAR_STRING, 45)), MysqlValueKind::Text);

        let mut out = Vec::new();
        for (value, kind) in [
            (mysql_async::Value::NULL, MysqlValueKind::Text),
            (mysql_async::Value::Bytes(b"-17.5".to_vec()), MysqlValueKind::Number),
            (mysql_async::Value::Bytes(b"it's".to_vec()), MysqlValueKind::Text),
            (mysql_async::Value::Bytes(vec![0x00, 0xff]), MysqlValueKind::Binary),
            // A backslash cannot be quoted portably (`NO_BACKSLASH_ESCAPES`), and control bytes
            // have no place in a script, so both take the hex form.
            (mysql_async::Value::Bytes(b"a\\b".to_vec()), MysqlValueKind::Text),
            (mysql_async::Value::Bytes(b"line\nfeed".to_vec()), MysqlValueKind::Text),
        ] { mysql_dump_value(&mut out, &value, kind).unwrap(); }
        assert_eq!(String::from_utf8(out).unwrap(), "NULL-17.5'it''s'X'00FF'X'615C62'X'6C696E650A66656564'");
    }

    #[test]
    fn mysql_temporal_values_keep_their_text_form() {
        assert_eq!(mysql_datetime_text(2024, 1, 2, 3, 4, 5, 0), "2024-01-02 03:04:05");
        assert_eq!(mysql_datetime_text(2024, 1, 2, 3, 4, 5, 120_000), "2024-01-02 03:04:05.120000");
        assert_eq!(mysql_time_text(false, 1, 2, 3, 4, 0), "26:03:04");
        assert_eq!(mysql_time_text(true, 0, 0, 0, 1, 500_000), "-00:00:01.500000");
    }

    #[test]
    fn postgres_dump_values_quote_text_and_hex_bytea() {
        let mut out = Vec::new();
        postgres_dump_value(&mut out, None, false).unwrap();
        postgres_dump_value(&mut out, Some("it's"), false).unwrap();
        postgres_dump_value(&mut out, Some("back\\slash"), false).unwrap();
        postgres_dump_value(&mut out, Some("\\x616263"), true).unwrap();
        assert_eq!(String::from_utf8(out).unwrap(), r"NULL'it''s'E'back\\slash'E'\\x616263'");
    }

    #[test]
    fn postgres_column_definitions_keep_serial_identity_and_defaults() {
        let column = |name: &str, type_sql: &str, default: Option<&str>| PostgresColumn {
            name: name.into(), type_sql: type_sql.into(), not_null: false, default: default.map(str::to_string), serial: postgres_serial_type(type_sql, default), identity: false,
        };
        let serial = column("id", "integer", Some("nextval('posts_id_seq'::regclass)"));
        assert_eq!(serial.definition(), "  \"id\" serial");
        assert!(serial.needs_sequence_reset());
        assert_eq!(column("title", "text", Some("'draft'::text")).definition(), "  \"title\" text DEFAULT 'draft'::text");
        assert!(!column("title", "text", None).needs_sequence_reset());
        // A sequence default on a type without a serial shorthand stays a plain default.
        assert_eq!(postgres_serial_type("numeric", Some("nextval('x'::regclass)")), None);
        let identity = PostgresColumn { identity: true, not_null: true, ..column("id", "bigint", None) };
        assert_eq!(identity.definition(), "  \"id\" bigint GENERATED BY DEFAULT AS IDENTITY NOT NULL");
        assert!(identity.needs_sequence_reset());
    }

    /// ColumnDefinition41 with a real name, type and charset so the driver's metadata path is
    /// exercised, not a hand-built `Value`.
    fn mysql_column_packet(name: &str, column_type: u8, charset: u16) -> Vec<u8> {
        let mut column = vec![3, b'd', b'e', b'f', 0, 0, 0];
        column.push(name.len() as u8); column.extend_from_slice(name.as_bytes()); column.push(0);
        column.push(12); column.extend_from_slice(&charset.to_le_bytes()); column.extend_from_slice(&1024u32.to_le_bytes());
        column.push(column_type); column.extend_from_slice(&[0, 0, 0, 0, 0]);
        column
    }

    async fn mysql_result(stream: &mut tokio::net::TcpStream, columns: &[(&str, u8, u16)], rows: &[Vec<Option<Vec<u8>>>]) {
        packet(stream, 1, &[columns.len() as u8]).await;
        let mut sequence = 2;
        for (name, kind, charset) in columns { packet(stream, sequence, &mysql_column_packet(name, *kind, *charset)).await; sequence += 1; }
        packet(stream, sequence, &[254, 0, 0, 2, 0]).await;
        sequence += 1;
        for row in rows {
            let mut body = Vec::new();
            for value in row { match value { Some(bytes) => { body.push(bytes.len() as u8); body.extend_from_slice(bytes); } None => body.push(0xfb) } }
            packet(stream, sequence, &body).await;
            sequence = sequence.wrapping_add(1);
        }
        packet(stream, sequence, &[254, 0, 0, 2, 0]).await;
    }

    /// One schema with one table, so the MySQL dump path runs against the real driver.
    async fn mysql_dump_server(listener: tokio::net::TcpListener) -> Vec<String> {
        const TEXT: u16 = 45;
        let (mut stream, _) = listener.accept().await.unwrap();
        let caps: u32 = 1 | 4 | 512 | 8192 | 32768 | 524288;
        let mut greeting = vec![10]; greeting.extend_from_slice(b"8.0.36-fixture\0");
        greeting.extend_from_slice(&1u32.to_le_bytes()); greeting.extend_from_slice(b"12345678\0");
        greeting.extend_from_slice(&(caps as u16).to_le_bytes()); greeting.push(45);
        greeting.extend_from_slice(&2u16.to_le_bytes()); greeting.extend_from_slice(&((caps >> 16) as u16).to_le_bytes());
        greeting.push(21); greeting.extend_from_slice(&[0; 10]); greeting.extend_from_slice(b"abcdefghijkl\0mysql_native_password\0");
        packet(&mut stream, 0, &greeting).await;
        read_packet(&mut stream).await.unwrap();
        packet(&mut stream, 2, &[0, 0, 0, 2, 0, 0, 0]).await;
        let mut queries = Vec::new();
        while let Ok(data) = read_packet(&mut stream).await {
            if data.first() == Some(&1) { break }
            let sql = String::from_utf8_lossy(&data[1..]).to_string();
            if sql.contains("information_schema.tables") {
                let requested = sql.split("AND table_name = '").nth(1).and_then(|rest| rest.split('\'').next());
                let rows = if requested.is_some_and(|name| name != "posts") { vec![] } else { vec![vec![Some(b"posts".to_vec()), Some(b"BASE TABLE".to_vec())]] };
                mysql_result(&mut stream, &[("table_name", 253, TEXT), ("table_type", 253, TEXT)], &rows).await;
            } else if sql.starts_with("SHOW CREATE TABLE") {
                mysql_result(&mut stream, &[("Table", 253, TEXT), ("Create Table", 253, TEXT)], &[vec![
                    Some(b"posts".to_vec()),
                    Some(b"CREATE TABLE `posts` (\n  `id` int NOT NULL,\n  `title` varchar(50) DEFAULT NULL\n) ENGINE=InnoDB".to_vec()),
                ]]).await;
            } else if sql.starts_with("SELECT * FROM") {
                mysql_result(&mut stream, &[("id", 3, 63), ("title", 253, TEXT), ("payload", 252, 63), ("created_at", 12, 63)], &[
                    vec![Some(b"1".to_vec()), Some(b"hello".to_vec()), Some(vec![0x00, 0xff]), Some(b"2024-01-02 03:04:05".to_vec())],
                    vec![Some(b"2".to_vec()), Some(b"quote's".to_vec()), None, Some(b"2024-05-06 07:08:09".to_vec())],
                ]).await;
            } else {
                packet(&mut stream, 1, &[0, 0, 0, 2, 0, 0, 0]).await;
            }
            queries.push(sql);
        }
        queries
    }

    #[tokio::test]
    async fn mysql_dump_writes_schema_and_batched_rows() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let config = network_config(DatabaseDriver::Mysql, listener.local_addr().unwrap().port());
        let server = tokio::spawn(mysql_dump_server(listener));
        let path = dump_path("mysql");
        let export = mysql_export_sql(config, &path, None, false).await.unwrap();
        assert_eq!((export.tables, export.rows), (1, 2));
        let script = std::fs::read_to_string(&path).unwrap();
        assert!(script.starts_with("-- Luna Mux database dump"));
        assert!(script.contains("DROP TABLE IF EXISTS `posts`;"));
        assert!(script.contains("CREATE TABLE `posts` ("));
        // Numbers stay bare, text is quoted, blobs and dates keep their own literal forms.
        assert!(script.contains("INSERT INTO `posts` (`id`, `title`, `payload`, `created_at`) VALUES\n  (1, 'hello', X'00FF', '2024-01-02 03:04:05'),\n  (2, 'quote''s', NULL, '2024-05-06 07:08:09');"));
        assert_eq!(export.bytes, std::fs::metadata(&path).unwrap().len());
        let queries = server.await.unwrap();
        assert!(queries.iter().any(|sql| sql.starts_with("SELECT table_name, table_type FROM information_schema.tables")));
        assert!(queries.iter().any(|sql| sql == "SHOW CREATE TABLE `posts`"));
        assert!(queries.iter().any(|sql| sql == "SELECT * FROM `posts`"));
        let _ = std::fs::remove_file(path);
    }

    #[tokio::test]
    async fn mysql_schema_only_dump_skips_data_queries_and_inserts() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let config = network_config(DatabaseDriver::Mysql, listener.local_addr().unwrap().port());
        let server = tokio::spawn(mysql_dump_server(listener));
        let path = dump_path("mysql-schema");
        let export = mysql_export_sql(config, &path, None, true).await.unwrap();
        assert_eq!((export.tables, export.rows), (1, 0));
        let script = std::fs::read_to_string(&path).unwrap();
        assert!(script.contains("CREATE TABLE `posts` ("));
        assert!(!script.contains("INSERT INTO"));
        let queries = server.await.unwrap();
        assert!(!queries.iter().any(|sql| sql.starts_with("SELECT * FROM")));
        let _ = std::fs::remove_file(path);
    }

    #[tokio::test]
    async fn mysql_dump_of_one_table_filters_the_query_and_reports_missing_tables() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let config = network_config(DatabaseDriver::Mysql, listener.local_addr().unwrap().port());
        let server = tokio::spawn(mysql_dump_server(listener));
        let path = dump_path("mysql-table");
        let export = mysql_export_sql(config, &path, Some("posts"), false).await.unwrap();
        assert_eq!((export.tables, export.rows), (1, 2));
        assert!(std::fs::read_to_string(&path).unwrap().starts_with("-- Luna Mux dump: posts"));
        // The driver probes session variables right after the handshake.
        let queries = server.await.unwrap().into_iter().filter(|sql| !sql.contains("@@max_allowed_packet")).collect::<Vec<_>>();
        assert!(queries.first().unwrap().contains("AND table_name = 'posts'"));
        let _ = std::fs::remove_file(path);

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let config = network_config(DatabaseDriver::Mysql, listener.local_addr().unwrap().port());
        let server = tokio::spawn(mysql_dump_server(listener));
        let path = dump_path("mysql-missing");
        let error = mysql_export_sql(config, &path, Some("missing"), false).await.unwrap_err();
        assert!(error.contains("找不到该表或视图"));
        assert_eq!(server.await.unwrap().iter().filter(|sql| sql.contains("information_schema")).count(), 1);
        assert!(!path.exists());
    }

    #[tokio::test]
    async fn network_import_sends_the_script_as_a_single_request() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let config = network_config(DatabaseDriver::Mysql, listener.local_addr().unwrap().port());
        let server = tokio::spawn(mysql_dump_server(listener));
        let script = "DROP TABLE IF EXISTS `posts`;\nCREATE TABLE `posts` (`id` int);\nINSERT INTO `posts` VALUES (1);\n";
        import_network_sql(config, script).await.unwrap();
        // The whole script is one request, so the server splits it, not a client-side parser.
        let statements = server.await.unwrap().into_iter().filter(|sql| !sql.contains("@@max_allowed_packet")).collect::<Vec<_>>();
        assert_eq!(statements, vec![script.to_string()]);
    }

    /// One schema with one table, answering the catalog queries the PostgreSQL dump makes.
    async fn postgres_dump_server(listener: tokio::net::TcpListener) -> Vec<String> {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        async fn message(stream: &mut tokio::net::TcpStream, kind: u8, payload: &[u8]) {
            stream.write_u8(kind).await.unwrap(); stream.write_u32(payload.len() as u32 + 4).await.unwrap(); stream.write_all(payload).await.unwrap();
        }
        async fn result(stream: &mut tokio::net::TcpStream, columns: &[&str], rows: &[Vec<Option<&str>>]) {
            let mut description = (columns.len() as u16).to_be_bytes().to_vec();
            for column in columns {
                description.extend_from_slice(format!("{column}\0").as_bytes());
                description.extend_from_slice(&0u32.to_be_bytes()); description.extend_from_slice(&0u16.to_be_bytes());
                description.extend_from_slice(&25u32.to_be_bytes()); description.extend_from_slice(&(-1i16).to_be_bytes());
                description.extend_from_slice(&(-1i32).to_be_bytes()); description.extend_from_slice(&0u16.to_be_bytes());
            }
            message(stream, b'T', &description).await;
            for row in rows {
                let mut body = (columns.len() as u16).to_be_bytes().to_vec();
                for value in row { match value { Some(text) => { body.extend_from_slice(&(text.len() as u32).to_be_bytes()); body.extend_from_slice(text.as_bytes()); } None => body.extend_from_slice(&(-1i32).to_be_bytes()) } }
                message(stream, b'D', &body).await;
            }
            message(stream, b'C', format!("SELECT {}\0", rows.len()).as_bytes()).await;
            message(stream, b'Z', b"I").await;
        }
        let (mut stream, _) = listener.accept().await.unwrap();
        let length = stream.read_u32().await.unwrap();
        let mut startup = vec![0; length as usize - 4]; stream.read_exact(&mut startup).await.unwrap();
        stream.write_all(b"R\0\0\0\x08\0\0\0\0Z\0\0\0\x05I").await.unwrap();
        let mut queries = Vec::new();
        loop {
            let mut kind = [0u8];
            if stream.read_exact(&mut kind).await.is_err() { break }
            let length = stream.read_u32().await.unwrap();
            let mut payload = vec![0; length as usize - 4]; stream.read_exact(&mut payload).await.unwrap();
            if kind[0] != b'Q' { break }
            let sql = String::from_utf8_lossy(&payload).trim_end_matches('\0').to_string();
            if sql.contains("information_schema.tables") {
                result(&mut stream, &["table_name", "table_type"], &[vec![Some("posts"), Some("BASE TABLE")]]).await;
            } else if sql.contains("FROM pg_attribute") {
                result(&mut stream, &["attname", "format_type", "attnotnull", "attidentity", "pg_get_expr"], &[
                    vec![Some("id"), Some("integer"), Some("t"), Some(""), Some("nextval('posts_id_seq'::regclass)")],
                    vec![Some("title"), Some("character varying(50)"), Some("f"), Some(""), None],
                    vec![Some("payload"), Some("bytea"), Some("f"), Some(""), None],
                ]).await;
            } else if sql.contains("FROM pg_index") {
                result(&mut stream, &["pg_get_indexdef"], &[vec![Some("CREATE INDEX posts_title_idx ON public.posts USING btree (title)")]]).await;
            } else if sql.contains("FROM pg_constraint") {
                result(&mut stream, &["conname", "contype", "pg_get_constraintdef"], &[
                    vec![Some("posts_pkey"), Some("p"), Some("PRIMARY KEY (id)")],
                    vec![Some("posts_user_fk"), Some("f"), Some("FOREIGN KEY (user_id) REFERENCES users(id)")],
                ]).await;
            } else if sql.starts_with("SELECT * FROM") {
                result(&mut stream, &["id", "title", "payload"], &[vec![Some("1"), Some("hello"), Some("\\x616263")]]).await;
            } else {
                message(&mut stream, b'C', b"SET\0").await; message(&mut stream, b'Z', b"I").await;
            }
            queries.push(sql);
        }
        queries
    }

    #[tokio::test]
    async fn postgres_dump_writes_tables_data_constraints_and_sequences() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let config = network_config(DatabaseDriver::Postgres, listener.local_addr().unwrap().port());
        let server = tokio::spawn(postgres_dump_server(listener));
        let path = dump_path("postgres");
        let export = postgres_export_sql(config, &path, None, false).await.unwrap();
        assert_eq!((export.tables, export.rows), (1, 1));
        let script = std::fs::read_to_string(&path).unwrap();
        assert!(script.contains("CREATE TABLE \"posts\" (\n  \"id\" serial NOT NULL,\n  \"title\" character varying(50),\n  \"payload\" bytea\n);"));
        // bytea has no text form, so it keeps the escaped hex literal PostgreSQL reads back.
        // Every value is a quoted literal; the column type coerces it back on import.
        assert!(script.contains("INSERT INTO \"posts\" (\"id\", \"title\", \"payload\") VALUES\n  ('1', 'hello', E'\\\\x616263');"));
        assert!(script.contains("CREATE INDEX posts_title_idx"));
        assert!(script.contains("ALTER TABLE \"posts\" ADD CONSTRAINT \"posts_pkey\" PRIMARY KEY (id);"));
        assert!(script.contains("SELECT setval(pg_get_serial_sequence('\"posts\"', 'id'), COALESCE(MAX(\"id\"), 1), COUNT(\"id\") > 0) FROM \"posts\";"));
        assert!(script.contains("ADD CONSTRAINT \"posts_user_fk\" FOREIGN KEY"));
        assert_eq!(export.bytes, std::fs::metadata(&path).unwrap().len());
        let _ = std::fs::remove_file(path);
    }

    #[tokio::test]
    async fn postgres_schema_only_dump_skips_data_queries_inserts_and_sequence_resets() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let config = network_config(DatabaseDriver::Postgres, listener.local_addr().unwrap().port());
        let server = tokio::spawn(postgres_dump_server(listener));
        let path = dump_path("postgres-schema");
        let export = postgres_export_sql(config, &path, None, true).await.unwrap();
        assert_eq!((export.tables, export.rows), (1, 0));
        let script = std::fs::read_to_string(&path).unwrap();
        assert!(script.contains("CREATE TABLE \"posts\" ("));
        assert!(!script.contains("INSERT INTO"));
        assert!(!script.contains("SELECT setval"));
        let queries = server.await.unwrap();
        assert!(!queries.iter().any(|sql| sql.starts_with("SELECT * FROM")));
        let _ = std::fs::remove_file(path);
    }

    #[tokio::test]
    async fn postgres_import_sends_the_script_as_one_batch() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let config = network_config(DatabaseDriver::Postgres, listener.local_addr().unwrap().port());
        let server = tokio::spawn(postgres_dump_server(listener));
        import_network_sql(config, "CREATE TABLE posts (id serial);\nINSERT INTO posts (id) VALUES (1);\n").await.unwrap();
        let queries = server.await.unwrap();
        assert_eq!(queries.len(), 1);
        assert!(queries[0].starts_with("SET client_encoding = 'UTF8';\n"));
        assert!(queries[0].contains("INSERT INTO posts (id) VALUES (1);"));
    }
}
