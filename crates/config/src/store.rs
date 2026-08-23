//! ConfigService：单一 SQLite 配置库 + 角色分区表 + schema 驱动 + 脱敏。

use rusqlite::{Connection, params};

use crate::vault::Vault;

/// 配置库错误。
#[derive(Debug)]
pub enum DbError {
    Sql(String),
    Io(String),
}

impl std::fmt::Display for DbError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DbError::Sql(e) => write!(f, "sql: {e}"),
            DbError::Io(e) => write!(f, "io: {e}"),
        }
    }
}

impl From<rusqlite::Error> for DbError {
    fn from(e: rusqlite::Error) -> Self { DbError::Sql(e.to_string()) }
}

impl From<std::io::Error> for DbError {
    fn from(e: std::io::Error) -> Self { DbError::Io(e.to_string()) }
}

/// 角色视图：配置按角色隔离（服务端看不到消费端表，反之亦然）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RoleView {
    /// 全局。
    Global,
    /// 服务端（组长）。
    Server,
    /// 消费端（组员）。
    Client,
}

/// 配置条目。
#[derive(Debug, Clone)]
pub struct ConfigEntry {
    pub key: String,
    pub value: String,
    pub role: String,
    /// 是否敏感（脱敏）。
    pub secret: bool,
}

/// 配置服务：单库 + 分区 + 加密 + 脱敏。
pub struct ConfigService {
    conn: Connection,
    vault: Vault,
}

impl ConfigService {
    /// 打开/创建配置库（建表）。
    pub fn open(data_dir: &std::path::Path, db_name: &str) -> Result<Self, DbError> {
        std::fs::create_dir_all(data_dir)?;
        let db_path = data_dir.join(db_name);
        let conn = Connection::open(&db_path)?;
        Self::init_schema(&conn)?;
        let vault = Vault::new(data_dir);
        Ok(Self { conn, vault })
    }

