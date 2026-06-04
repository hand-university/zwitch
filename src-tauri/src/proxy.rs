use crate::config::{AppConfig, DEVICE_FINGERPRINT_HEADER, LOCAL_PROXY_HOST};
use crate::store::{load_auth, save_auth};
use crate::user_api::{exchange_device_code, normalize_api_base_url};
use axum::{
    body::Body,
    extract::{Request, State},
    http::{header, HeaderName, StatusCode},
    response::{IntoResponse, Response},
    Router,
};
use futures_util::StreamExt;
use std::net::TcpListener as StdTcpListener;
use std::sync::atomic::{AtomicU16, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};
use tokio::net::TcpListener;
use tokio::sync::Mutex;

/// 本地拦截服务实际监听的端口（启动时动态分配）。0 表示尚未启动。
static PROXY_PORT: AtomicU16 = AtomicU16::new(0);

static CREDENTIALS: OnceLock<Arc<CredentialManager>> = OnceLock::new();

/// 凭证过期前主动刷新的缓冲时间。
const CREDENTIAL_REFRESH_BUFFER: Duration = Duration::from_secs(600);

/// 返回本地拦截服务当前监听的端口；未启动时为 0。
pub fn local_port() -> u16 {
    PROXY_PORT.load(Ordering::Relaxed)
}

/// 写入 CLI 配置文件的本地拦截地址（按工具区分前缀），使用动态分配的端口。
pub fn local_proxy_base(tool_id: &str) -> String {
    format!("http://{LOCAL_PROXY_HOST}:{}/{tool_id}", local_port())
}

/// 登出或授权失效时清空内存中的临时凭证缓存。
pub fn clear_credential_cache() {
    if let Some(manager) = CREDENTIALS.get() {
        let manager = manager.clone();
        tauri::async_runtime::spawn(async move {
            manager.invalidate().await;
        });
    }
}

struct CachedCredential {
    token: String,
    refresh_after: Instant,
}

/// 管理设备码换取的 24h 临时凭证，供本地拦截服务注入上游请求。
struct CredentialManager {
    app: tauri::AppHandle,
    fingerprint: String,
    cache: Mutex<Option<CachedCredential>>,
}

impl CredentialManager {
    fn new(app: tauri::AppHandle, fingerprint: String) -> Self {
        Self {
            app,
            fingerprint,
            cache: Mutex::new(None),
        }
    }

    async fn invalidate(&self) {
        *self.cache.lock().await = None;
    }

    async fn get(&self) -> Result<String, String> {
        {
            let cache = self.cache.lock().await;
            if let Some(cached) = cache.as_ref() {
                if Instant::now() < cached.refresh_after {
                    return Ok(cached.token.clone());
                }
            }
        }
        self.refresh().await
    }

    async fn refresh(&self) -> Result<String, String> {
        let auth = load_auth(&self.app).map_err(|e| e.to_string())?;
        let code = auth
            .authorization_code
            .clone()
            .ok_or_else(|| "未登录，无法换取临时凭证".to_string())?;
        let base = resolve_api_base_url(&auth);

        let resp = exchange_device_code(&code, &self.fingerprint, &base)
            .await
            .map_err(|e| e.message())?;

        if let Some(new_base) = resp
            .base_url
            .as_deref()
            .and_then(normalize_api_base_url)
        {
            let mut updated = auth;
            updated.api_base_url = Some(new_base);
            let _ = save_auth(&self.app, &updated);
        }

        let ttl = resp.expires_in.unwrap_or(86400);
        let refresh_after =
            Instant::now() + Duration::from_secs(ttl).saturating_sub(CREDENTIAL_REFRESH_BUFFER);

        let token = resp.credential().to_string();
        *self.cache.lock().await = Some(CachedCredential {
            token: token.clone(),
            refresh_after,
        });
        Ok(token)
    }
}

fn resolve_api_base_url(auth: &crate::store::StoredAuth) -> String {
    auth.api_base_url
        .as_deref()
        .and_then(normalize_api_base_url)
        .unwrap_or_else(|| AppConfig::default().api_base_url)
}

