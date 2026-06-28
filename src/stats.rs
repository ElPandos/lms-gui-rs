//! In-memory traffic statistics tracker.

use chrono::{DateTime, Utc};
use serde::Serialize;
use std::collections::VecDeque;

/// Tracks request counts, API calls, downloads, and errors with an event log.
#[derive(Clone, Serialize)]
pub struct TrafficStats {
    pub total_requests: u64,
    pub api_calls: u64,
    pub model_downloads: u64,
    pub chat_completions: u64,
    pub errors: u64,
    pub started_at: DateTime<Utc>,
    pub recent_events: VecDeque<Event>,
}

impl Default for TrafficStats {
    fn default() -> Self {
        Self {
            total_requests: 0,
            api_calls: 0,
            model_downloads: 0,
            chat_completions: 0,
            errors: 0,
            started_at: Utc::now(),
            recent_events: VecDeque::new(),
        }
    }
}

/// A timestamped traffic event (API call, download, or error).
#[derive(Clone, Serialize)]
pub struct Event {
    pub timestamp: DateTime<Utc>,
    pub kind: String,
    pub detail: String,
}

impl TrafficStats {
    /// Increment the total request counter.
    pub fn record_request(&mut self) {
        self.total_requests += 1;
    }

    /// Record an API call with a detail string.
    pub fn record_api_call(&mut self, detail: &str) {
        self.api_calls += 1;
        self.total_requests += 1;
        self.push_event("api_call", detail);
    }

    /// Record a model download event.
    pub fn record_download(&mut self, model: &str) {
        self.model_downloads += 1;
        self.total_requests += 1;
        self.push_event("download", model);
    }

    /// Record an error event.
    pub fn record_error(&mut self, detail: &str) {
        tracing::warn!(detail = %detail, "Error event recorded");
        self.errors += 1;
        self.push_event("error", detail);
    }

    /// Record a chat completion.
    pub fn record_chat(&mut self, model: &str) {
        self.chat_completions += 1;
        self.total_requests += 1;
        self.push_event("chat", model);
    }

    /// Reset all counters (keeps started_at).
    pub fn reset(&mut self) {
        self.total_requests = 0;
        self.api_calls = 0;
        self.model_downloads = 0;
        self.chat_completions = 0;
        self.errors = 0;
        self.recent_events.clear();
        self.started_at = Utc::now();
    }

    /// Compute uptime in seconds.
    pub fn uptime_secs(&self) -> i64 {
        (Utc::now() - self.started_at).num_seconds()
    }

    /// Append a timestamped event, evicting the oldest when over 100 entries.
    fn push_event(&mut self, kind: &str, detail: &str) {
        self.recent_events.push_back(Event {
            timestamp: Utc::now(),
            kind: kind.to_string(),
            detail: detail.to_string(),
        });
        if self.recent_events.len() > 100 {
            self.recent_events.pop_front();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bench_percentile(samples: &mut [std::time::Duration], pct: f64) -> std::time::Duration {
        if samples.is_empty() {
            return std::time::Duration::ZERO;
        }
        samples.sort();
        let idx = ((samples.len() as f64 - 1.0) * pct).round() as usize;
        samples[idx]
    }

    #[test]
    fn bench_stats_push_200_events() {
        let iterations = 1000;
        let warmup = iterations / 10;
        for _ in 0..warmup {
            let mut s = TrafficStats::default();
            for i in 0..200 {
                s.record_api_call(&format!("call_{}", i));
            }
        }
        let mut samples: Vec<std::time::Duration> = Vec::with_capacity(iterations);
        for _ in 0..iterations {
            let start = std::time::Instant::now();
            let mut s = TrafficStats::default();
            for i in 0..200 {
                s.record_api_call(&format!("call_{}", i));
            }
            samples.push(start.elapsed());
        }
        let p50 = bench_percentile(&mut samples.clone(), 0.50);
        let p95 = bench_percentile(&mut samples.clone(), 0.95);
        let p99 = bench_percentile(&mut samples.clone(), 0.99);
        let mean_ns = samples.iter().map(|d| d.as_nanos()).sum::<u128>() / samples.len() as u128;
        eprintln!(
            "BENCH\tstats_push_200_events\tp50={}\tp95={}\tp99={}\tmean={}",
            p50.as_nanos(),
            p95.as_nanos(),
            p99.as_nanos(),
            mean_ns
        );
    }
}
