//! 本地用量统计：解析上游响应里的 token 用量，按模型计费并按日聚合。
//!
//! 本地拦截服务（`proxy.rs`）转发上游响应时旁路调用本模块，
//! 区分输入 / 输出 / 创建缓存 / 读取缓存四类 token，计算费用，
//! 并按本地日期写入持久化存储，供前端展示用量与活跃日历。

use serde::{Deserialize, Serialize};

/// 上游厂商，由本地代理的 tool_id 映射而来。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Provider {
    OpenAI,
    Anthropic,
    Gemini,
}

impl Provider {
    /// 由本地代理路由前缀（codex/claude/gemini）推断厂商。
    pub fn from_tool_id(tool_id: &str) -> Option<Self> {
        match tool_id {
            "codex" => Some(Provider::OpenAI),
            "claude" => Some(Provider::Anthropic),
            "gemini" => Some(Provider::Gemini),
            _ => None,
        }
    }
}

/// 一次请求的 token 用量，区分四类。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TokenUsage {
    /// 普通输入 token（不含命中缓存的部分）。
    pub input_tokens: u64,
    /// 输出 token。
    pub output_tokens: u64,
    /// 创建/写入缓存的 token（目前仅 Anthropic 原生区分）。
    pub cache_creation_tokens: u64,
    /// 读取/命中缓存的 token。
    pub cache_read_tokens: u64,
}

impl TokenUsage {
    pub fn is_empty(&self) -> bool {
        self.input_tokens == 0
            && self.output_tokens == 0
            && self.cache_creation_tokens == 0
            && self.cache_read_tokens == 0
    }
}

/// 单位百万 token 的美元单价。未知模型价格全为 0（仍记录 token）。
#[derive(Debug, Clone, Copy)]
struct ModelPrice {
    input: f64,
    output: f64,
    cache_write: f64,
    cache_read: f64,
}

impl ModelPrice {
    const ZERO: ModelPrice = ModelPrice {
        input: 0.0,
        output: 0.0,
        cache_write: 0.0,
        cache_read: 0.0,
    };

    /// 按四类 token 计算费用（美元）。
    fn cost(&self, usage: &TokenUsage) -> f64 {
        let per = |tokens: u64, price: f64| tokens as f64 / 1_000_000.0 * price;
        per(usage.input_tokens, self.input)
            + per(usage.output_tokens, self.output)
            + per(usage.cache_creation_tokens, self.cache_write)
            + per(usage.cache_read_tokens, self.cache_read)
    }
}

