use crate::config::AppConfig;
use crate::device;
use crate::store::{clear_auth, load_auth, save_auth, StoredAuth};
use crate::user_api::{
    exchange_device_code, fetch_user_me, parse_deep_link_auth, register_device, revoke_device,
    ApiError, UserProfile,
};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_opener::OpenerExt;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthState {
    pub is_logged_in: bool,
    pub api_key: Option<String>,
    pub user_name: Option<String>,
    pub avatar: Option<String>,
    pub department: Option<String>,
    pub title: Option<String>,
}

pub fn get_auth_state(app: &AppHandle) -> Result<AuthState, String> {
    let auth = load_auth(app)?;
    Ok(auth_to_state(&auth))
}

fn auth_to_state(auth: &StoredAuth) -> AuthState {
    AuthState {
        is_logged_in: auth.authorization_code.is_some(),
        api_key: None,
        user_name: auth.user_name.clone(),
        avatar: auth.avatar.clone(),
        department: auth.department.clone(),
        title: auth.title.clone(),
    }
}

fn resolve_api_base_url(auth: &StoredAuth) -> String {
    crate::user_api::resolve_api_base_url(auth.api_base_url.as_deref())
}

fn clear_logged_in_state(app: &AppHandle) -> Result<AuthState, String> {
    crate::proxy::clear_credential_cache();
    let _ = crate::cli_tools::apply_config_injection(app);
    clear_auth(app)?;
    emit_auth_changed(app)
}

pub fn logout(app: &AppHandle) -> Result<(), String> {
    revoke_current_device(app);
    clear_logged_in_state(app)?;
    Ok(())
}

/// 授权失效时清理本地登录态并通知前端回到登录页。
pub fn force_logout(app: &AppHandle, reason: &str) -> Result<AuthState, String> {
    let state = clear_logged_in_state(app)?;
    emit_login_failed(app, reason);
    Ok(state)
}

/// 会话失效时强制登出并返回错误文案（已有登录态才清理，避免未登录时误报）。
pub fn fail_auth_session(app: &AppHandle, reason: impl Into<String>) -> String {
    let reason = reason.into();
    if load_auth(app)
        .ok()
        .and_then(|auth| auth.authorization_code)
        .is_some()
    {
        let _ = force_logout(app, &reason);
    }
    reason
}

/// 后台尽力吊销授权码，让后端解除设备绑定；失败不阻断本地登出。
fn revoke_current_device(app: &AppHandle) {
    let Ok(auth) = load_auth(app) else {
        return;
    };
    let Some(code) = auth.authorization_code.clone() else {
        return;
    };
    let Ok(fingerprint) = device::get_or_create_fingerprint(app) else {
        return;
    };
    let base = resolve_api_base_url(&auth);
    tauri::async_runtime::spawn(async move {
        let _ = revoke_device(&code, &fingerprint, &base).await;
    });
}

pub fn open_login_window(app: &AppHandle) -> Result<(), String> {
    let config = AppConfig::default();
    let login_url = config.login_url()?;

    app.opener()
        .open_url(login_url, None::<&str>)
        .map_err(|e| format!("无法打开系统浏览器: {e}"))?;

    Ok(())
}

pub fn handle_deep_link(app: &AppHandle, url: &str) -> Result<(), String> {
    let config = AppConfig::default();
    let payload = parse_deep_link_auth(url, &config.deeplink_scheme, &config.deeplink_host)
        .ok_or_else(|| {
            format!(
                "无效的 Deep link，期望格式: {}?access_token=xxx",
                config.deeplink_callback_url()
            )
        })?;

    let app_handle = app.clone();
    tauri::async_runtime::spawn(async move {
        if let Err(error) = complete_login(&app_handle, payload).await {
            emit_login_failed(&app_handle, &error);
        }
    });

    Ok(())
}

pub async fn refresh_user_profile(app: &AppHandle) -> Result<AuthState, String> {
    match sync_profile(app).await {
        Ok(_) => emit_auth_changed(app),
        Err(ApiError::AuthCodeRejected) => {
            Ok(force_logout(
                app,
                &ApiError::AuthCodeRejected.message(),
            )?)
        }
        Err(ApiError::Unauthorized) => Ok(force_logout(
            app,
            &ApiError::Unauthorized.message(),
        )?),
        Err(e) => Err(e.message()),
    }
}

/// 用当前 access_token 拉取用户资料；若 token 过期则用授权码静默续期后重试。
async fn sync_profile(app: &AppHandle) -> Result<(), ApiError> {
    let auth = load_auth(app).map_err(ApiError::Other)?;
    let base = resolve_api_base_url(&auth);

    let attempt = match auth.access_token.as_deref() {
        Some(token) => fetch_user_me(token, &base).await,
        None => Err(ApiError::Unauthorized),
    };

    let (session_token, profile, base_after) = match attempt {
        Ok(profile) => (
            auth.access_token.clone().unwrap_or_default(),
            profile,
            base.clone(),
        ),
        Err(ApiError::Unauthorized) => {
            let token = refresh_access_token(app, &auth, &base).await?;
            // 续期可能更新了 base_url，重新读取后再请求。
            let refreshed = load_auth(app).map_err(ApiError::Other)?;
            let base = resolve_api_base_url(&refreshed);
            let profile = fetch_user_me(&token, &base).await?;
            (token, profile, base)
        }
        Err(e) => return Err(e),
    };

    save_profile(app, &session_token, Some(base_after.as_str()), profile)
        .map_err(ApiError::Other)?;

    Ok(())
}