/// 按 CLI 工具映射到 Bifrost 网关的 integration 前缀，避免 Codex/Claude/Gemini 走错路由。
fn build_upstream_url(tool_id: &str, gateway: &str, rest: &str) -> String {
    let gateway = gateway.trim_end_matches('/');
    let rest = rest.trim_start_matches('/');

    match tool_id {
        "codex" => {
            if rest.starts_with("openai/") {
                format!("{gateway}/{rest}")
            } else if rest.starts_with("v1/") {
                format!("{gateway}/openai/{rest}")
            } else {
                format!("{gateway}/openai/v1/{rest}")
            }
        }
        "claude" => {
            if rest.starts_with("anthropic/") {
                format!("{gateway}/{rest}")
            } else if rest.starts_with("v1/") {
                format!("{gateway}/anthropic/{rest}")
            } else {
                format!("{gateway}/anthropic/v1/{rest}")
            }
        }
        "gemini" => {
            if rest.starts_with("genai/") {
                format!("{gateway}/{rest}")
            } else {
                format!("{gateway}/genai/{rest}")
            }
        }
        _ => {
            if rest.starts_with("v1/") {
                format!("{gateway}/{rest}")
            } else {
                format!("{gateway}/v1/{rest}")
            }
        }
    }
}

/// 本地拦截服务的运行时状态。
struct ProxyState {
    app: tauri::AppHandle,
    fingerprint: String,
    credentials: Arc<CredentialManager>,
    client: reqwest::Client,
}

/// 启动本地拦截服务。CLI 仅配置指向本服务的 base_url；本服务换取临时凭证后
/// 注入 `Authorization: Bearer bf-tmp-...` 与 `X-Device-Fingerprint` 再转发上游。
pub fn start(app: tauri::AppHandle, fingerprint: String) -> Option<u16> {
    let credentials = Arc::new(CredentialManager::new(app.clone(), fingerprint.clone()));
    let _ = CREDENTIALS.set(credentials.clone());

    let state = Arc::new(ProxyState {
        app: app.clone(),
        fingerprint,
        credentials,
        client: reqwest::Client::new(),
    });

    let std_listener = match StdTcpListener::bind((LOCAL_PROXY_HOST, 0)) {
        Ok(listener) => listener,
        Err(e) => {
            eprintln!("本地拦截服务绑定空闲端口失败: {e}");
            return None;
        }
    };
    let port = match std_listener.local_addr() {
        Ok(addr) => addr.port(),
        Err(e) => {
            eprintln!("读取本地拦截服务端口失败: {e}");
            return None;
        }
    };
    if let Err(e) = std_listener.set_nonblocking(true) {
        eprintln!("设置本地拦截服务非阻塞失败: {e}");
        return None;
    }
    PROXY_PORT.store(port, Ordering::Relaxed);

    tauri::async_runtime::spawn(async move {
        let listener = match TcpListener::from_std(std_listener) {
            Ok(listener) => listener,
            Err(e) => {
                eprintln!("本地拦截服务启动失败: {e}");
                return;
            }
        };

        let app = Router::new().fallback(proxy_handler).with_state(state);
        if let Err(e) = axum::serve(listener, app).await {
            eprintln!("本地拦截服务异常退出: {e}");
        }
    });

    Some(port)
}

async fn proxy_handler(State(state): State<Arc<ProxyState>>, req: Request) -> Response {
    match proxy_handler_inner(state, req).await {
        Ok(resp) => resp,
        Err((status, message)) => error_response(status, message),
    }
}

async fn proxy_handler_inner(
    state: Arc<ProxyState>,
    req: Request,
) -> Result<Response, (StatusCode, String)> {
    let (parts, body) = req.into_parts();
    let body_bytes = axum::body::to_bytes(body, usize::MAX)
        .await
        .map_err(|e| (StatusCode::BAD_REQUEST, format!("读取请求体失败: {e}")))?;

    let mut credential = state
        .credentials
        .get()
        .await
        .map_err(|e| (StatusCode::UNAUTHORIZED, e))?;

    let response = forward_request(&state, &parts, &body_bytes, &credential).await?;

    if response.status() != StatusCode::UNAUTHORIZED {
        return Ok(response);
    }

    state.credentials.invalidate().await;
    credential = state
        .credentials
        .get()
        .await
        .map_err(|e| (StatusCode::UNAUTHORIZED, e))?;

    forward_request(&state, &parts, &body_bytes, &credential).await
}