/// 模型单价表（USD / 1M tokens）。匹配规则：模型名包含某个前缀关键字即命中，
/// 取最长匹配。价格随厂商调整，可在此集中维护。
const PRICE_TABLE: &[(&str, ModelPrice)] = &[
    // ---- Anthropic Claude ----
    (
        "claude-3-5-haiku",
        ModelPrice { input: 0.80, output: 4.0, cache_write: 1.0, cache_read: 0.08 },
    ),
    (
        "claude-3-haiku",
        ModelPrice { input: 0.25, output: 1.25, cache_write: 0.30, cache_read: 0.03 },
    ),
    (
        "claude-3-opus",
        ModelPrice { input: 15.0, output: 75.0, cache_write: 18.75, cache_read: 1.50 },
    ),
    (
        "claude-opus-4",
        ModelPrice { input: 15.0, output: 75.0, cache_write: 18.75, cache_read: 1.50 },
    ),
    (
        "claude-sonnet-4",
        ModelPrice { input: 3.0, output: 15.0, cache_write: 3.75, cache_read: 0.30 },
    ),
    (
        "claude-3-7-sonnet",
        ModelPrice { input: 3.0, output: 15.0, cache_write: 3.75, cache_read: 0.30 },
    ),
    (
        "claude-3-5-sonnet",
        ModelPrice { input: 3.0, output: 15.0, cache_write: 3.75, cache_read: 0.30 },
    ),
    // ---- OpenAI ----
    (
        "gpt-4o-mini",
        ModelPrice { input: 0.15, output: 0.60, cache_write: 0.0, cache_read: 0.075 },
    ),
    (
        "gpt-4o",
        ModelPrice { input: 2.50, output: 10.0, cache_write: 0.0, cache_read: 1.25 },
    ),
    (
        "gpt-4.1-mini",
        ModelPrice { input: 0.40, output: 1.60, cache_write: 0.0, cache_read: 0.10 },
    ),
    (
        "gpt-4.1-nano",
        ModelPrice { input: 0.10, output: 0.40, cache_write: 0.0, cache_read: 0.025 },
    ),
    (
        "gpt-4.1",
        ModelPrice { input: 2.0, output: 8.0, cache_write: 0.0, cache_read: 0.50 },
    ),
    (
        "gpt-5-mini",
        ModelPrice { input: 0.25, output: 2.0, cache_write: 0.0, cache_read: 0.025 },
    ),
    (
        "gpt-5",
        ModelPrice { input: 1.25, output: 10.0, cache_write: 0.0, cache_read: 0.125 },
    ),
    (
        "o4-mini",
        ModelPrice { input: 1.10, output: 4.40, cache_write: 0.0, cache_read: 0.275 },
    ),
    (
        "o3-mini",
        ModelPrice { input: 1.10, output: 4.40, cache_write: 0.0, cache_read: 0.55 },
    ),
    (
        "o3",
        ModelPrice { input: 2.0, output: 8.0, cache_write: 0.0, cache_read: 0.50 },
    ),
    // ---- Google Gemini ----
    (
        "gemini-2.5-pro",
        ModelPrice { input: 1.25, output: 10.0, cache_write: 0.0, cache_read: 0.31 },
    ),
    (
        "gemini-2.5-flash",
        ModelPrice { input: 0.30, output: 2.50, cache_write: 0.0, cache_read: 0.075 },
    ),
    (
        "gemini-2.0-flash",
        ModelPrice { input: 0.10, output: 0.40, cache_write: 0.0, cache_read: 0.025 },
    ),
    (
        "gemini-1.5-pro",
        ModelPrice { input: 1.25, output: 5.0, cache_write: 0.0, cache_read: 0.3125 },
    ),
    (
        "gemini-1.5-flash",
        ModelPrice { input: 0.075, output: 0.30, cache_write: 0.0, cache_read: 0.01875 },
    ),
];

fn price_for(model: &str) -> ModelPrice {
    let lower = model.to_ascii_lowercase();
    PRICE_TABLE
        .iter()
        .filter(|(key, _)| lower.contains(key))
        .max_by_key(|(key, _)| key.len())
        .map(|(_, price)| *price)
        .unwrap_or(ModelPrice::ZERO)
}

/// 根据模型名计算一次用量的费用（美元）。未知模型返回 0。
pub fn cost_for(model: &str, usage: &TokenUsage) -> f64 {
    price_for(model).cost(usage)
}

/// 从请求体（及 Gemini 的 URL 路径）中解析模型名。
pub fn extract_model(provider: Provider, request_path: &str, request_body: &[u8]) -> String {
    if let Ok(value) = serde_json::from_slice::<serde_json::Value>(request_body) {
        if let Some(model) = value.get("model").and_then(|m| m.as_str()) {
            if !model.is_empty() {
                return model.to_string();
            }
        }
    }
    // Gemini 把模型放在 URL：.../models/{model}:generateContent
    if provider == Provider::Gemini {
        if let Some(model) = gemini_model_from_path(request_path) {
            return model;
        }
    }
    "unknown".to_string()
}

fn gemini_model_from_path(path: &str) -> Option<String> {
    let after = path.rsplit("models/").next()?;
    if after == path {
        return None;
    }
    let model = after.split([':', '/', '?']).next()?.trim();
    if model.is_empty() {
        None
    } else {
        Some(model.to_string())
    }
}

/// 判断上游响应是否表示请求成功（仅 HTTP 2xx 不够：流式接口常在错误时仍返回 200）。
pub fn is_successful_response(provider: Provider, body: &[u8], streaming: bool) -> bool {
    if streaming {
        is_successful_streaming(body)
    } else {
        is_successful_non_streaming(provider, body)
    }
}

fn is_successful_non_streaming(provider: Provider, body: &[u8]) -> bool {
    let Ok(value) = serde_json::from_slice::<serde_json::Value>(body) else {
        return false;
    };
    if value.get("error").is_some() {
        return false;
    }
    match provider {
        Provider::OpenAI => is_successful_openai_non_streaming(&value),
        Provider::Anthropic => value.get("type").and_then(|t| t.as_str()) == Some("message"),
        Provider::Gemini => value.get("error").is_none() && value.get("candidates").is_some(),
    }
}

