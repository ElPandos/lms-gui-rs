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

fn default_temperature() -> f64 { 0.7 }
fn default_max_tokens() -> u32 { 1024 }

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

/// Send a chat completion request to the loaded model.
pub async fn chat_send(
    State(state): State<AppState>,
    Json(req): Json<ChatRequest>,
) -> Json<ChatResponse> {
    let start = std::time::Instant::now();
    match state.lms.chat_completion(&req).await {
        Ok((content, tokens)) => Json(ChatResponse {
            success: true,
            content,
            duration_ms: start.elapsed().as_millis() as u64,
            tokens_used: tokens,
        }),
        Err(e) => Json(ChatResponse {
            success: false,
            content: e,
            duration_ms: start.elapsed().as_millis() as u64,
            tokens_used: 0,
        }),
    }
}

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
    let mut results: Vec<SpeedTestCall> = Vec::new();

    // Warmup call (not counted)
    let warmup_req = ChatRequest {
        model: req.model.clone(),
        message: "Say hello. Reply with one word.".to_string(),
        temperature: 0.0,
        max_tokens: req.max_tokens,
        top_p: None,
        frequency_penalty: None,
        presence_penalty: None,
        system_prompt: Some("You are a precise assistant. Always give the shortest possible answer.".to_string()),
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
            system_prompt: Some("You are a precise assistant. Always give the shortest possible answer.".to_string()),
        };

        let start = std::time::Instant::now();
        let (response, tokens) = state.lms.chat_completion(&chat_req).await.unwrap_or_default();
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

    // Calculate stats
    let durations: Vec<f64> = results.iter().map(|r| r.duration_ms as f64).collect();
    let n = durations.len() as f64;
    let mean = durations.iter().sum::<f64>() / n;
    let variance = durations.iter().map(|d| (d - mean).powi(2)).sum::<f64>() / n;
    let std_dev = variance.sqrt();

    // Mark outliers based on sigma
    if req.sigma > 0 {
        let threshold = std_dev * req.sigma as f64;
        for r in &mut results {
            if (r.duration_ms as f64 - mean).abs() > threshold {
                r.outlier = true;
            }
        }
    }

    let filtered: Vec<f64> = results.iter()
        .filter(|r| !r.outlier)
        .map(|r| r.duration_ms as f64)
        .collect();
    let filtered_mean = if filtered.is_empty() { mean } else { filtered.iter().sum::<f64>() / filtered.len() as f64 };

    let mut sorted = durations.clone();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let median = if sorted.len() % 2 == 0 {
        (sorted[sorted.len() / 2 - 1] + sorted[sorted.len() / 2]) / 2.0
    } else {
        sorted[sorted.len() / 2]
    };

    let stats = SpeedTestStats {
        mean_ms: mean,
        median_ms: median,
        min_ms: *durations.iter().min_by(|a, b| a.partial_cmp(b).unwrap()).unwrap_or(&0.0) as u64,
        max_ms: *durations.iter().max_by(|a, b| a.partial_cmp(b).unwrap()).unwrap_or(&0.0) as u64,
        std_dev_ms: std_dev,
        filtered_mean_ms: filtered_mean,
        total_calls: req.num_calls,
        filtered_calls: filtered.len() as u32,
        sigma: req.sigma,
    };

    Json(SpeedTestResult { success: true, results, stats })
}
