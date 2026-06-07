use crate::config::{
    AppConfig, DEVICE_AUTHORIZE_PATH, DEVICE_REVOKE_PATH, DEVICE_TOKEN_PATH,
};
use serde::Deserialize;

/// 接口错误分类，便于上层区分"静默续期"与"强制重新登录"。
#[derive(Debug)]
pub enum ApiError {
    /// access_token 过期/无效，可用授权码换新后重试。
    Unauthorized,
    /// 授权码被后端拒绝（登出/禁用/解绑），需要重新登录。
    AuthCodeRejected,
    /// 其它错误（网络、解析等）。
    Other(String),
}

impl ApiError {
    pub fn message(&self) -> String {
        match self {
            ApiError::Unauthorized => "登录态已过期".to_string(),
            ApiError::AuthCodeRejected => "设备授权已失效，请重新登录".to_string(),
            ApiError::Other(msg) => msg.clone(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct DeepLinkAuth {
    pub access_token: String,
    pub api_base_url: Option<String>,
}

pub fn parse_deep_link_auth(
    url: &str,
    expected_scheme: &str,
    expected_host: &str,
) -> Option<DeepLinkAuth> {
    let parsed = url::Url::parse(url).ok()?;
    if parsed.scheme() != expected_scheme {
        return None;
    }

    let host_matches = parsed.host_str() == Some(expected_host);
    let path_matches = parsed.path().trim_matches('/') == expected_host;
    if !host_matches && !path_matches {
        return None;
    }

    let mut access_token = None;
    let mut api_base_url = None;

    for (key, value) in parsed.query_pairs() {
        match key.as_ref() {
            "access_token" => {
                let token = value.trim();
                if !token.is_empty() {
                    access_token = Some(token.to_string());
                }
            }
            "base_url" => {
                api_base_url = normalize_api_base_url(value.trim());
            }
            _ => {}
        }
    }

    Some(DeepLinkAuth {
        access_token: access_token?,
        api_base_url,
    })
}

pub fn normalize_api_base_url(raw: &str) -> Option<String> {
    if raw.is_empty() {
        return None;
    }
    let parsed = url::Url::parse(raw).ok()?;
    if parsed.scheme() != "http" && parsed.scheme() != "https" {
        return None;
    }
    let host = parsed.host_str()?;

    let authority = match parsed.port() {
        Some(port) => format!("{host}:{port}"),
        None => host.to_string(),
    };

    let path = parsed.path().trim_end_matches('/');
    let mut base = format!("{}://{}", parsed.scheme(), authority);
    if !path.is_empty() && path != "/" {
        base.push_str(path);
    }

    #[cfg(debug_assertions)]
    if base == "http://localhost" || base == "https://localhost" {
        base = crate::config::API_BASE_URL.to_string();
    }

    Some(enforce_https_for_remote(base))
}

fn is_origin_only_api_base(url: &str) -> bool {
    let Ok(parsed) = url::Url::parse(url) else {
        return false;
    };
    let path = parsed.path().trim_end_matches('/');
    path.is_empty() || path == "/"
}

fn is_dev_only_api_base(url: &str) -> bool {
    let Ok(parsed) = url::Url::parse(url) else {
        return false;
    };
    matches!(
        parsed.host_str(),
        Some("localhost") | Some("127.0.0.1") | Some("::1")
    )
}

fn url_hosts_match(a: &url::Url, b: &url::Url) -> bool {
    // 仅比较 host；release 会将 http 升级为 https，默认端口不同不应影响路径补全。
    a.host_str() == b.host_str()
}

/// release 构建将远程 http 地址升级为 https（本地开发地址除外）。
#[cfg(not(debug_assertions))]
fn enforce_https_for_remote(url: String) -> String {
    if is_dev_only_api_base(&url) {
        return url;
    }
    let Ok(mut parsed) = url::Url::parse(&url) else {
        return url;
    };
    if parsed.scheme() != "http" {
        return url;
    }
    let _ = parsed.set_scheme("https");
    if parsed.port() == Some(80) {
        let _ = parsed.set_port(None);
    }
    parsed.to_string()
}

#[cfg(debug_assertions)]
fn enforce_https_for_remote(url: String) -> String {
    url
}

fn normalize_known_production_api_base(url: &str) -> Option<String> {
    let Ok(parsed) = url::Url::parse(url) else {
        return None;
    };
    if parsed.host_str() == Some("ft-app.wxhand.com") {
        return Some(crate::config::API_BASE_URL_PROD.to_string());
    }
    None
}

/// 解析实际请求的 API 根地址：release 构建忽略本地开发地址。
pub fn resolve_api_base_url(stored: Option<&str>) -> String {
    let default = AppConfig::default().api_base_url;
    let Some(normalized) = stored.and_then(normalize_api_base_url) else {
        return enforce_https_for_remote(default);
    };

    if let Some(prod_base) = normalize_known_production_api_base(&normalized) {
        return enforce_https_for_remote(prod_base);
    }

    if is_dev_only_api_base(&normalized) {
        #[cfg(not(debug_assertions))]
        {
            return enforce_https_for_remote(default);
        }
    }

    // 设备 token 返回的 base_url 可能只有域名（如 http://ft-app.wxhand.com），需补全 /zai。
    if is_origin_only_api_base(&normalized) {
        let Ok(default_url) = url::Url::parse(&default) else {
            return enforce_https_for_remote(normalized);
        };
        let Ok(stored_url) = url::Url::parse(&normalized) else {
            return enforce_https_for_remote(normalized);
        };
        if url_hosts_match(&default_url, &stored_url) {
            return enforce_https_for_remote(default);
        }
    }

    enforce_https_for_remote(normalized)
}

#[derive(Debug, Deserialize)]
pub struct UserMeResponse {
    pub api_key: String,
    pub user: UserInfo,
    #[serde(default)]
    pub dingtalk: Option<DingtalkInfo>,
}

#[derive(Debug, Deserialize)]
pub struct UserInfo {
    pub name: Option<String>,
    pub display_name: Option<String>,
    pub avatar: Option<String>,
    pub display_avatar: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct DingtalkInfo {
    pub profile: Option<DingtalkProfile>,
    #[serde(default)]
    pub departments: Vec<Department>,
}

#[derive(Debug, Deserialize)]
pub struct DingtalkProfile {
    pub name: Option<String>,
    pub avatar: Option<String>,
    pub title: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct Department {
    pub name: Option<String>,
}

#[derive(Debug, Clone)]
pub struct UserProfile {
    pub api_key: String,
    pub name: String,
    pub avatar: Option<String>,
    pub department: Option<String>,
    pub title: Option<String>,
}

impl UserMeResponse {
    pub fn into_profile(self) -> UserProfile {
        let name = self
            .user
            .display_name
            .or(self.user.name)
            .or_else(|| {
                self.dingtalk
                    .as_ref()
                    .and_then(|d| d.profile.as_ref())
                    .and_then(|p| p.name.clone())
            })
            .unwrap_or_else(|| "用户".into());

        let avatar = pick_non_empty([
            self.user.display_avatar.as_deref(),
            self.user.avatar.as_deref(),
            self.dingtalk
                .as_ref()
                .and_then(|d| d.profile.as_ref())
                .and_then(|p| p.avatar.as_deref()),
        ]);

        let department = self
            .dingtalk
            .as_ref()
            .and_then(|d| d.departments.first())
            .and_then(|dept| dept.name.clone());

        let title = self
            .dingtalk
            .as_ref()
            .and_then(|d| d.profile.as_ref())
            .and_then(|p| p.title.clone());

        UserProfile {
            api_key: self.api_key,
            name,
            avatar,
            department,
            title,
        }
    }
}

fn pick_non_empty(values: [Option<&str>; 3]) -> Option<String> {
    values
        .into_iter()
        .flatten()
        .find(|value| !value.is_empty())
        .map(str::to_string)
}

pub async fn fetch_user_me(
    session_token: &str,
    api_base_url: &str,
) -> Result<UserProfile, ApiError> {
    let url = format!("{}/api/aone/users/me", api_base_url.trim_end_matches('/'));

    let response = reqwest::Client::new()
        .get(&url)
        .header("Authorization", format!("Bearer {session_token}"))
        .header("Cookie", format!("token={session_token}"))
        .send()
        .await
        .map_err(|e| ApiError::Other(format!("请求用户信息失败: {e}")))?;

    let status = response.status();
    let content_type = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or("unknown")
        .to_string();

    if status == reqwest::StatusCode::UNAUTHORIZED {
        return Err(ApiError::Unauthorized);
    }

    let body = response
        .text()
        .await
        .map_err(|e| ApiError::Other(format!("读取用户信息失败: {e}（请求地址: {url}）")))?;

    if !status.is_success() {
        return Err(ApiError::Other(format!(
            "获取用户信息失败: HTTP {status}（请求地址: {url}）"
        )));
    }

    serde_json::from_str::<UserMeResponse>(&body)
        .map(|parsed| parsed.into_profile())
        .map_err(|e| ApiError::Other(format!(
            "解析用户信息失败: {e}（请求地址: {url}，Content-Type: {content_type}）"
        )))
}

#[derive(Debug, Deserialize)]
struct AuthorizeResponse {
    authorization_code: String,
}

/// 用 access_token + 设备指纹向后端注册设备，换取持久授权码。
pub async fn register_device(
    access_token: &str,
    device_fingerprint: &str,
    api_base_url: &str,
) -> Result<String, ApiError> {
    let url = format!(
        "{}{}",
        api_base_url.trim_end_matches('/'),
        DEVICE_AUTHORIZE_PATH
    );

    let response = reqwest::Client::new()
        .post(url)
        .header("Authorization", format!("Bearer {access_token}"))
        .header("Cookie", format!("token={access_token}"))
        .json(&serde_json::json!({
            "device_fingerprint": device_fingerprint,
            "device_name": device_name(),
        }))
        .send()
        .await
        .map_err(|e| ApiError::Other(format!("设备注册请求失败: {e}")))?;

    if response.status() == reqwest::StatusCode::UNAUTHORIZED {
        return Err(ApiError::Unauthorized);
    }
    if !response.status().is_success() {
        return Err(ApiError::Other(format!(
            "设备注册失败: HTTP {}",
            response.status()
        )));
    }

    response
        .json::<AuthorizeResponse>()
        .await
        .map(|body| body.authorization_code)
        .map_err(|e| ApiError::Other(format!("解析授权码失败: {e}")))
}

#[derive(Debug, Deserialize)]
pub struct DeviceTokenResponse {
    #[serde(default)]
    pub credential: Option<String>,
    pub access_token: String,
    #[serde(default)]
    pub token_type: Option<String>,
    #[serde(default)]
    pub expires_in: Option<u64>,
    #[serde(default)]
    pub expires_at: Option<String>,
    #[serde(default)]
    pub base_url: Option<String>,
}

impl DeviceTokenResponse {
    /// 24h 临时凭证，`access_token` 为向后兼容别名。
    pub fn credential(&self) -> &str {
        self.credential
            .as_deref()
            .filter(|value| !value.is_empty())
            .unwrap_or(&self.access_token)
    }
}

/// 用授权码 + 设备指纹换取新的短期 access_token。
pub async fn exchange_device_code(
    authorization_code: &str,
    device_fingerprint: &str,
    api_base_url: &str,
) -> Result<DeviceTokenResponse, ApiError> {
    let url = format!(
        "{}{}",
        api_base_url.trim_end_matches('/'),
        DEVICE_TOKEN_PATH
    );

    let response = reqwest::Client::new()
        .post(url)
        .json(&serde_json::json!({
            "authorization_code": authorization_code,
            "device_fingerprint": device_fingerprint,
        }))
        .send()
        .await
        .map_err(|e| ApiError::Other(format!("换取 token 请求失败: {e}")))?;

    let status = response.status();
    if status == reqwest::StatusCode::UNAUTHORIZED || status == reqwest::StatusCode::FORBIDDEN {
        return Err(ApiError::AuthCodeRejected);
    }
    if !status.is_success() {
        return Err(ApiError::Other(format!("换取 token 失败: HTTP {status}")));
    }

    response
        .json::<DeviceTokenResponse>()
        .await
        .map_err(|e| ApiError::Other(format!("解析 token 失败: {e}")))
}

/// 登出时吊销授权码，让后端解除设备绑定。失败不阻断本地登出。
pub async fn revoke_device(
    authorization_code: &str,
    device_fingerprint: &str,
    api_base_url: &str,
) -> Result<(), ApiError> {
    let url = format!(
        "{}{}",
        api_base_url.trim_end_matches('/'),
        DEVICE_REVOKE_PATH
    );

    reqwest::Client::new()
        .post(url)
        .json(&serde_json::json!({
            "authorization_code": authorization_code,
            "device_fingerprint": device_fingerprint,
        }))
        .send()
        .await
        .map_err(|e| ApiError::Other(format!("吊销授权码请求失败: {e}")))?;

    Ok(())
}

fn device_name() -> String {
    let host = std::env::var("HOSTNAME")
        .ok()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| std::env::consts::OS.to_string());
    format!("zwitch ({host})")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_api_base_url_keeps_path_prefix() {
        assert_eq!(
            normalize_api_base_url("https://ft-app.wxhand.com/zai/"),
            Some("https://ft-app.wxhand.com/zai".to_string())
        );
        assert_eq!(
            normalize_api_base_url("https://ft-app.wxhand.com/zai"),
            Some("https://ft-app.wxhand.com/zai".to_string())
        );
    }

    #[test]
    fn normalize_api_base_url_keeps_port() {
        assert_eq!(
            normalize_api_base_url("http://localhost:8080/zai"),
            Some("http://localhost:8080/zai".to_string())
        );
    }

    #[cfg(not(debug_assertions))]
    #[test]
    fn resolve_api_base_url_restores_missing_path_prefix_in_release() {
        assert_eq!(
            resolve_api_base_url(Some("https://ft-app.wxhand.com")),
            AppConfig::default().api_base_url
        );
        assert_eq!(
            resolve_api_base_url(Some("http://ft-app.wxhand.com")),
            AppConfig::default().api_base_url
        );
    }

    #[cfg(not(debug_assertions))]
    #[test]
    fn resolve_api_base_url_upgrades_http_to_https_in_release() {
        assert_eq!(
            resolve_api_base_url(Some("http://ft-app.wxhand.com/zai")),
            "https://ft-app.wxhand.com/zai"
        );
    }
}
