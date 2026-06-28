//! Data models and CLI output parsers for LM Studio commands.

use serde::{Deserialize, Serialize};
use std::sync::LazyLock;

// Pre-compiled regexes for HfModel parsing (hoisted out of per-call hot paths).
// Previously these were compiled inside quantizations()/param_count() on every
// call — a major CPU/allocation hotspot when rendering search result pages.
static RE_QUANT: LazyLock<regex::Regex> = LazyLock::new(|| {
    regex::Regex::new(r"(?i)[iI]?-?(Q\d[\w_]*|F\d+|BF16|FP16|F16|Q\d+_[\w_]+)")
        .expect("quant regex is statically validated")
});
static RE_PARAM_B: LazyLock<regex::Regex> = LazyLock::new(|| {
    regex::Regex::new(r"(\d+\.?\d*)[bB]").expect("param_b regex is statically validated")
});
static RE_PARAM_MOE: LazyLock<regex::Regex> = LazyLock::new(|| {
    regex::Regex::new(r"(\d+)x(\d+\.?\d*)[bB]").expect("param_moe regex is statically validated")
});

/// A model entry from the LMS `/v1/models` API response.
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Model {
    pub id: String,
    pub object: String,
    pub owned_by: String,
    #[serde(default)]
    pub max_context_length: Option<u64>,
}

/// Wrapper for the LMS `/v1/models` JSON response.
#[derive(Debug, Serialize, Deserialize)]
pub struct ModelsResponse {
    pub data: Vec<Model>,
    pub object: String,
}

/// Runtime list and update status from `lms runtime`.
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct RuntimeInfo {
    pub runtimes: String,
    pub update_status: String,
}

/// JSON body for model action requests (download, load, unload, delete).
#[derive(Debug, Deserialize)]
pub struct DownloadRequest {
    pub model_name: String,
    /// Optional parallel slots override for `lms load --parallel N`.
    /// If omitted, falls back to the `default-parallel` setting.
    #[serde(default)]
    pub parallel: Option<u32>,
}

/// Generic success/failure response for action endpoints.
#[derive(Debug, Serialize)]
pub struct CommandResult {
    pub success: bool,
    pub message: String,
}

/// Query parameters for search and log endpoints.
#[derive(Debug, Deserialize)]
pub struct SearchQuery {
    pub q: Option<String>,
    pub dl: Option<String>,
    pub source: Option<String>,
    pub sort: Option<String>,
    /// Filter by model format ("gguf" or "mlx") for LMS search.
    pub format: Option<String>,
    /// Filter by HuggingFace pipeline tag (e.g. "text-generation").
    pub pipeline_tag: Option<String>,
    /// If "1", clear (kill + delete log) the download for model `q`.
    pub clear: Option<String>,
}

// Parsed CLI output structures

/// A model parsed from `lms ls` CLI output.
#[derive(Debug, Clone, Default)]
#[allow(dead_code)]
pub struct LocalModel {
    pub name: String,
    pub params: String,
    pub arch: String,
    pub size: String,
    pub device: String,
    pub status: String,
    pub model_type: String, // "LLM" or "Embedding"
    /// Max context length, from v0 API (max_context_length).
    pub max_context: String,
    /// Quantization level, from v0 API.
    pub quantization: String,
    /// Publisher, from v0 API.
    pub publisher: String,
    /// Compatibility type (gguf/mlx), from v0 API.
    pub compat_type: String,
    /// Model type from v0 API (llm/vlm/embeddings).
    pub model_vtype: String,
    /// Case-preserved filesystem path (resolved from disk, not API).
    pub file_path: String,
}

impl LocalModel {
    /// Returns the display path — the real case-preserved filesystem path if available,
    /// otherwise a best-effort construction from publisher + name.
    pub fn display_path(&self) -> String {
        if !self.file_path.is_empty() {
            self.file_path.replacen("/home/hts", "~", 1)
        } else {
            let publisher = if self.publisher.is_empty() {
                "unknown"
            } else {
                &self.publisher
            };
            format!("~/.lmstudio/models/{}/{}", publisher, self.name)
        }
    }
}

/// A model search result from `lms get` output.
#[derive(Debug, Clone)]
pub struct SearchResult {
    pub name: String,
    pub description: String,
}

/// A file in a HuggingFace model repo.
#[derive(Debug, Clone, Deserialize)]
pub struct HfSibling {
    #[serde(rename = "rfilename")]
    pub rfilename: String,
}

/// A model from the HuggingFace Hub API.
#[derive(Debug, Clone, Deserialize)]
pub struct HfModel {
    #[serde(rename = "modelId")]
    pub model_id: String,
    #[serde(default)]
    pub downloads: u64,
    #[serde(default)]
    pub likes: u64,
    /// HF tags array (contains `license:xxx`, `language:en`, `base_model:quantized:xxx`, `gguf`, etc.).
    #[serde(default)]
    pub tags: Vec<String>,
    /// e.g. "text-generation", "image-text-to-text".
    #[serde(default)]
    pub pipeline_tag: Option<String>,
    /// Whether the repo is gated (boolean from API: false = open).
    #[serde(default)]
    pub gated: bool,
    /// ISO timestamp of last modification.
    #[serde(default, rename = "lastModified")]
    pub last_modified: Option<String>,
    /// Repo author/owner.
    #[serde(default)]
    pub author: Option<String>,
    /// Files in the repo (GGUF files live here).
    #[serde(default)]
    pub siblings: Vec<HfSibling>,
}

/// Lookup table: (quant_name, bytes_per_param, description)
/// Aliases (F16/FP16, Q5_0/Q5_1, Q4_0/Q4_1) have duplicate rows since
/// they share both bytes and description in the current code.
static QUANT_TABLE: &[(&str, f64, &str)] = &[
    (
        "F16",
        2.0,
        "16-bit float — lossless, largest size, best quality",
    ),
    (
        "FP16",
        2.0,
        "16-bit float — lossless, largest size, best quality",
    ),
    ("BF16", 2.0, "Bfloat16 — lossless, large size, best quality"),
    (
        "Q8_0",
        1.06,
        "8-bit quantized — near-lossless, good speed/quality balance",
    ),
    ("Q6_K", 0.688, "6-bit quantized — high quality, large size"),
    (
        "Q5_K_M",
        0.637,
        "5-bit (K-medium) — high quality, good size/quality balance",
    ),
    (
        "Q5_K_S",
        0.617,
        "5-bit (K-small) — high quality, slightly smaller than K_M",
    ),
    (
        "Q5_0",
        0.656,
        "5-bit quantized — good quality, moderate size",
    ),
    (
        "Q5_1",
        0.656,
        "5-bit quantized — good quality, moderate size",
    ),
    (
        "Q4_K_M",
        0.581,
        "4-bit (K-medium) — recommended sweet spot, best quality/size ratio",
    ),
    (
        "Q4_K_S",
        0.562,
        "4-bit (K-small) — slightly smaller than K_M, minimal quality loss",
    ),
    (
        "Q4_0",
        0.566,
        "4-bit quantized — good balance, older format",
    ),
    (
        "Q4_1",
        0.566,
        "4-bit quantized — good balance, older format",
    ),
    (
        "Q3_K_M",
        0.446,
        "3-bit (K-medium) — smaller size, noticeable quality degradation",
    ),
    (
        "Q3_K_L",
        0.482,
        "3-bit (K-large) — larger than K_M, better quality than Q3_K_M",
    ),
    (
        "Q3_K_S",
        0.416,
        "3-bit (K-small) — very small, lower quality",
    ),
    (
        "Q2_K",
        0.352,
        "2-bit quantized — smallest size, significant quality loss",
    ),
];

