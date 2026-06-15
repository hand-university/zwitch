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

// ---------------------------------------------------------------------------
// OpenCode 用量兜底：上游 OpenAI 兼容路由对灰度模型只回报 input_tokens，
// output/cache 恒为 0，导致 OpenCode 的「Context」面板显示 0 tokens / 0% / $0。
// 代理在转发给 OpenCode 时，按响应文本估算 output_tokens 并补写进 `usage`，
// 仅在上游缺失（output == 0）时介入，绝不覆盖上游已返回的真实值。
// ---------------------------------------------------------------------------

/// 按文本估算输出 token 数：ASCII 约 4 字符/token，其余（CJK 等）按 1 token/字（偏保守上界）。
pub fn estimate_output_tokens(text: &str) -> u64 {
    let mut ascii = 0u64;
    let mut wide = 0u64;
    for ch in text.chars() {
        if ch.is_ascii() {
            ascii += 1;
        } else {
            wide += 1;
        }
    }
    ascii.div_ceil(4) + wide
}

/// 从单个流式事件里提取增量输出文本（Responses API 的 `response.output_text.delta`
/// 或 Chat Completions 的 `choices[].delta.content`）。仅取可见输出，不含推理摘要。
fn extract_event_output_text(value: &serde_json::Value) -> Option<String> {
    if value.get("type").and_then(|v| v.as_str()) == Some("response.output_text.delta") {
        if let Some(delta) = value.get("delta").and_then(|v| v.as_str()) {
            return Some(delta.to_string());
        }
    }
    if let Some(choices) = value.get("choices").and_then(|v| v.as_array()) {
        let mut acc = String::new();
        for choice in choices {
            if let Some(content) = choice
                .get("delta")
                .and_then(|d| d.get("content"))
                .and_then(|c| c.as_str())
            {
                acc.push_str(content);
            }
        }
        if !acc.is_empty() {
            return Some(acc);
        }
    }
    None
}

/// 从非流式响应体里提取完整输出文本（Responses API 的 `output[].content[].text`
/// 或 Chat Completions 的 `choices[].message.content`）。
fn extract_full_output_text(value: &serde_json::Value) -> String {
    let mut acc = String::new();
    if let Some(output) = value.get("output").and_then(|v| v.as_array()) {
        for item in output {
            if let Some(content) = item.get("content").and_then(|v| v.as_array()) {
                for part in content {
                    if let Some(text) = part.get("text").and_then(|v| v.as_str()) {
                        acc.push_str(text);
                    }
                }
            }
        }
    }
    if acc.is_empty() {
        if let Some(choices) = value.get("choices").and_then(|v| v.as_array()) {
            for choice in choices {
                if let Some(text) = choice
                    .get("message")
                    .and_then(|m| m.get("content"))
                    .and_then(|c| c.as_str())
                {
                    acc.push_str(text);
                }
            }
        }
    }
    acc
}

/// 是否为携带最终用量的终止事件：Chat Completions 末尾分片（顶层 `usage`）或
/// Responses API 的 `response.completed`。中间事件（如 `response.in_progress`）不改写。
fn is_terminal_usage_event(value: &serde_json::Value) -> bool {
    if value.get("usage").map(|u| u.is_object()).unwrap_or(false) {
        return true;
    }
    value.get("type").and_then(|v| v.as_str()) == Some("response.completed")
}

/// 取出可改写的 usage 对象：顶层 `usage` 或 `response.usage`。
fn usage_map_mut(value: &mut serde_json::Value) -> Option<&mut serde_json::Map<String, serde_json::Value>> {
    if value.get("usage").map(|u| u.is_object()).unwrap_or(false) {
        return value.get_mut("usage").and_then(|u| u.as_object_mut());
    }
    if let Some(response) = value.get_mut("response") {
        if response.get("usage").map(|u| u.is_object()).unwrap_or(false) {
            return response.get_mut("usage").and_then(|u| u.as_object_mut());
        }
    }
    None
}

/// 若 usage 的输出 token 缺失或为 0，则写入估算值并同步 `total_tokens`。
/// 返回是否发生改写（上游已有非零输出时不改写）。
fn rewrite_event_usage(value: &mut serde_json::Value, output_estimate: u64) -> bool {
    if output_estimate == 0 {
        return false;
    }
    let Some(usage) = usage_map_mut(value) else {
        return false;
    };
    let out_key = if usage.contains_key("completion_tokens") && !usage.contains_key("output_tokens") {
        "completion_tokens"
    } else {
        "output_tokens"
    };
    let current = usage.get(out_key).and_then(|v| v.as_u64()).unwrap_or(0);
    if current > 0 {
        return false;
    }
    let input = usage
        .get("input_tokens")
        .and_then(|v| v.as_u64())
        .or_else(|| usage.get("prompt_tokens").and_then(|v| v.as_u64()))
        .unwrap_or(0);
    let reasoning = usage
        .get("output_tokens_details")
        .and_then(|d| d.get("reasoning_tokens"))
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    usage.insert(out_key.to_string(), serde_json::Value::from(output_estimate));
    usage.insert(
        "total_tokens".to_string(),
        serde_json::Value::from(input + output_estimate + reasoning),
    );
    true
}