async fn forward_request(
    state: &ProxyState,
    parts: &axum::http::request::Parts,
    body_bytes: &axum::body::Bytes,
    credential: &str,
) -> Result<Response, (StatusCode, String)> {
    let path = parts.uri.path();
    let query = parts.uri.query();

    let trimmed = path.trim_start_matches('/');
    let mut segments = trimmed.splitn(2, '/');
    let tool_id = segments.next().unwrap_or("");
    let rest = segments.next().unwrap_or("");

    let auth = load_auth(&state.app).unwrap_or_default();
    let gateway = resolve_api_base_url(&auth);
    let mut target = build_upstream_url(tool_id, &gateway, rest);
    if let Some(q) = query {
        target.push('?');
        target.push_str(q);
    }

    let mut builder = state.client.request(parts.method.clone(), &target);

    static SKIP_HEADERS: &[HeaderName] = &[
        header::HOST,
        header::AUTHORIZATION,
        header::PROXY_AUTHORIZATION,
    ];

    for (name, value) in parts.headers.iter() {
        if SKIP_HEADERS.iter().any(|skip| skip == name) {
            continue;
        }
        builder = builder.header(name, value);
    }

    builder = builder
        .header(header::AUTHORIZATION, format!("Bearer {credential}"))
        .header(DEVICE_FINGERPRINT_HEADER, state.fingerprint.as_str())
        .body(body_bytes.clone());

    let upstream_resp = builder
        .send()
        .await
        .map_err(|e| (StatusCode::BAD_GATEWAY, format!("转发上游失败: {e}")))?;

    build_upstream_response(upstream_resp)
        .await
        .map_err(|e| (StatusCode::BAD_GATEWAY, e))
}

fn is_streaming_content_type(content_type: Option<&str>) -> bool {
    let Some(content_type) = content_type else {
        return false;
    };
    let lower = content_type.to_ascii_lowercase();
    lower.contains("text/event-stream")
        || lower.contains("application/x-ndjson")
        || lower.contains("application/stream+json")
}

async fn build_upstream_response(
    upstream_resp: reqwest::Response,
) -> Result<Response, String> {
    let status = upstream_resp.status();
    let resp_headers = upstream_resp.headers().clone();
    let content_type = resp_headers
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok());

    let mut response = Response::builder().status(status);
    for (name, value) in resp_headers.iter() {
        if name == header::CONTENT_LENGTH || name == header::TRANSFER_ENCODING {
            continue;
        }
        response = response.header(name, value);
    }

    if is_streaming_content_type(content_type) {
        let stream = upstream_resp.bytes_stream().map(|chunk| {
            chunk.map_err(|e| -> Box<dyn std::error::Error + Send + Sync> { Box::new(e) })
        });
        return response
            .body(Body::from_stream(stream))
            .map_err(|_| "构建流式响应失败".to_string());
    }

    let bytes = upstream_resp
        .bytes()
        .await
        .map_err(|e| format!("读取上游响应失败: {e}"))?;

    response
        .body(Body::from(bytes))
        .map_err(|_| "构建响应失败".to_string())
}

fn error_response(status: StatusCode, message: String) -> Response {
    (status, message).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codex_responses_route() {
        assert_eq!(
            build_upstream_url("codex", "http://localhost:8080", "responses"),
            "http://localhost:8080/openai/v1/responses"
        );
        assert_eq!(
            build_upstream_url("codex", "http://localhost:8080", "v1/responses"),
            "http://localhost:8080/openai/v1/responses"
        );
    }

    #[test]
    fn claude_messages_route() {
        assert_eq!(
            build_upstream_url("claude", "http://localhost:8080", "v1/messages"),
            "http://localhost:8080/anthropic/v1/messages"
        );
    }

    #[test]
    fn gemini_route() {
        assert_eq!(
            build_upstream_url(
                "gemini",
                "http://localhost:8080",
                "v1beta/models/gemini-pro:streamGenerateContent"
            ),
            "http://localhost:8080/genai/v1beta/models/gemini-pro:streamGenerateContent"
        );
    }
}
