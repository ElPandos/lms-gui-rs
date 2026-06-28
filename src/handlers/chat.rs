//! Chat completion and latency speed test handlers.

use axum::extract::State;
use axum::Json;

use crate::AppState;

/// Request body for chat completions.
#[derive(Debug, serde::Deserialize)]
pub struct ChatRequest {
    pub model: String,
    pub message: String,
    #[serde(default = "default_temperature")]
    pub temperature: f64,
    #[serde(default = "default_max_tokens")]
    pub max_tokens: u32,
    #[serde(default)]
    pub top_p: Option<f64>,
    #[serde(default)]
    pub frequency_penalty: Option<f64>,
    #[serde(default)]
    pub presence_penalty: Option<f64>,
    #[serde(default)]
    pub system_prompt: Option<String>,
}

/// Default sampling temperature for chat completions (serde default fn).
fn default_temperature() -> f64 {
    0.7
}

/// Default max-token cap for chat completions (serde default fn).
fn default_max_tokens() -> u32 {
    1024
}

/// Response from a chat completion call.
#[derive(Debug, serde::Serialize)]
pub struct ChatResponse {
    pub success: bool,
    pub content: String,
    pub duration_ms: u64,
    pub tokens_used: u32,
}

/// Request body for the latency speed test.
#[derive(Debug, serde::Deserialize)]
pub struct SpeedTestRequest {
    pub model: String,
    pub num_calls: u32,
    pub sigma: u32, // 0=no filter, 1/2/3 = sigma levels
    #[serde(default = "default_max_tokens")]
    pub max_tokens: u32,
}

/// Aggregated speed test output with per-call data and statistics.
#[derive(Debug, serde::Serialize)]
pub struct SpeedTestResult {
    pub success: bool,
    pub results: Vec<SpeedTestCall>,
    pub stats: SpeedTestStats,
}

/// A single call in a speed test run.
#[derive(Debug, serde::Serialize, Clone)]
pub struct SpeedTestCall {
    pub index: u32,
    pub duration_ms: u64,
    pub tokens: u32,
    pub prompt: String,
    pub response: String,
    pub outlier: bool,
}

/// Summary statistics for a speed test run.
#[derive(Debug, serde::Serialize)]
pub struct SpeedTestStats {
    pub mean_ms: f64,
    pub median_ms: f64,
    pub min_ms: u64,
    pub max_ms: u64,
    pub std_dev_ms: f64,
    pub filtered_mean_ms: f64,
    pub total_calls: u32,
    pub filtered_calls: u32,
    pub sigma: u32,
}

/// Compute speed test statistics from call results. Marks outliers in-place.
///
/// - `results`: mutable slice of test calls (outlier field updated in-place)
/// - `total_calls`: the original requested call count (may differ from results.len()
///   if some calls failed)
/// - `sigma`: outlier threshold in standard deviations (0 = no filtering)
fn compute_speedtest_stats(
    results: &mut [SpeedTestCall],
    total_calls: u32,
    sigma: u32,
) -> SpeedTestStats {
    let durations: Vec<f64> = results.iter().map(|r| r.duration_ms as f64).collect();
    let n = durations.len() as f64;
    let mean = durations.iter().sum::<f64>() / n;
    let variance = durations.iter().map(|d| (d - mean).powi(2)).sum::<f64>() / n;
    let std_dev = variance.sqrt();

    // Mark outliers based on sigma
    if sigma > 0 {
        let threshold = std_dev * sigma as f64;
        for r in results.iter_mut() {
            if (r.duration_ms as f64 - mean).abs() > threshold {
                r.outlier = true;
            }
        }
    }

    let filtered: Vec<f64> = results
        .iter()
        .filter(|r| !r.outlier)
        .map(|r| r.duration_ms as f64)
        .collect();
    let filtered_mean = if filtered.is_empty() {
        mean
    } else {
        filtered.iter().sum::<f64>() / filtered.len() as f64
    };

    let mut sorted = durations.clone();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let median = if sorted.len().is_multiple_of(2) {
        (sorted[sorted.len() / 2 - 1] + sorted[sorted.len() / 2]) / 2.0
    } else {
        sorted[sorted.len() / 2]
    };

    SpeedTestStats {
        mean_ms: mean,
        median_ms: median,
        min_ms: *durations
            .iter()
            .min_by(|a, b| a.partial_cmp(b).unwrap())
            .unwrap_or(&0.0) as u64,
        max_ms: *durations
            .iter()
            .max_by(|a, b| a.partial_cmp(b).unwrap())
            .unwrap_or(&0.0) as u64,
        std_dev_ms: std_dev,
        filtered_mean_ms: filtered_mean,
        total_calls,
        filtered_calls: filtered.len() as u32,
        sigma,
    }
}

