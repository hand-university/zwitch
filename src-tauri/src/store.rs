use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use tauri::AppHandle;
use tauri_plugin_store::StoreExt;

const STORE_PATH: &str = "zd-switch.json";

type AppStore = std::sync::Arc<tauri_plugin_store::Store<tauri::Wry>>;

pub fn get_store(app: &AppHandle) -> Result<AppStore, String> {
    app.store(STORE_PATH)
        .map_err(|e| format!("无法打开存储: {e}"))
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct StoredAuth {
    /// 短期工作态 token，可随时被授权码换取的新 token 覆盖。
    pub access_token: Option<String>,
    /// 桌面端长期凭证：设备授权码，用于独立于 web 续期 access_token。
    #[serde(default)]
    pub authorization_code: Option<String>,
    pub api_key: Option<String>,
    pub api_base_url: Option<String>,
    pub user_name: Option<String>,
    pub avatar: Option<String>,
    pub department: Option<String>,
    pub title: Option<String>,
    pub cookies: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct StoredSettings {
    /// 功能总开关：关闭时所有工具配置都会被还原，无视各自的开关。
    #[serde(default)]
    pub proxy_enabled: bool,
    #[serde(default)]
    pub tool_switches: HashMap<String, bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ConfigBackup {
    pub files: HashMap<String, HashMap<String, String>>,
}

pub fn load_auth(app: &AppHandle) -> Result<StoredAuth, String> {
    let store = get_store(app)?;
    Ok(store
        .get("auth")
        .and_then(|v| serde_json::from_value(v).ok())
        .unwrap_or_default())
}

pub fn save_auth(app: &AppHandle, auth: &StoredAuth) -> Result<(), String> {
    let store = get_store(app)?;
    store.set("auth", serde_json::to_value(auth).map_err(|e| e.to_string())?);
    store.save().map_err(|e| e.to_string())
}

pub fn clear_auth(app: &AppHandle) -> Result<(), String> {
    save_auth(app, &StoredAuth::default())
}

pub fn load_settings(app: &AppHandle) -> Result<StoredSettings, String> {
    let store = get_store(app)?;
    Ok(store
        .get("settings")
        .and_then(|v| serde_json::from_value(v).ok())
        .unwrap_or_default())
}

pub fn save_settings(app: &AppHandle, settings: &StoredSettings) -> Result<(), String> {
    let store = get_store(app)?;
    store.set(
        "settings",
        serde_json::to_value(settings).map_err(|e| e.to_string())?,
    );
    store.save().map_err(|e| e.to_string())
}

pub fn load_backup(app: &AppHandle) -> Result<ConfigBackup, String> {
    let store = get_store(app)?;
    Ok(store
        .get("config_backup")
        .and_then(|v| serde_json::from_value(v).ok())
        .unwrap_or_default())
}

pub fn save_backup(app: &AppHandle, backup: &ConfigBackup) -> Result<(), String> {
    let store = get_store(app)?;
    store.set(
        "config_backup",
        serde_json::to_value(backup).map_err(|e| e.to_string())?,
    );
    store.save().map_err(|e| e.to_string())
}
