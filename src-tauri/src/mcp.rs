//! pi 的 MCP 工具「改名透传」。
//!
//! 背景：pi（带 MCP 扩展）会把 MCP 工具以 `mcp_<server>_<tool>` 的原生工具形式放进
//! 请求的 `tools` 数组。运营商网关一旦在请求里看到 `mcp_*` 工具，会直接返回空响应、
//! 立刻结束（连 input token 都没有），等于整条请求被丢弃。
//!
//! 策略（保留端到端的「结构化」工具调用，pi 侧零改动）：
//!   1. 请求侧：把 body 里所有 `mcp_*` 工具名（tools 定义 + 历史里的工具调用）改写成
//!      网关接受的 sentinel 前缀 `zwext_`，各工具的真实 schema 原样保留；
//!   2. 响应侧：把模型产出里所有 `zwext_` 还原回 `mcp_`，pi 收到的就是它认识的原始
//!      工具名，正常执行 MCP 工具。
//!
//! 请求侧只改「工具名字段」（避免误伤用户正文里恰好含 `mcp_` 的文本）；
//! 响应侧用纯字节替换（`zwext_` 是私有 sentinel，只可能来自我们的改名，全量替换安全，
//! 且能覆盖模型在正文里顺带提到的工具名）。

use serde_json::{Map, Value};

/// pi-mcp 扩展的工具命名前缀（上游不接受）。
const MCP_PREFIX: &str = "mcp_";

/// 改名后的前缀：刻意不含 "mcp" 子串、不与 pi 原生/常见工具名冲突，
/// 且作为响应侧反向字节替换的私有 sentinel 足够独特。
const SENTINEL_PREFIX: &str = "zwext_";

/// 请求侧：把 pi 请求体里所有 `mcp_*` 工具名改写成 sentinel 前缀。
///
/// 返回 `Some(new_body)` 表示已改写；返回 `None` 表示无需改写（非 pi、无 mcp 工具）
/// 或解析失败，调用方应原样转发原始 body。
pub fn rewrite_request_tools(tool_id: &str, body: &[u8]) -> Option<Vec<u8>> {
    let rename = rename_enabled();
    let minimal = minimal_schema_enabled();
    // 两个开关都关：原样透传（含 mcp_ 名字）。
    if !rename && !minimal {
        return None;
    }
    rewrite_request_tools_inner(tool_id, body, rename, minimal)
}

fn rewrite_request_tools_inner(
    tool_id: &str,
    body: &[u8],
    rename: bool,
    minimal: bool,
) -> Option<Vec<u8>> {
    if tool_id != "pi" {
        return None;
    }

    let mut root: Value = serde_json::from_slice(body).ok()?;
    let mut changed = false;
    // 改名透传：mcp_* -> sentinel（默认关闭，ZD_MCP_RENAME=1 启用）。
    if rename {
        changed |= rename_in_request(&mut root, MCP_PREFIX, SENTINEL_PREFIX);
    }
    // 二分诊断：把 MCP 工具 schema 压成最简（独立于改名，ZD_MCP_MINIMAL_SCHEMA=1 启用）。
    if minimal {
        changed |= minimize_mcp_tool_schemas(&mut root);
    }
    if !changed {
        return None;
    }
    serde_json::to_vec(&root).ok()
}

/// 改名透传开关：默认关闭，设 `ZD_MCP_RENAME=1` 启用。
fn rename_enabled() -> bool {
    std::env::var("ZD_MCP_RENAME")
        .map(|v| !v.is_empty() && v != "0")
        .unwrap_or(false)
}

/// 临时诊断开关：设 `ZD_MCP_MINIMAL_SCHEMA=1` 时把 MCP 工具参数 schema 压成最简。
fn minimal_schema_enabled() -> bool {
    std::env::var("ZD_MCP_MINIMAL_SCHEMA")
        .map(|v| !v.is_empty() && v != "0")
        .unwrap_or(false)
}

fn minimal_schema() -> Value {
    serde_json::json!({ "type": "object", "properties": {} })
}

/// 工具名（含 `{function:{name}}`）是否为 MCP 工具：原始 `mcp_` 前缀或改名后的 sentinel 前缀。
fn tool_is_mcp(tool: &Value) -> bool {
    let name = tool
        .get("name")
        .and_then(Value::as_str)
        .or_else(|| {
            tool.get("function")
                .and_then(|f| f.get("name"))
                .and_then(Value::as_str)
        });
    name.map(|n| n.starts_with(MCP_PREFIX) || n.starts_with(SENTINEL_PREFIX))
        .unwrap_or(false)
}

