use serde::{Deserialize, Serialize};

/// Dev 登录页
pub const LOGIN_BASE_URL: &str = "http://localhost:8080";
/// 线上登录页
pub const LOGIN_BASE_URL_PROD: &str = "https://ft-app.wxhand.com/zai";

/// Dev API
pub const API_BASE_URL: &str = "http://localhost:8080";
/// 线上 API
pub const API_BASE_URL_PROD: &str = "https://ft-app.wxhand.com/zai";

/// Dev 上游代理地址（本地拦截服务最终转发的目标）
pub const PROXY_BASE_URL: &str = "http://localhost:8080/v1";
/// 线上上游代理地址
pub const PROXY_BASE_URL_PROD: &str = "https://ft-app.wxhand.com/zai/v1";

/// 本地拦截服务监听地址。各 AI CLI 的 base_url 会被指向
/// `http://{LOCAL_PROXY_HOST}:{动态端口}/{tool_id}`。
/// CLI 无需配置 API Key；本地服务用设备码换取 24h 临时凭证
/// （`Authorization: Bearer bf-tmp-...` + `X-Device-Fingerprint`）后转发上游。
/// 端口在启动时动态分配（见 `proxy::start`）。
pub const LOCAL_PROXY_HOST: &str = "127.0.0.1";
/// 注入设备指纹的请求头名称。
pub const DEVICE_FINGERPRINT_HEADER: &str = "X-Device-Fingerprint";

/// 打开登录页时携带的来源标识
pub const LOGIN_SOURCE: &str = "zwitch";

/// Deep link scheme，登录成功后回调形如 zwitch://open?access_token=xxx
pub const DEEPLINK_SCHEME: &str = "zwitch";
pub const DEEPLINK_HOST: &str = "open";

/// 设备授权码接口契约（需后端支持）。
///
/// 1. 注册设备：`POST {api_base}{DEVICE_AUTHORIZE_PATH}`
///    - Header: `Authorization: Bearer {access_token}`
///    - Body(JSON): `{ "device_fingerprint": "...", "device_name": "..." }`
///    - 返回(JSON): `{ "authorization_code": "..." }`
///    - 后端记录 `authorization_code ↔ device_fingerprint ↔ user_id`
/// 2. 换取临时凭证：`POST {api_base}{DEVICE_TOKEN_PATH}`
///    - Body(JSON): `{ "authorization_code": "...", "device_fingerprint": "..." }`
///    - 返回(JSON): `{ "credential": "bf-tmp-...", "expires_in": 86400, "base_url"?: "..." }`
///    - 授权码失效（登出/禁用）返回 401/403
/// 3. 吊销：`POST {api_base}{DEVICE_REVOKE_PATH}`
///    - Body(JSON): `{ "authorization_code": "...", "device_fingerprint": "..." }`
pub const DEVICE_AUTHORIZE_PATH: &str = "/api/aone/devices/authorize";
pub const DEVICE_TOKEN_PATH: &str = "/api/aone/devices/token";
pub const DEVICE_REVOKE_PATH: &str = "/api/aone/devices/revoke";

/// GitHub 仓库 hand-university/zwitch（Actions 发版后由 tauri-action 上传 latest.json）
/// Tauri updater 静态清单，指向 GitHub Releases 最新版
pub const UPDATE_ENDPOINT_PROD: &str =
    "https://github.com/hand-university/zwitch/releases/latest/download/latest.json";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppConfig {
    pub login_base_url: String,
    pub api_base_url: String,
    pub proxy_base_url: String,
    pub login_source: String,
    pub deeplink_scheme: String,
    pub deeplink_host: String,
}

impl Default for AppConfig {
    fn default() -> Self {
        let is_dev = cfg!(debug_assertions);
        Self {
            login_base_url: if is_dev {
                LOGIN_BASE_URL.into()
            } else {
                LOGIN_BASE_URL_PROD.into()
            },
            api_base_url: if is_dev {
                API_BASE_URL.into()
            } else {
                API_BASE_URL_PROD.into()
            },
            proxy_base_url: if is_dev {
                PROXY_BASE_URL.into()
            } else {
                PROXY_BASE_URL_PROD.into()
            },
            login_source: LOGIN_SOURCE.into(),
            deeplink_scheme: DEEPLINK_SCHEME.into(),
            deeplink_host: DEEPLINK_HOST.into(),
        }
    }
}

impl AppConfig {
    pub fn deeplink_callback_url(&self) -> String {
        format!("{}://{}", self.deeplink_scheme, self.deeplink_host)
    }

    /// release 使用经 https 规范化后的公共 base URL。
    pub fn resolved_public_base_url(&self) -> String {
        crate::user_api::resolve_api_base_url(Some(self.api_base_url.as_str()))
    }

    /// 打开系统浏览器登录页：`{BASE_URL}/login?source=zwitch`
    pub fn login_url(&self) -> Result<String, String> {
        let base = self.resolved_public_base_url();
        let mut url = url::Url::parse(&format!("{}/login", base.trim_end_matches('/')))
            .map_err(|e| format!("无效的登录 URL: {e}"))?;
        url.query_pairs_mut().append_pair("source", &self.login_source);
        Ok(url.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn login_url_uses_https_base_in_release() {
        let config = AppConfig::default();
        let url = config.login_url().expect("login url");
        assert!(url.contains("source=zwitch"));
        #[cfg(not(debug_assertions))]
        {
            assert_eq!(
                url,
                "https://ft-app.wxhand.com/zai/login?source=zwitch",
                "unexpected login url"
            );
        }
        #[cfg(debug_assertions)]
        {
            assert!(
                url.starts_with("http://localhost:8080/login"),
                "unexpected login url: {url}"
            );
        }
    }
}