    /// 建表：角色分区。
    fn init_schema(conn: &Connection) -> Result<(), DbError> {
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS settings (key TEXT PRIMARY KEY, value TEXT NOT NULL, secret INTEGER NOT NULL DEFAULT 0); \
             CREATE TABLE IF NOT EXISTS server_config (key TEXT PRIMARY KEY, value TEXT NOT NULL, secret INTEGER NOT NULL DEFAULT 0); \
             CREATE TABLE IF NOT EXISTS client_config (key TEXT PRIMARY KEY, value TEXT NOT NULL, secret INTEGER NOT NULL DEFAULT 0); \
             CREATE TABLE IF NOT EXISTS node_identity (key TEXT PRIMARY KEY, value TEXT NOT NULL, secret INTEGER NOT NULL DEFAULT 0); \
             CREATE TABLE IF NOT EXISTS members (member_id TEXT PRIMARY KEY, machine_name TEXT NOT NULL, ip TEXT, display_name TEXT, last_seen INTEGER, joined_at INTEGER); \
             CREATE TABLE IF NOT EXISTS usage (member_id TEXT PRIMARY KEY, prompt_tokens INTEGER DEFAULT 0, completion_tokens INTEGER DEFAULT 0, calls INTEGER DEFAULT 0); \
             CREATE TABLE IF NOT EXISTS client_credentials (leader_id TEXT PRIMARY KEY, token_encrypted TEXT, password_encrypted TEXT)"
        )?;
        Ok(())
    }

    /// 表名（按角色）。
    fn table_for(role: RoleView) -> &'static str {
        match role {
            RoleView::Global => "settings",
            RoleView::Server => "server_config",
            RoleView::Client => "client_config",
        }
    }

    /// 设置配置（敏感值加密存储）。
    pub fn set(&self, role: RoleView, key: &str, value: &str, secret: bool) -> Result<(), DbError> {
        let table = Self::table_for(role);
        let stored = if secret { self.vault.encrypt(value) } else { value.to_string() };
        let sql = format!("INSERT INTO {table} (key, value, secret) VALUES (?1, ?2, ?3) ON CONFLICT(key) DO UPDATE SET value=excluded.value, secret=excluded.secret");
        self.conn.execute(&sql, params![key, stored, secret as i32])?;
        Ok(())
    }

    /// 读取配置（敏感值解密返回明文；调用方负责脱敏展示）。
    pub fn get(&self, role: RoleView, key: &str) -> Result<Option<String>, DbError> {
        let table = Self::table_for(role);
        let sql = format!("SELECT value, secret FROM {table} WHERE key = ?1");
        let mut stmt = self.conn.prepare(&sql)?;
        let mut rows = stmt.query(params![key])?;
        if let Some(row) = rows.next()? {
            let stored: String = row.get(0)?;
            let secret: i32 = row.get(1)?;
            let value = if secret == 1 { self.vault.decrypt(&stored).unwrap_or_default() } else { stored };
            Ok(Some(value))
        } else {
            Ok(None)
        }
    }

    /// 列出配置（已脱敏：敏感值只回显是否已设置）。
    pub fn list(&self, role: RoleView) -> Result<Vec<ConfigEntry>, DbError> {
        let table = Self::table_for(role);
        let sql = format!("SELECT key, value, secret FROM {table}");
        let mut stmt = self.conn.prepare(&sql)?;
        let mut out = Vec::new();
        let rows = stmt.query_map([], |row| {
            Ok(ConfigEntry {
                key: row.get(0)?,
                value: row.get(1)?,
                role: table.to_string(),
                secret: row.get::<_, i32>(2)? == 1,
            })
        })?;
        for r in rows {
            let mut e = r?;
            if e.secret {
                e.value = if self.vault.decrypt(&e.value).map(|v| !v.is_empty()).unwrap_or(false) {
                    "[set]".to_string()
                } else {
                    "[unset]".to_string()
                };
            }
            out.push(e);
        }
        Ok(out)
    }

    /// 删除配置。
    pub fn delete(&self, role: RoleView, key: &str) -> Result<(), DbError> {
        let table = Self::table_for(role);
        let sql = format!("DELETE FROM {table} WHERE key = ?1");
        self.conn.execute(&sql, params![key])?;
        Ok(())
    }

    /// 节点身份（全局）。
    pub fn node_identity(&self) -> Result<Option<String>, DbError> {
        self.get(RoleView::Global, "machine_name")
    }

    pub fn set_node_identity(&self, machine_name: &str, display_name: &str) -> Result<(), DbError> {
        self.set(RoleView::Global, "machine_name", machine_name, false)?;
        self.set(RoleView::Global, "display_name", display_name, false)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // 每个测试独立目录（原子计数器），避免并行测试互相清理
    fn test_db() -> (ConfigService, std::path::PathBuf) {
        use std::sync::atomic::{AtomicU64, Ordering};
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("aipg-store-test-{}-{}", std::process::id(), n));
        let _ = std::fs::remove_dir_all(&dir);
        let svc = ConfigService::open(&dir, "test.db").unwrap();
        (svc, dir)
    }

    #[test]
    fn role_partition_isolation() {
        let (svc, dir) = test_db();
        svc.set(RoleView::Server, "port", "39091", false).unwrap();
        svc.set(RoleView::Client, "leader_list", "[{\"name\":\"l1\"}]", false).unwrap();
        assert!(svc.get(RoleView::Server, "leader_list").unwrap().is_none());
        assert_eq!(svc.get(RoleView::Server, "port").unwrap().as_deref(), Some("39091"));
        assert!(svc.get(RoleView::Client, "port").unwrap().is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn secret_encrypted_and_redacted() {
        let (svc, dir) = test_db();
        svc.set(RoleView::Server, "password", "hunter2", true).unwrap();
        assert_eq!(svc.get(RoleView::Server, "password").unwrap().as_deref(), Some("hunter2"));
        let list = svc.list(RoleView::Server).unwrap();
        let pw = list.iter().find(|e| e.key == "password").unwrap();
        assert_eq!(pw.value, "[set]");
        drop(svc);
        let raw = std::fs::read(dir.join("test.db")).unwrap();
        assert!(!raw.windows(7).any(|w| w == b"hunter2"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn node_identity_roundtrip() {
        let (svc, dir) = test_db();
        svc.set_node_identity("pc-1", "alice").unwrap();
        assert_eq!(svc.node_identity().unwrap().as_deref(), Some("pc-1"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}