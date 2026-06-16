//! In-memory traffic statistics tracker.

use chrono::{DateTime, Utc};
use serde::Serialize;

/// Tracks request counts, API calls, downloads, and errors with an event log.
#[derive(Default, Clone, Serialize)]
pub struct TrafficStats {
    pub total_requests: u64,
    pub api_calls: u64,
    pub model_downloads: u64,
    pub errors: u64,
    pub recent_events: Vec<Event>,
}

#[derive(Clone, Serialize)]
pub struct Event {
    pub timestamp: DateTime<Utc>,
    pub kind: String,
    pub detail: String,
}

impl TrafficStats {
    pub fn record_request(&mut self) {
        self.total_requests += 1;
    }

    pub fn record_api_call(&mut self, detail: &str) {
        self.api_calls += 1;
        self.total_requests += 1;
        self.push_event("api_call", detail);
    }

    pub fn record_download(&mut self, model: &str) {
        self.model_downloads += 1;
        self.total_requests += 1;
        self.push_event("download", model);
    }

    pub fn record_error(&mut self, detail: &str) {
        self.errors += 1;
        self.push_event("error", detail);
    }

    fn push_event(&mut self, kind: &str, detail: &str) {
        self.recent_events.push(Event {
            timestamp: Utc::now(),
            kind: kind.to_string(),
            detail: detail.to_string(),
        });
        if self.recent_events.len() > 100 {
            self.recent_events.remove(0);
        }
    }
}
