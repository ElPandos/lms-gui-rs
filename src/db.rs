//! SQLite database for persisting settings, chat history, and test results.

use rusqlite::{Connection, params};
use std::sync::Mutex;

/// SQLite-backed persistence for settings, chat, and test data.
pub struct Database {
    conn: Mutex<Connection>,
}

impl Database {
    /// Open (or create) the database at `path` and initialize tables.
    pub fn new(path: &str) -> Result<Self, String> {
        let conn = Connection::open(path).map_err(|e| format!("DB open error: {}", e))?;
        let db = Self { conn: Mutex::new(conn) };
        db.init_tables()?;
        Ok(db)
    }

    fn init_tables(&self) -> Result<(), String> {
        let conn = self.conn.lock().unwrap();
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS settings (
                key TEXT PRIMARY KEY,
                value TEXT NOT NULL,
                updated_at TEXT DEFAULT (datetime('now'))
            );
            CREATE TABLE IF NOT EXISTS chat_messages (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                role TEXT NOT NULL,
                model TEXT,
                content TEXT NOT NULL,
                settings_json TEXT,
                response_json TEXT,
                duration_ms INTEGER,
                tokens INTEGER,
                created_at TEXT DEFAULT (datetime('now'))
            );
            CREATE TABLE IF NOT EXISTS test_results (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                test_type TEXT NOT NULL,
                model TEXT NOT NULL,
                num_calls INTEGER,
                max_tokens INTEGER,
                sigma INTEGER,
                results_json TEXT NOT NULL,
                stats_json TEXT NOT NULL,
                created_at TEXT DEFAULT (datetime('now'))
            );
            CREATE TABLE IF NOT EXISTS test_reports (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                scope TEXT NOT NULL,
                test_type TEXT NOT NULL,
                models_json TEXT NOT NULL,
                report_json TEXT NOT NULL,
                created_at TEXT DEFAULT (datetime('now'))
            );"
        ).map_err(|e| format!("DB init error: {}", e))
    }

    // === Settings ===

    /// Insert or update a setting by key.
    pub fn set_setting(&self, key: &str, value: &str) -> Result<(), String> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT OR REPLACE INTO settings (key, value, updated_at) VALUES (?1, ?2, datetime('now'))",
            params![key, value],
        ).map_err(|e| format!("DB set error: {}", e))?;
        Ok(())
    }

    /// Retrieve a single setting value by key.
    pub fn get_setting(&self, key: &str) -> Option<String> {
        let conn = self.conn.lock().unwrap();
        conn.query_row(
            "SELECT value FROM settings WHERE key = ?1",
            params![key],
            |row| row.get(0),
        ).ok()
    }

    /// Retrieve all settings as key-value pairs.
    pub fn get_all_settings(&self) -> Vec<(String, String)> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare("SELECT key, value FROM settings ORDER BY key").unwrap();
        stmt.query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
            .unwrap()
            .filter_map(|r| r.ok())
            .collect()
    }

    // === Chat Messages ===

    /// Persist a chat message with optional metadata.
    pub fn save_chat_message(&self, role: &str, model: &str, content: &str, settings_json: Option<&str>, response_json: Option<&str>, duration_ms: Option<u64>, tokens: Option<u32>) -> Result<i64, String> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO chat_messages (role, model, content, settings_json, response_json, duration_ms, tokens) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![role, model, content, settings_json, response_json, duration_ms.map(|v| v as i64), tokens.map(|v| v as i32)],
        ).map_err(|e| format!("DB chat save error: {}", e))?;
        Ok(conn.last_insert_rowid())
    }

    /// Retrieve the most recent chat messages (chronological order).
    pub fn get_chat_history(&self, limit: u32) -> Vec<serde_json::Value> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, role, model, content, settings_json, response_json, duration_ms, tokens, created_at FROM chat_messages ORDER BY id DESC LIMIT ?1"
        ).unwrap();
        let rows: Vec<serde_json::Value> = stmt.query_map(params![limit], |row| {
            Ok(serde_json::json!({
                "id": row.get::<_, i64>(0)?,
                "role": row.get::<_, String>(1)?,
                "model": row.get::<_, String>(2).unwrap_or_default(),
                "content": row.get::<_, String>(3)?,
                "settings": row.get::<_, Option<String>>(4)?,
                "response": row.get::<_, Option<String>>(5)?,
                "duration_ms": row.get::<_, Option<i64>>(6)?,
                "tokens": row.get::<_, Option<i32>>(7)?,
                "created_at": row.get::<_, String>(8)?,
            }))
        }).unwrap().filter_map(|r| r.ok()).collect();
        rows.into_iter().rev().collect()
    }

    /// Delete all chat messages.
    pub fn clear_chat_history(&self) -> Result<(), String> {
        let conn = self.conn.lock().unwrap();
        conn.execute("DELETE FROM chat_messages", []).map_err(|e| format!("DB clear error: {}", e))?;
        Ok(())
    }

    // === Test Results ===

    /// Persist a speed test result with stats JSON.
    pub fn save_test_result(&self, test_type: &str, model: &str, num_calls: u32, max_tokens: u32, sigma: u32, results_json: &str, stats_json: &str) -> Result<i64, String> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO test_results (test_type, model, num_calls, max_tokens, sigma, results_json, stats_json) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![test_type, model, num_calls as i32, max_tokens as i32, sigma as i32, results_json, stats_json],
        ).map_err(|e| format!("DB test save error: {}", e))?;
        Ok(conn.last_insert_rowid())
    }

    /// Retrieve recent test results (newest first).
    pub fn get_test_results(&self, limit: u32) -> Vec<serde_json::Value> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, test_type, model, num_calls, max_tokens, sigma, stats_json, created_at FROM test_results ORDER BY id DESC LIMIT ?1"
        ).unwrap();
        stmt.query_map(params![limit], |row| {
            Ok(serde_json::json!({
                "id": row.get::<_, i64>(0)?,
                "test_type": row.get::<_, String>(1)?,
                "model": row.get::<_, String>(2)?,
                "num_calls": row.get::<_, i32>(3)?,
                "max_tokens": row.get::<_, i32>(4)?,
                "sigma": row.get::<_, i32>(5)?,
                "stats": row.get::<_, String>(6)?,
                "created_at": row.get::<_, String>(7)?,
            }))
        }).unwrap().filter_map(|r| r.ok()).collect()
    }

    // === Export / Import ===

    /// Export all data (settings, chat, tests) as JSON.
    pub fn export_all(&self) -> serde_json::Value {
        serde_json::json!({
            "version": 1,
            "settings": self.get_all_settings(),
            "chat_history": self.get_chat_history(1000),
            "test_results": self.get_test_results(1000),
        })
    }

    /// Import settings and chat history from a JSON export.
    pub fn import_all(&self, data: &serde_json::Value) -> Result<(), String> {
        // Import settings
        if let Some(settings) = data["settings"].as_array() {
            for s in settings {
                if let (Some(k), Some(v)) = (s[0].as_str(), s[1].as_str()) {
                    self.set_setting(k, v)?;
                }
            }
        }
        // Import chat
        if let Some(msgs) = data["chat_history"].as_array() {
            for m in msgs {
                self.save_chat_message(
                    m["role"].as_str().unwrap_or(""),
                    m["model"].as_str().unwrap_or(""),
                    m["content"].as_str().unwrap_or(""),
                    m["settings"].as_str(),
                    m["response"].as_str(),
                    m["duration_ms"].as_u64(),
                    m["tokens"].as_u64().map(|v| v as u32),
                )?;
            }
        }
        Ok(())
    }
}