/// 仅对 MCP 工具，把参数 schema 压成最简并去掉 strict；原生工具不动。
fn minimize_mcp_tool_schemas(root: &mut Value) -> bool {
    let Some(obj) = root.as_object_mut() else {
        return false;
    };
    let Some(Value::Array(tools)) = obj.get_mut("tools") else {
        return false;
    };

    let mut changed = false;
    for tool in tools.iter_mut() {
        if !tool_is_mcp(tool) {
            continue;
        }
        let Some(t) = tool.as_object_mut() else {
            continue;
        };
        changed |= t.remove("strict").is_some();
        for key in ["input_schema", "parameters"] {
            if t.contains_key(key) {
                t.insert(key.to_string(), minimal_schema());
                changed = true;
            }
        }
        if let Some(Value::Object(func)) = t.get_mut("function") {
            changed |= func.remove("strict").is_some();
            if func.contains_key("parameters") {
                func.insert("parameters".to_string(), minimal_schema());
                changed = true;
            }
        }
    }
    changed
}

/// 响应侧（非流式）：把整段响应里的 sentinel 还原回 `mcp_`。
pub fn restore_response_bytes(body: &[u8]) -> Vec<u8> {
    replace_all(body, SENTINEL_PREFIX.as_bytes(), MCP_PREFIX.as_bytes())
}

/// 响应侧（流式）：增量还原一个分片，处理 sentinel 被切分在分片边界的情况。
///
/// `carry` 由调用方在整条流上持有：保存「可能是 sentinel 前缀一部分」的尾部字节，
/// 等后续分片到达再判定。返回本次可安全下发的字节。
pub fn restore_stream_chunk(carry: &mut Vec<u8>, chunk: &[u8]) -> Vec<u8> {
    carry.extend_from_slice(chunk);
    let data = std::mem::take(carry);

    let pat = SENTINEL_PREFIX.as_bytes();
    // 末尾若是 sentinel 的真前缀，先扣留，避免把跨分片的 sentinel 切坏。
    let keep = suffix_that_is_prefix(&data, pat);
    let split = data.len() - keep;
    let out = replace_all(&data[..split], pat, MCP_PREFIX.as_bytes());
    carry.extend_from_slice(&data[split..]);
    out
}

/// 响应侧（流式）：流结束时把扣留的尾部字节冲刷出去。
pub fn restore_stream_flush(carry: &mut Vec<u8>) -> Vec<u8> {
    let data = std::mem::take(carry);
    replace_all(&data, SENTINEL_PREFIX.as_bytes(), MCP_PREFIX.as_bytes())
}

/// 在请求 JSON 上做工具名改写，返回是否发生了改动。
fn rename_in_request(root: &mut Value, from: &str, to: &str) -> bool {
    let Some(obj) = root.as_object_mut() else {
        return false;
    };
    let mut changed = false;

    if let Some(Value::Array(tools)) = obj.get_mut("tools") {
        for tool in tools.iter_mut() {
            changed |= rename_tool(tool, from, to);
        }
    }

    // Anthropic / OpenAI Chat 用 `messages`；OpenAI Responses 用 `input`。
    for key in ["messages", "input"] {
        if let Some(Value::Array(msgs)) = obj.get_mut(key) {
            for msg in msgs.iter_mut() {
                changed |= rename_message(msg, from, to);
            }
        }
    }

    changed
}

/// 改写单个工具定义的名字，兼容 `{name}` 与 `{function:{name}}` 两种形状。
fn rename_tool(tool: &mut Value, from: &str, to: &str) -> bool {
    let Some(obj) = tool.as_object_mut() else {
        return false;
    };
    let mut changed = rename_field(obj, "name", from, to);
    if let Some(Value::Object(func)) = obj.get_mut("function") {
        changed |= rename_field(func, "name", from, to);
    }
    changed
}

/// 改写一条消息里出现的所有工具调用名。
fn rename_message(msg: &mut Value, from: &str, to: &str) -> bool {
    let Some(obj) = msg.as_object_mut() else {
        return false;
    };
    let mut changed = false;

    // OpenAI Responses 的扁平调用项：{type:"function_call", name, call_id}
    if matches!(
        obj.get("type").and_then(Value::as_str),
        Some("function_call") | Some("tool_use")
    ) {
        changed |= rename_field(obj, "name", from, to);
    }

    // OpenAI Chat 的 tool 结果消息可能携带 name 字段。
    if obj.get("role").and_then(Value::as_str) == Some("tool") {
        changed |= rename_field(obj, "name", from, to);
    }

    // OpenAI Chat 的 assistant.tool_calls[].function.name
    if let Some(Value::Array(calls)) = obj.get_mut("tool_calls") {
        for call in calls.iter_mut() {
            if let Some(call_obj) = call.as_object_mut() {
                if let Some(Value::Object(func)) = call_obj.get_mut("function") {
                    changed |= rename_field(func, "name", from, to);
                }
            }
        }
    }

    // Anthropic / Responses 的 content 块里 tool_use / function_call 的 name
    if let Some(Value::Array(blocks)) = obj.get_mut("content") {
        for block in blocks.iter_mut() {
            if let Some(block_obj) = block.as_object_mut() {
                let is_call = matches!(
                    block_obj.get("type").and_then(Value::as_str),
                    Some("tool_use") | Some("function_call")
                );
                if is_call {
                    changed |= rename_field(block_obj, "name", from, to);
                }
            }
        }
    }

    changed
}

