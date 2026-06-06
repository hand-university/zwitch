use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use tauri::AppHandle;
use tauri_plugin_store::StoreExt;

const STORE_PATH: &str = "zwitch.json";
const LEGACY_STORE_PATH: &str = "zd-switch.json";
const STORE_KEYS: &[&str] = &[
    "auth",
    "settings",
    "config_backup",
    "marketplace_registry",
    "device",
];

type AppStore = std::sync::Arc<tauri_plugin_store::Store<tauri::Wry>>;

pub fn get_store(app: &AppHandle) -> Result<AppStore, String> {
    let store = app
        .store(STORE_PATH)
        .map_err(|e| format!("无法打开存储: {e}"))?;
    migrate_legacy_store(app, &store)?;
    Ok(store)
}

fn migrate_legacy_store(app: &AppHandle, store: &AppStore) -> Result<(), String> {
    if store.get("auth").is_some() {
        return Ok(());
    }

    let Ok(legacy) = app.store(LEGACY_STORE_PATH) else {
        return Ok(());
    };

    let mut migrated = false;
    for key in STORE_KEYS {
        if let Some(value) = legacy.get(key) {
            store.set(*key, value);
            migrated = true;
        }
    }

    if migrated {
        store.save().map_err(|e| e.to_string())?;
    }

    Ok(())
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
    /// 功能总开关：开启时为所有已安装工具注入配置，关闭时还原全部配置。
    #[serde(default)]
    pub proxy_enabled: bool,
    /// 历史字段，已不再使用。
    #[serde(default)]
    pub tool_switches: HashMap<String, bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ConfigBackup {
    pub files: HashMap<String, HashMap<String, String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MarketplaceItemRecord {
    pub remote_id: Option<u64>,
    pub name: String,
    pub platform: String,
    pub item_type: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub version: Option<String>,
    #[serde(default)]
    pub enabled: bool,
    #[serde(default = "default_true")]
    pub remote_available: bool,
    pub origin: String,
    #[serde(default)]
    pub updated_at: Option<String>,
    /// Claude 侧真实插件 ID，如 `z-web@zsdx`
    #[serde(default)]
    pub claude_plugin_keys: Vec<String>,
    /// Codex 侧真实插件 ID，如 `z-web@zsdx`
    #[serde(default)]
    pub codex_plugin_keys: Vec<String>,
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct MarketplaceRegistry {
    #[serde(default)]
    pub items: HashMap<String, MarketplaceItemRecord>,
    /// 用户主动删除的条目，本地扫描时不再自动恢复；云端同步与探索安装会清除。
    #[serde(default)]
    pub suppressed_keys: HashSet<String>,
    #[serde(default)]
    pub last_synced_at: Option<String>,
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
    store.set(
        "auth",
        serde_json::to_value(auth).map_err(|e| e.to_string())?,
    );
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

pub fn load_marketplace_registry(app: &AppHandle) -> Result<MarketplaceRegistry, String> {
    let store = get_store(app)?;
    Ok(store
        .get("marketplace_registry")
        .and_then(|v| serde_json::from_value(v).ok())
        .unwrap_or_default())
}

pub fn save_marketplace_registry(
    app: &AppHandle,
    registry: &MarketplaceRegistry,
) -> Result<(), String> {
    let store = get_store(app)?;
    store.set(
        "marketplace_registry",
        serde_json::to_value(registry).map_err(|e| e.to_string())?,
    );
    store.save().map_err(|e| e.to_string())
}