/// Send a chat completion request to the loaded model.
pub async fn chat_send(
    State(state): State<AppState>,
    Json(req): Json<ChatRequest>,
) -> Json<ChatResponse> {
    tracing::info!(model = %req.model, max_tokens = req.max_tokens, "Chat request received");
    let start = std::time::Instant::now();
    let mut s = state.stats.write().await;
    s.record_chat(&req.model);
    drop(s);
    match state.lms.chat_completion(&req).await {
        Ok((content, tokens)) => {
            let ms = start.elapsed().as_millis() as u64;
            tracing::info!(model = %req.model, duration_ms = ms, tokens, "Chat response sent");
            Json(ChatResponse {
                success: true,
                content,
                duration_ms: ms,
                tokens_used: tokens,
            })
        }
        Err(e) => {
            tracing::error!(model = %req.model, error = %e, "Chat completion failed");
            Json(ChatResponse {
                success: false,
                content: e,
                duration_ms: start.elapsed().as_millis() as u64,
                tokens_used: 0,
            })
        }
    }
}

/// Rotating set of short factual prompts used by the speed test to minimize
/// output-token variance across calls.
const SPEED_TEST_PROMPTS: &[&str] = &[
    "What is 2+2? Reply with only the number.",
    "Name the capital of France. Reply with only the city name.",
    "What color is the sky on a clear day? Reply with one word.",
    "How many legs does a cat have? Reply with only the number.",
    "What is the chemical symbol for water? Reply with only the formula.",
    "Name the largest planet in our solar system. Reply with only the name.",
    "What is the boiling point of water in Celsius? Reply with only the number.",
    "How many continents are there? Reply with only the number.",
    "What is the square root of 64? Reply with only the number.",
    "Name the first element on the periodic table. Reply with only the name.",
    "What year did World War 2 end? Reply with only the number.",
    "How many days are in a week? Reply with only the number.",
    "What is the speed of light in km/s? Reply with only the number.",
    "Name the author of Romeo and Juliet. Reply with only the name.",
    "What is the freezing point of water in Fahrenheit? Reply with only the number.",
    "How many letters in the English alphabet? Reply with only the number.",
    "What is 10 multiplied by 10? Reply with only the number.",
    "Name the smallest prime number. Reply with only the number.",
    "What gas do humans breathe in? Reply with only the name.",
    "How many hours in a day? Reply with only the number.",
];