impl HfModel {
    /// Extract quantization levels from sibling filenames (e.g. "model-Q4_K_M.gguf" → "Q4_K_M").
    /// Returns sorted, deduplicated list.
    pub fn quantizations(&self) -> Vec<String> {
        let mut quants: Vec<String> = self
            .siblings
            .iter()
            .filter_map(|s| {
                let name = s.rfilename.to_lowercase();
                if !name.ends_with(".gguf") {
                    return None;
                }
                RE_QUANT
                    .captures(&name)
                    .and_then(|c| c.get(1).map(|m| m.as_str().to_uppercase()))
            })
            .collect();
        quants.sort();
        quants.dedup();
        quants
    }

    /// Extract license from tags (format: "license:apache-2.0").
    pub fn license(&self) -> Option<String> {
        self.tags
            .iter()
            .find(|t| t.starts_with("license:"))
            .map(|t| t.trim_start_matches("license:").to_string())
    }

    /// Extract languages from tags (2-letter codes that aren't known prefixes).
    pub fn languages(&self) -> Vec<String> {
        self.tags
            .iter()
            .filter(|t| t.len() == 2 && t.chars().all(|c| c.is_ascii_lowercase()))
            .cloned()
            .collect()
    }

    /// Extract base model if this is a quantized derivative (format: "base_model:quantized:xxx").
    pub fn base_model(&self) -> Option<String> {
        self.tags
            .iter()
            .find(|t| t.starts_with("base_model:quantized:"))
            .map(|t| t.trim_start_matches("base_model:quantized:").to_string())
    }

    /// Estimate parameter count (in billions) from the model_id (e.g. "Llama-3.2-3B" → 3.0).
    pub fn param_count(&self) -> f64 {
        // Match patterns like "7b", "13B", "70b", "3.2B" in the model id
        if let Some(caps) = RE_PARAM_B.captures(&self.model_id) {
            if let Some(m) = caps.get(1) {
                return m.as_str().parse::<f64>().unwrap_or(0.0);
            }
        }
        // Try MoE pattern "NxNB" (e.g. "12x4B")
        if let Some(caps) = RE_PARAM_MOE.captures(&self.model_id) {
            if let (Some(n), Some(b)) = (caps.get(1), caps.get(2)) {
                let n: f64 = n.as_str().parse().unwrap_or(0.0);
                let b: f64 = b.as_str().parse().unwrap_or(0.0);
                return n * b;
            }
        }
        0.0
    }

    /// Recommend the best quantization that fits in the available VRAM.
    /// Returns the quant name (matching one from quantizations()) or None.
    pub fn recommended_quant(&self, vram_gb: &u64) -> Option<String> {
        let params = self.param_count();
        if params <= 0.0 || *vram_gb == 0 {
            return None;
        }
        let available = *vram_gb as f64;
        // Reserve ~20% for KV cache / context overhead
        let usable = available * 0.8;
        // VRAM estimate per quant (GB per billion params, approximate)
        let quants = self.quantizations();
        // Preference order: highest quality first
        let preference = [
            "F16", "Q8_0", "Q6_K", "Q5_K_M", "Q5_K_S", "Q4_K_M", "Q4_K_S", "Q4_0", "Q3_K_M",
            "Q3_K_L", "Q3_K_S", "Q2_K",
        ];
        for q in &preference {
            if quants.iter().any(|cq| cq.eq_ignore_ascii_case(q)) {
                let bytes_per_param = Self::quant_vram_bytes(q);
                let needed_gb = (params * bytes_per_param) / 1_000_000_000.0;
                if needed_gb <= usable {
                    // Return with original case from the model's list
                    return quants.iter().find(|cq| cq.eq_ignore_ascii_case(q)).cloned();
                }
            }
        }
        // Nothing fits — return the smallest available
        quants.into_iter().next()
    }

    /// Approximate VRAM usage in bytes per parameter for a given quantization.
    fn quant_vram_bytes(quant: &str) -> f64 {
        let q = quant.to_uppercase();
        QUANT_TABLE
            .iter()
            .find(|(name, _, _)| *name == q)
            .map(|(_, bytes, _)| *bytes)
            .unwrap_or(0.6) // default to Q4-ish
    }

    /// Human-readable description for a quantization level (for tooltips).
    pub fn quant_desc(&self, quant: &str) -> &'static str {
        let q = quant.to_uppercase();
        QUANT_TABLE
            .iter()
            .find(|(name, _, _)| *name == q)
            .map(|(_, _, desc)| *desc)
            .unwrap_or("Unknown quantization level")
    }
}

/// Model metadata from LM Studio v0 REST API (GET /api/v0/models).
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct LmsV0Model {
    pub id: String,
    #[serde(default)]
    pub r#type: String,
    #[serde(default)]
    pub publisher: String,
    #[serde(default)]
    pub arch: String,
    #[serde(default)]
    pub compatibility_type: String,
    #[serde(default)]
    pub quantization: String,
    #[serde(default)]
    pub state: String,
    #[serde(default)]
    pub max_context_length: Option<u64>,
}

/// Wrapper for the v0 API list response.
#[derive(Debug, Deserialize)]
pub struct LmsV0ModelsResponse {
    pub data: Vec<LmsV0Model>,
}

/// A model currently loaded in memory, parsed from `lms ps`.
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

/// An inference runtime entry from `lms runtime ls`.
#[derive(Debug, Clone)]
pub struct RuntimeEntry {
    pub engine: String,
    pub selected: bool,
    pub format: String,
    pub version: String,
}

/// Parsed host hardware and system information.
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

/// Parse pipe-delimited host info string into a [`HostInfo`] struct.
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
            ..Default::default()
        });
    }
    models
}

