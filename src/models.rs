//! Data models and CLI output parsers for LM Studio commands.

use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Model {
    pub id: String,
    pub object: String,
    pub owned_by: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ModelsResponse {
    pub data: Vec<Model>,
    pub object: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct RuntimeInfo {
    pub runtimes: String,
    pub update_status: String,
}

#[derive(Debug, Deserialize)]
pub struct DownloadRequest {
    pub model_name: String,
}

#[derive(Debug, Serialize)]
pub struct CommandResult {
    pub success: bool,
    pub message: String,
}

#[derive(Debug, Deserialize)]
pub struct SearchQuery {
    pub q: Option<String>,
    pub dl: Option<String>,
    pub source: Option<String>,
    pub sort: Option<String>,
}

// Parsed CLI output structures

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct LocalModel {
    pub name: String,
    pub params: String,
    pub arch: String,
    pub size: String,
    pub device: String,
    pub status: String,
    pub model_type: String, // "LLM" or "Embedding"
}

#[derive(Debug, Clone)]
pub struct SearchResult {
    pub name: String,
    pub description: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct HfModel {
    #[serde(rename = "modelId")]
    pub model_id: String,
    #[serde(default)]
    pub downloads: u64,
    #[serde(default)]
    pub likes: u64,
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct LoadedModel {
    pub identifier: String,
    pub model: String,
    pub status: String,
    pub size: String,
    pub context: String,
    pub device: String,
}

#[derive(Debug, Clone)]
pub struct RuntimeEntry {
    pub engine: String,
    pub selected: bool,
    pub format: String,
}

#[derive(Debug, Clone)]
pub struct HostInfo {
    pub hostname: String,
    pub cpu: String,
    pub cores: String,
    pub ram: String,
    pub gpu: String,
    pub gpu_count: String,
    pub uptime: String,
    pub disk: String,
    pub nvidia_driver: String,
    pub cuda_version: String,
    pub os: String,
}

pub fn parse_host_info(output: &str) -> HostInfo {
    let parts: Vec<&str> = output.trim().split('|').collect();
    HostInfo {
        hostname: parts.first().unwrap_or(&"").to_string(),
        cpu: parts.get(1).unwrap_or(&"").to_string(),
        cores: parts.get(2).unwrap_or(&"").to_string(),
        ram: parts.get(3).unwrap_or(&"").to_string(),
        gpu: parts.get(4).unwrap_or(&"").to_string(),
        gpu_count: parts.get(5).unwrap_or(&"0").to_string(),
        uptime: parts.get(6).unwrap_or(&"").to_string(),
        disk: parts.get(7).unwrap_or(&"").to_string(),
        nvidia_driver: parts.get(8).unwrap_or(&"").to_string(),
        cuda_version: parts.get(9).unwrap_or(&"").to_string(),
        os: parts.get(10).unwrap_or(&"").to_string(),
    }
}

/// Parse `lms ls` output into structured LocalModel entries
pub fn parse_local_models(output: &str) -> Vec<LocalModel> {
    let mut models = Vec::new();
    let mut current_type = String::new();

    for line in output.lines() {
        let trimmed = line.trim();

        // Detect section headers
        if trimmed.starts_with("LLM") {
            current_type = "LLM".to_string();
            continue;
        }
        if trimmed.starts_with("EMBEDDING") {
            current_type = "Embedding".to_string();
            continue;
        }
        // Skip non-data lines
        if trimmed.is_empty() || trimmed.starts_with("You have") || trimmed.contains("PARAMS") {
            continue;
        }

        // Split name from rest (double-space separated)
        let parts: Vec<&str> = trimmed.splitn(2, "  ").collect();
        let mut name = parts[0].trim().to_string();
        // Strip "(N variant)" or "(N variants)" suffix
        if let Some(idx) = name.find(" (") {
            name.truncate(idx);
        }
        if name.is_empty() {
            continue;
        }

        let cols: Vec<&str> = parts.get(1).unwrap_or(&"").split_whitespace().collect();
        let (params, arch, size, device, status) = parse_model_columns(&cols);

        models.push(LocalModel {
            name,
            params,
            arch,
            size,
            device,
            status,
            model_type: current_type.clone(),
        });
    }
    models
}

fn parse_model_columns(cols: &[&str]) -> (String, String, String, String, String) {
    if cols.len() < 5 {
        let params = cols.first().unwrap_or(&"").to_string();
        let arch = cols.get(1).unwrap_or(&"").to_string();
        let size = cols.get(2..4).map(|s| s.join(" ")).unwrap_or_default();
        return (params, arch, size, String::new(), String::new());
    }

    let params = cols[0].to_string();
    let arch = cols[1].to_string();

    // Find size (number followed by GB/MB)
    let mut size = String::new();
    let mut after_size = 2;
    for i in 2..cols.len() {
        if cols[i].ends_with("GB") || cols[i].ends_with("MB") {
            size = cols[2..=i].join(" ");
            after_size = i + 1;
            break;
        }
        if cols[i].parse::<f64>().is_ok() && cols.get(i + 1).is_some_and(|u| *u == "GB" || *u == "MB") {
            size = format!("{} {}", cols[i], cols[i + 1]);
            after_size = i + 2;
            break;
        }
    }

    let device = cols.get(after_size).unwrap_or(&"").to_string();
    let status = cols.get(after_size + 1..).map(|s| s.join(" ")).unwrap_or_default();

    (params, arch, size, device, status)
}

/// Parse `lms get` search output into structured results (deduplicated)
pub fn parse_search_results(output: &str) -> Vec<SearchResult> {
    let mut results = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for line in output.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty()
            || trimmed.contains("navigate")
            || trimmed.contains("select?")
            || trimmed.contains("Select a model")
            || trimmed.starts_with("Searching")
            || trimmed.starts_with("No exact match")
            || trimmed.starts_with("?")
        {
            continue;
        }

        let clean = trimmed.trim_start_matches(['❯', '>', ' ', '?']);

        // Try multiple dash separators (em-dash, en-dash, double-hyphen)
        let split = clean.split_once(" \u{2014} ")
            .or_else(|| clean.split_once(" — "))
            .or_else(|| clean.split_once(" - "))
            .or_else(|| clean.split_once(" -- "));

        if let Some((name, desc)) = split {
            let name = name.trim().to_string();
            if name.contains('/') && name.len() > 3 && seen.insert(name.clone()) {
                results.push(SearchResult {
                    name,
                    description: desc.trim().to_string(),
                });
            }
        } else if clean.contains('/') && !clean.contains(' ') && clean.len() > 3 {
            let name = clean.to_string();
            if seen.insert(name.clone()) {
                results.push(SearchResult {
                    name,
                    description: String::new(),
                });
            }
        }
    }
    results
}

/// Parse `lms ps` output into structured entries
pub fn parse_loaded_models(output: &str) -> Vec<LoadedModel> {
    let mut models = Vec::new();
    let lines: Vec<&str> = output.lines().collect();

    // Skip header lines (IDENTIFIER, STATUS, etc.)
    for line in &lines {
        let trimmed = line.trim();
        if trimmed.is_empty()
            || trimmed.starts_with("IDENTIFIER")
            || trimmed.starts_with("STATUS")
            || trimmed.starts_with("PARALLEL")
        {
            continue;
        }
        let cols: Vec<&str> = trimmed.split_whitespace().collect();
        if cols.len() >= 2 && cols[0].contains('/') {
            let identifier = cols[0].to_string();
            let model = if cols.len() > 1 && cols[1].contains('/') { cols[1].to_string() } else { identifier.clone() };
            let status = cols.iter().find(|c| ["IDLE", "PROCESSING", "LOADING"].contains(c)).unwrap_or(&"").to_string();
            let size = cols.iter().zip(cols.iter().skip(1))
                .find(|(a, b)| a.parse::<f64>().is_ok() && (**b == "GB" || **b == "MB"))
                .map(|(a, b)| format!("{} {}", a, b))
                .unwrap_or_default();
            let device = cols.iter().find(|c| ["Local", "GPU", "CPU"].contains(c)).unwrap_or(&"").to_string();
            let context = cols.iter().find(|c| c.parse::<u32>().is_ok() && c.len() >= 3).unwrap_or(&"").to_string();

            models.push(LoadedModel { identifier, model, status, size, context, device });
        }
    }
    models
}

/// Parse `lms runtime ls` output into structured entries
pub fn parse_runtimes(output: &str) -> Vec<RuntimeEntry> {
    let mut entries = Vec::new();
    for line in output.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with("LLM ENGINE") || trimmed.contains("SELECTED") || trimmed.contains("MODEL FORMAT") {
            continue;
        }
        let selected = trimmed.contains('✓');
        let parts: Vec<&str> = trimmed.split_whitespace().collect();
        if let Some(engine) = parts.first() {
            if engine.starts_with("llama") || engine.starts_with("mlx") || engine.contains("cpp") {
                let format = parts.last().unwrap_or(&"").to_string();
                entries.push(RuntimeEntry {
                    engine: engine.to_string(),
                    selected,
                    format,
                });
            }
        }
    }
    entries
}