/// Run a latency speed test against a model with multiple calls.
pub async fn chat_speedtest(
    State(state): State<AppState>,
    Json(req): Json<SpeedTestRequest>,
) -> Json<SpeedTestResult> {
    tracing::info!(model = %req.model, num_calls = req.num_calls, max_tokens = req.max_tokens, sigma = req.sigma, "Speed test started");
    let mut results: Vec<SpeedTestCall> = Vec::new();

    // Record speed test as a chat event
    {
        let mut s = state.stats.write().await;
        s.record_chat(&format!("speedtest:{} ({}x)", req.model, req.num_calls));
    }

    // Warmup call (not counted)
    let warmup_req = ChatRequest {
        model: req.model.clone(),
        message: "Say hello. Reply with one word.".to_string(),
        temperature: 0.0,
        max_tokens: req.max_tokens,
        top_p: None,
        frequency_penalty: None,
        presence_penalty: None,
        system_prompt: Some(
            "You are a precise assistant. Always give the shortest possible answer.".to_string(),
        ),
    };
    let _ = state.lms.chat_completion(&warmup_req).await;

    // Actual test calls
    for i in 0..req.num_calls {
        let prompt = SPEED_TEST_PROMPTS[i as usize % SPEED_TEST_PROMPTS.len()];
        let chat_req = ChatRequest {
            model: req.model.clone(),
            message: prompt.to_string(),
            temperature: 0.0,
            max_tokens: req.max_tokens,
            top_p: None,
            frequency_penalty: None,
            presence_penalty: None,
            system_prompt: Some(
                "You are a precise assistant. Always give the shortest possible answer."
                    .to_string(),
            ),
        };

        let start = std::time::Instant::now();
        let (response, tokens) = state
            .lms
            .chat_completion(&chat_req)
            .await
            .unwrap_or_default();
        let duration = start.elapsed().as_millis() as u64;

        results.push(SpeedTestCall {
            index: i + 1,
            duration_ms: duration,
            tokens,
            prompt: prompt.to_string(),
            response,
            outlier: false,
        });
    }

    let stats = compute_speedtest_stats(&mut results, req.num_calls, req.sigma);

    tracing::info!(model = %req.model, mean_ms = %stats.mean_ms, min_ms = stats.min_ms, max_ms = stats.max_ms, "Speed test completed");
    Json(SpeedTestResult {
        success: true,
        results,
        stats,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a [`SpeedTestCall`] with fixed tokens/prompt/response for tests.
    fn make_call(index: u32, duration_ms: u64) -> SpeedTestCall {
        SpeedTestCall {
            index,
            duration_ms,
            tokens: 10,
            prompt: "test".to_string(),
            response: "ok".to_string(),
            outlier: false,
        }
    }

    #[test]
    fn stats_basic_3_calls_sigma_0() {
        let mut results = vec![make_call(1, 100), make_call(2, 200), make_call(3, 300)];
        let stats = compute_speedtest_stats(&mut results, 3, 0);
        assert_eq!(stats.total_calls, 3);
        assert_eq!(stats.sigma, 0);
        assert_eq!(stats.mean_ms, 200.0); // (100+200+300)/3
        assert_eq!(stats.median_ms, 200.0); // middle of [100,200,300]
        assert_eq!(stats.min_ms, 100);
        assert_eq!(stats.max_ms, 300);
        // sigma=0 means no outlier marking
        assert!(!results[0].outlier);
        assert!(!results[1].outlier);
        assert!(!results[2].outlier);
        assert_eq!(stats.filtered_calls, 3); // no outliers with sigma=0
    }

    #[test]
    fn stats_even_count_median() {
        let mut results = vec![
            make_call(1, 100),
            make_call(2, 200),
            make_call(3, 300),
            make_call(4, 400),
        ];
        let stats = compute_speedtest_stats(&mut results, 4, 0);
        assert_eq!(stats.median_ms, 250.0); // (200+300)/2
    }

    #[test]
    fn stats_outlier_marking_sigma_1() {
        // mean=250, std_dev=150, threshold=150; |500-250|=250>150 -> outlier
        let mut results = vec![
            make_call(1, 100),
            make_call(2, 200),
            make_call(3, 200),
            make_call(4, 500),
        ];
        let stats = compute_speedtest_stats(&mut results, 4, 1);
        assert!(results[3].outlier); // 500ms is an outlier
        assert!(!results[0].outlier); // 100ms is within 1 sigma
        assert!(stats.filtered_calls < 4);
    }

    #[test]
    fn stats_single_call() {
        let mut results = vec![make_call(1, 42)];
        let stats = compute_speedtest_stats(&mut results, 1, 1);
        assert_eq!(stats.mean_ms, 42.0);
        assert_eq!(stats.median_ms, 42.0);
        assert_eq!(stats.min_ms, 42);
        assert_eq!(stats.max_ms, 42);
        assert_eq!(stats.std_dev_ms, 0.0);
        assert!(!results[0].outlier);
    }
}