/// Parse the whitespace-split column tail of a `lms ls` row into
/// `(params, arch, size, device, status)`. Handles both compact ("4.5GB")
/// and split ("12 GB") size formats.
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
        if cols[i].parse::<f64>().is_ok()
            && cols.get(i + 1).is_some_and(|u| *u == "GB" || *u == "MB")
        {
            size = format!("{} {}", cols[i], cols[i + 1]);
            after_size = i + 2;
            break;
        }
    }

    let device = cols.get(after_size).unwrap_or(&"").to_string();
    let status = cols
        .get(after_size + 1..)
        .map(|s| s.join(" "))
        .unwrap_or_default();

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
        let split = clean
            .split_once(" \u{2014} ")
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

/// Parse `lms ps` output into structured entries.
///
/// Uses header column positions when available to correctly extract fields.
pub fn parse_loaded_models(output: &str) -> Vec<LoadedModel> {
    let mut models = Vec::new();
    let lines: Vec<&str> = output.lines().collect();

    // Find header line to determine column positions
    let header_idx = lines.iter().position(|l| l.contains("IDENTIFIER"));

    // Try position-based parsing using header offsets
    if let Some(hi) = header_idx {
        let header = lines[hi];
        let status_col = header.find("STATUS");
        let size_col = header.find("SIZE");
        let context_col = header.find("CONTEXT");
        let parallel_col = header.find("PARALLEL");
        let device_col = header.find("DEVICE");

        for line in &lines[hi + 1..] {
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }

            // Extract by column positions if available
            let identifier = line.split_whitespace().next().unwrap_or("").to_string();
            if identifier.is_empty() {
                continue;
            }

            let model = line.split_whitespace().nth(1).unwrap_or("").to_string();

            let status = if let Some(sc) = status_col {
                let end = size_col.unwrap_or(line.len()).min(line.len());
                if sc < line.len() {
                    line.get(sc..end).unwrap_or("").trim().to_string()
                } else {
                    String::new()
                }
            } else {
                String::new()
            };

            let size = if let (Some(sc), Some(ec)) = (size_col, context_col) {
                if sc < line.len() {
                    line.get(sc..ec.min(line.len()))
                        .unwrap_or("")
                        .trim()
                        .to_string()
                } else {
                    String::new()
                }
            } else {
                String::new()
            };

            let context = if let (Some(sc), Some(ec)) = (context_col, parallel_col.or(device_col)) {
                if sc < line.len() {
                    line.get(sc..ec.min(line.len()))
                        .unwrap_or("")
                        .trim()
                        .to_string()
                } else {
                    String::new()
                }
            } else {
                String::new()
            };

            let device = if let Some(dc) = device_col {
                if dc < line.len() {
                    line.get(dc..)
                        .unwrap_or("")
                        .split_whitespace()
                        .next()
                        .unwrap_or("")
                        .to_string()
                } else {
                    String::new()
                }
            } else {
                String::new()
            };

            models.push(LoadedModel {
                identifier,
                model,
                status,
                size,
                context,
                device,
            });
        }
    } else {
        // Fallback: keyword-based parsing
        for line in &lines {
            let trimmed = line.trim();
            if trimmed.is_empty() || trimmed.starts_with("No models") {
                continue;
            }

            let cols: Vec<&str> = trimmed.split_whitespace().collect();
            if cols.len() < 2 {
                continue;
            }

            let identifier = cols[0].to_string();
            let model = cols[1].to_string();
            let status = cols
                .iter()
                .find(|c| ["IDLE", "PROCESSING", "LOADING"].contains(c))
                .unwrap_or(&"")
                .to_string();
            let size = cols
                .iter()
                .zip(cols.iter().skip(1))
                .find(|(a, b)| a.parse::<f64>().is_ok() && (**b == "GB" || **b == "MB"))
                .map(|(a, b)| format!("{} {}", a, b))
                .unwrap_or_default();
            let device = cols
                .iter()
                .find(|c| ["Local", "GPU", "CPU"].contains(c))
                .unwrap_or(&"")
                .to_string();
            let context = cols
                .iter()
                .find(|c| c.parse::<u32>().is_ok() && c.len() >= 3)
                .unwrap_or(&"")
                .to_string();

            models.push(LoadedModel {
                identifier,
                model,
                status,
                size,
                context,
                device,
            });
        }
    }
    models
}

/// Parse `lms runtime ls` output into structured entries
pub fn parse_runtimes(output: &str) -> Vec<RuntimeEntry> {
    let mut entries = Vec::new();
    for line in output.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty()
            || trimmed.starts_with("LLM ENGINE")
            || trimmed.contains("SELECTED")
            || trimmed.contains("MODEL FORMAT")
        {
            continue;
        }
        let selected = trimmed.contains('✓');
        let parts: Vec<&str> = trimmed.split_whitespace().collect();
        if let Some(engine) = parts.first() {
            if engine.starts_with("llama") || engine.starts_with("mlx") || engine.contains("cpp") {
                let format = parts.last().unwrap_or(&"").to_string();
                // Look for a version-like token (contains a digit, isn't engine or format)
                let version = parts
                    .iter()
                    .skip(1)
                    .filter(|p| **p != "✓" && **p != format.as_str())
                    .find(|p| p.chars().any(|c| c.is_ascii_digit()))
                    .unwrap_or(&"")
                    .to_string();
                entries.push(RuntimeEntry {
                    engine: engine.to_string(),
                    selected,
                    format,
                    version,
                });
            }
        }
    }
    entries
}

#[cfg(test)]
mod tests {
    use super::*;

    // ===================== parse_host_info =====================

    #[test]
    fn host_info_full_11_fields() {
        let input = "myhost|i9-13900K|24|64GB|RTX 4090|1|3 days|2TB|535.104|12.2|Ubuntu 22.04";
        let h = parse_host_info(input);
        assert_eq!(h.hostname, "myhost");
        assert_eq!(h.cpu, "i9-13900K");
        assert_eq!(h.cores, "24");
        assert_eq!(h.ram, "64GB");
        assert_eq!(h.gpu, "RTX 4090");
        assert_eq!(h.gpu_count, "1");
        assert_eq!(h.uptime, "3 days");
        assert_eq!(h.disk, "2TB");
        assert_eq!(h.nvidia_driver, "535.104");
        assert_eq!(h.cuda_version, "12.2");
        assert_eq!(h.os, "Ubuntu 22.04");
    }

    #[test]
    fn host_info_partial_fewer_fields_defaults_to_empty() {
        let h = parse_host_info("host1|cpu1|8|32GB");
        assert_eq!(h.hostname, "host1");
        assert_eq!(h.cpu, "cpu1");
        assert_eq!(h.cores, "8");
        assert_eq!(h.ram, "32GB");
        assert_eq!(h.gpu, "");
        assert_eq!(h.gpu_count, "0");
        assert_eq!(h.uptime, "");
        assert_eq!(h.disk, "");
        assert_eq!(h.nvidia_driver, "");
        assert_eq!(h.cuda_version, "");
        assert_eq!(h.os, "");
    }