fn is_successful_openai_non_streaming(value: &serde_json::Value) -> bool {
    if let Some(status) = value.get("status").and_then(|s| s.as_str()) {
        if status == "failed" || status == "incomplete" {
            return false;
        }
    }
    value.get("choices").is_some()
        || value.get("output").is_some()
        || value.get("status").and_then(|s| s.as_str()) == Some("completed")
}

fn is_successful_streaming(body: &[u8]) -> bool {
    let text = String::from_utf8_lossy(body);
    for line in text.lines() {
        if line.trim() == "event: error" {
            return false;
        }
    }
    for payload in iter_json_payloads(&text) {
        let Ok(value) = serde_json::from_str::<serde_json::Value>(&payload) else {
            continue;
        };
        if value.get("error").is_some() {
            return false;
        }
        if let Some(kind) = value.get("type").and_then(|t| t.as_str()) {
            if kind == "error" || kind == "response.failed" {
                return false;
            }
        }
        if let Some(status) = value
            .get("response")
            .and_then(|r| r.get("status"))
            .and_then(|s| s.as_str())
        {
            if status == "failed" || status == "incomplete" {
                return false;
            }
        }
    }
    true
}

/// 解析非流式响应体中的 token 用量。
pub fn parse_usage(provider: Provider, body: &[u8]) -> Option<TokenUsage> {
    let value: serde_json::Value = serde_json::from_slice(body).ok()?;
    let usage = match provider {
        Provider::OpenAI => parse_openai_usage(&value),
        Provider::Anthropic => parse_anthropic_usage(&value),
        Provider::Gemini => parse_gemini_usage(&value),
    }?;
    if usage.is_empty() {
        None
    } else {
        Some(usage)
    }
}

/// 解析流式响应（SSE / NDJSON）累积出的 token 用量。
pub fn parse_streaming_usage(provider: Provider, body: &[u8]) -> Option<TokenUsage> {
    let text = String::from_utf8_lossy(body);
    let mut acc = TokenUsage::default();
    let mut found = false;

    for payload in iter_json_payloads(&text) {
        let Ok(value) = serde_json::from_str::<serde_json::Value>(&payload) else {
            continue;
        };
        let parsed = match provider {
            Provider::OpenAI => parse_openai_usage(&value),
            Provider::Anthropic => parse_anthropic_usage(&value),
            Provider::Gemini => parse_gemini_usage(&value),
        };
        let Some(usage) = parsed else {
            continue;
        };
        if usage.is_empty() {
            continue;
        }
        found = true;
        merge_streaming(provider, &mut acc, &usage);
    }

    if found && !acc.is_empty() {
        Some(acc)
    } else {
        None
    }
}

/// 流式分片用量合并策略：
/// - Anthropic 的 input/cache 来自 message_start，output 来自后续 message_delta（取最大）。
/// - OpenAI / Gemini 末尾分片即为累计值，直接取最大。
fn merge_streaming(provider: Provider, acc: &mut TokenUsage, next: &TokenUsage) {
    match provider {
        Provider::Anthropic => {
            if next.input_tokens > 0 {
                acc.input_tokens = next.input_tokens;
            }
            if next.cache_creation_tokens > 0 {
                acc.cache_creation_tokens = next.cache_creation_tokens;
            }
            if next.cache_read_tokens > 0 {
                acc.cache_read_tokens = next.cache_read_tokens;
            }
            acc.output_tokens = acc.output_tokens.max(next.output_tokens);
        }
        _ => {
            // OpenAI / Gemini 的末尾分片携带累计值，后到的覆盖先到的。
            *acc = *next;
        }
    }
}

/// 从 SSE/NDJSON 文本里提取候选 JSON 串：`data:` 行优先，其余按行兜底。
fn iter_json_payloads(text: &str) -> Vec<String> {
    let mut payloads = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let candidate = if let Some(rest) = line.strip_prefix("data:") {
            rest.trim()
        } else if line.starts_with('{') || line.starts_with('[') {
            line
        } else {
            continue;
        };
        if candidate == "[DONE]" || candidate.is_empty() {
            continue;
        }
        payloads.push(candidate.to_string());
    }
    payloads
}

