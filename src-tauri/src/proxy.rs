use crate::config::{DEVICE_FINGERPRINT_HEADER, LOCAL_PROXY_HOST};
use crate::store::{load_auth, save_auth};
use crate::usage::{self, Provider};
use crate::user_api::exchange_device_code;
use axum::{
    body::Body,
    extract::{Request, State},
    http::{header, HeaderName, Method, StatusCode},
    response::{IntoResponse, Response},
    Router,
};
use futures_util::StreamExt;
use std::net::TcpListener as StdTcpListener;
use std::sync::atomic::{AtomicU16, Ordering};
use std::sync::{Arc, Mutex as StdMutex, OnceLock};
use std::time::{Duration, Instant};
use tokio::net::TcpListener;
use tokio::sync::Mutex;

/// 本地拦截服务实际监听的端口（启动时动态分配）。0 表示尚未启动。
static PROXY_PORT: AtomicU16 = AtomicU16::new(0);

static PROXY_SHUTDOWN: OnceLock<StdMutex<Option<tokio::sync::oneshot::Sender<()>>>> =
    OnceLock::new();

static CREDENTIALS: StdMutex<Option<Arc<CredentialManager>>> = StdMutex::new(None);

/// Claude Code 可能通过 `X-Api-Key` 携带本地 API Key，需剥离后改由代理注入临时凭证。
static X_API_KEY_HEADER: HeaderName = HeaderName::from_static("x-api-key");

/// 凭证过期前主动刷新的缓冲时间。
const CREDENTIAL_REFRESH_BUFFER: Duration = Duration::from_secs(600);

/// 返回本地拦截服务当前监听的端口；未启动时为 0。
pub fn local_port() -> u16 {
    PROXY_PORT.load(Ordering::Relaxed)
}

/// 写入 CLI 配置文件的本地拦截地址（按工具区分前缀），使用动态分配的端口。
pub fn local_proxy_base(tool_id: &str) -> String {
    local_proxy_base_for_port(tool_id, local_port())
}

/// 按指定端口生成本地拦截 base_url。
pub fn local_proxy_base_for_port(tool_id: &str, port: u16) -> String {
    format!("http://{LOCAL_PROXY_HOST}:{port}/{tool_id}")
}

/// 从已注入的本地代理 URL 中解析端口。
pub fn parse_local_proxy_port(url: &str) -> Option<u16> {
    let prefix = format!("http://{LOCAL_PROXY_HOST}:");
    let rest = url.strip_prefix(&prefix)?;
    let (port_str, _) = rest.split_once('/')?;
    port_str.parse().ok()
}

/// 判断端口是否可用于绑定：当前已由本服务占用视为可用，否则尝试绑定探测。
pub fn is_port_available(port: u16) -> bool {
    if port == 0 {
        return false;
    }
    if port == local_port() {
        return true;
    }
    StdTcpListener::bind((LOCAL_PROXY_HOST, port)).is_ok()
}

/// 绑定到端口 0，返回系统分配的空闲端口。
pub fn find_available_port() -> Option<u16> {
    let listener = StdTcpListener::bind((LOCAL_PROXY_HOST, 0)).ok()?;
    listener.local_addr().ok().map(|addr| addr.port())
}

fn request_proxy_shutdown() {
    if let Some(lock) = PROXY_SHUTDOWN.get() {
        if let Some(tx) = lock.lock().ok().and_then(|mut guard| guard.take()) {
            let _ = tx.send(());
        }
    }
}

fn store_proxy_shutdown(tx: tokio::sync::oneshot::Sender<()>) {
    let lock = PROXY_SHUTDOWN.get_or_init(|| StdMutex::new(None));
    if let Ok(mut guard) = lock.lock() {
        *guard = Some(tx);
    }
}

/// 确保本地拦截服务在 `preferred` 端口监听；若该端口被其他进程占用则改用空闲端口。
pub fn ensure_listening(
    app: tauri::AppHandle,
    fingerprint: String,
    preferred: Option<u16>,
) -> Option<u16> {
    let port = match preferred {
        Some(p) if is_port_available(p) => p,
        Some(_) => find_available_port()?,
        None if local_port() != 0 => {
            warm_credential_cache();
            return Some(local_port());
        }
        None => return start(app, fingerprint),
    };

    if local_port() == port {
        warm_credential_cache();
        return Some(port);
    }

    request_proxy_shutdown();
    let started = start_on_port(app, fingerprint, port);
    if started.is_some() {
        warm_credential_cache();
    }
    started
}

