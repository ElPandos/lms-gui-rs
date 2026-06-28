//! SQLite database for persisting settings, chat history, and test results.

use rusqlite::{params, Connection};
use std::sync::Mutex;

/// SQLite-backed persistence for settings, chat, and test data.
pub struct Database {
    conn: Mutex<Connection>,
}

/// Parameters for saving a chat message (avoids 7-positional-arg call sites).
pub struct ChatMessage<'a> {
    pub role: &'a str,
    pub model: &'a str,
    pub content: &'a str,
    pub settings_json: Option<&'a str>,
    pub response_json: Option<&'a str>,
    pub duration_ms: Option<u64>,
    pub tokens: Option<u32>,
}

impl Database {
    /// Open (or create) the database at `path` and initialize tables.
    pub fn new(path: &str) -> Result<Self, String> {
        tracing::info!(path = %path, "Opening database");
        let conn = Connection::open(path).map_err(|e| {
            tracing::error!(path = %path, error = %e, "Failed to open database");
            format!("DB open error: {}", e)
        })?;
        let db = Self {
            conn: Mutex::new(conn),
        };
        db.init_tables()?;
        tracing::debug!("Database tables initialized");
        Ok(db)
    }

    fn init_tables(&self) -> Result<(), String> {
        let conn = self.conn.lock().unwrap_or_else(|e| {
            tracing::error!(error = %e, "DB mutex poisoned — recovering");
            e.into_inner()
        });
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
            );",
        )
        .map_err(|e| format!("DB init error: {}", e))
    }

    // === Settings ===

    /// Insert or update a setting by key.
    pub fn set_setting(&self, key: &str, value: &str) -> Result<(), String> {
        tracing::debug!(key = %key, "Saving setting");
        let conn = self.conn.lock().unwrap_or_else(|e| {
            tracing::error!(error = %e, "DB mutex poisoned — recovering");
            e.into_inner()
        });
        conn.execute(
            "INSERT OR REPLACE INTO settings (key, value, updated_at) VALUES (?1, ?2, datetime('now'))",
            params![key, value],
        ).map_err(|e| format!("DB set error: {}", e))?;
        Ok(())
    }

    /// Retrieve a single setting value by key.
    pub fn get_setting(&self, key: &str) -> Option<String> {
        let conn = self.conn.lock().unwrap_or_else(|e| {
            tracing::error!(error = %e, "DB mutex poisoned — recovering");
            e.into_inner()
        });
        conn.query_row(
            "SELECT value FROM settings WHERE key = ?1",
            params![key],
            |row| row.get(0),
        )
        .ok()
    }

    /// Retrieve all settings as key-value pairs.
    pub fn get_all_settings(&self) -> Vec<(String, String)> {
        let conn = self.conn.lock().unwrap_or_else(|e| {
            tracing::error!(error = %e, "DB mutex poisoned — recovering");
            e.into_inner()
        });
        let mut stmt = match conn.prepare("SELECT key, value FROM settings ORDER BY key") {
            Ok(s) => s,
            Err(e) => {
                tracing::error!(error = %e, "Failed to prepare settings query");
                return Vec::new();
            }
        };
        let result = match stmt.query_map([], |row| Ok((row.get(0)?, row.get(1)?))) {
            Ok(rows) => rows.filter_map(|r| r.ok()).collect::<Vec<_>>(),
            Err(e) => {
                tracing::error!(error = %e, "Failed to query settings");
                Vec::new()
            }
        };
        result
    }

    // === Chat Messages ===

    /// Persist a chat message with optional metadata.
    pub fn save_chat_message(&self, msg: &ChatMessage) -> Result<i64, String> {
        tracing::debug!(role = %msg.role, model = %msg.model, "Saving chat message");
        let conn = self.conn.lock().unwrap_or_else(|e| {
            tracing::error!(error = %e, "DB mutex poisoned — recovering");
            e.into_inner()
        });
        conn.execute(
            "INSERT INTO chat_messages (role, model, content, settings_json, response_json, duration_ms, tokens) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![msg.role, msg.model, msg.content, msg.settings_json, msg.response_json, msg.duration_ms.map(|v| v as i64), msg.tokens.map(|v| v as i32)],
        ).map_err(|e| format!("DB chat save error: {}", e))?;
        Ok(conn.last_insert_rowid())
    }

    /// Retrieve the most recent chat messages (chronological order).
    pub fn get_chat_history(&self, limit: u32) -> Vec<serde_json::Value> {
        let conn = self.conn.lock().unwrap_or_else(|e| {
            tracing::error!(error = %e, "DB mutex poisoned — recovering");
            e.into_inner()
        });
        let mut stmt = match conn.prepare(
            "SELECT id, role, model, content, settings_json, response_json, duration_ms, tokens, created_at FROM chat_messages ORDER BY id DESC LIMIT ?1"
        ) {
            Ok(s) => s,
            Err(e) => {
                tracing::error!(error = %e, "Failed to prepare chat history query");
                return Vec::new();
            }
        };
        let result = stmt.query_map(params![limit], |row| {
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
        });
        let result = match result {
            Ok(rows) => {
                let rows: Vec<serde_json::Value> = rows.filter_map(|r| r.ok()).collect();
                rows.into_iter().rev().collect::<Vec<_>>()
            }
            Err(e) => {
                tracing::error!(error = %e, "Failed to query chat history");
                Vec::new()
            }
        };
        result
    }

    /// Delete all chat messages.
    pub fn clear_chat_history(&self) -> Result<(), String> {
        tracing::info!("Clearing all chat history");
        let conn = self.conn.lock().unwrap_or_else(|e| {
            tracing::error!(error = %e, "DB mutex poisoned — recovering");
            e.into_inner()
        });
        conn.execute("DELETE FROM chat_messages", [])
            .map_err(|e| format!("DB clear error: {}", e))?;
        Ok(())
    }

    // === Test Results ===

    /// Persist a speed test result with stats JSON.
    #[allow(clippy::too_many_arguments)]
    pub fn save_test_result(
        &self,
        test_type: &str,
        model: &str,
        num_calls: u32,
        max_tokens: u32,
        sigma: u32,
        results_json: &str,
        stats_json: &str,
    ) -> Result<i64, String> {
        tracing::info!(test_type = %test_type, model = %model, num_calls, "Saving test result");
        let conn = self.conn.lock().unwrap_or_else(|e| {
            tracing::error!(error = %e, "DB mutex poisoned — recovering");
            e.into_inner()
        });
        conn.execute(
            "INSERT INTO test_results (test_type, model, num_calls, max_tokens, sigma, results_json, stats_json) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![test_type, model, num_calls as i32, max_tokens as i32, sigma as i32, results_json, stats_json],
        ).map_err(|e| format!("DB test save error: {}", e))?;
        Ok(conn.last_insert_rowid())
    }

    /// Retrieve recent test results (newest first).
    pub fn get_test_results(&self, limit: u32) -> Vec<serde_json::Value> {
        let conn = self.conn.lock().unwrap_or_else(|e| {
            tracing::error!(error = %e, "DB mutex poisoned — recovering");
            e.into_inner()
        });
        let mut stmt = match conn.prepare(
            "SELECT id, test_type, model, num_calls, max_tokens, sigma, stats_json, created_at FROM test_results ORDER BY id DESC LIMIT ?1"
        ) {
            Ok(s) => s,
            Err(e) => {
                tracing::error!(error = %e, "Failed to prepare test results query");
                return Vec::new();
            }
        };
        let result = match stmt.query_map(params![limit], |row| {
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
        }) {
            Ok(rows) => rows.filter_map(|r| r.ok()).collect::<Vec<_>>(),
            Err(e) => {
                tracing::error!(error = %e, "Failed to query test results");
                Vec::new()
            }
        };
        result
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
        tracing::info!("Importing data from JSON export");
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
                self.save_chat_message(&ChatMessage {
                    role: m["role"].as_str().unwrap_or(""),
                    model: m["model"].as_str().unwrap_or(""),
                    content: m["content"].as_str().unwrap_or(""),
                    settings_json: m["settings"].as_str(),
                    response_json: m["response"].as_str(),
                    duration_ms: m["duration_ms"].as_u64(),
                    tokens: m["tokens"].as_u64().map(|v| v as u32),
                })?;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn fresh_db() -> Database {
        Database::new(":memory:").expect("failed to open in-memory db")
    }

    // === set_setting / get_setting ===

    #[test]
    fn set_and_get_setting() {
        let db = fresh_db();
        db.set_setting("key1", "val1").unwrap();
        assert_eq!(db.get_setting("key1"), Some("val1".to_string()));
    }

    #[test]
    fn set_setting_replaces_existing_value() {
        let db = fresh_db();
        db.set_setting("key1", "val1").unwrap();
        db.set_setting("key1", "val2").unwrap();
        assert_eq!(db.get_setting("key1"), Some("val2".to_string()));
    }

    #[test]
    fn get_setting_nonexistent_returns_none() {
        let db = fresh_db();
        assert_eq!(db.get_setting("nonexistent"), None);
    }

    // === get_all_settings ===

    #[test]
    fn get_all_settings_returns_sorted_pairs() {
        let db = fresh_db();
        db.set_setting("charlie", "3").unwrap();
        db.set_setting("alpha", "1").unwrap();
        db.set_setting("bravo", "2").unwrap();

        let all = db.get_all_settings();
        assert_eq!(all.len(), 3);
        assert_eq!(all[0], ("alpha".to_string(), "1".to_string()));
        assert_eq!(all[1], ("bravo".to_string(), "2".to_string()));
        assert_eq!(all[2], ("charlie".to_string(), "3".to_string()));
    }

    #[test]
    fn get_all_settings_empty_when_none_set() {
        let db = fresh_db();
        assert!(db.get_all_settings().is_empty());
    }

    // === save_chat_message / get_chat_history ===

    #[test]
    fn save_and_get_chat_message_all_fields() {
        let db = fresh_db();
        let id = db
            .save_chat_message(&ChatMessage {
                role: "user",
                model: "llama-3",
                content: "Hello world",
                settings_json: Some(r#"{"temperature":0.7}"#),
                response_json: Some(r#"{"id":"resp1"}"#),
                duration_ms: Some(1500),
                tokens: Some(42),
            })
            .unwrap();
        assert!(id > 0);

        let history = db.get_chat_history(10);
        assert_eq!(history.len(), 1);
        let msg = &history[0];
        assert_eq!(msg["id"].as_i64(), Some(id));
        assert_eq!(msg["role"].as_str(), Some("user"));
        assert_eq!(msg["model"].as_str(), Some("llama-3"));
        assert_eq!(msg["content"].as_str(), Some("Hello world"));
        assert_eq!(msg["settings"].as_str(), Some(r#"{"temperature":0.7}"#));
        assert_eq!(msg["response"].as_str(), Some(r#"{"id":"resp1"}"#));
        assert_eq!(msg["duration_ms"].as_i64(), Some(1500));
        assert_eq!(msg["tokens"].as_i64(), Some(42));
        assert!(msg["created_at"].as_str().is_some());
    }

    #[test]
    fn save_and_get_chat_message_optional_fields_none() {
        let db = fresh_db();
        db.save_chat_message(&ChatMessage {
            role: "assistant",
            model: "gpt-4",
            content: "Hi",
            settings_json: None,
            response_json: None,
            duration_ms: None,
            tokens: None,
        })
        .unwrap();

        let history = db.get_chat_history(10);
        assert_eq!(history.len(), 1);
        let msg = &history[0];
        assert_eq!(msg["role"].as_str(), Some("assistant"));
        assert_eq!(msg["model"].as_str(), Some("gpt-4"));
        assert_eq!(msg["content"].as_str(), Some("Hi"));
        assert_eq!(msg["settings"], json!(null));
        assert_eq!(msg["response"], json!(null));
        assert_eq!(msg["duration_ms"], json!(null));
        assert_eq!(msg["tokens"], json!(null));
    }

    #[test]
    fn chat_history_with_limit_returns_most_recent() {
        let db = fresh_db();
        db.save_chat_message(&ChatMessage {
            role: "user",
            model: "m",
            content: "first",
            settings_json: None,
            response_json: None,
            duration_ms: None,
            tokens: None,
        })
        .unwrap();
        db.save_chat_message(&ChatMessage {
            role: "user",
            model: "m",
            content: "second",
            settings_json: None,
            response_json: None,
            duration_ms: None,
            tokens: None,
        })
        .unwrap();
        db.save_chat_message(&ChatMessage {
            role: "user",
            model: "m",
            content: "third",
            settings_json: None,
            response_json: None,
            duration_ms: None,
            tokens: None,
        })
        .unwrap();

        let history = db.get_chat_history(2);
        assert_eq!(history.len(), 2);
        // Most recent 2 (id 2,3) then reversed to chronological: "second", "third"
        assert_eq!(history[0]["content"].as_str(), Some("second"));
        assert_eq!(history[1]["content"].as_str(), Some("third"));
    }

    #[test]
    fn chat_history_returns_chronological_order() {
        let db = fresh_db();
        db.save_chat_message(&ChatMessage {
            role: "user",
            model: "m",
            content: "first",
            settings_json: None,
            response_json: None,
            duration_ms: None,
            tokens: None,
        })
        .unwrap();
        db.save_chat_message(&ChatMessage {
            role: "user",
            model: "m",
            content: "second",
            settings_json: None,
            response_json: None,
            duration_ms: None,
            tokens: None,
        })
        .unwrap();
        db.save_chat_message(&ChatMessage {
            role: "user",
            model: "m",
            content: "third",
            settings_json: None,
            response_json: None,
            duration_ms: None,
            tokens: None,
        })
        .unwrap();

        let history = db.get_chat_history(10);
        assert_eq!(history.len(), 3);
        assert_eq!(history[0]["content"].as_str(), Some("first"));
        assert_eq!(history[1]["content"].as_str(), Some("second"));
        assert_eq!(history[2]["content"].as_str(), Some("third"));
    }

    #[test]
    fn chat_history_empty_when_none_saved() {
        let db = fresh_db();
        assert!(db.get_chat_history(10).is_empty());
    }

    // === clear_chat_history ===

    #[test]
    fn clear_chat_history_removes_all_messages() {
        let db = fresh_db();
        db.save_chat_message(&ChatMessage {
            role: "user",
            model: "m",
            content: "a",
            settings_json: None,
            response_json: None,
            duration_ms: None,
            tokens: None,
        })
        .unwrap();
        db.save_chat_message(&ChatMessage {
            role: "user",
            model: "m",
            content: "b",
            settings_json: None,
            response_json: None,
            duration_ms: None,
            tokens: None,
        })
        .unwrap();
        assert_eq!(db.get_chat_history(10).len(), 2);

        db.clear_chat_history().unwrap();

        assert!(db.get_chat_history(10).is_empty());
    }

    // === save_test_result / get_test_results ===

    #[test]
    fn save_and_get_test_result() {
        let db = fresh_db();
        let id = db
            .save_test_result(
                "speed",
                "llama-3",
                5,
                100,
                3,
                r#"[{"t":120}]"#,
                r#"{"mean":120}"#,
            )
            .unwrap();
        assert!(id > 0);

        let results = db.get_test_results(10);
        assert_eq!(results.len(), 1);
        let r = &results[0];
        assert_eq!(r["id"].as_i64(), Some(id));
        assert_eq!(r["test_type"].as_str(), Some("speed"));
        assert_eq!(r["model"].as_str(), Some("llama-3"));
        assert_eq!(r["num_calls"].as_i64(), Some(5));
        assert_eq!(r["max_tokens"].as_i64(), Some(100));
        assert_eq!(r["sigma"].as_i64(), Some(3));
        assert_eq!(r["stats"].as_str(), Some(r#"{"mean":120}"#));
        assert!(r["created_at"].as_str().is_some());
        // Note: results_json is NOT returned by get_test_results (current behavior)
        assert!(r.get("results").is_none());
        assert!(r.get("results_json").is_none());
    }

    #[test]
    fn get_test_results_empty_when_none_saved() {
        let db = fresh_db();
        assert!(db.get_test_results(10).is_empty());
    }

    #[test]
    fn get_test_results_returns_newest_first() {
        let db = fresh_db();
        db.save_test_result("speed", "m1", 1, 10, 1, "[]", "{}")
            .unwrap();
        db.save_test_result("speed", "m2", 2, 20, 2, "[]", "{}")
            .unwrap();
        db.save_test_result("speed", "m3", 3, 30, 3, "[]", "{}")
            .unwrap();

        let results = db.get_test_results(10);
        assert_eq!(results.len(), 3);
        // Newest first (DESC by id) — not reversed
        assert_eq!(results[0]["model"].as_str(), Some("m3"));
        assert_eq!(results[1]["model"].as_str(), Some("m2"));
        assert_eq!(results[2]["model"].as_str(), Some("m1"));
    }

    // === export_all / import_all round-trip ===

    #[test]
    fn export_all_structure_has_version_and_arrays() {
        let db = fresh_db();
        let export = db.export_all();
        assert_eq!(export["version"].as_i64(), Some(1));
        assert!(export["settings"].is_array());
        assert!(export["chat_history"].is_array());
        assert!(export["test_results"].is_array());
    }

    #[test]
    fn export_import_round_trip_preserves_settings_and_chat() {
        let db1 = fresh_db();
        db1.set_setting("s1", "v1").unwrap();
        db1.set_setting("s2", "v2").unwrap();
        db1.save_chat_message(&ChatMessage {
            role: "user",
            model: "llama-3",
            content: "hello",
            settings_json: Some(r#"{"temp":0.5}"#),
            response_json: Some(r#"{"id":"r1"}"#),
            duration_ms: Some(800),
            tokens: Some(10),
        })
        .unwrap();
        db1.save_chat_message(&ChatMessage {
            role: "assistant",
            model: "llama-3",
            content: "hi",
            settings_json: None,
            response_json: None,
            duration_ms: None,
            tokens: None,
        })
        .unwrap();
        // Also save a test result — it WILL be exported but NOT imported (current behavior)
        db1.save_test_result("speed", "llama-3", 3, 50, 2, r#"[]"#, r#"{"mean":100}"#)
            .unwrap();

        let export = db1.export_all();

        let db2 = fresh_db();
        db2.import_all(&export).unwrap();

        // Settings preserved
        assert_eq!(db2.get_setting("s1"), Some("v1".to_string()));
        assert_eq!(db2.get_setting("s2"), Some("v2".to_string()));
        let all = db2.get_all_settings();
        assert_eq!(all.len(), 2);

        // Chat history preserved (chronological order, all 2 messages)
        let history = db2.get_chat_history(100);
        assert_eq!(history.len(), 2);
        assert_eq!(history[0]["role"].as_str(), Some("user"));
        assert_eq!(history[0]["content"].as_str(), Some("hello"));
        assert_eq!(history[0]["settings"].as_str(), Some(r#"{"temp":0.5}"#));
        assert_eq!(history[0]["response"].as_str(), Some(r#"{"id":"r1"}"#));
        assert_eq!(history[0]["duration_ms"].as_i64(), Some(800));
        assert_eq!(history[0]["tokens"].as_i64(), Some(10));
        assert_eq!(history[1]["role"].as_str(), Some("assistant"));
        assert_eq!(history[1]["content"].as_str(), Some("hi"));
        assert_eq!(history[1]["settings"], json!(null));

        // Test results NOT imported (current behavior — not a bug to fix)
        assert!(db2.get_test_results(100).is_empty());
    }

    #[test]
    fn import_all_into_non_empty_db_merges_settings() {
        let db = fresh_db();
        db.set_setting("existing", "old").unwrap();

        let import_data = json!({
            "version": 1,
            "settings": [["existing", "new"], ["extra", "val"]],
            "chat_history": [],
            "test_results": [],
        });

        db.import_all(&import_data).unwrap();

        // INSERT OR REPLACE semantics → "existing" overwritten
        assert_eq!(db.get_setting("existing"), Some("new".to_string()));
        assert_eq!(db.get_setting("extra"), Some("val".to_string()));
    }

    #[test]
    fn import_all_with_empty_data_is_noop() {
        let db = fresh_db();
        db.set_setting("k", "v").unwrap();

        let import_data = json!({
            "version": 1,
            "settings": [],
            "chat_history": [],
            "test_results": [],
        });

        db.import_all(&import_data).unwrap();

        assert_eq!(db.get_setting("k"), Some("v".to_string()));
        assert!(db.get_chat_history(10).is_empty());
    }

    #[test]
    fn import_all_with_missing_fields_uses_defaults() {
        let db = fresh_db();

        // Chat message with only required-ish fields present; missing optional
        // fields fall back to defaults (empty strings / None)
        let import_data = json!({
            "version": 1,
            "settings": [],
            "chat_history": [
                {"role": "user", "content": "partial"}
            ],
            "test_results": [],
        });

        db.import_all(&import_data).unwrap();

        let history = db.get_chat_history(10);
        assert_eq!(history.len(), 1);
        assert_eq!(history[0]["role"].as_str(), Some("user"));
        assert_eq!(history[0]["model"].as_str(), Some("")); // unwrap_or("")
        assert_eq!(history[0]["content"].as_str(), Some("partial"));
        assert_eq!(history[0]["settings"], json!(null));
        assert_eq!(history[0]["response"], json!(null));
        assert_eq!(history[0]["duration_ms"], json!(null));
        assert_eq!(history[0]["tokens"], json!(null));
    }
}
