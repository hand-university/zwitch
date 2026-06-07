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
}

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
        let display_name = if model.key_name.is_empty() {
            format!("{} [灰度]", model.id)
        } else {
            format!("{} [灰度]", model.key_name)
        };
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
            let display_name = if model.key_name.is_empty() {
                format!("{} [灰度]", model.id)
            } else {
                format!("{} [灰度]", model.key_name)
            };
            json!({
                "id": model.id,
                "display_name": display_name,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn append_openai_models_list_keeps_upstream_and_adds_grayscale() {
        let upstream = r#"{"object":"list","data":[{"id":"gpt-4o","object":"model"}]}"#;
        let additional = vec![GrayscaleModelEntry {
            id: "gpt-5.4".into(),
            provider: "openai".into(),
            key_id: String::new(),
            key_name: String::new(),
            source: "grayscale".into(),
        }];
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
        let additional = vec![GrayscaleModelEntry {
            id: "gpt-5.4".into(),
            provider: "openai".into(),
            key_id: String::new(),
            key_name: String::new(),
            source: "grayscale".into(),
        }];
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
}
