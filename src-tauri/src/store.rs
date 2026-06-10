use crate::usage::TokenUsage;
use chrono::Local;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::sync::Mutex;
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
    "usage",
];

/// 序列化用量写入：避免并发请求的 read-modify-write 互相覆盖。
static USAGE_LOCK: Mutex<()> = Mutex::new(());

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
    /// 浏览器登录后的 Aone 会话 token，供灰度模型、用户资料等管理接口使用。
    #[serde(default)]
    pub session_token: Option<String>,
    /// 设备临时凭证（`bf-tmp-...`），供本地代理转发 AI 请求使用。
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
    /// 功能总开关：开启时为已启用的工具注入配置，关闭时还原全部配置。
    #[serde(default)]
    pub proxy_enabled: bool,
    /// 各 CLI 配置注入开关，缺省为关闭；显式 `true` 表示用户已手动开启。
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

/// 一类聚合实体（某天 / 某模型 / 全局）累计的 token、费用与请求次数。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct UsageTotals {
    #[serde(default)]
    pub input_tokens: u64,
    #[serde(default)]
    pub output_tokens: u64,
    #[serde(default)]
    pub cache_creation_tokens: u64,
    #[serde(default)]
    pub cache_read_tokens: u64,
    #[serde(default)]
    pub cost_usd: f64,
    #[serde(default)]
    pub requests: u64,
}

impl UsageTotals {
    pub fn total_tokens(&self) -> u64 {
        self.input_tokens + self.output_tokens + self.cache_creation_tokens + self.cache_read_tokens
    }

    fn add(&mut self, usage: &TokenUsage, cost_usd: f64) {
        self.input_tokens += usage.input_tokens;
        self.output_tokens += usage.output_tokens;
        self.cache_creation_tokens += usage.cache_creation_tokens;
        self.cache_read_tokens += usage.cache_read_tokens;
        self.cost_usd += cost_usd;
        self.requests += 1;
    }
}

/// 某一天的用量：总计 + 按模型拆分。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DailyUsage {
    #[serde(flatten)]
    pub totals: UsageTotals,
    #[serde(default)]
    pub models: HashMap<String, UsageTotals>,
}

/// 用量持久化根：按本地日期（YYYY-MM-DD）聚合，并维护全局与按模型总计。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct UsageStore {
    #[serde(default)]
    pub days: HashMap<String, DailyUsage>,
    #[serde(default)]
    pub models: HashMap<String, UsageTotals>,
    #[serde(default)]
    pub total: UsageTotals,
    #[serde(default)]
    pub first_recorded_at: Option<String>,
    #[serde(default)]
    pub last_recorded_at: Option<String>,
}

pub fn load_usage(app: &AppHandle) -> Result<UsageStore, String> {
    let store = get_store(app)?;
    Ok(store
        .get("usage")
        .and_then(|v| serde_json::from_value(v).ok())
        .unwrap_or_default())
}

fn save_usage(app: &AppHandle, usage: &UsageStore) -> Result<(), String> {
    let store = get_store(app)?;
    store.set(
        "usage",
        serde_json::to_value(usage).map_err(|e| e.to_string())?,
    );
    store.save().map_err(|e| e.to_string())
}

/// 记录一次请求的用量：按当天日期与模型聚合并累加费用。
pub fn record_usage(
    app: &AppHandle,
    model: &str,
    usage: &TokenUsage,
    cost_usd: f64,
) -> Result<(), String> {
    if usage.is_empty() {
        return Ok(());
    }
    let _guard = USAGE_LOCK.lock().map_err(|_| "用量锁中毒".to_string())?;

    let now = Local::now();
    let date = now.format("%Y-%m-%d").to_string();
    let timestamp = now.to_rfc3339();
    let model_key = if model.is_empty() { "unknown" } else { model };

    let mut store = load_usage(app)?;

    store.total.add(usage, cost_usd);
    store
        .models
        .entry(model_key.to_string())
        .or_default()
        .add(usage, cost_usd);

    let day = store.days.entry(date).or_default();
    day.totals.add(usage, cost_usd);
    day.models
        .entry(model_key.to_string())
        .or_default()
        .add(usage, cost_usd);

    if store.first_recorded_at.is_none() {
        store.first_recorded_at = Some(timestamp.clone());
    }
    store.last_recorded_at = Some(timestamp);

    save_usage(app, &store)
}

/// 清空全部用量统计。
pub fn clear_usage(app: &AppHandle) -> Result<(), String> {
    let _guard = USAGE_LOCK.lock().map_err(|_| "用量锁中毒".to_string())?;
    save_usage(app, &UsageStore::default())
}