    #[test]
    fn host_info_empty_string_all_defaults() {
        let h = parse_host_info("");
        assert_eq!(h.hostname, "");
        assert_eq!(h.cpu, "");
        assert_eq!(h.cores, "");
        assert_eq!(h.ram, "");
        assert_eq!(h.gpu, "");
        assert_eq!(h.gpu_count, "0");
        assert_eq!(h.uptime, "");
        assert_eq!(h.disk, "");
        assert_eq!(h.nvidia_driver, "");
        assert_eq!(h.cuda_version, "");
        assert_eq!(h.os, "");
    }

    #[test]
    fn host_info_trims_surrounding_whitespace() {
        let h = parse_host_info("  host|cpu|8  ");
        assert_eq!(h.hostname, "host");
        assert_eq!(h.cpu, "cpu");
        assert_eq!(h.cores, "8");
    }

    // ===================== parse_local_models =====================

    #[test]
    fn local_models_llm_section_sets_type() {
        let output = "LLM MODELS\n\nYou have 2 LLMs loaded.\n\n  Llama-3.2-3B-Instruct  3.21B  llama  6.4 GB  Apple Metal  Not loaded\n  Qwen-7B-Chat           7.0B   qwen   4.5GB   Apple Metal  Not loaded\n";
        let models = parse_local_models(output);
        assert_eq!(models.len(), 2);
        assert_eq!(models[0].name, "Llama-3.2-3B-Instruct");
        assert_eq!(models[0].model_type, "LLM");
        assert_eq!(models[1].name, "Qwen-7B-Chat");
        assert_eq!(models[1].model_type, "LLM");
    }

    #[test]
    fn local_models_embedding_section_sets_type() {
        let output = "EMBEDDING MODELS\n\nYou have 1 embedding model loaded.\n\n  nomic-embed-text  0.137B  nomic  0.27 GB  Apple Metal  Not loaded\n";
        let models = parse_local_models(output);
        assert_eq!(models.len(), 1);
        assert_eq!(models[0].name, "nomic-embed-text");
        assert_eq!(models[0].model_type, "Embedding");
    }

    #[test]
    fn local_models_strips_variants_suffix() {
        let output = "LLM MODELS\n\n  bge-large-1.5 (2 variants)  0.3B  bge  1.2GB  GPU  Loaded\n";
        let models = parse_local_models(output);
        assert_eq!(models.len(), 1);
        assert_eq!(models[0].name, "bge-large-1.5");
    }

    #[test]
    fn local_models_strips_single_variant_suffix() {
        let output = "LLM MODELS\n\n  model-x (1 variant)  0.3B  bge  1.2GB  GPU  Loaded\n";
        let models = parse_local_models(output);
        assert_eq!(models.len(), 1);
        assert_eq!(models[0].name, "model-x");
    }

    #[test]
    fn local_models_skips_empty_you_have_and_params_lines() {
        let output = "LLM MODELS\n\n\nYou have 0 LLMs loaded.\n\n  PARAMS  ARCH  SIZE  DEVICE  STATUS\n  model-a  1B  arch  2GB  GPU  Loaded\n";
        let models = parse_local_models(output);
        assert_eq!(models.len(), 1);
        assert_eq!(models[0].name, "model-a");
    }

    #[test]
    fn local_models_size_with_gb_suffix_compact() {
        let output = "LLM MODELS\n\n  model-a  3B  llama  4.5GB  Apple  Loaded\n";
        let models = parse_local_models(output);
        assert_eq!(models.len(), 1);
        assert_eq!(models[0].size, "4.5GB");
        assert_eq!(models[0].params, "3B");
        assert_eq!(models[0].arch, "llama");
        assert_eq!(models[0].device, "Apple");
        assert_eq!(models[0].status, "Loaded");
    }

    #[test]
    fn local_models_size_split_number_unit() {
        let output = "LLM MODELS\n\n  model-a  7B  llama  12 GB  Apple  Loaded\n";
        let models = parse_local_models(output);
        assert_eq!(models.len(), 1);
        assert_eq!(models[0].size, "12 GB");
    }

    #[test]
    fn local_models_fewer_than_5_columns_partial_tuple() {
        // CHARACTERIZATION: cols.len()=3 < 5, so parse_model_columns takes the
        // early-return branch. cols.get(2..4) on a 3-element slice returns None
        // (range end 4 > len 3), so size defaults to "". This is current behavior.
        let output = "LLM MODELS\n\n  model-a  3B  llama  4.5GB\n";
        let models = parse_local_models(output);
        assert_eq!(models.len(), 1);
        assert_eq!(models[0].name, "model-a");
        assert_eq!(models[0].params, "3B");
        assert_eq!(models[0].arch, "llama");
        assert_eq!(models[0].size, ""); // cols.get(2..4) returns None for 3-element slice
        assert_eq!(models[0].device, "");
        assert_eq!(models[0].status, "");
    }

    #[test]
    fn local_models_preserves_section_type_across_lines() {
        let output = "LLM MODELS\n\n  model-a  3B  llama  4.5GB  GPU  Loaded\n  model-b  7B  llama  12 GB  GPU  Loaded\n\nEMBEDDING MODELS\n\n  embed-1  0.1B  bge  0.2GB  GPU  Loaded\n";
        let models = parse_local_models(output);
        assert_eq!(models.len(), 3);
        assert_eq!(models[0].model_type, "LLM");
        assert_eq!(models[1].model_type, "LLM");
        assert_eq!(models[2].model_type, "Embedding");
    }

    #[test]
    fn local_models_status_with_spaces_joined() {
        let output = "LLM MODELS\n\n  model-a  3B  llama  4.5GB  GPU  Not loaded yet\n";
        let models = parse_local_models(output);
        assert_eq!(models.len(), 1);
        assert_eq!(models[0].status, "Not loaded yet");
    }

    #[test]
    fn local_models_empty_output() {
        let models = parse_local_models("");
        assert!(models.is_empty());
    }

    #[test]
    fn local_models_no_section_header_empty_type() {
        let output = "  model-a  3B  llama  4.5GB  GPU  Loaded\n";
        let models = parse_local_models(output);
        assert_eq!(models.len(), 1);
        assert_eq!(models[0].model_type, "");
    }
    // ===================== parse_search_results =====================