fn as_u64(value: Option<&serde_json::Value>) -> u64 {
    value.and_then(|v| v.as_u64()).unwrap_or(0)
}

/// OpenAI：兼容 Responses API（input_tokens / output_tokens）与 Chat Completions
/// （prompt_tokens / completion_tokens）。流式 `response.completed` 事件包在 `response` 下。
fn parse_openai_usage(value: &serde_json::Value) -> Option<TokenUsage> {
    let usage = value
        .get("usage")
        .or_else(|| value.get("response").and_then(|r| r.get("usage")))?;

    let input = if usage.get("input_tokens").is_some() {
        as_u64(usage.get("input_tokens"))
    } else {
        as_u64(usage.get("prompt_tokens"))
    };
    let output = if usage.get("output_tokens").is_some() {
        as_u64(usage.get("output_tokens"))
    } else {
        as_u64(usage.get("completion_tokens"))
    };

    let cache_read = usage
        .get("input_tokens_details")
        .or_else(|| usage.get("prompt_tokens_details"))
        .map(|d| as_u64(d.get("cached_tokens")))
        .unwrap_or(0);

    // OpenAI 的 input/prompt 通常已包含缓存命中部分，单独拆出缓存读取以便区分。
    let input = input.saturating_sub(cache_read);

    Some(TokenUsage {
        input_tokens: input,
        output_tokens: output,
        cache_creation_tokens: 0,
        cache_read_tokens: cache_read,
    })
}

/// Anthropic：usage 可能位于顶层（非流式 / message_delta）或 message 下（message_start）。
fn parse_anthropic_usage(value: &serde_json::Value) -> Option<TokenUsage> {
    let usage = value
        .get("usage")
        .or_else(|| value.get("message").and_then(|m| m.get("usage")))?;

    Some(TokenUsage {
        input_tokens: as_u64(usage.get("input_tokens")),
        output_tokens: as_u64(usage.get("output_tokens")),
        cache_creation_tokens: as_u64(usage.get("cache_creation_input_tokens")),
        cache_read_tokens: as_u64(usage.get("cache_read_input_tokens")),
    })
}