fn register_credentials(manager: Arc<CredentialManager>) {
    if let Ok(mut guard) = CREDENTIALS.lock() {
        *guard = Some(manager);
    }
}

fn active_credentials() -> Option<Arc<CredentialManager>> {
    CREDENTIALS.lock().ok().and_then(|guard| guard.clone())
}

/// 登出或授权失效时清空内存中的临时凭证缓存。
pub fn clear_credential_cache() {
    if let Some(manager) = active_credentials() {
        tauri::async_runtime::spawn(async move {
            manager.invalidate().await;
        });
    }
}

/// 在代理启动或开启注入后预热临时凭证，避免 CLI 并发首包时重复换取导致旧凭证失效。
pub fn warm_credential_cache() {
    if let Some(manager) = active_credentials() {
        tauri::async_runtime::spawn(async move {
            if let Err(error) = manager.get().await {
                eprintln!("预热临时凭证失败: {error}");
            }
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
    /// 合并并发刷新，避免多请求同时换取临时凭证导致先发出的凭证被作废。
    refresh_lock: Mutex<()>,
}

impl CredentialManager {
    fn new(app: tauri::AppHandle, fingerprint: String) -> Self {
        Self {
            app,
            fingerprint,
            cache: Mutex::new(None),
            refresh_lock: Mutex::new(()),
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

        let _refresh_guard = self.refresh_lock.lock().await;

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
            .map_err(|e| {
                if matches!(e, crate::user_api::ApiError::AuthCodeRejected) {
                    let app = self.app.clone();
                    let message = e.message();
                    tauri::async_runtime::spawn(async move {
                        let _ = crate::auth::force_logout_async(&app, &message).await;
                    });
                }
                e.message()
            })?;

        if resp.base_url.is_some() {
            let mut updated = auth;
            updated.api_base_url =
                Some(crate::user_api::resolve_api_base_url(resp.base_url.as_deref()));
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
    crate::user_api::resolve_api_base_url(auth.api_base_url.as_deref())
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
        "opencode" => {
            if rest.starts_with("openai/")
                || rest.starts_with("anthropic/")
                || rest.starts_with("genai/")
            {
                format!("{gateway}/{rest}")
            } else if rest.starts_with("v1/") {
                format!("{gateway}/openai/{rest}")
            } else {
                format!("{gateway}/{rest}")
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
    let started = start_on_port(app, fingerprint, 0);
    if started.is_some() {
        warm_credential_cache();
    }
    started
}

fn start_on_port(app: tauri::AppHandle, fingerprint: String, port: u16) -> Option<u16> {
    let credentials = Arc::new(CredentialManager::new(app.clone(), fingerprint.clone()));
    register_credentials(credentials.clone());

    let state = Arc::new(ProxyState {
        app: app.clone(),
        fingerprint,
        credentials,
        client: reqwest::Client::new(),
    });

    let bind_port = if port == 0 { 0 } else { port };
    let std_listener = match StdTcpListener::bind((LOCAL_PROXY_HOST, bind_port)) {
        Ok(listener) => listener,
        Err(e) => {
            eprintln!("本地拦截服务绑定端口 {bind_port} 失败: {e}");
            return None;
        }
    };
    let bound_port = match std_listener.local_addr() {
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
    PROXY_PORT.store(bound_port, Ordering::Relaxed);

    let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel::<()>();
    store_proxy_shutdown(shutdown_tx);

    tauri::async_runtime::spawn(async move {
        let listener = match TcpListener::from_std(std_listener) {
            Ok(listener) => listener,
            Err(e) => {
                eprintln!("本地拦截服务启动失败: {e}");
                return;
            }
        };

        let app = Router::new().fallback(proxy_handler).with_state(state);
        let serve = axum::serve(listener, app);
        tokio::select! {
            result = serve => {
                if let Err(e) = result {
                    eprintln!("本地拦截服务异常退出: {e}");
                }
            }
            _ = shutdown_rx => {}
        }
    });

    Some(bound_port)
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

fn is_models_list_request(method: &Method, rest: &str) -> bool {
    *method == Method::GET
        && (rest == "v1/models" || rest.ends_with("/v1/models"))
}

fn merge_grayscale_models_into_list(
    tool_id: &str,
    upstream_body: &str,
    additional: &[crate::grayscale_api::GrayscaleModelEntry],
) -> Result<String, String> {
    match tool_id {
        "codex" | "opencode" => {
            crate::grayscale_api::append_openai_models_list(upstream_body, additional)
        }
        "claude" => crate::grayscale_api::append_anthropic_models_list(upstream_body, additional),
        _ => Ok(upstream_body.to_string()),
    }
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
        if SKIP_HEADERS.iter().any(|skip| skip == name) || name == X_API_KEY_HEADER {
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

    let additional_models = if is_models_list_request(&parts.method, rest) {
        crate::grayscale_api::grayscale_additional_models(tool_id)
    } else {
        Vec::new()
    };

    if !additional_models.is_empty() && upstream_resp.status().is_success() {
        let status = upstream_resp.status();
        let resp_headers = upstream_resp.headers().clone();
        let bytes = upstream_resp
            .bytes()
            .await
            .map_err(|e| (StatusCode::BAD_GATEWAY, format!("读取上游模型列表失败: {e}")))?;
        let upstream_text = String::from_utf8_lossy(&bytes);
        let merged = merge_grayscale_models_into_list(tool_id, &upstream_text, &additional_models)
            .unwrap_or_else(|error| {
                eprintln!("合并灰度模型列表失败: {error}");
                upstream_text.into_owned()
            });

        let mut response = Response::builder().status(status);
        for (name, value) in resp_headers.iter() {
            if name == header::CONTENT_LENGTH || name == header::TRANSFER_ENCODING {
                continue;
            }
            response = response.header(name, value);
        }
        return response
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(merged))
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("构建模型列表响应失败: {e}")));
    }

    let usage_ctx = Provider::from_tool_id(tool_id).map(|provider| UsageContext {
        app: state.app.clone(),
        provider,
        model: usage::extract_model(provider, path, body_bytes),
    });

    // OpenCode 走自定义 OpenAI 兼容路由，上游对灰度模型不回报 output_tokens；
    // 代理在转发时按响应文本估算补写，避免 OpenCode 上下文面板显示 0。
    let rewrite_usage = tool_id == "opencode";

    build_upstream_response(upstream_resp, usage_ctx, rewrite_usage)
        .await
        .map_err(|e| (StatusCode::BAD_GATEWAY, e))
}

/// 一次转发的用量采集上下文：仅在成功响应时旁路记录，绝不影响转发本身。
struct UsageContext {
    app: tauri::AppHandle,
    provider: Provider,
    model: String,
}

impl UsageContext {
    /// 解析出 token 用量后计费并持久化（放到阻塞线程，避免拖慢响应）。
    fn record(self, usage: crate::usage::TokenUsage) {
        if usage.is_empty() {
            return;
        }
        let cost = usage::cost_for(&self.model, &usage);
        let app = self.app;
        let model = self.model;
        tauri::async_runtime::spawn_blocking(move || {
            if let Err(e) = crate::store::record_usage(&app, &model, &usage, cost) {
                eprintln!("记录用量失败: {e}");
            }
        });
    }

    fn record_non_streaming(self, body: &[u8]) {
        if !usage::is_successful_response(self.provider, body, false) {
            return;
        }
        if let Some(usage) = usage::parse_usage(self.provider, body) {
            self.record(usage);
        }
    }

    fn record_streaming(self, body: &[u8]) {
        if !usage::is_successful_response(self.provider, body, true) {
            return;
        }
        if let Some(usage) = usage::parse_streaming_usage(self.provider, body) {
            self.record(usage);
        }
    }
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
    usage_ctx: Option<UsageContext>,
    rewrite_usage: bool,
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

    // 仅在成功响应时统计用量；失败响应没有有效 usage。
    let usage_ctx = usage_ctx.filter(|_| status.is_success());
    let rewrite_usage = rewrite_usage && status.is_success();

    if is_streaming_content_type(content_type) {
        if rewrite_usage {
            let stream = rewrite_streaming_usage(upstream_resp.bytes_stream());
            return response
                .body(Body::from_stream(stream))
                .map_err(|_| "构建流式响应失败".to_string());
        }
        let stream = tap_streaming(upstream_resp.bytes_stream(), usage_ctx);
        return response
            .body(Body::from_stream(stream))
            .map_err(|_| "构建流式响应失败".to_string());
    }

    let bytes = upstream_resp
        .bytes()
        .await
        .map_err(|e| format!("读取上游响应失败: {e}"))?;

    if rewrite_usage {
        let rewritten = usage::rewrite_non_streaming_output_usage(&bytes);
        return response
            .body(Body::from(rewritten))
            .map_err(|_| "构建响应失败".to_string());
    }

    if let Some(ctx) = usage_ctx {
        ctx.record_non_streaming(&bytes);
    }

    response
        .body(Body::from(bytes))
        .map_err(|_| "构建响应失败".to_string())
}

/// 流式改写：透传上游分片，仅在终止用量事件上为 OpenCode 补写估算的 output_tokens。
fn rewrite_streaming_usage<S>(
    inner: S,
) -> impl futures_util::Stream<Item = Result<bytes::Bytes, Box<dyn std::error::Error + Send + Sync>>>
where
    S: futures_util::Stream<Item = reqwest::Result<bytes::Bytes>> + Unpin,
{
    struct RewriteState<S> {
        inner: S,
        rewriter: usage::OpencodeUsageRewriter,
        ended: bool,
    }

    let init = RewriteState {
        inner,
        rewriter: usage::OpencodeUsageRewriter::new(),
        ended: false,
    };

    futures_util::stream::unfold(init, |mut state| async move {
        if state.ended {
            return None;
        }
        match state.inner.next().await {
            Some(Ok(chunk)) => {
                let out = state.rewriter.push(&chunk);
                let item: Result<bytes::Bytes, Box<dyn std::error::Error + Send + Sync>> =
                    Ok(bytes::Bytes::from(out));
                Some((item, state))
            }
            Some(Err(e)) => {
                state.ended = true;
                let item: Result<bytes::Bytes, Box<dyn std::error::Error + Send + Sync>> =
                    Err(Box::new(e));
                Some((item, state))
            }
            None => {
                let tail = state.rewriter.finish();
                state.ended = true;
                let item: Result<bytes::Bytes, Box<dyn std::error::Error + Send + Sync>> =
                    Ok(bytes::Bytes::from(tail));
                Some((item, state))
            }
        }
    })
}

/// 旁路累积流式分片：原样透传给 CLI，待流结束时解析用量。
fn tap_streaming<S>(
    inner: S,
    usage_ctx: Option<UsageContext>,
) -> impl futures_util::Stream<Item = Result<bytes::Bytes, Box<dyn std::error::Error + Send + Sync>>>
where
    S: futures_util::Stream<Item = reqwest::Result<bytes::Bytes>> + Unpin,
{
    struct TapState<S> {
        inner: S,
        buffer: Vec<u8>,
        usage_ctx: Option<UsageContext>,
    }

    let init = TapState {
        inner,
        buffer: Vec::new(),
        usage_ctx,
    };

    futures_util::stream::unfold(init, |mut state| async move {
        match state.inner.next().await {
            Some(Ok(chunk)) => {
                if state.usage_ctx.is_some() {
                    state.buffer.extend_from_slice(&chunk);
                }
                let item: Result<bytes::Bytes, Box<dyn std::error::Error + Send + Sync>> =
                    Ok(chunk);
                Some((item, state))
            }
            Some(Err(e)) => {
                // 流传输中断视为失败，丢弃已缓冲的用量。
                state.usage_ctx = None;
                let item: Result<bytes::Bytes, Box<dyn std::error::Error + Send + Sync>> =
                    Err(Box::new(e));
                Some((item, state))
            }
            None => {
                if let Some(ctx) = state.usage_ctx.take() {
                    ctx.record_streaming(&state.buffer);
                }
                None
            }
        }
    })
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
    fn parse_local_proxy_port_extracts_port() {
        assert_eq!(
            parse_local_proxy_port("http://127.0.0.1:51805/codex"),
            Some(51805)
        );
        assert_eq!(parse_local_proxy_port("http://example.com/codex"), None);
    }

    #[test]
    fn local_proxy_base_for_port_formats_url() {
        assert_eq!(
            local_proxy_base_for_port("claude", 58432),
            "http://127.0.0.1:58432/claude"
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
