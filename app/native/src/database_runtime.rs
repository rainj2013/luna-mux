//! Database driver abstraction and lifecycle management for Database Panes.
//!
//! SQLite is embedded through rusqlite. Network drivers are represented by the
//! same abstraction and can be enabled without changing the Pane contract.

use std::{collections::HashMap, path::PathBuf, sync::Mutex, time::Duration};

#[cfg(test)]
use std::sync::Arc;

use rusqlite::{Connection, OpenFlags};
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
    pub fn import_sql(&self, id: &str, script: &str) -> Result<(), String> {
        if script.trim().is_empty() { return Err("SQL 脚本不能为空".into()); }
        let mut map = self.runtimes.lock().map_err(|_| "database runtime lock poisoned".to_string())?;
        let record = map.get_mut(id).ok_or_else(|| "database runtime not found".to_string())?;
        let Some(DatabaseHandle::Sqlite(connection)) = record.handle.as_mut() else { return Err("database is disconnected".into()); };
        let transaction = connection.transaction().map_err(|e| e.to_string())?;
        transaction.execute_batch(script).map_err(|e| e.to_string())?;
        transaction.commit().map_err(|e| e.to_string())
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
    let host = config.host.ok_or_else(|| "数据库主机不能为空".to_string())?;
    let port = config.port.unwrap_or(if matches!(config.driver, DatabaseDriver::Postgres) { 5432 } else { 3306 });
    let user = config.username.unwrap_or_default();
    let database = config.database.unwrap_or_default();
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
            let mut cfg = tokio_postgres::Config::new(); cfg.host(&host).port(port).user(&user); if !database.is_empty() { cfg.dbname(&database); } if let Some(password) = config.password.as_deref() { cfg.password(password); }
            if config.ssl_enabled { cfg.ssl_mode(tokio_postgres::config::SslMode::Require); }
            if config.ssl_enabled {
                let (tls, _) = tokio_postgres_rustls::MakeRustlsConnect::with_native_certs().map_err(|_| "系统证书加载失败".to_string())?;
                let (client, connection) = tokio::time::timeout(CONNECTION_TIMEOUT, cfg.connect(tls)).await.map_err(|_| "数据库连接超时（8 秒）".to_string())?.map_err(|e| e.to_string())?;
                let _connection_task = ConnectionTask(tokio::spawn(async move { let _ = connection.await; }));
                read_postgres_result(&client, sql, limit).await
            } else {
                let (client, connection) = tokio::time::timeout(CONNECTION_TIMEOUT, cfg.connect(tokio_postgres::NoTls)).await.map_err(|_| "数据库连接超时（8 秒）".to_string())?.map_err(|e| e.to_string())?;
                let _connection_task = ConnectionTask(tokio::spawn(async move { let _ = connection.await; }));
                read_postgres_result(&client, sql, limit).await
            }
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
}