/// Gemini：usageMetadata.promptTokenCount / candidatesTokenCount / cachedContentTokenCount。
fn parse_gemini_usage(value: &serde_json::Value) -> Option<TokenUsage> {
    let usage = value.get("usageMetadata")?;
    let prompt = as_u64(usage.get("promptTokenCount"));
    let cache_read = as_u64(usage.get("cachedContentTokenCount"));

    Some(TokenUsage {
        input_tokens: prompt.saturating_sub(cache_read),
        output_tokens: as_u64(usage.get("candidatesTokenCount")),
        cache_creation_tokens: 0,
        cache_read_tokens: cache_read,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn openai_non_stream() {
        let body = br#"{"usage":{"input_tokens":120,"output_tokens":30,"input_tokens_details":{"cached_tokens":20}}}"#;
        let usage = parse_usage(Provider::OpenAI, body).unwrap();
        assert_eq!(usage.input_tokens, 100);
        assert_eq!(usage.output_tokens, 30);
        assert_eq!(usage.cache_read_tokens, 20);
    }

    #[test]
    fn openai_chat_completions() {
        let body = br#"{"usage":{"prompt_tokens":50,"completion_tokens":10,"prompt_tokens_details":{"cached_tokens":40}}}"#;
        let usage = parse_usage(Provider::OpenAI, body).unwrap();
        assert_eq!(usage.input_tokens, 10);
        assert_eq!(usage.cache_read_tokens, 40);
        assert_eq!(usage.output_tokens, 10);
    }

    #[test]
    fn anthropic_streaming_merges_events() {
        let body = concat!(
            "event: message_start\n",
            "data: {\"type\":\"message_start\",\"message\":{\"usage\":{\"input_tokens\":200,\"cache_creation_input_tokens\":15,\"cache_read_input_tokens\":80,\"output_tokens\":1}}}\n\n",
            "event: message_delta\n",
            "data: {\"type\":\"message_delta\",\"usage\":{\"output_tokens\":42}}\n\n",
        );
        let usage = parse_streaming_usage(Provider::Anthropic, body.as_bytes()).unwrap();
        assert_eq!(usage.input_tokens, 200);
        assert_eq!(usage.cache_creation_tokens, 15);
        assert_eq!(usage.cache_read_tokens, 80);
        assert_eq!(usage.output_tokens, 42);
    }

    #[test]
    fn gemini_streaming_takes_last() {
        let body = concat!(
            "data: {\"usageMetadata\":{\"promptTokenCount\":10,\"candidatesTokenCount\":5}}\n\n",
            "data: {\"usageMetadata\":{\"promptTokenCount\":10,\"candidatesTokenCount\":25,\"cachedContentTokenCount\":4}}\n\n",
        );
        let usage = parse_streaming_usage(Provider::Gemini, body.as_bytes()).unwrap();
        assert_eq!(usage.input_tokens, 6);
        assert_eq!(usage.output_tokens, 25);
        assert_eq!(usage.cache_read_tokens, 4);
    }

    #[test]
    fn gemini_model_from_url() {
        assert_eq!(
            gemini_model_from_path("v1beta/models/gemini-2.5-pro:streamGenerateContent"),
            Some("gemini-2.5-pro".to_string())
        );
    }

    #[test]
    fn cost_uses_longest_match() {
        let usage = TokenUsage {
            input_tokens: 1_000_000,
            output_tokens: 0,
            cache_creation_tokens: 0,
            cache_read_tokens: 0,
        };
        // claude-3-5-sonnet 命中 sonnet 价格而非更短的 claude-3 前缀。
        let cost = cost_for("claude-3-5-sonnet-20241022", &usage);
        assert!((cost - 3.0).abs() < 1e-9);
    }

    #[test]
    fn unknown_model_is_free() {
        let usage = TokenUsage {
            input_tokens: 1_000,
            output_tokens: 1_000,
            cache_creation_tokens: 0,
            cache_read_tokens: 0,
        };
        assert_eq!(cost_for("mystery-model", &usage), 0.0);
    }

    #[test]
    fn openai_error_with_usage_is_not_successful() {
        let body = br#"{"error":{"message":"rate limit"},"usage":{"input_tokens":10,"output_tokens":0}}"#;
        assert!(!is_successful_response(Provider::OpenAI, body, false));
    }

    #[test]
    fn openai_completed_response_is_successful() {
        let body = br#"{"id":"resp_1","status":"completed","output":[],"usage":{"input_tokens":10,"output_tokens":5}}"#;
        assert!(is_successful_response(Provider::OpenAI, body, false));
    }

    #[test]
    fn anthropic_error_type_is_not_successful() {
        let body = br#"{"type":"error","error":{"type":"overloaded_error"},"usage":{"input_tokens":100,"output_tokens":0}}"#;
        assert!(!is_successful_response(Provider::Anthropic, body, false));
    }

    #[test]
    fn anthropic_message_is_successful() {
        let body = br#"{"type":"message","usage":{"input_tokens":100,"output_tokens":20}}"#;
        assert!(is_successful_response(Provider::Anthropic, body, false));
    }

    #[test]
    fn streaming_error_event_is_not_successful() {
        let body = concat!(
            "event: message_start\n",
            "data: {\"type\":\"message_start\",\"message\":{\"usage\":{\"input_tokens\":200,\"output_tokens\":1}}}\n\n",
            "event: error\n",
            "data: {\"type\":\"error\",\"error\":{\"type\":\"overloaded_error\"}}\n\n",
        );
        assert!(!is_successful_response(Provider::Anthropic, body.as_bytes(), true));
    }

    #[test]
    fn streaming_openai_failed_is_not_successful() {
        let body = concat!(
            "data: {\"type\":\"response.created\"}\n\n",
            "data: {\"type\":\"response.failed\",\"response\":{\"status\":\"failed\"}}\n\n",
        );
        assert!(!is_successful_response(Provider::OpenAI, body.as_bytes(), true));
    }

    #[test]
    fn successful_streaming_anthropic_is_recordable() {
        let body = concat!(
            "event: message_start\n",
            "data: {\"type\":\"message_start\",\"message\":{\"usage\":{\"input_tokens\":200,\"output_tokens\":1}}}\n\n",
            "event: message_delta\n",
            "data: {\"type\":\"message_delta\",\"usage\":{\"output_tokens\":42}}\n\n",
            "event: message_stop\n",
            "data: {\"type\":\"message_stop\"}\n\n",
        );
        assert!(is_successful_response(Provider::Anthropic, body.as_bytes(), true));
    }
}