/// 改写非流式 OpenCode 响应：按完整输出文本估算 output_tokens 并补写 usage。
pub fn rewrite_non_streaming_output_usage(body: &[u8]) -> Vec<u8> {
    let Ok(mut value) = serde_json::from_slice::<serde_json::Value>(body) else {
        return body.to_vec();
    };
    let estimate = estimate_output_tokens(&extract_full_output_text(&value));
    if rewrite_event_usage(&mut value, estimate) {
        serde_json::to_vec(&value).unwrap_or_else(|_| body.to_vec())
    } else {
        body.to_vec()
    }
}

/// 流式 SSE 改写器：原样透传各分片（保留流式体验），仅在终止用量事件上
/// 补写估算的 output_tokens。处理跨分片拆行，逐行解析 `data:` 负载。
#[derive(Default)]
pub struct OpencodeUsageRewriter {
    line_buf: Vec<u8>,
    output_text: String,
}

impl OpencodeUsageRewriter {
    pub fn new() -> Self {
        Self::default()
    }

    /// 喂入一段上游分片，返回应转发给 OpenCode 的字节（可能与输入相同）。
    pub fn push(&mut self, chunk: &[u8]) -> Vec<u8> {
        self.line_buf.extend_from_slice(chunk);
        let mut out = Vec::new();
        while let Some(pos) = self.line_buf.iter().position(|&b| b == b'\n') {
            let line: Vec<u8> = self.line_buf.drain(..=pos).collect();
            out.extend_from_slice(&self.process_line(&line));
        }
        out
    }

    /// 流结束时冲洗剩余未带换行的尾行。
    pub fn finish(&mut self) -> Vec<u8> {
        if self.line_buf.is_empty() {
            return Vec::new();
        }
        let line = std::mem::take(&mut self.line_buf);
        self.process_line(&line)
    }

    fn process_line(&mut self, line: &[u8]) -> Vec<u8> {
        let Ok(text) = std::str::from_utf8(line) else {
            return line.to_vec();
        };
        let trimmed = text.trim_end_matches('\n').trim_end_matches('\r');
        let Some(payload) = trimmed.strip_prefix("data:").map(|rest| rest.trim()) else {
            return line.to_vec();
        };
        if payload.is_empty() || payload == "[DONE]" {
            return line.to_vec();
        }
        let Ok(mut value) = serde_json::from_str::<serde_json::Value>(payload) else {
            return line.to_vec();
        };

        if let Some(delta) = extract_event_output_text(&value) {
            self.output_text.push_str(&delta);
        }

        if is_terminal_usage_event(&value) {
            let estimate = estimate_output_tokens(&self.output_text);
            if rewrite_event_usage(&mut value, estimate) {
                if let Ok(new_payload) = serde_json::to_string(&value) {
                    let ending = if text.ends_with("\r\n") {
                        "\r\n"
                    } else if text.ends_with('\n') {
                        "\n"
                    } else {
                        ""
                    };
                    return format!("data: {new_payload}{ending}").into_bytes();
                }
            }
        }

        line.to_vec()
    }
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

    #[test]
    fn estimate_output_tokens_blends_ascii_and_wide() {
        assert_eq!(estimate_output_tokens(""), 0);
        assert_eq!(estimate_output_tokens("abcd"), 1); // 4 ascii -> 1
        assert_eq!(estimate_output_tokens("abcde"), 2); // ceil(5/4)
        assert_eq!(estimate_output_tokens("你好"), 2); // 2 wide chars -> 2
    }

    fn run_rewriter(chunks: &[&str]) -> String {
        let mut rw = OpencodeUsageRewriter::new();
        let mut out = Vec::new();
        for chunk in chunks {
            out.extend_from_slice(&rw.push(chunk.as_bytes()));
        }
        out.extend_from_slice(&rw.finish());
        String::from_utf8(out).unwrap()
    }

    fn usage_from_completed_stream(output: &str) -> serde_json::Value {
        for line in output.lines() {
            let Some(payload) = line.strip_prefix("data:").map(|r| r.trim()) else {
                continue;
            };
            let Ok(value) = serde_json::from_str::<serde_json::Value>(payload) else {
                continue;
            };
            if let Some(usage) = value
                .get("usage")
                .or_else(|| value.get("response").and_then(|r| r.get("usage")))
            {
                return usage.clone();
            }
        }
        panic!("no usage event found in:\n{output}");
    }