/// 获取可用于 API 请求的 session token，必要时静默续期。
pub async fn resolve_session_token(app: &AppHandle) -> Result<String, String> {
    let auth = load_auth(app)?;
    if auth.authorization_code.is_none() {
        return Err(fail_auth_session(app, "请先登录"));
    }
    let base = resolve_api_base_url(&auth);
    if let Some(token) = auth.access_token.clone() {
        return Ok(token);
    }
    match refresh_access_token(app, &auth, &base).await {
        Ok(token) => Ok(token),
        Err(ApiError::AuthCodeRejected) => {
            force_logout(app, &ApiError::AuthCodeRejected.message())?;
            Err(ApiError::AuthCodeRejected.message())
        }
        Err(e) => Err(e.message()),
    }
}

/// 强制用授权码换取新的 access_token。
pub async fn force_refresh_session_token(app: &AppHandle) -> Result<String, ApiError> {
    let auth = load_auth(app).map_err(ApiError::Other)?;
    let base = resolve_api_base_url(&auth);
    refresh_access_token(app, &auth, &base).await
}

/// 用持久授权码 + 设备指纹换取新的 access_token 并落盘。
async fn refresh_access_token(
    app: &AppHandle,
    auth: &StoredAuth,
    base: &str,
) -> Result<String, ApiError> {
    let code = auth
        .authorization_code
        .clone()
        .ok_or(ApiError::AuthCodeRejected)?;
    let fingerprint = device::get_or_create_fingerprint(app).map_err(ApiError::Other)?;

    let resp = exchange_device_code(&code, &fingerprint, base).await?;

    let mut updated = load_auth(app).map_err(ApiError::Other)?;
    updated.access_token = Some(resp.access_token.clone());
    if resp.base_url.is_some() {
        updated.api_base_url =
            Some(crate::user_api::resolve_api_base_url(resp.base_url.as_deref()));
    }
    save_auth(app, &updated).map_err(ApiError::Other)?;

    Ok(resp.access_token)
}

/// 登录回调里的 base_url 可能为 http 或缺少 /zai；release 统一使用应用配置的 https 地址。
fn resolve_login_api_base_url(deeplink_base: Option<&str>) -> String {
    #[cfg(not(debug_assertions))]
    {
        let _ = deeplink_base;
        return crate::user_api::resolve_api_base_url(None);
    }
    #[cfg(debug_assertions)]
    {
        crate::user_api::resolve_api_base_url(deeplink_base)
    }
}

async fn complete_login(
    app: &AppHandle,
    payload: crate::user_api::DeepLinkAuth,
) -> Result<(), String> {
    let api_base_url = resolve_login_api_base_url(payload.api_base_url.as_deref());

    let fingerprint = device::get_or_create_fingerprint(app)?;

    // 用一次性 access_token 注册设备，换取桌面端长期持有的授权码。
    let authorization_code = register_device(&payload.access_token, &fingerprint, &api_base_url)
        .await
        .map_err(|e| e.message())?;

    let profile = fetch_user_me(&payload.access_token, &api_base_url)
        .await
        .map_err(|e| e.message())?;

    // 先持久化授权码与 base_url，再保存资料（save_profile 会保留授权码）。
    let mut auth = load_auth(app)?;
    auth.authorization_code = Some(authorization_code);
    auth.api_base_url = Some(api_base_url.clone());
    save_auth(app, &auth)?;

    save_profile(app, &payload.access_token, Some(&api_base_url), profile)?;
    emit_auth_changed(app)?;
    Ok(())
}

/// 合并保存资料与 access_token，保留 authorization_code / cookies 等既有字段。
fn save_profile(
    app: &AppHandle,
    session_token: &str,
    api_base_url: Option<&str>,
    profile: UserProfile,
) -> Result<(), String> {
    let mut auth = load_auth(app)?;
    auth.access_token = Some(session_token.to_string());
    if let Some(base) = api_base_url {
        auth.api_base_url = Some(base.to_string());
    }
    auth.user_name = Some(profile.name);
    auth.avatar = profile.avatar;
    auth.department = profile.department;
    auth.title = profile.title;
    save_auth(app, &auth)
}

fn emit_auth_changed(app: &AppHandle) -> Result<AuthState, String> {
    let state = get_auth_state(app)?;
    app.emit("auth-changed", state.clone())
        .map_err(|e| format!("事件发送失败: {e}"))?;
    Ok(state)
}

pub fn emit_login_failed(app: &AppHandle, message: &str) {
    let _ = app.emit("login-failed", message.to_string());
}

pub fn setup_deep_link(app: &AppHandle) -> Result<(), String> {
    use tauri_plugin_deep_link::DeepLinkExt;

    let handle = app.clone();
    app.deep_link().on_open_url(move |event| {
        focus_main_window(&handle);
        for url in event.urls() {
            if let Err(error) = handle_deep_link(&handle, url.as_ref()) {
                emit_login_failed(&handle, &error);
            }
        }
    });

    if let Some(urls) = app
        .deep_link()
        .get_current()
        .map_err(|e| format!("读取 Deep link 失败: {e}"))?
    {
        focus_main_window(app);
        for url in urls {
            if let Err(error) = handle_deep_link(app, url.as_ref()) {
                emit_login_failed(app, &error);
            }
        }
    }

    Ok(())
}

fn focus_main_window(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
    }
}