    #[test]
    fn search_results_em_dash_separator() {
        let output = "meta-llama/Llama-3.2-3B-Instruct \u{2014} The Llama 3.2 3B model";
        let r = parse_search_results(output);
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].name, "meta-llama/Llama-3.2-3B-Instruct");
        assert_eq!(r[0].description, "The Llama 3.2 3B model");
    }

    #[test]
    fn search_results_double_hyphen_separator() {
        let output = "meta-llama/Llama-3.2-3B-Instruct -- The Llama 3.2 3B model";
        let r = parse_search_results(output);
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].name, "meta-llama/Llama-3.2-3B-Instruct");
        assert_eq!(r[0].description, "The Llama 3.2 3B model");
    }

    #[test]
    fn search_results_single_hyphen_separator() {
        let output = "meta-llama/Llama-3.2-3B-Instruct - The Llama 3.2 3B model";
        let r = parse_search_results(output);
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].name, "meta-llama/Llama-3.2-3B-Instruct");
        assert_eq!(r[0].description, "The Llama 3.2 3B model");
    }

    #[test]
    fn search_results_en_dash_not_handled_falls_through() {
        // U+2013 en-dash is NOT in the separator list; line contains a space so the
        // bare-name branch (no spaces) is skipped too -> 0 results.
        let output = "meta-llama/Llama-3.2-3B-Instruct \u{2013} The Llama 3.2 3B model";
        let r = parse_search_results(output);
        assert_eq!(r.len(), 0);
    }

    #[test]
    fn search_results_deduplicates_same_name() {
        let output = "meta-llama/Llama-3.2-3B-Instruct \u{2014} first description\nmeta-llama/Llama-3.2-3B-Instruct \u{2014} second description\n";
        let r = parse_search_results(output);
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].description, "first description");
    }

    #[test]
    fn search_results_bare_name_no_spaces_no_separator() {
        let output = "meta-llama/Llama-3.2-3B-Instruct";
        let r = parse_search_results(output);
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].name, "meta-llama/Llama-3.2-3B-Instruct");
        assert_eq!(r[0].description, "");
    }

    #[test]
    fn search_results_bare_name_must_have_slash() {
        let output = "Llama-3.2-3B-Instruct";
        let r = parse_search_results(output);
        assert_eq!(r.len(), 0);
    }

    #[test]
    fn search_results_bare_name_must_be_longer_than_3() {
        let r = parse_search_results("a/b");
        assert_eq!(r.len(), 0);
    }

    #[test]
    fn search_results_skips_navigate_lines() {
        let output = "meta-llama/Llama-3.2-3B-Instruct \u{2014} desc\nUse arrow keys to navigate";
        let r = parse_search_results(output);
        assert_eq!(r.len(), 1);
    }

    #[test]
    fn search_results_skips_select_question_lines() {
        let output = "select? a model\nmeta-llama/Llama-3.2-3B-Instruct \u{2014} desc";
        let r = parse_search_results(output);
        assert_eq!(r.len(), 1);
    }

    #[test]
    fn search_results_skips_select_a_model_lines() {
        let output = "Select a model to download\nmeta-llama/Llama-3.2-3B-Instruct \u{2014} desc";
        let r = parse_search_results(output);
        assert_eq!(r.len(), 1);
    }

    #[test]
    fn search_results_skips_searching_lines() {
        let output = "Searching for models...\nmeta-llama/Llama-3.2-3B-Instruct \u{2014} desc";
        let r = parse_search_results(output);
        assert_eq!(r.len(), 1);
    }

    #[test]
    fn search_results_skips_no_exact_match_lines() {
        let output = "No exact match found\nmeta-llama/Llama-3.2-3B-Instruct \u{2014} desc";
        let r = parse_search_results(output);
        assert_eq!(r.len(), 1);
    }

    #[test]
    fn search_results_skips_question_prefix_lines() {
        let output = "? Help\nmeta-llama/Llama-3.2-3B-Instruct \u{2014} desc";
        let r = parse_search_results(output);
        assert_eq!(r.len(), 1);
    }

    #[test]
    fn search_results_strips_prompt_markers() {
        let output = "\u{276F} meta-llama/Llama-3.2-3B-Instruct \u{2014} desc";
        let r = parse_search_results(output);
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].name, "meta-llama/Llama-3.2-3B-Instruct");
    }

    #[test]
    fn search_results_name_must_contain_slash_with_separator() {
        let output = "Llama-3.2-3B-Instruct \u{2014} desc";
        let r = parse_search_results(output);
        assert_eq!(r.len(), 0);
    }

    #[test]
    fn search_results_name_must_be_longer_than_3_with_separator() {
        let output = "a/b \u{2014} desc";
        let r = parse_search_results(output);
        assert_eq!(r.len(), 0);
    }

    // ===================== parse_loaded_models =====================

    #[test]
    fn loaded_models_empty_input() {
        let r = parse_loaded_models("");
        assert_eq!(r.len(), 0);
    }

    #[test]
    fn loaded_models_header_based_parsing() {
        let output = "IDENTIFIER              STATUS      SIZE      CONTEXT    PARALLEL    DEVICE\nllama-3.2-3b-instruct   IDLE        6.4 GB    4096       1           Apple\nqwen-7b-chat            PROCESSING  4.5 GB    8192       4           GPU\n";
        let r = parse_loaded_models(output);
        assert_eq!(r.len(), 2);
        assert_eq!(r[0].identifier, "llama-3.2-3b-instruct");
        assert_eq!(r[0].model, "IDLE");
        assert_eq!(r[0].status, "IDLE");
        assert_eq!(r[0].size, "6.4 GB");
        assert_eq!(r[0].context, "4096");
        assert_eq!(r[0].device, "Apple");
        assert_eq!(r[1].identifier, "qwen-7b-chat");
        assert_eq!(r[1].device, "GPU");
    }

    #[test]
    fn loaded_models_header_skips_empty_lines() {
        let output = "IDENTIFIER   STATUS   SIZE   CONTEXT   PARALLEL   DEVICE\nllama-3.2-3b   IDLE   6.4GB  4096      1          Apple\n\n";
        let r = parse_loaded_models(output);
        assert_eq!(r.len(), 1);
    }

    #[test]
    fn loaded_models_keyword_fallback_idle() {
        // Fallback device detection only recognizes "Local", "GPU", "CPU" (not "Apple").
        let output = "llama-3.2-3b llama:3.2 IDLE 6.4 GB 4096 GPU";
        let r = parse_loaded_models(output);
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].identifier, "llama-3.2-3b");
        assert_eq!(r[0].model, "llama:3.2");
        assert_eq!(r[0].status, "IDLE");
        assert_eq!(r[0].size, "6.4 GB");
        assert_eq!(r[0].device, "GPU");
        assert_eq!(r[0].context, "4096");
    }

    #[test]
    fn loaded_models_keyword_fallback_processing() {
        let output = "qwen-7b qwen:7 PROCESSING 4.5 GB 8192 GPU";
        let r = parse_loaded_models(output);
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].status, "PROCESSING");
        assert_eq!(r[0].device, "GPU");
    }

    #[test]
    fn loaded_models_keyword_fallback_loading() {
        let output = "mistral-7b mistral:7 LOADING 4.0 GB 32768 CPU";
        let r = parse_loaded_models(output);
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].status, "LOADING");
        assert_eq!(r[0].device, "CPU");
    }

    #[test]
    fn loaded_models_keyword_fallback_skips_no_models_line() {
        let output = "No models are currently loaded.\nfoo bar IDLE 1.0 GB 4096 GPU";
        let r = parse_loaded_models(output);
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].identifier, "foo");
    }

    #[test]
    fn loaded_models_keyword_fallback_skips_single_token_lines() {
        let output = "lonely\nfoo bar IDLE 1.0 GB 4096 GPU";
        let r = parse_loaded_models(output);
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].identifier, "foo");
    }

    #[test]
    fn loaded_models_keyword_fallback_size_mb() {
        let output = "small model:1 IDLE 512 MB 2048 GPU";
        let r = parse_loaded_models(output);
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].size, "512 MB");
    }

    #[test]
    fn loaded_models_keyword_fallback_context_requires_3_digits() {
        let output = "m model:1 IDLE 1.0 GB 4096 GPU";
        let r = parse_loaded_models(output);
        assert_eq!(r[0].context, "4096");
    }

    #[test]
    fn loaded_models_keyword_fallback_short_number_not_context() {
        let output = "m model:1 IDLE 1.0 GB 42 GPU";
        let r = parse_loaded_models(output);
        assert_eq!(r[0].context, "");
    }

    #[test]
    fn loaded_models_keyword_fallback_no_matching_fields() {
        let output = "foo bar baz qux";
        let r = parse_loaded_models(output);
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].identifier, "foo");
        assert_eq!(r[0].model, "bar");
        assert_eq!(r[0].status, "");
        assert_eq!(r[0].size, "");
        assert_eq!(r[0].device, "");
        assert_eq!(r[0].context, "");
    }

    // ===================== parse_runtimes =====================

    #[test]
    fn runtimes_selected_runtime() {
        // CHARACTERIZATION: parse_runtimes uses parts.last() as format.
        // For "llama.cpp  ✓  gguf  b1234", parts = ["llama.cpp", "✓", "gguf", "b1234"].
        // format = parts.last() = "b1234". Version filter excludes "✓" and format
        // ("b1234"), then "gguf" has no digit → version = "". This is current behavior —
        // the parser assumes version comes before format in the token order.
        let output = "LLM ENGINE        SELECTED    MODEL FORMAT    VERSION\nllama.cpp         \u{2713}           gguf            b1234\n";
        let r = parse_runtimes(output);
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].engine, "llama.cpp");
        assert!(r[0].selected);
        assert_eq!(r[0].format, "b1234"); // parts.last() = "b1234"
        assert_eq!(r[0].version, ""); // "gguf" has no digit, "b1234" excluded as format
    }

    #[test]
    fn runtimes_unselected_runtime() {
        let output = "llama.cpp     gguf   b1234";
        let r = parse_runtimes(output);
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].engine, "llama.cpp");
        assert!(!r[0].selected);
    }

    #[test]
    fn runtimes_mlx_engine() {
        let output = "mlx     \u{2713}     mlx   v0.1";
        let r = parse_runtimes(output);
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].engine, "mlx");
        assert!(r[0].selected);
        assert_eq!(r[0].format, "v0.1");
        assert_eq!(r[0].version, "");
    }

    #[test]
    fn runtimes_skips_header_lines() {
        let output = "LLM ENGINE   SELECTED   MODEL FORMAT   VERSION\nllama.cpp    \u{2713}          gguf           b1\n";
        let r = parse_runtimes(output);
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].engine, "llama.cpp");
    }

    #[test]
    fn runtimes_skips_non_engine_lines() {
        let output = "foo bar baz v1\nllama.cpp \u{2713} gguf b2";
        let r = parse_runtimes(output);
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].engine, "llama.cpp");
    }

    #[test]
    fn runtimes_skips_empty_lines() {
        let output = "\n\nllama.cpp \u{2713} gguf b1\n";
        let r = parse_runtimes(output);
        assert_eq!(r.len(), 1);
    }

    #[test]
    fn runtimes_version_is_first_digit_token_not_engine_or_format() {
        let output = "llama.cpp \u{2713} gguf b9999 extra";
        let r = parse_runtimes(output);
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].format, "extra");
        assert_eq!(r[0].version, "b9999");
    }

    #[test]
    fn runtimes_cpp_substring_matches() {
        let output = "my-cpp-engine gguf v9";
        let r = parse_runtimes(output);
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].engine, "my-cpp-engine");
    }

    // ===================== HfModel helpers =====================

    fn hf(id: &str) -> HfModel {
        HfModel {
            model_id: id.to_string(),
            downloads: 0,
            likes: 0,
            tags: vec![],
            pipeline_tag: None,
            gated: false,
            last_modified: None,
            author: None,
            siblings: vec![],
        }
    }

    fn hf_with_siblings(names: &[&str]) -> HfModel {
        HfModel {
            model_id: "test/model".to_string(),
            downloads: 0,
            likes: 0,
            tags: vec![],
            pipeline_tag: None,
            gated: false,
            last_modified: None,
            author: None,
            siblings: names
                .iter()
                .map(|n| HfSibling {
                    rfilename: n.to_string(),
                })
                .collect(),
        }
    }

    fn hf_with_tags(tags: &[&str]) -> HfModel {
        HfModel {
            model_id: "test/model".to_string(),
            downloads: 0,
            likes: 0,
            tags: tags.iter().map(|t| t.to_string()).collect(),
            pipeline_tag: None,
            gated: false,
            last_modified: None,
            author: None,
            siblings: vec![],
        }
    }

    fn hf_with_quant(id: &str, quants: &[&str]) -> HfModel {
        HfModel {
            model_id: id.to_string(),
            downloads: 0,
            likes: 0,
            tags: vec![],
            pipeline_tag: None,
            gated: false,
            last_modified: None,
            author: None,
            siblings: quants
                .iter()
                .map(|q| HfSibling {
                    rfilename: format!("model-{}.gguf", q),
                })
                .collect(),
        }
    }

    // ===================== HfModel::param_count =====================

    #[test]
    fn param_count_llama_decimal_b() {
        // CHARACTERIZATION: "Llama-3.2-3B" — the regex (\d+\.?\d*)[bB] matches
        // the first occurrence "3B" (from "3.2-3B"), not "3.2B". Returns 3.0.
        let m = hf("Llama-3.2-3B");
        assert_eq!(m.param_count(), 3.0);
    }

    #[test]
    fn param_count_qwen_integer_b() {
        let m = hf("Qwen-7B");
        assert_eq!(m.param_count(), 7.0);
    }

    #[test]
    fn param_count_mixtral_moe_known_bug() {
        // KNOWN BUG: first regex matches "4B" in "12x4B" before the MoE regex runs.
        // The MoE branch is unreachable whenever the model id contains any "NB" pattern.
        // Assert the BUGGY value (4.0), NOT the correct 48.0. Do NOT fix.
        let m = hf("Mixtral-12x4B");
        assert_eq!(m.param_count(), 4.0);
    }

    #[test]
    fn param_count_no_b_suffix_returns_zero() {
        let m = hf("model-no-b");
        assert_eq!(m.param_count(), 0.0);
    }

    #[test]
    fn param_count_lowercase_b_matches() {
        let m = hf("model-13b");
        assert_eq!(m.param_count(), 13.0);
    }

    #[test]
    fn param_count_takes_first_match() {
        let m = hf("model-7B-13B");
        assert_eq!(m.param_count(), 7.0);
    }

    // ===================== HfModel::quantizations =====================

    #[test]
    fn quantizations_extracts_and_sorts() {
        let m = hf_with_siblings(&["model-Q4_K_M.gguf", "model-Q8_0.gguf"]);
        let q = m.quantizations();
        assert_eq!(q, vec!["Q4_K_M".to_string(), "Q8_0".to_string()]);
    }

    #[test]
    fn quantizations_deduplicates() {
        let m = hf_with_siblings(&["model-Q4_K_M.gguf", "other-Q4_K_M.gguf"]);
        let q = m.quantizations();
        assert_eq!(q, vec!["Q4_K_M".to_string()]);
    }

    #[test]
    fn quantizations_uppercases() {
        let m = hf_with_siblings(&["model-q4_k_m.gguf"]);
        let q = m.quantizations();
        assert_eq!(q, vec!["Q4_K_M".to_string()]);
    }

    #[test]
    fn quantizations_skips_non_gguf_files() {
        let m = hf_with_siblings(&["model-Q4_K_M.gguf", "config.json", "tokenizer.json"]);
        let q = m.quantizations();
        assert_eq!(q, vec!["Q4_K_M".to_string()]);
    }

    #[test]
    fn quantizations_empty_when_no_gguf() {
        let m = hf_with_siblings(&["config.json"]);
        let q = m.quantizations();
        assert!(q.is_empty());
    }

    #[test]
    fn quantizations_matches_f16() {
        let m = hf_with_siblings(&["model-F16.gguf"]);
        let q = m.quantizations();
        assert_eq!(q, vec!["F16".to_string()]);
    }

    // ===================== HfModel::license =====================

    #[test]
    fn license_extracts_from_tag() {
        let m = hf_with_tags(&["license:apache-2.0", "en"]);
        assert_eq!(m.license(), Some("apache-2.0".to_string()));
    }

    #[test]
    fn license_returns_none_when_absent() {
        let m = hf_with_tags(&["en", "fr"]);
        assert_eq!(m.license(), None);
    }

    #[test]
    fn license_returns_first_license_tag() {
        let m = hf_with_tags(&["license:mit", "license:apache-2.0"]);
        assert_eq!(m.license(), Some("mit".to_string()));
    }

    // ===================== HfModel::languages =====================

    #[test]
    fn languages_extracts_two_letter_lowercase() {
        // Any 2-letter all-lowercase token counts as a language, including "xx".
        let m = hf_with_tags(&["en", "fr", "license:mit", "xx"]);
        assert_eq!(
            m.languages(),
            vec!["en".to_string(), "fr".to_string(), "xx".to_string()]
        );
    }

    #[test]
    fn languages_excludes_uppercase_two_letter() {
        let m = hf_with_tags(&["EN", "en"]);
        assert_eq!(m.languages(), vec!["en".to_string()]);
    }

    #[test]
    fn languages_excludes_three_letter_codes() {
        let m = hf_with_tags(&["en", "eng", "fra"]);
        assert_eq!(m.languages(), vec!["en".to_string()]);
    }

    #[test]
    fn languages_empty_when_none_match() {
        let m = hf_with_tags(&["license:mit", "text-generation"]);
        assert!(m.languages().is_empty());
    }

    // ===================== HfModel::base_model =====================

    #[test]
    fn base_model_extracts_quantized_derivative() {
        let m = hf_with_tags(&["base_model:quantized:meta/llama"]);
        assert_eq!(m.base_model(), Some("meta/llama".to_string()));
    }

    #[test]
    fn base_model_returns_none_when_absent() {
        let m = hf_with_tags(&["en", "license:mit"]);
        assert_eq!(m.base_model(), None);
    }

    #[test]
    fn base_model_requires_quantized_prefix() {
        let m = hf_with_tags(&["base_model:adapter:meta/llama"]);
        assert_eq!(m.base_model(), None);
    }

    // ===================== HfModel::recommended_quant =====================

    #[test]
    fn recommended_quant_vram_zero_returns_none() {
        let m = hf_with_quant("Llama-7B", &["Q4_K_M", "Q8_0"]);
        assert_eq!(m.recommended_quant(&0), None);
    }

    #[test]
    fn recommended_quant_params_zero_returns_none() {
        let m = hf_with_quant("model-no-b", &["Q4_K_M", "Q8_0"]);
        assert_eq!(m.recommended_quant(&100), None);
    }

    #[test]
    fn recommended_quant_large_vram_returns_q8_0_if_available() {
        let m = hf_with_quant("Llama-7B", &["Q4_K_M", "Q8_0"]);
        assert_eq!(m.recommended_quant(&100), Some("Q8_0".to_string()));
    }

    #[test]
    fn recommended_quant_small_vram_falls_to_smallest() {
        // CHARACTERIZATION: 70B model, 1GB VRAM. The VRAM formula is
        // (params * bytes_per_param) / 1_000_000_000.0. Since params is in
        // billions (70.0) and the formula divides by 1e9 (not 1e18), the
        // "needed_gb" is tiny (e.g. 70 * 1.06 / 1e9 = 0.0000000742 GB), so
        // Q8_0 always "fits" and is returned. This is a known scaling bug —
        // the formula double-divides by billions. Assert current behavior.
        let m = hf_with_quant("Llama-70B", &["Q8_0", "Q4_K_M"]);
        let r = m.recommended_quant(&1);
        assert!(r.is_some());
        // Q8_0 is first in preference order and "fits" due to the scaling bug
        assert_eq!(r, Some("Q8_0".to_string()));
    }

    // ===================== HfModel::quant_desc =====================

    #[test]
    fn quant_desc_q4_k_m_nonempty() {
        let m = hf("test");
        let d = m.quant_desc("Q4_K_M");
        assert!(!d.is_empty());
    }

    #[test]
    fn quant_desc_q8_0_nonempty() {
        let m = hf("test");
        let d = m.quant_desc("Q8_0");
        assert!(!d.is_empty());
    }

    #[test]
    fn quant_desc_unknown_returns_default() {
        let m = hf("test");
        assert_eq!(m.quant_desc("ZZZZ"), "Unknown quantization level");
    }

    #[test]
    fn quant_desc_case_insensitive() {
        let m = hf("test");
        let d_lower = m.quant_desc("q4_k_m");
        let d_upper = m.quant_desc("Q4_K_M");
        assert_eq!(d_lower, d_upper);
        assert!(!d_lower.is_empty());
    }

    // ===================== Performance benchmarks =====================
    // Run with: cargo test --release bench_ -- --nocapture
    // Methodology: 1000 iterations, discard first 10% as warmup, report p50/p95/p99.

    fn bench_percentile(samples: &mut [std::time::Duration], pct: f64) -> std::time::Duration {
        if samples.is_empty() {
            return std::time::Duration::ZERO;
        }
        samples.sort();
        let idx = ((samples.len() as f64 - 1.0) * pct).round() as usize;
        samples[idx]
    }

    fn bench_report(label: &str, samples: &mut [std::time::Duration]) {
        let p50 = bench_percentile(samples, 0.50);
        let p95 = bench_percentile(samples, 0.95);
        let p99 = bench_percentile(samples, 0.99);
        let mean_ns = samples.iter().map(|d| d.as_nanos()).sum::<u128>() / samples.len() as u128;
        eprintln!(
            "BENCH\t{}\tp50={}\tp95={}\tp99={}\tmean={}",
            label,
            p50.as_nanos(),
            p95.as_nanos(),
            p99.as_nanos(),
            mean_ns
        );
    }

    fn bench_with_n_quants(id: &str, quants: &[&str]) -> HfModel {
        HfModel {
            model_id: id.to_string(),
            downloads: 0,
            likes: 0,
            tags: vec![],
            pipeline_tag: None,
            gated: false,
            last_modified: None,
            author: None,
            siblings: quants
                .iter()
                .map(|q| HfSibling {
                    rfilename: format!("model-{}.gguf", q),
                })
                .collect(),
        }
    }

    #[test]
    fn bench_quantizations_10_siblings() {
        let quants = [
            "F16", "Q8_0", "Q6_K", "Q5_K_M", "Q5_K_S", "Q4_K_M", "Q4_K_S", "Q3_K_M", "Q3_K_L",
            "Q2_K",
        ];
        let m = bench_with_n_quants("meta-llama/Llama-3.2-7B", &quants);
        let iterations = 1000;
        let warmup = iterations / 10;
        for _ in 0..warmup {
            let _ = m.quantizations();
        }
        let mut samples: Vec<std::time::Duration> = Vec::with_capacity(iterations);
        for _ in 0..iterations {
            let start = std::time::Instant::now();
            let _ = m.quantizations();
            samples.push(start.elapsed());
        }
        bench_report("quantizations_10_siblings", &mut samples);
    }

    #[test]
    fn bench_param_count_20_models() {
        let ids = [
            "meta-llama/Llama-3.2-3B",
            "meta-llama/Llama-3.2-7B",
            "meta-llama/Llama-3.2-70B",
            "Qwen/Qwen2.5-7B",
            "Qwen/Qwen2.5-14B",
            "Qwen/Qwen2.5-32B",
            "Qwen/Qwen2.5-72B",
            "mistralai/Mistral-7B",
            "mistralai/Mixtral-8x7B",
            "mistralai/Mistral-Nemo-12B",
            "google/gemma-2-2b",
            "google/gemma-2-9b",
            "google/gemma-2-27b",
            "microsoft/Phi-3.5-mini-3.8B",
            "microsoft/Phi-3.5-MoE-42B",
            "deepseek-ai/deepseek-coder-6.7B",
            "deepseek-ai/deepseek-coder-33B",
            "tiiuae/falcon-7B",
            "tiiuae/falcon-40B",
            "HuggingFaceTB/SmolLM2-1.7B",
        ];
        let models: Vec<HfModel> = ids
            .iter()
            .map(|id| HfModel {
                model_id: id.to_string(),
                downloads: 0,
                likes: 0,
                tags: vec![],
                pipeline_tag: None,
                gated: false,
                last_modified: None,
                author: None,
                siblings: vec![],
            })
            .collect();
        let iterations = 1000;
        let warmup = iterations / 10;
        for _ in 0..warmup {
            for m in &models {
                let _ = m.param_count();
            }
        }
        let mut samples: Vec<std::time::Duration> = Vec::with_capacity(iterations);
        for _ in 0..iterations {
            let start = std::time::Instant::now();
            for m in &models {
                let _ = m.param_count();
            }
            samples.push(start.elapsed());
        }
        bench_report("param_count_20_models", &mut samples);
    }

    #[test]
    fn bench_search_page_20_models_full() {
        let quants = [
            "F16", "Q8_0", "Q6_K", "Q5_K_M", "Q5_K_S", "Q4_K_M", "Q4_K_S", "Q3_K_M", "Q3_K_L",
            "Q2_K",
        ];
        let ids = [
            "meta-llama/Llama-3.2-3B",
            "meta-llama/Llama-3.2-7B",
            "meta-llama/Llama-3.2-70B",
            "Qwen/Qwen2.5-7B",
            "Qwen/Qwen2.5-14B",
            "Qwen/Qwen2.5-32B",
            "Qwen/Qwen2.5-72B",
            "mistralai/Mistral-7B",
            "mistralai/Mixtral-8x7B",
            "mistralai/Mistral-Nemo-12B",
            "google/gemma-2-2b",
            "google/gemma-2-9b",
            "google/gemma-2-27b",
            "microsoft/Phi-3.5-mini-3.8B",
            "microsoft/Phi-3.5-MoE-42B",
            "deepseek-ai/deepseek-coder-6.7B",
            "deepseek-ai/deepseek-coder-33B",
            "tiiuae/falcon-7B",
            "tiiuae/falcon-40B",
            "HuggingFaceTB/SmolLM2-1.7B",
        ];
        let models: Vec<HfModel> = ids
            .iter()
            .map(|id| bench_with_n_quants(id, &quants))
            .collect();
        let iterations = 1000;
        let warmup = iterations / 10;
        for _ in 0..warmup {
            for m in &models {
                let _ = m.quantizations();
                let _ = m.param_count();
            }
        }
        let mut samples: Vec<std::time::Duration> = Vec::with_capacity(iterations);
        for _ in 0..iterations {
            let start = std::time::Instant::now();
            for m in &models {
                let _ = m.quantizations();
                let _ = m.param_count();
            }
            samples.push(start.elapsed());
        }
        bench_report("search_page_20_models_full", &mut samples);
    }
}