    #[test]
    fn rewriter_injects_output_for_responses_api() {
        let body = concat!(
            "event: response.output_text.delta\n",
            "data: {\"type\":\"response.output_text.delta\",\"delta\":\"hello \"}\n\n",
            "data: {\"type\":\"response.output_text.delta\",\"delta\":\"world\"}\n\n",
            "event: response.completed\n",
            "data: {\"type\":\"response.completed\",\"response\":{\"usage\":{\"input_tokens\":1200,\"output_tokens\":0,\"total_tokens\":1200}}}\n\n",
        );
        let out = run_rewriter(&[body]);
        let usage = usage_from_completed_stream(&out);
        assert_eq!(usage.get("output_tokens").and_then(|v| v.as_u64()), Some(3)); // "hello world" -> ceil(11/4)=3
        assert_eq!(usage.get("input_tokens").and_then(|v| v.as_u64()), Some(1200));
        assert_eq!(usage.get("total_tokens").and_then(|v| v.as_u64()), Some(1203));
        // 增量分片原样透传
        assert!(out.contains("\"delta\":\"hello \""));
    }

    #[test]
    fn rewriter_injects_output_for_chat_completions() {
        let body = concat!(
            "data: {\"choices\":[{\"delta\":{\"content\":\"abcd\"}}]}\n\n",
            "data: {\"choices\":[{\"delta\":{\"content\":\"efgh\"}}]}\n\n",
            "data: {\"choices\":[],\"usage\":{\"prompt_tokens\":50,\"completion_tokens\":0,\"total_tokens\":50}}\n\n",
            "data: [DONE]\n\n",
        );
        let out = run_rewriter(&[body]);
        let usage = usage_from_completed_stream(&out);
        assert_eq!(usage.get("completion_tokens").and_then(|v| v.as_u64()), Some(2)); // 8 ascii -> 2
        assert_eq!(usage.get("total_tokens").and_then(|v| v.as_u64()), Some(52));
        assert!(out.contains("[DONE]"));
    }

    #[test]
    fn rewriter_handles_split_chunks_identically() {
        let body = concat!(
            "data: {\"type\":\"response.output_text.delta\",\"delta\":\"hello world\"}\n\n",
            "data: {\"type\":\"response.completed\",\"response\":{\"usage\":{\"input_tokens\":10,\"output_tokens\":0}}}\n\n",
        );
        let mid = body.len() / 2;
        let split = run_rewriter(&[&body[..mid], &body[mid..]]);
        let usage = usage_from_completed_stream(&split);
        assert_eq!(usage.get("output_tokens").and_then(|v| v.as_u64()), Some(3));
    }

    #[test]
    fn rewriter_does_not_clobber_real_output() {
        let body = concat!(
            "data: {\"type\":\"response.output_text.delta\",\"delta\":\"hi\"}\n\n",
            "data: {\"type\":\"response.completed\",\"response\":{\"usage\":{\"input_tokens\":10,\"output_tokens\":99,\"total_tokens\":109}}}\n\n",
        );
        let out = run_rewriter(&[body]);
        let usage = usage_from_completed_stream(&out);
        assert_eq!(usage.get("output_tokens").and_then(|v| v.as_u64()), Some(99));
        assert_eq!(usage.get("total_tokens").and_then(|v| v.as_u64()), Some(109));
    }

    #[test]
    fn rewriter_passthrough_when_no_output_text() {
        let body = "data: {\"type\":\"response.completed\",\"response\":{\"usage\":{\"input_tokens\":10,\"output_tokens\":0}}}\n\n";
        let out = run_rewriter(&[body]);
        let usage = usage_from_completed_stream(&out);
        assert_eq!(usage.get("output_tokens").and_then(|v| v.as_u64()), Some(0));
    }

    #[test]
    fn non_streaming_rewrite_responses_api() {
        let body = br#"{"output":[{"content":[{"type":"output_text","text":"hello world"}]}],"usage":{"input_tokens":1200,"output_tokens":0,"total_tokens":1200}}"#;
        let out = rewrite_non_streaming_output_usage(body);
        let value: serde_json::Value = serde_json::from_slice(&out).unwrap();
        assert_eq!(value["usage"]["output_tokens"].as_u64(), Some(3));
        assert_eq!(value["usage"]["total_tokens"].as_u64(), Some(1203));
    }

    #[test]
    fn non_streaming_rewrite_skips_error_body() {
        let body = br#"{"is_bifrost_error":true,"status_code":504,"error":{"message":"timeout"}}"#;
        let out = rewrite_non_streaming_output_usage(body);
        assert_eq!(out, body);
    }
}
