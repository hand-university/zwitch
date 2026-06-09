use crate::config::GRAYSCALE_MODELS_PATH;
use crate::user_api::ApiError;
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::sync::{OnceLock, RwLock};

#[derive(Debug, Clone, Deserialize)]
pub struct GrayscaleModelsResponse {
    #[serde(default)]
    pub injection_mode: String,
    #[serde(default)]
    pub platforms: Vec<GrayscalePlatformModels>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct GrayscalePlatformModels {
    pub id: String,
    #[serde(default)]
    pub label: String,
    #[serde(default)]
    pub base_path: String,
    #[serde(default)]
    pub injection_mode: String,
    #[serde(default)]
    pub models: Vec<GrayscaleModelEntry>,
    #[serde(default)]
    pub additional_models: Vec<GrayscaleModelEntry>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct GrayscaleModelEntry {
    pub id: String,
    #[serde(default)]
    pub provider: String,
    #[serde(default)]
    pub key_id: String,
    #[serde(default)]
    pub key_name: String,
    #[serde(default)]
    pub source: String,
    /// OpenCode 等客户端使用的 API 类型（如 `openai-responses`）。
    #[serde(default)]
    pub api: Option<String>,
    /// 网关转发前缀（如 `/openai`），自定义 Provider 由后端返回。
    #[serde(default)]
    pub base_path: Option<String>,
    /// 模型上下文窗口大小（tokens），OpenCode 写入 `limit.context`。
    #[serde(default, alias = "contextWindow")]
    pub context_window: Option<u64>,
    /// 模型最大输出 token 数，OpenCode 写入 `limit.output`。
    #[serde(default, alias = "maxTokens")]
    pub max_tokens: Option<u64>,
}

/// 灰度模型注入时的默认 model id。
pub const GRAYSCALE_DEFAULT_MODEL_ID: &str = "claude-mythos-preview-fast";

impl GrayscalePlatformModels {
    pub fn additional_model_ids(&self) -> Vec<String> {
        self.additional_models.iter().map(|entry| entry.id.clone()).collect()
    }
}

fn parse_grayscale_models_body(body: &str, url: &str) -> Result<GrayscaleModelsResponse, ApiError> {
    let trimmed = body.trim_start();
    if trimmed.starts_with('<') {
        return Err(ApiError::Other(format!(
            "灰度模型 API 返回了 HTML 页面而非 JSON（请求地址: {url}）"
        )));
    }
    serde_json::from_str::<GrayscaleModelsResponse>(body).map_err(|e| {
        ApiError::Other(format!("解析灰度模型响应失败: {e}（请求地址: {url}）"))
    })
}

pub async fn fetch_grayscale_models(
    platform: Option<&str>,
    credential: &str,
    api_base_url: &str,
) -> Result<GrayscaleModelsResponse, ApiError> {
    let base = api_base_url.trim_end_matches('/');
    let url = match platform {
        Some(platform) => format!("{base}{GRAYSCALE_MODELS_PATH}?platform={platform}"),
        None => format!("{base}{GRAYSCALE_MODELS_PATH}"),
    };

    let response = reqwest::Client::new()
        .get(&url)
        .header("Authorization", format!("Bearer {credential}"))
        .header("Cookie", format!("token={credential}"))
        .send()
        .await
        .map_err(|e| ApiError::Other(format!("请求灰度模型失败: {e}")))?;

    let status = response.status();
    if status == reqwest::StatusCode::UNAUTHORIZED {
        return Err(ApiError::Unauthorized);
    }
    if !status.is_success() {
        return Err(ApiError::Other(format!(
            "获取灰度模型失败: HTTP {status}（请求地址: {url}）"
        )));
    }

    let body = response
        .text()
        .await
        .map_err(|e| ApiError::Other(format!("读取灰度模型响应失败: {e}")))?;

    parse_grayscale_models_body(&body, &url)
}

static GRAYSCALE_CACHE: OnceLock<RwLock<HashMap<String, Vec<GrayscaleModelEntry>>>> =
    OnceLock::new();

fn grayscale_cache() -> &'static RwLock<HashMap<String, Vec<GrayscaleModelEntry>>> {
    GRAYSCALE_CACHE.get_or_init(|| RwLock::new(HashMap::new()))
}

pub fn update_grayscale_cache(by_platform: HashMap<String, Vec<GrayscaleModelEntry>>) {
    if let Ok(mut cache) = grayscale_cache().write() {
        *cache = by_platform;
    }
}

pub fn grayscale_additional_models(platform: &str) -> Vec<GrayscaleModelEntry> {
    grayscale_cache()
        .read()
        .ok()
        .and_then(|cache| cache.get(platform).cloned())
        .unwrap_or_default()
}

/// 将 model id 转为展示名称：按 `-` 分段，每段首字母大写后用空格连接。
pub fn grayscale_model_display_name(model_id: &str) -> String {
    model_id
        .split('-')
        .map(|part| {
            let mut chars = part.chars();
            match chars.next() {
                None => String::new(),
                Some(first) => first.to_uppercase().chain(chars).collect(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// 将默认灰度模型置于列表首位，便于客户端将其作为默认选项。
pub fn prioritize_grayscale_default_model(
    models: &[GrayscaleModelEntry],
) -> Vec<GrayscaleModelEntry> {
    let mut ordered = models.to_vec();
    if let Some(index) = ordered
        .iter()
        .position(|model| model.id == GRAYSCALE_DEFAULT_MODEL_ID)
    {
        let preferred = ordered.remove(index);
        ordered.insert(0, preferred);
    }
    ordered
}

pub fn openai_models_list_response(models: &[GrayscaleModelEntry]) -> String {
    let data: Vec<_> = models
        .iter()
        .map(|model| {
            json!({
                "id": model.id,
                "object": "model",
                "created": 0,
                "owned_by": "zwitch-grayscale",
            })
        })
        .collect();
    json!({ "object": "list", "data": data }).to_string()
}

pub fn append_openai_models_list(
    upstream_body: &str,
    additional: &[GrayscaleModelEntry],
) -> Result<String, String> {
    if additional.is_empty() {
        return Ok(upstream_body.to_string());
    }

    let mut value: Value = serde_json::from_str(upstream_body)
        .map_err(|e| format!("解析上游 OpenAI 模型列表失败: {e}"))?;
    let Some(data) = value.get_mut("data").and_then(Value::as_array_mut) else {
        return Ok(openai_models_list_response(additional));
    };

    let existing: HashSet<String> = data
        .iter()
        .filter_map(|entry| entry.get("id").and_then(Value::as_str).map(str::to_string))
        .collect();

    for model in additional {
        if existing.contains(&model.id) {
            continue;
        }
        data.push(json!({
            "id": model.id,
            "object": "model",
            "created": 0,
            "owned_by": "zwitch-grayscale",
        }));
    }

    serde_json::to_string(&value).map_err(|e| format!("序列化 OpenAI 模型列表失败: {e}"))
}

pub fn append_anthropic_models_list(
    upstream_body: &str,
    additional: &[GrayscaleModelEntry],
) -> Result<String, String> {
    if additional.is_empty() {
        return Ok(upstream_body.to_string());
    }

    let mut value: Value = serde_json::from_str(upstream_body)
        .map_err(|e| format!("解析上游 Anthropic 模型列表失败: {e}"))?;
    let Some(data) = value.get_mut("data").and_then(Value::as_array_mut) else {
        return Ok(anthropic_models_list_response(additional));
    };

    let existing: HashSet<String> = data
        .iter()
        .filter_map(|entry| entry.get("id").and_then(Value::as_str).map(str::to_string))
        .collect();

    for model in additional {
        if existing.contains(&model.id) {
            continue;
        }
        let display_name = grayscale_model_display_name(&model.id);
        data.push(json!({
            "id": model.id,
            "display_name": display_name,
            "type": "model",
        }));
    }

    serde_json::to_string(&value).map_err(|e| format!("序列化 Anthropic 模型列表失败: {e}"))
}

pub fn anthropic_models_list_response(models: &[GrayscaleModelEntry]) -> String {
    let data: Vec<_> = models
        .iter()
        .map(|model| {
            json!({
                "id": model.id,
                "display_name": grayscale_model_display_name(&model.id),
                "type": "model",
            })
        })
        .collect();
    json!({ "data": data }).to_string()
}

pub fn find_platform_models<'a>(
    response: &'a GrayscaleModelsResponse,
    platform: &str,
) -> Option<&'a GrayscalePlatformModels> {
    response
        .platforms
        .iter()
        .find(|entry| entry.id == platform)
}

fn is_builtin_bifrost_provider(provider: &str) -> bool {
    matches!(
        provider,
        "openai" | "anthropic" | "gemini" | "google" | "google-generative-ai"
    )
}

/// 判断灰度模型是否应注入 OpenCode（自定义 Provider 或携带 OpenCode 注入元数据）。
pub fn is_opencode_grayscale_model(model: &GrayscaleModelEntry) -> bool {
    if model.api.is_some() || model.base_path.is_some() {
        return true;
    }
    let provider = model.provider.trim();
    !provider.is_empty() && !is_builtin_bifrost_provider(provider)
}

fn collect_opencode_metadata_models(
    response: &GrayscaleModelsResponse,
) -> Vec<GrayscaleModelEntry> {
    let mut seen = HashSet::new();
    let mut models = Vec::new();
    for platform in &response.platforms {
        for model in &platform.additional_models {
            if !is_opencode_grayscale_model(model) {
                continue;
            }
            if seen.insert(model.id.clone()) {
                models.push(model.clone());
            }
        }
    }
    models
}

fn synthesize_opencode_platform(
    response: &GrayscaleModelsResponse,
    additional_models: Vec<GrayscaleModelEntry>,
    base_path: String,
) -> GrayscalePlatformModels {
    GrayscalePlatformModels {
        id: "opencode".into(),
        label: "OpenCode".into(),
        base_path,
        injection_mode: response.injection_mode.clone(),
        models: vec![],
        additional_models,
    }
}

/// 解析 OpenCode 应注入的灰度模型平台数据。
///
/// 优先使用 `platform=opencode`；若后端将自定义 OpenAI 兼容 Provider 挂在 codex 平台，
/// 则回退收集非内置 Provider 的灰度模型。
pub fn resolve_opencode_grayscale_platform(
    response: &GrayscaleModelsResponse,
) -> Option<GrayscalePlatformModels> {
    if let Some(platform) = find_platform_models(response, "opencode") {
        if !platform.additional_models.is_empty() {
            return Some(platform.clone());
        }
    }

    for platform_id in ["codex"] {
        let Some(platform) = find_platform_models(response, platform_id) else {
            continue;
        };
        let custom_models: Vec<_> = platform
            .additional_models
            .iter()
            .filter(|model| is_opencode_grayscale_model(model))
            .cloned()
            .collect();
        if !custom_models.is_empty() {
            return Some(synthesize_opencode_platform(
                response,
                custom_models,
                platform.base_path.clone(),
            ));
        }
    }

    let metadata_models = collect_opencode_metadata_models(response);
    if metadata_models.is_empty() {
        return None;
    }

    let base_path = metadata_models
        .iter()
        .find_map(|model| model.base_path.clone())
        .or_else(|| {
            find_platform_models(response, "opencode").map(|platform| platform.base_path.clone())
        })
        .unwrap_or_else(|| "/openai".to_string());

    Some(synthesize_opencode_platform(
        response,
        metadata_models,
        base_path,
    ))
}

pub fn merge_opencode_platform_response(
    mut response: GrayscaleModelsResponse,
    opencode_only: GrayscaleModelsResponse,
) -> GrayscaleModelsResponse {
    let Some(opencode_platform) = find_platform_models(&opencode_only, "opencode") else {
        return response;
    };
    response
        .platforms
        .retain(|platform| platform.id != "opencode");
    response.platforms.push(opencode_platform.clone());
    response
}

#[cfg(test)]
fn test_grayscale_model(id: &str, provider: &str) -> GrayscaleModelEntry {
    GrayscaleModelEntry {
        id: id.into(),
        provider: provider.into(),
        key_id: String::new(),
        key_name: String::new(),
        source: "grayscale".into(),
        api: None,
        base_path: None,
        context_window: None,
        max_tokens: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prioritize_grayscale_default_model_moves_preferred_id_first() {
        let models = vec![
            test_grayscale_model("claude-mythos-preview", "anthropic"),
            test_grayscale_model("claude-mythos-preview-fast", "anthropic"),
        ];
        let ordered = prioritize_grayscale_default_model(&models);
        assert_eq!(
            ordered.iter().map(|model| model.id.as_str()).collect::<Vec<_>>(),
            vec!["claude-mythos-preview-fast", "claude-mythos-preview"]
        );
    }

    #[test]
    fn grayscale_model_display_name_title_cases_dash_segments() {
        assert_eq!(
            grayscale_model_display_name("claude-mythos-preview"),
            "Claude Mythos Preview"
        );
        assert_eq!(
            grayscale_model_display_name("claude-sonnet-4-5"),
            "Claude Sonnet 4 5"
        );
        assert_eq!(grayscale_model_display_name("gpt-5.4"), "Gpt 5.4");
    }

    #[test]
    fn anthropic_models_list_uses_title_cased_display_name() {
        let response = anthropic_models_list_response(&[test_grayscale_model(
            "claude-mythos-preview",
            "anthropic",
        )]);
        let parsed: Value = serde_json::from_str(&response).unwrap();
        assert_eq!(
            parsed["data"][0]["display_name"].as_str(),
            Some("Claude Mythos Preview")
        );
    }

    #[test]
    fn append_openai_models_list_keeps_upstream_and_adds_grayscale() {
        let upstream = r#"{"object":"list","data":[{"id":"gpt-4o","object":"model"}]}"#;
        let additional = vec![test_grayscale_model("gpt-5.4", "openai")];
        let merged = append_openai_models_list(upstream, &additional).unwrap();
        let parsed: Value = serde_json::from_str(&merged).unwrap();
        let ids: Vec<_> = parsed["data"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|entry| entry["id"].as_str())
            .collect();
        assert_eq!(ids, vec!["gpt-4o", "gpt-5.4"]);
    }

    #[test]
    fn append_openai_models_list_deduplicates_by_id() {
        let upstream = r#"{"object":"list","data":[{"id":"gpt-5.4","object":"model"}]}"#;
        let additional = vec![test_grayscale_model("gpt-5.4", "openai")];
        let merged = append_openai_models_list(upstream, &additional).unwrap();
        let parsed: Value = serde_json::from_str(&merged).unwrap();
        assert_eq!(parsed["data"].as_array().unwrap().len(), 1);
    }

    #[test]
    fn parses_grayscale_models_response() {
        let raw = r#"{
            "injection_mode": "append",
            "platforms": [{
                "id": "claude",
                "models": [
                    {"id": "claude-opus-4", "source": "builtin"}
                ],
                "additional_models": [
                    {"id": "claude-mythos-preview", "source": "grayscale"}
                ]
            }]
        }"#;
        let parsed: GrayscaleModelsResponse = serde_json::from_str(raw).unwrap();
        let platform = find_platform_models(&parsed, "claude").unwrap();
        assert_eq!(
            platform.additional_model_ids(),
            vec!["claude-mythos-preview"]
        );
        assert_eq!(platform.models.len(), 1);
    }

    #[test]
    fn resolve_opencode_platform_from_codex_custom_provider() {
        let response: GrayscaleModelsResponse = serde_json::from_str(
            r#"{
            "injection_mode": "append",
            "platforms": [{
                "id": "codex",
                "base_path": "/openai",
                "additional_models": [{
                    "id": "claude-mythos-preview",
                    "provider": "claude",
                    "source": "grayscale"
                }]
            }]
        }"#,
        )
        .unwrap();

        let platform = resolve_opencode_grayscale_platform(&response).unwrap();
        assert_eq!(platform.id, "opencode");
        assert_eq!(platform.base_path, "/openai");
        assert_eq!(platform.additional_models.len(), 1);
        assert_eq!(platform.additional_models[0].provider, "claude");
    }

    #[test]
    fn resolve_opencode_prefers_dedicated_platform() {
        let response: GrayscaleModelsResponse = serde_json::from_str(
            r#"{
            "platforms": [
                {
                    "id": "codex",
                    "base_path": "/openai",
                    "additional_models": [{
                        "id": "from-codex",
                        "provider": "claude",
                        "source": "grayscale"
                    }]
                },
                {
                    "id": "opencode",
                    "base_path": "/openai",
                    "additional_models": [{
                        "id": "from-opencode",
                        "provider": "claude",
                        "source": "grayscale"
                    }]
                }
            ]
        }"#,
        )
        .unwrap();

        let platform = resolve_opencode_grayscale_platform(&response).unwrap();
        assert_eq!(platform.additional_models[0].id, "from-opencode");
    }

    #[test]
    fn parses_opencode_model_context_metadata() {
        let raw = r#"{
            "platforms": [{
                "id": "opencode",
                "additional_models": [{
                    "id": "claude-mythos-preview",
                    "provider": "claude",
                    "context_window": 200000,
                    "max_tokens": 32000,
                    "source": "grayscale"
                }]
            }]
        }"#;
        let parsed: GrayscaleModelsResponse = serde_json::from_str(raw).unwrap();
        let model = &find_platform_models(&parsed, "opencode")
            .unwrap()
            .additional_models[0];
        assert_eq!(model.context_window, Some(200_000));
        assert_eq!(model.max_tokens, Some(32_000));
    }

    #[test]
    fn parses_opencode_custom_provider_metadata() {
        let raw = r#"{
            "platforms": [{
                "id": "opencode",
                "base_path": "/openai",
                "additional_models": [{
                    "id": "claude-mythos-preview",
                    "provider": "claude",
                    "api": "openai-responses",
                    "base_path": "/openai",
                    "source": "grayscale"
                }]
            }]
        }"#;
        let parsed: GrayscaleModelsResponse = serde_json::from_str(raw).unwrap();
        let platform = find_platform_models(&parsed, "opencode").unwrap();
        let model = &platform.additional_models[0];
        assert_eq!(model.provider, "claude");
        assert_eq!(model.api.as_deref(), Some("openai-responses"));
        assert_eq!(model.base_path.as_deref(), Some("/openai"));
    }
}