/// 若 `obj[key]` 是以 `from` 开头的字符串，替换其前缀为 `to`，返回是否改动。
fn rename_field(obj: &mut Map<String, Value>, key: &str, from: &str, to: &str) -> bool {
    if let Some(Value::String(s)) = obj.get(key) {
        if let Some(rest) = s.strip_prefix(from) {
            let renamed = format!("{to}{rest}");
            obj.insert(key.to_string(), Value::String(renamed));
            return true;
        }
    }
    false
}

/// 全量字节替换：把 `from` 的所有出现替换为 `to`。
fn replace_all(haystack: &[u8], from: &[u8], to: &[u8]) -> Vec<u8> {
    if from.is_empty() || haystack.len() < from.len() {
        return haystack.to_vec();
    }
    let mut out = Vec::with_capacity(haystack.len());
    let mut i = 0;
    while i < haystack.len() {
        if haystack[i..].starts_with(from) {
            out.extend_from_slice(to);
            i += from.len();
        } else {
            out.push(haystack[i]);
            i += 1;
        }
    }
    out
}

/// 返回 `data` 末尾「同时是 `pat` 真前缀」的最长后缀长度（< pat.len()）。
fn suffix_that_is_prefix(data: &[u8], pat: &[u8]) -> usize {
    let max = (pat.len().saturating_sub(1)).min(data.len());
    for k in (1..=max).rev() {
        if data[data.len() - k..] == pat[..k] {
            return k;
        }
    }
    0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn non_pi_is_untouched() {
        let body = br#"{"tools":[{"name":"mcp_x"}]}"#;
        assert!(rewrite_request_tools_inner("claude", body, true, false).is_none());
    }

    #[test]
    fn no_mcp_tools_untouched() {
        let body = serde_json::to_vec(&serde_json::json!({
            "messages": [{"role":"user","content":"hi"}],
            "tools": [{"name":"read","input_schema":{}}]
        }))
        .unwrap();
        assert!(rewrite_request_tools_inner("pi", &body, false).is_none());
    }

    #[test]
    fn renames_anthropic_tools_and_keeps_schema() {
        let body = serde_json::to_vec(&serde_json::json!({
            "system": "base",
            "messages": [{"role":"user","content":"hello"}],
            "tools": [
                {"name":"read","input_schema":{"type":"object"}},
                {"name":"mcp_docs_search","description":"d","input_schema":{"type":"object","properties":{"q":{"type":"string"}}}}
            ]
        }))
        .unwrap();

        let out = rewrite_request_tools_inner("pi", &body, false).unwrap();
        let v: Value = serde_json::from_slice(&out).unwrap();
        let tools = v["tools"].as_array().unwrap();
        let names: Vec<&str> = tools.iter().filter_map(|t| t["name"].as_str()).collect();
        assert_eq!(names, vec!["read", "zwext_docs_search"]);
        // schema 原样保留。
        assert_eq!(tools[1]["input_schema"]["properties"]["q"]["type"], "string");
    }

    #[test]
    fn renames_chat_function_tools_and_history_calls() {
        let body = serde_json::to_vec(&serde_json::json!({
            "messages": [
                {"role":"user","content":"x"},
                {"role":"assistant","tool_calls":[
                    {"id":"c1","type":"function","function":{"name":"mcp_docs_search","arguments":"{}"}}
                ]},
                {"role":"tool","tool_call_id":"c1","name":"mcp_docs_search","content":"r"}
            ],
            "tools": [
                {"type":"function","function":{"name":"mcp_docs_search","parameters":{}}}
            ]
        }))
        .unwrap();

        let out = rewrite_request_tools_inner("pi", &body, false).unwrap();
        let v: Value = serde_json::from_slice(&out).unwrap();
        assert_eq!(
            v["tools"][0]["function"]["name"].as_str().unwrap(),
            "zwext_docs_search"
        );
        let msgs = v["messages"].as_array().unwrap();
        assert_eq!(
            msgs[1]["tool_calls"][0]["function"]["name"].as_str().unwrap(),
            "zwext_docs_search"
        );
        assert_eq!(msgs[2]["name"].as_str().unwrap(), "zwext_docs_search");
    }

    #[test]
    fn renames_anthropic_history_tool_use_block() {
        let body = serde_json::to_vec(&serde_json::json!({
            "messages": [
                {"role":"assistant","content":[
                    {"type":"tool_use","id":"t1","name":"mcp_docs_search","input":{}}
                ]},
                {"role":"user","content":[
                    {"type":"tool_result","tool_use_id":"t1","content":"ok"}
                ]}
            ],
            "tools": [{"name":"mcp_docs_search","input_schema":{}}]
        }))
        .unwrap();

        let out = rewrite_request_tools_inner("pi", &body, false).unwrap();
        let v: Value = serde_json::from_slice(&out).unwrap();
        assert_eq!(
            v["messages"][0]["content"][0]["name"].as_str().unwrap(),
            "zwext_docs_search"
        );
    }

    #[test]
    fn renames_responses_flat_function_call() {
        let body = serde_json::to_vec(&serde_json::json!({
            "instructions": "base",
            "input": [
                {"type":"function_call","name":"mcp_docs_search","call_id":"c1","arguments":"{}"}
            ],
            "tools": [{"type":"function","name":"mcp_docs_search","parameters":{}}]
        }))
        .unwrap();

        let out = rewrite_request_tools_inner("pi", &body, false).unwrap();
        let v: Value = serde_json::from_slice(&out).unwrap();
        assert_eq!(v["tools"][0]["name"].as_str().unwrap(), "zwext_docs_search");
        assert_eq!(
            v["input"][0]["name"].as_str().unwrap(),
            "zwext_docs_search"
        );
    }

    #[test]
    fn restore_response_swaps_back() {
        let body = br#"{"content":[{"type":"tool_use","name":"zwext_docs_search"}]}"#;
        let out = restore_response_bytes(body);
        let s = String::from_utf8(out).unwrap();
        assert!(s.contains("mcp_docs_search"));
        assert!(!s.contains("zwext_"));
    }

    #[test]
    fn user_text_with_mcp_prefix_is_not_corrupted_in_request() {
        // 用户正文里恰好含 mcp_ 不应被改名（只改工具名字段）。
        let body = serde_json::to_vec(&serde_json::json!({
            "messages": [{"role":"user","content":"what is mcp_docs_search"}],
            "tools": [{"name":"mcp_docs_search","input_schema":{}}]
        }))
        .unwrap();
        let out = rewrite_request_tools_inner("pi", &body, false).unwrap();
        let v: Value = serde_json::from_slice(&out).unwrap();
        assert_eq!(v["messages"][0]["content"], "what is mcp_docs_search");
        assert_eq!(v["tools"][0]["name"], "zwext_docs_search");
    }

    #[test]
    fn stream_restore_handles_split_sentinel() {
        let mut carry = Vec::new();
        // 把 "zwext_docs" 切成两段："zwe" 与 "xt_docs"。
        let mut out = restore_stream_chunk(&mut carry, b"prefix zwe");
        out.extend(restore_stream_chunk(&mut carry, b"xt_docs suffix"));
        out.extend(restore_stream_flush(&mut carry));
        let s = String::from_utf8(out).unwrap();
        assert_eq!(s, "prefix mcp_docs suffix");
    }

    #[test]
    fn stream_restore_handles_sentinel_at_chunk_end() {
        let mut carry = Vec::new();
        let mut out = restore_stream_chunk(&mut carry, b"a zwext_");
        out.extend(restore_stream_chunk(&mut carry, b"tool b"));
        out.extend(restore_stream_flush(&mut carry));
        assert_eq!(String::from_utf8(out).unwrap(), "a mcp_tool b");
    }

    #[test]
    fn minimize_only_touches_renamed_tools() {
        let mut root: Value = serde_json::json!({
            "tools": [
                {"name":"read","input_schema":{"type":"object","properties":{"path":{"type":"string"}}}},
                {"name":"zwext_docs_search","strict":true,"input_schema":{"type":"object","properties":{"q":{"type":"string"}},"additionalProperties":false}}
            ]
        });
        assert!(minimize_mcp_tool_schemas(&mut root));
        // 原生 read 不动。
        assert_eq!(
            root["tools"][0]["input_schema"]["properties"]["path"]["type"],
            "string"
        );
        // 改名工具 schema 被压成最简、strict 去掉。
        assert_eq!(root["tools"][1]["input_schema"], minimal_schema());
        assert!(root["tools"][1].get("strict").is_none());
    }

    #[test]
    fn stream_restore_no_sentinel_passthrough() {
        let mut carry = Vec::new();
        let mut out = restore_stream_chunk(&mut carry, b"hello ");
        out.extend(restore_stream_chunk(&mut carry, b"world"));
        out.extend(restore_stream_flush(&mut carry));
        assert_eq!(String::from_utf8(out).unwrap(), "hello world");
    }
}
