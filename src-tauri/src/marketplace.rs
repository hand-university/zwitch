use crate::auth;
use crate::codex_plugins::{
    apply_codex_plugin, cleanup_codex_marketplace_refs_for_item, codex_plugin_keys_installed,
    is_valid_codex_plugin_dir, remove_codex_plugin_install, resolve_codex_plugin_installs,
    scan_codex_installed_plugins as import_codex_plugins_from_config,
    sync_codex_plugin_enabled_states,
};
use crate::store::{
    load_auth, load_marketplace_registry, save_marketplace_registry, MarketplaceItemRecord,
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use tauri::{AppHandle, Manager};

#[derive(Debug, Deserialize)]
struct ClaudeMarketplaceManifest {
    name: String,
    plugins: Vec<ClaudeMarketplacePluginRef>,
}

#[derive(Debug, Deserialize)]
struct ClaudeMarketplacePluginRef {
    name: String,
    source: String,
}

#[derive(Debug, Clone)]
struct ClaudePluginInstallRef {
    plugin_name: String,
    marketplace_name: String,
    version: String,
    source: PathBuf,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct MarketplaceItemView {
    pub id: String,
    pub remote_id: Option<u64>,
    pub name: String,
    pub platform: String,
    pub item_type: String,
    pub description: Option<String>,
    pub version: Option<String>,
    pub enabled: bool,
    pub remote_available: bool,
    pub origin: String,
    pub installed: bool,
    pub updated_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct ExploreItemView {
    pub id: String,
    pub remote_id: u64,
    pub name: String,
    pub platform: String,
    pub item_type: String,
    pub description: Option<String>,
    pub version: Option<String>,
    pub updated_at: Option<String>,
    pub installed: bool,
    pub enabled: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct MarketplaceSyncResult {
    pub synced: u32,
    pub updated: u32,
    pub removed_from_remote: u32,
    pub discovered_local: u32,
}

#[derive(Debug, Deserialize)]
struct RemoteItemsResponse {
    count: u32,
    items: Vec<RemoteItem>,
}

#[derive(Debug, Deserialize)]
struct RemoteItem {
    id: u64,
    name: String,
    platform: String,
    item_type: String,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    version: Option<String>,
    #[serde(default)]
    updated_at: Option<String>,
    #[serde(default)]
    content: Option<RemoteItemContent>,
}

#[derive(Debug, Deserialize)]
struct RemoteItemContent {
    #[serde(default)]
    skill_md: Option<String>,
    #[serde(default)]
    plugin_json: Option<serde_json::Value>,
    #[serde(default)]
    files: HashMap<String, String>,
}

pub async fn get_explore_items(app: &AppHandle) -> Result<Vec<ExploreItemView>, String> {
    let remote_items = fetch_remote_items(app).await?;
    let registry = load_marketplace_registry(app)?;

    let mut items: Vec<ExploreItemView> = remote_items
        .iter()
        .map(|item| {
            let key = item_key(&item.platform, &item.item_type, &item.name);
            let local = registry.items.get(&key);
            ExploreItemView {
                id: key,
                remote_id: item.id,
                name: item.name.clone(),
                platform: item.platform.clone(),
                item_type: item.item_type.clone(),
                description: item.description.clone(),
                version: item.version.clone(),
                updated_at: item.updated_at.clone(),
                installed: local.is_some(),
                enabled: local.map(|record| record.enabled).unwrap_or(false),
            }
        })
        .collect();

    items.sort_by(|a, b| a.item_type.cmp(&b.item_type).then(a.name.cmp(&b.name)));
    Ok(items)
}

pub async fn install_marketplace_item(
    app: &AppHandle,
    platform: String,
    item_type: String,
    name: String,
) -> Result<(), String> {
    let remote_items = fetch_remote_items(app).await?;
    let item = remote_items
        .iter()
        .find(|entry| {
            entry.platform == platform && entry.item_type == item_type && entry.name == name
        })
        .ok_or_else(|| format!("云端未找到条目: {name}"))?;

    let mut registry = load_marketplace_registry(app)?;
    upsert_remote_item(app, &mut registry, item, true)?;
    save_marketplace_registry(app, &registry)
}

pub fn get_marketplace_items(app: &AppHandle) -> Result<Vec<MarketplaceItemView>, String> {
    let _ = scan_claude_installed_plugins(app)?;
    let _ = scan_codex_installed_plugins(app)?;
    let registry = load_marketplace_registry(app)?;
    let mut items: Vec<MarketplaceItemView> = registry
        .items
        .values()
        .map(|record| record_to_view(app, record))
        .collect();
    items.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(items)
}

pub async fn sync_marketplace(app: &AppHandle) -> Result<MarketplaceSyncResult, String> {
    let remote_items = fetch_remote_items(app).await?;
    let mut registry = load_marketplace_registry(app)?;
    let mut result = MarketplaceSyncResult {
        synced: 0,
        updated: 0,
        removed_from_remote: 0,
        discovered_local: 0,
    };

    let remote_keys: HashSet<String> = remote_items
        .iter()
        .map(|item| item_key(&item.platform, &item.item_type, &item.name))
        .collect();

    for item in &remote_items {
        let upsert = upsert_remote_item(app, &mut registry, item, true)?;
        if upsert.is_new {
            result.synced += 1;
        } else if upsert.content_changed {
            result.updated += 1;
        }
    }

    for (key, record) in registry.items.iter_mut() {
        if record.remote_available && !remote_keys.contains(key) {
            record.remote_available = false;
            result.removed_from_remote += 1;
        }
    }

    registry.last_synced_at = Some(chrono_lite_now());
    save_marketplace_registry(app, &registry)?;

    let discovered = scan_local_items(app)?;
    result.discovered_local = discovered;

    Ok(result)
}

pub fn set_marketplace_item_enabled(
    app: &AppHandle,
    platform: String,
    item_type: String,
    name: String,
    enabled: bool,
) -> Result<(), String> {
    set_item_enabled_inner(app, &platform, &item_type, &name, Some(enabled))?;
    Ok(())
}

pub fn delete_marketplace_item(
    app: &AppHandle,
    platform: String,
    item_type: String,
    name: String,
) -> Result<(), String> {
    let key = item_key(&platform, &item_type, &name);
    let registry = load_marketplace_registry(app)?;
    let Some(record) = registry.items.get(&key) else {
        return Err(format!("未找到条目: {name}"));
    };

    if record.enabled {
        set_item_enabled_inner(app, &platform, &item_type, &name, Some(false))?;
    }

    let mut registry = load_marketplace_registry(app)?;
    registry.items.remove(&key);
    registry.suppressed_keys.insert(key);

    if let Ok(staging) = staging_item_dir(app, &platform, &item_type, &name) {
        let _ = remove_item_from_cli(&platform, &item_type, &staging, &name);
        let _ = remove_path_all(&staging);
    }
    if platform == "claude" && item_type == "plugin" {
        let _ = cleanup_claude_marketplace_refs_for_item(&name);
    }
    if platform == "codex" && item_type == "plugin" {
        let _ = cleanup_codex_marketplace_refs_for_item(&name);
    }

    save_marketplace_registry(app, &registry)
}

pub fn scan_local_marketplace(app: &AppHandle) -> Result<u32, String> {
    scan_local_items(app)
}

struct UpsertResult {
    is_new: bool,
    content_changed: bool,
}

fn upsert_remote_item(
    app: &AppHandle,
    registry: &mut crate::store::MarketplaceRegistry,
    item: &RemoteItem,
    apply_cli: bool,
) -> Result<UpsertResult, String> {
    let key = item_key(&item.platform, &item.item_type, &item.name);
    registry.suppressed_keys.remove(&key);

    let staging = staging_item_dir(app, &item.platform, &item.item_type, &item.name)?;
    let previous = registry.items.get(&key).cloned();
    let is_new = previous.is_none();
    let content_changed = is_new
        || !staging.exists()
        || previous
            .as_ref()
            .and_then(|p| p.updated_at.as_deref())
            .zip(item.updated_at.as_deref())
            .map(|(old, new)| old != new)
            .unwrap_or(true);

    if content_changed {
        if let Some(content) = &item.content {
            write_item_content(content, &item.platform, &item.item_type, &staging)?;
        } else if !staging.exists() {
            return Err(format!("条目 {} 缺少 content 且本地无缓存", item.name));
        }
    }

    let enabled = previous.as_ref().map(|r| r.enabled).unwrap_or(true);
    let origin = match previous.as_ref().map(|r| r.origin.as_str()) {
        Some("local") => "both".to_string(),
        _ => "remote".to_string(),
    };
    let claude_plugin_keys = if item.platform == "claude" && item.item_type == "plugin" {
        resolve_claude_plugin_installs(&staging, &item.name)
            .map(|installs| {
                installs
                    .iter()
                    .map(|install| format!("{}@{}", install.plugin_name, install.marketplace_name))
                    .collect()
            })
            .unwrap_or_else(|_| {
                previous
                    .as_ref()
                    .map(|record| record.claude_plugin_keys.clone())
                    .unwrap_or_default()
            })
    } else {
        Vec::new()
    };
    let codex_plugin_keys = if item.platform == "codex" && item.item_type == "plugin" {
        resolve_codex_plugin_installs(&staging, &item.name)
            .map(|installs| {
                installs
                    .iter()
                    .map(|install| format!("{}@{}", install.plugin_name, install.marketplace_name))
                    .collect()
            })
            .unwrap_or_else(|_| {
                previous
                    .as_ref()
                    .map(|record| record.codex_plugin_keys.clone())
                    .unwrap_or_default()
            })
    } else {
        Vec::new()
    };

    registry.items.insert(
        key,
        MarketplaceItemRecord {
            remote_id: Some(item.id),
            name: item.name.clone(),
            platform: item.platform.clone(),
            item_type: item.item_type.clone(),
            description: item.description.clone(),
            version: item.version.clone(),
            enabled,
            remote_available: true,
            origin,
            updated_at: item.updated_at.clone(),
            claude_plugin_keys: claude_plugin_keys.clone(),
            codex_plugin_keys: codex_plugin_keys.clone(),
        },
    );

    if apply_cli && enabled && (is_new || content_changed) {
        apply_item_to_cli(
            &item.platform,
            &item.item_type,
            &staging,
            &item.name,
            &claude_plugin_keys,
            &codex_plugin_keys,
        )?;
    }

    Ok(UpsertResult {
        is_new,
        content_changed,
    })
}

fn set_item_enabled_inner(
    app: &AppHandle,
    platform: &str,
    item_type: &str,
    name: &str,
    enabled: Option<bool>,
) -> Result<bool, String> {
    let key = item_key(platform, item_type, name);
    let mut registry = load_marketplace_registry(app)?;
    let record = registry
        .items
        .get_mut(&key)
        .ok_or_else(|| format!("未找到条目: {name}"))?;

    let next = enabled.unwrap_or(!record.enabled);
    record.enabled = next;

    let staging = staging_item_dir(app, platform, item_type, name)?;

    let claude_plugin_keys = record.claude_plugin_keys.clone();
    let codex_plugin_keys = record.codex_plugin_keys.clone();
    if next {
        if !staging.exists() {
            return Err(format!("条目 {name} 本地文件不存在，请先同步"));
        }
        apply_item_to_cli(
            platform,
            item_type,
            &staging,
            name,
            &claude_plugin_keys,
            &codex_plugin_keys,
        )?;
    } else {
        remove_item_from_cli(platform, item_type, &staging, name)?;
    }

    save_marketplace_registry(app, &registry)?;
    Ok(next)
}

fn scan_local_items(app: &AppHandle) -> Result<u32, String> {
    let mut registry = load_marketplace_registry(app)?;
    let mut discovered = 0u32;

    let scans = [
        ("claude", "plugin", home_dir()?.join(".claude/skills")),
        ("claude", "skill", home_dir()?.join(".claude/skills")),
        ("codex", "plugin", home_dir()?.join(".codex/plugins")),
        ("codex", "skill", home_dir()?.join(".agents/skills")),
    ];

    for (platform, item_type, root) in scans {
        if !root.is_dir() {
            continue;
        }

        for entry in fs::read_dir(&root).map_err(|e| format!("读取目录失败: {e}"))? {
            let entry = entry.map_err(|e| format!("读取目录项失败: {e}"))?;
            if !entry.file_type().map_err(|e| e.to_string())?.is_dir() {
                continue;
            }

            let name = entry.file_name().to_string_lossy().to_string();
            if name.starts_with('.') {
                continue;
            }
            if platform == "codex"
                && item_type == "plugin"
                && (name == "cache" || name == "marketplaces")
            {
                continue;
            }
            let key = item_key(platform, item_type, &name);
            if registry.suppressed_keys.contains(&key) || registry.items.contains_key(&key) {
                continue;
            }

            let path = entry.path();
            if !is_valid_item_dir(&path, platform, item_type) {
                continue;
            }

            let staging = staging_item_dir(app, platform, item_type, &name)?;
            if !staging.exists() {
                copy_dir_all(&path, &staging)?;
            }

            let codex_plugin_keys = if platform == "codex" && item_type == "plugin" {
                resolve_codex_plugin_installs(&path, &name)
                    .map(|installs| {
                        installs
                            .iter()
                            .map(|install| {
                                format!("{}@{}", install.plugin_name, install.marketplace_name)
                            })
                            .collect()
                    })
                    .unwrap_or_default()
            } else {
                Vec::new()
            };

            registry.items.insert(
                key,
                MarketplaceItemRecord {
                    remote_id: None,
                    name: name.clone(),
                    platform: platform.to_string(),
                    item_type: item_type.to_string(),
                    description: read_local_description(&path, platform, item_type),
                    version: read_local_version(&path, platform, item_type),
                    enabled: true,
                    remote_available: false,
                    origin: "local".to_string(),
                    updated_at: None,
                    claude_plugin_keys: Vec::new(),
                    codex_plugin_keys,
                },
            );
            discovered += 1;
        }
    }

    discovered += scan_claude_installed_plugins_inner(app, &mut registry)?;
    discovered += scan_codex_installed_plugins_inner(app, &mut registry)?;

    if discovered > 0 {
        save_marketplace_registry(app, &registry)?;
    }

    Ok(discovered)
}

fn scan_claude_installed_plugins(app: &AppHandle) -> Result<u32, String> {
    let mut registry = load_marketplace_registry(app)?;
    let discovered = scan_claude_installed_plugins_inner(app, &mut registry)?;
    let enabled_changed = sync_claude_plugin_enabled_states(&mut registry);
    if discovered > 0 || enabled_changed {
        save_marketplace_registry(app, &registry)?;
    }
    Ok(discovered)
}

fn scan_codex_installed_plugins_inner(
    app: &AppHandle,
    registry: &mut crate::store::MarketplaceRegistry,
) -> Result<u32, String> {
    import_codex_plugins_from_config(registry, |plugin_name, _cache_path| {
        staging_item_dir(app, "codex", "plugin", plugin_name)
    })
}

fn scan_codex_installed_plugins(app: &AppHandle) -> Result<u32, String> {
    let mut registry = load_marketplace_registry(app)?;
    let discovered = scan_codex_installed_plugins_inner(app, &mut registry)?;
    let enabled_changed = sync_codex_plugin_enabled_states(&mut registry);
    if discovered > 0 || enabled_changed {
        save_marketplace_registry(app, &registry)?;
    }
    Ok(discovered)
}

fn scan_claude_installed_plugins_inner(
    app: &AppHandle,
    registry: &mut crate::store::MarketplaceRegistry,
) -> Result<u32, String> {
    let mut discovered = 0u32;
    let installed = read_claude_installed_plugins()?;
    let settings = read_claude_settings().ok();
    let enabled_plugins = settings
        .as_ref()
        .and_then(|value| value.get("enabledPlugins"))
        .and_then(|value| value.as_object());

    let Some(plugins) = installed
        .pointer("/plugins")
        .and_then(|value| value.as_object())
    else {
        return Ok(0);
    };

    for (plugin_key, entries) in plugins {
        let Some((plugin_name, marketplace_name)) = plugin_key.rsplit_once('@') else {
            continue;
        };
        let key = item_key("claude", "plugin", plugin_name);
        if registry.suppressed_keys.contains(&key) {
            continue;
        }

        let Some(entry) = entries.as_array().and_then(|items| items.first()) else {
            continue;
        };
        let Some(install_path) = entry.get("installPath").and_then(|value| value.as_str()) else {
            continue;
        };
        let path = PathBuf::from(install_path);
        if !path.join(".claude-plugin/plugin.json").is_file() {
            continue;
        }

        let version = entry
            .get("version")
            .and_then(|value| value.as_str())
            .map(str::to_string);
        let enabled = enabled_plugins
            .and_then(|map| map.get(plugin_key))
            .and_then(|value| value.as_bool())
            .unwrap_or(false);
        let claude_plugin_keys = vec![plugin_key.clone()];

        if let Some(record) = registry.items.get_mut(&key) {
            record.claude_plugin_keys = claude_plugin_keys;
            record.enabled = enabled;
            if record.version.is_none() {
                record.version = version.clone();
            }
            if record.description.is_none() {
                record.description = read_local_description(&path, "claude", "plugin");
            }
            continue;
        }

        let staging = staging_item_dir(app, "claude", "plugin", plugin_name)?;
        if !staging.exists() {
            copy_dir_all(&path, &staging)?;
        }

        registry.items.insert(
            key,
            MarketplaceItemRecord {
                remote_id: None,
                name: plugin_name.to_string(),
                platform: "claude".to_string(),
                item_type: "plugin".to_string(),
                description: read_local_description(&path, "claude", "plugin"),
                version,
                enabled,
                remote_available: false,
                origin: "local".to_string(),
                updated_at: None,
                claude_plugin_keys,
                codex_plugin_keys: Vec::new(),
            },
        );
        discovered += 1;

        let _ = marketplace_name;
    }

    Ok(discovered)
}

fn sync_claude_plugin_enabled_states(registry: &mut crate::store::MarketplaceRegistry) -> bool {
    let Ok(settings) = read_claude_settings() else {
        return false;
    };
    let Some(enabled_plugins) = settings
        .get("enabledPlugins")
        .and_then(|value| value.as_object())
    else {
        return false;
    };

    let mut changed = false;
    for record in registry.items.values_mut() {
        if record.platform != "claude" || record.item_type != "plugin" {
            continue;
        }
        if record.claude_plugin_keys.is_empty() {
            continue;
        }
        let enabled = record.claude_plugin_keys.iter().all(|key| {
            enabled_plugins
                .get(key)
                .and_then(|value| value.as_bool())
                .unwrap_or(false)
        });
        if record.enabled != enabled {
            record.enabled = enabled;
            changed = true;
        }
    }
    changed
}

async fn fetch_remote_items(app: &AppHandle) -> Result<Vec<RemoteItem>, String> {
    let auth = load_auth(app)?;
    if auth.authorization_code.is_none() {
        return Err(auth::fail_auth_session_async(app, "请先登录").await);
    }

    let base = resolve_api_base_url(&auth);
    let token = auth::resolve_session_token(app).await?;
    let url = format!("{}/api/marketplace/my/items", base.trim_end_matches('/'));

    let client = reqwest::Client::new();
    let mut response = client
        .get(&url)
        .header("Authorization", format!("Bearer {token}"))
        .header("Cookie", format!("token={token}"))
        .send()
        .await
        .map_err(|e| format!("请求市场条目失败: {e}"))?;

    if response.status() == reqwest::StatusCode::UNAUTHORIZED {
        return Err(
            auth::fail_auth_session_async(
                app,
                "登录会话已过期，请重新登录",
            )
            .await,
        );
    }

    if !response.status().is_success() {
        return Err(format!("获取市场条目失败: HTTP {}", response.status()));
    }

    response
        .json::<RemoteItemsResponse>()
        .await
        .map(|body| body.items)
        .map_err(|e| format!("解析市场条目失败: {e}"))
}

fn write_item_content(
    content: &RemoteItemContent,
    platform: &str,
    item_type: &str,
    root: &Path,
) -> Result<(), String> {
    if root.exists() {
        remove_path_all(root)?;
    }
    fs::create_dir_all(root).map_err(|e| format!("创建目录失败: {e}"))?;

    if item_type == "skill" {
        if let Some(skill_md) = &content.skill_md {
            let skill_path = root.join("SKILL.md");
            if let Some(parent) = skill_path.parent() {
                fs::create_dir_all(parent).map_err(|e| format!("创建目录失败: {e}"))?;
            }
            fs::write(&skill_path, skill_md).map_err(|e| format!("写入 SKILL.md 失败: {e}"))?;
        }
    }

    if item_type == "plugin" {
        if let Some(plugin_json) = &content.plugin_json {
            let manifest_dir = if platform == "codex" {
                root.join(".codex-plugin")
            } else {
                root.join(".claude-plugin")
            };
            fs::create_dir_all(&manifest_dir).map_err(|e| format!("创建目录失败: {e}"))?;
            let has_plugins_array = plugin_json
                .get("plugins")
                .and_then(|value| value.as_array())
                .is_some();
            let manifest_path = if platform == "codex" && has_plugins_array {
                root.join(".agents/plugins/marketplace.json")
            } else if platform == "claude" && has_plugins_array {
                manifest_dir.join("marketplace.json")
            } else {
                manifest_dir.join("plugin.json")
            };
            if manifest_path.parent().is_some() && platform == "codex" && has_plugins_array {
                fs::create_dir_all(manifest_path.parent().unwrap())
                    .map_err(|e| format!("创建目录失败: {e}"))?;
            }
            let body = serde_json::to_string_pretty(plugin_json)
                .map_err(|e| format!("序列化 manifest 失败: {e}"))?;
            fs::write(&manifest_path, body).map_err(|e| format!("写入 manifest 失败: {e}"))?;
        }
    }

    for (rel, body) in &content.files {
        let file_path = root.join(rel);
        if let Some(parent) = file_path.parent() {
            fs::create_dir_all(parent).map_err(|e| format!("创建目录失败: {e}"))?;
        }
        fs::write(&file_path, body).map_err(|e| format!("写入文件 {rel} 失败: {e}"))?;
    }

    Ok(())
}

fn apply_to_cli(staging: &Path, cli: &Path) -> Result<(), String> {
    if !staging.exists() {
        return Err("本地缓存不存在".into());
    }
    remove_path_all(cli)?;
    if let Some(parent) = cli.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("创建 CLI 目录失败: {e}"))?;
    }
    copy_dir_all(staging, cli)
}

fn apply_item_to_cli(
    platform: &str,
    item_type: &str,
    staging: &Path,
    item_name: &str,
    claude_plugin_keys: &[String],
    codex_plugin_keys: &[String],
) -> Result<(), String> {
    match (platform, item_type) {
        ("claude", "plugin") => apply_claude_plugin(staging, item_name, claude_plugin_keys),
        ("codex", "plugin") => apply_codex_plugin(staging, item_name, codex_plugin_keys),
        _ => {
            let cli = cli_item_dir(platform, item_type, item_name)?;
            apply_to_cli(staging, &cli)
        }
    }
}

fn remove_item_from_cli(
    platform: &str,
    item_type: &str,
    staging: &Path,
    item_name: &str,
) -> Result<(), String> {
    match (platform, item_type) {
        ("claude", "plugin") => {
            remove_claude_plugin_install(staging, item_name)?;
            remove_legacy_claude_plugin_path(item_name)
        }
        ("codex", "plugin") => remove_codex_plugin_install(staging, item_name),
        _ => {
            let cli = cli_item_dir(platform, item_type, item_name)?;
            remove_path_all(&cli)
        }
    }
}

fn resolve_claude_plugin_installs(
    staging: &Path,
    _item_name: &str,
) -> Result<Vec<ClaudePluginInstallRef>, String> {
    let marketplace_json = staging.join(".claude-plugin/marketplace.json");
    if marketplace_json.is_file() {
        let raw = fs::read_to_string(&marketplace_json)
            .map_err(|e| format!("读取 marketplace.json 失败: {e}"))?;
        let manifest: ClaudeMarketplaceManifest =
            serde_json::from_str(&raw).map_err(|e| format!("解析 marketplace.json 失败: {e}"))?;
        let marketplace_name = manifest.name;
        return manifest
            .plugins
            .into_iter()
            .map(|entry| {
                let source = entry
                    .source
                    .strip_prefix("./")
                    .unwrap_or(&entry.source)
                    .to_string();
                let source_path = staging.join(source);
                Ok(ClaudePluginInstallRef {
                    plugin_name: entry.name,
                    marketplace_name: marketplace_name.clone(),
                    version: read_claude_plugin_version(&source_path)?,
                    source: source_path,
                })
            })
            .collect();
    }

    if staging.join(".claude-plugin/plugin.json").is_file() {
        let plugin_name = read_claude_plugin_name(staging)?;
        return Ok(vec![ClaudePluginInstallRef {
            plugin_name: plugin_name.clone(),
            marketplace_name: plugin_name,
            version: read_claude_plugin_version(staging)?,
            source: staging.to_path_buf(),
        }]);
    }

    Err(format!("无法识别 Claude 插件结构: {}", staging.display()))
}

fn read_claude_plugin_name(src: &Path) -> Result<String, String> {
    let plugin_json = src.join(".claude-plugin/plugin.json");
    let value: serde_json::Value = serde_json::from_str(
        &fs::read_to_string(&plugin_json).map_err(|e| format!("读取 plugin.json 失败: {e}"))?,
    )
    .map_err(|e| format!("解析 plugin.json 失败: {e}"))?;
    value
        .get("name")
        .and_then(|entry| entry.as_str())
        .map(str::to_string)
        .ok_or_else(|| format!("{} 缺少 name 字段", plugin_json.display()))
}

fn read_claude_plugin_version(src: &Path) -> Result<String, String> {
    let plugin_json = src.join(".claude-plugin/plugin.json");
    let value: serde_json::Value = serde_json::from_str(
        &fs::read_to_string(&plugin_json).map_err(|e| format!("读取 plugin.json 失败: {e}"))?,
    )
    .map_err(|e| format!("解析 plugin.json 失败: {e}"))?;
    Ok(value
        .get("version")
        .and_then(|entry| entry.as_str())
        .unwrap_or("0.0.0")
        .to_string())
}

fn claude_settings_path() -> Result<PathBuf, String> {
    Ok(home_dir()?.join(".claude/settings.json"))
}

fn claude_known_marketplaces_path() -> Result<PathBuf, String> {
    Ok(home_dir()?.join(".claude/plugins/known_marketplaces.json"))
}

fn claude_installed_plugins_path() -> Result<PathBuf, String> {
    Ok(home_dir()?.join(".claude/plugins/installed_plugins.json"))
}

fn claude_plugin_cache_dir(
    marketplace_name: &str,
    plugin_name: &str,
    version: &str,
) -> Result<PathBuf, String> {
    Ok(home_dir()?
        .join(".claude/plugins/cache")
        .join(marketplace_name)
        .join(plugin_name)
        .join(version))
}

fn claude_marketplace_dir(marketplace_name: &str) -> Result<PathBuf, String> {
    Ok(home_dir()?
        .join(".claude/plugins/marketplaces")
        .join(marketplace_name))
}

fn persist_claude_marketplace(staging: &Path, marketplace_name: &str) -> Result<PathBuf, String> {
    let dest = claude_marketplace_dir(marketplace_name)?;
    remove_path_all(&dest)?;
    copy_dir_all(staging, &dest)?;
    Ok(dest)
}

fn read_json_file(path: &Path) -> Result<serde_json::Value, String> {
    if path.is_file() {
        let content =
            fs::read_to_string(path).map_err(|e| format!("读取 {} 失败: {e}", path.display()))?;
        serde_json::from_str(&content).map_err(|e| format!("解析 {} 失败: {e}", path.display()))
    } else {
        Ok(json!({}))
    }
}

fn write_json_file(path: &Path, value: &serde_json::Value) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("创建目录失败: {e}"))?;
    }
    let body = serde_json::to_string_pretty(value)
        .map_err(|e| format!("序列化 {} 失败: {e}", path.display()))?;
    fs::write(path, body).map_err(|e| format!("写入 {} 失败: {e}", path.display()))
}

fn read_claude_settings() -> Result<serde_json::Value, String> {
    read_json_file(&claude_settings_path()?)
}

fn write_claude_settings(value: &serde_json::Value) -> Result<(), String> {
    write_json_file(&claude_settings_path()?, value)
}

fn read_claude_known_marketplaces() -> Result<serde_json::Value, String> {
    read_json_file(&claude_known_marketplaces_path()?)
}

fn write_claude_known_marketplaces(value: &serde_json::Value) -> Result<(), String> {
    write_json_file(&claude_known_marketplaces_path()?, value)
}

fn read_claude_installed_plugins() -> Result<serde_json::Value, String> {
    let path = claude_installed_plugins_path()?;
    if path.is_file() {
        read_json_file(&path)
    } else {
        Ok(json!({
            "version": 2,
            "plugins": {}
        }))
    }
}

fn write_claude_installed_plugins(value: &serde_json::Value) -> Result<(), String> {
    write_json_file(&claude_installed_plugins_path()?, value)
}

fn canonical_path(path: &Path) -> Result<PathBuf, String> {
    fs::canonicalize(path).map_err(|e| format!("解析路径失败: {e}"))
}

fn iso_timestamp_now() -> String {
    let ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or(0);
    unix_ms_to_iso(ms)
}

fn unix_ms_to_iso(ms: u128) -> String {
    let secs = (ms / 1000) as i64;
    let millis = (ms % 1000) as u32;
    let days = secs / 86_400;
    let time_of_day = secs % 86_400;
    let hours = time_of_day / 3_600;
    let minutes = (time_of_day % 3_600) / 60;
    let seconds = time_of_day % 60;

    let z = days + 719_468;
    let era = (if z >= 0 { z } else { z - 146_096 }) / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let mut year = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    if month <= 2 {
        year += 1;
    }

    format!("{year:04}-{month:02}-{day:02}T{hours:02}:{minutes:02}:{seconds:02}.{millis:03}Z")
}

fn register_claude_marketplace(
    marketplace_root: &Path,
    marketplace_name: &str,
) -> Result<(), String> {
    if !marketplace_root
        .join(".claude-plugin/marketplace.json")
        .is_file()
    {
        return Ok(());
    }

    let install_location = canonical_path(marketplace_root)?;
    let install_location_str = install_location.to_string_lossy().to_string();
    let timestamp = iso_timestamp_now();

    let mut known = read_claude_known_marketplaces()?;
    let root = known
        .as_object_mut()
        .ok_or_else(|| "known_marketplaces.json 格式无效".to_string())?;
    root.insert(
        marketplace_name.to_string(),
        json!({
            "source": {
                "source": "directory",
                "path": install_location_str.clone()
            },
            "installLocation": install_location_str.clone(),
            "lastUpdated": timestamp
        }),
    );
    write_claude_known_marketplaces(&known)?;

    let mut settings = read_claude_settings()?;
    let root = settings
        .as_object_mut()
        .ok_or_else(|| "Claude settings 格式无效".to_string())?;
    if !root.contains_key("extraKnownMarketplaces") {
        root.insert("extraKnownMarketplaces".to_string(), json!({}));
    }
    root.get_mut("extraKnownMarketplaces")
        .and_then(|value| value.as_object_mut())
        .ok_or_else(|| "extraKnownMarketplaces 格式无效".to_string())?
        .insert(
            marketplace_name.to_string(),
            json!({
                "source": {
                    "source": "directory",
                    "path": install_location_str
                }
            }),
        );
    write_claude_settings(&settings)
}

fn claude_marketplace_has_installed_plugins(marketplace_name: &str) -> Result<bool, String> {
    let suffix = format!("@{marketplace_name}");
    let installed = read_claude_installed_plugins()?;
    Ok(installed
        .pointer("/plugins")
        .and_then(|value| value.as_object())
        .is_some_and(|plugins| plugins.keys().any(|key| key.ends_with(&suffix))))
}

fn unregister_claude_marketplace(marketplace_name: &str) -> Result<(), String> {
    let mut known = read_claude_known_marketplaces()?;
    if let Some(root) = known.as_object_mut() {
        root.remove(marketplace_name);
    }
    write_claude_known_marketplaces(&known)?;

    let mut settings = read_claude_settings()?;
    if let Some(root) = settings.as_object_mut() {
        if let Some(extra) = root
            .get_mut("extraKnownMarketplaces")
            .and_then(|value| value.as_object_mut())
        {
            extra.remove(marketplace_name);
            if extra.is_empty() {
                root.remove("extraKnownMarketplaces");
            }
        }
    }
    write_claude_settings(&settings)?;

    remove_path_all(&claude_marketplace_dir(marketplace_name)?)
}

fn unregister_claude_marketplace_if_empty(marketplace_name: &str) -> Result<(), String> {
    if claude_marketplace_has_installed_plugins(marketplace_name)? {
        return Ok(());
    }
    unregister_claude_marketplace(marketplace_name)
}

fn claude_marketplace_entry_path(entry: &serde_json::Value) -> Option<&str> {
    entry
        .pointer("/installLocation")
        .or_else(|| entry.pointer("/source/path"))
        .and_then(|value| value.as_str())
}

fn cleanup_claude_marketplace_refs_for_item(item_name: &str) -> Result<(), String> {
    let known = read_claude_known_marketplaces()?;
    let Some(obj) = known.as_object() else {
        return Ok(());
    };

    let marker = format!("/plugins/{item_name}");
    let to_remove: Vec<String> = obj
        .iter()
        .filter_map(|(marketplace_name, entry)| {
            let path = claude_marketplace_entry_path(entry)?;
            if !path.contains(&marker) && !path.ends_with(&format!("/{item_name}")) {
                return None;
            }
            let manifest_missing = !Path::new(path)
                .join(".claude-plugin/marketplace.json")
                .is_file();
            let no_plugins =
                !claude_marketplace_has_installed_plugins(marketplace_name).unwrap_or(true);
            if manifest_missing || no_plugins {
                Some(marketplace_name.clone())
            } else {
                None
            }
        })
        .collect();

    for marketplace_name in to_remove {
        unregister_claude_marketplace(&marketplace_name)?;
    }
    Ok(())
}

fn install_claude_marketplace_plugin(install: &ClaudePluginInstallRef) -> Result<(), String> {
    if !install.source.join(".claude-plugin/plugin.json").is_file() {
        return Err(format!("插件 {} 缺少 plugin.json", install.plugin_name));
    }

    let cache_dir = claude_plugin_cache_dir(
        &install.marketplace_name,
        &install.plugin_name,
        &install.version,
    )?;
    remove_path_all(&cache_dir)?;
    copy_dir_all(&install.source, &cache_dir)?;

    let plugin_key = format!("{}@{}", install.plugin_name, install.marketplace_name);
    let timestamp = iso_timestamp_now();
    let install_path = cache_dir.to_string_lossy().to_string();

    let mut installed = read_claude_installed_plugins()?;
    let root = installed
        .as_object_mut()
        .ok_or_else(|| "installed_plugins.json 格式无效".to_string())?;
    if !root.contains_key("version") {
        root.insert("version".to_string(), json!(2));
    }
    if !root.contains_key("plugins") {
        root.insert("plugins".to_string(), json!({}));
    }
    root.get_mut("plugins")
        .and_then(|value| value.as_object_mut())
        .ok_or_else(|| "installed_plugins.plugins 格式无效".to_string())?
        .insert(
            plugin_key.clone(),
            json!([{
                "scope": "user",
                "installPath": install_path,
                "version": install.version,
                "installedAt": timestamp,
                "lastUpdated": timestamp
            }]),
        );
    write_claude_installed_plugins(&installed)?;

    let mut settings = read_claude_settings()?;
    let root = settings
        .as_object_mut()
        .ok_or_else(|| "Claude settings 格式无效".to_string())?;
    if !root.contains_key("enabledPlugins") {
        root.insert("enabledPlugins".to_string(), json!({}));
    }
    root.get_mut("enabledPlugins")
        .and_then(|value| value.as_object_mut())
        .ok_or_else(|| "enabledPlugins 格式无效".to_string())?
        .insert(plugin_key.clone(), json!(true));
    write_claude_settings(&settings)
}

fn uninstall_claude_marketplace_plugin(install: &ClaudePluginInstallRef) -> Result<(), String> {
    let plugin_key = format!("{}@{}", install.plugin_name, install.marketplace_name);

    if let Ok(installed) = read_claude_installed_plugins() {
        if let Some(entries) = installed
            .pointer(&format!("/plugins/{plugin_key}"))
            .and_then(|value| value.as_array())
        {
            for entry in entries {
                if let Some(path) = entry.get("installPath").and_then(|value| value.as_str()) {
                    let _ = remove_path_all(Path::new(path));
                }
            }
        }

        let mut installed = installed;
        if let Some(plugins) = installed
            .get_mut("plugins")
            .and_then(|value| value.as_object_mut())
        {
            plugins.remove(&plugin_key);
        }
        let _ = write_claude_installed_plugins(&installed);
    }

    let _ = remove_path_all(&claude_plugin_cache_dir(
        &install.marketplace_name,
        &install.plugin_name,
        &install.version,
    )?);

    let mut settings = read_claude_settings()?;
    if let Some(enabled) = settings
        .get_mut("enabledPlugins")
        .and_then(|value| value.as_object_mut())
    {
        enabled.remove(&plugin_key);
        enabled.remove(&format!("{}@skills-dir", install.plugin_name));
    }
    write_claude_settings(&settings)?;

    let _ = remove_path_all(
        &home_dir()?
            .join(".claude/skills")
            .join(&install.plugin_name),
    );
    Ok(())
}

fn claude_installs_from_plugin_keys(
    keys: &[String],
    staging: &Path,
    item_name: &str,
) -> Result<Vec<ClaudePluginInstallRef>, String> {
    if keys.is_empty() {
        return resolve_claude_plugin_installs(staging, item_name);
    }

    if staging.join(".claude-plugin/marketplace.json").is_file() {
        let key_set: HashSet<String> = keys.iter().cloned().collect();
        return resolve_claude_plugin_installs(staging, item_name).map(|installs| {
            installs
                .into_iter()
                .filter(|install| {
                    key_set.contains(&format!(
                        "{}@{}",
                        install.plugin_name, install.marketplace_name
                    ))
                })
                .collect()
        });
    }

    let installed = read_claude_installed_plugins()?;
    keys.iter()
        .map(|plugin_key| {
            let (plugin_name, marketplace_name) = plugin_key
                .rsplit_once('@')
                .ok_or_else(|| format!("无效的 Claude 插件 ID: {plugin_key}"))?;
            let entry = installed
                .pointer(&format!("/plugins/{plugin_key}"))
                .and_then(|value| value.as_array())
                .and_then(|items| items.first())
                .ok_or_else(|| format!("installed_plugins 中未找到 {plugin_key}"))?;
            let version = entry
                .get("version")
                .and_then(|value| value.as_str())
                .unwrap_or("0.0.0")
                .to_string();
            let source = if staging.join(".claude-plugin/plugin.json").is_file() {
                staging.to_path_buf()
            } else if let Some(path) = entry.get("installPath").and_then(|value| value.as_str()) {
                PathBuf::from(path)
            } else {
                claude_plugin_cache_dir(marketplace_name, plugin_name, &version)?
            };
            Ok(ClaudePluginInstallRef {
                plugin_name: plugin_name.to_string(),
                marketplace_name: marketplace_name.to_string(),
                version,
                source,
            })
        })
        .collect()
}

fn claude_plugin_keys_installed(keys: &[String]) -> bool {
    if keys.is_empty() {
        return false;
    }
    let Ok(installed) = read_claude_installed_plugins() else {
        return false;
    };
    keys.iter().all(|plugin_key| {
        installed
            .pointer(&format!("/plugins/{plugin_key}"))
            .and_then(|value| value.as_array())
            .is_some_and(|entries| {
                entries.iter().any(|entry| {
                    entry
                        .get("installPath")
                        .and_then(|value| value.as_str())
                        .map(|path| Path::new(path).join(".claude-plugin/plugin.json").is_file())
                        .unwrap_or(false)
                })
            })
    })
}

fn apply_claude_plugin(
    staging: &Path,
    item_name: &str,
    claude_plugin_keys: &[String],
) -> Result<(), String> {
    if !staging.exists() {
        return Err("本地缓存不存在".into());
    }

    let installs = claude_installs_from_plugin_keys(claude_plugin_keys, staging, item_name)?;
    if let Some(first) = installs.first() {
        let marketplace_root = if staging.join(".claude-plugin/marketplace.json").is_file() {
            persist_claude_marketplace(staging, &first.marketplace_name)?
        } else {
            staging.to_path_buf()
        };
        register_claude_marketplace(&marketplace_root, &first.marketplace_name)?;
    }
    for install in &installs {
        install_claude_marketplace_plugin(install)?;
    }
    remove_legacy_claude_plugin_path(item_name)
}

fn installs_for_marketplace(marketplace_name: &str) -> Vec<ClaudePluginInstallRef> {
    let Ok(installed) = read_claude_installed_plugins() else {
        return Vec::new();
    };
    let Some(plugins) = installed
        .pointer("/plugins")
        .and_then(|value| value.as_object())
    else {
        return Vec::new();
    };
    let suffix = format!("@{marketplace_name}");

    plugins
        .iter()
        .filter_map(|(plugin_key, entries)| {
            if !plugin_key.ends_with(&suffix) {
                return None;
            }
            let plugin_name = plugin_key.strip_suffix(&suffix)?.to_string();
            let entry = entries.as_array()?.first()?;
            let install_path = entry.get("installPath")?.as_str()?;
            let version = entry
                .get("version")
                .and_then(|value| value.as_str())
                .unwrap_or("0.0.0")
                .to_string();
            Some(ClaudePluginInstallRef {
                plugin_name,
                marketplace_name: marketplace_name.to_string(),
                version,
                source: PathBuf::from(install_path),
            })
        })
        .collect()
}

fn infer_claude_installs_from_registry(item_name: &str) -> Vec<ClaudePluginInstallRef> {
    if let Ok(known) = read_claude_known_marketplaces() {
        if let Some(obj) = known.as_object() {
            let marker = format!("/plugins/{item_name}");
            for (marketplace_name, entry) in obj {
                if let Some(path) = claude_marketplace_entry_path(entry) {
                    if path.contains(&marker) || path.ends_with(&format!("/{item_name}")) {
                        let installs = installs_for_marketplace(marketplace_name);
                        if !installs.is_empty() {
                            return installs;
                        }
                        return vec![ClaudePluginInstallRef {
                            plugin_name: item_name.to_string(),
                            marketplace_name: marketplace_name.clone(),
                            version: "0.0.0".to_string(),
                            source: PathBuf::from(path),
                        }];
                    }
                }
            }
        }
    }

    installs_for_marketplace(item_name)
}

fn remove_claude_plugin_install(staging: &Path, item_name: &str) -> Result<(), String> {
    let installs = resolve_claude_plugin_installs(staging, item_name)
        .unwrap_or_else(|_| infer_claude_installs_from_registry(item_name));
    if installs.is_empty() {
        let _ = cleanup_claude_marketplace_refs_for_item(item_name);
        return Ok(());
    }

    let marketplace_names: HashSet<String> = installs
        .iter()
        .map(|install| install.marketplace_name.clone())
        .collect();
    for install in &installs {
        uninstall_claude_marketplace_plugin(install)?;
    }
    for marketplace_name in marketplace_names {
        unregister_claude_marketplace_if_empty(&marketplace_name)?;
    }
    Ok(())
}

fn remove_legacy_claude_plugin_path(item_name: &str) -> Result<(), String> {
    let legacy = home_dir()?.join(".claude/plugins").join(item_name);
    remove_path_all(&legacy)
}

fn claude_plugin_installed(staging: &Path, item_name: &str, claude_plugin_keys: &[String]) -> bool {
    if claude_plugin_keys_installed(claude_plugin_keys) {
        return true;
    }

    let Ok(installs) = resolve_claude_plugin_installs(staging, item_name) else {
        return false;
    };
    let keys: Vec<String> = installs
        .iter()
        .map(|install| format!("{}@{}", install.plugin_name, install.marketplace_name))
        .collect();
    claude_plugin_keys_installed(&keys)
}

fn record_to_view(app: &AppHandle, record: &MarketplaceItemRecord) -> MarketplaceItemView {
    let id = item_key(&record.platform, &record.item_type, &record.name);
    let staging_exists = staging_item_dir(app, &record.platform, &record.item_type, &record.name)
        .map(|p| p.exists())
        .unwrap_or(false);
    let cli_exists = if record.platform == "claude" && record.item_type == "plugin" {
        if claude_plugin_keys_installed(&record.claude_plugin_keys) {
            true
        } else {
            staging_item_dir(app, &record.platform, &record.item_type, &record.name)
                .ok()
                .filter(|staging| staging.exists())
                .map(|staging| {
                    claude_plugin_installed(&staging, &record.name, &record.claude_plugin_keys)
                })
                .unwrap_or(false)
        }
    } else if record.platform == "codex" && record.item_type == "plugin" {
        codex_plugin_keys_installed(&record.codex_plugin_keys)
            || cli_item_dir(&record.platform, &record.item_type, &record.name)
                .map(|p| p.exists())
                .unwrap_or(false)
    } else {
        cli_item_dir(&record.platform, &record.item_type, &record.name)
            .map(|p| p.exists())
            .unwrap_or(false)
    };

    MarketplaceItemView {
        id,
        remote_id: record.remote_id,
        name: record.name.clone(),
        platform: record.platform.clone(),
        item_type: record.item_type.clone(),
        description: record.description.clone(),
        version: record.version.clone(),
        enabled: record.enabled,
        remote_available: record.remote_available,
        origin: record.origin.clone(),
        installed: staging_exists || cli_exists,
        updated_at: record.updated_at.clone(),
    }
}

fn staging_item_dir(
    app: &AppHandle,
    platform: &str,
    item_type: &str,
    name: &str,
) -> Result<PathBuf, String> {
    let root = app_data_root(app)?;
    Ok(root.join(platform).join(format!("{item_type}s")).join(name))
}

fn app_data_root(app: &AppHandle) -> Result<PathBuf, String> {
    let base = app
        .path()
        .app_data_dir()
        .map_err(|e| format!("无法获取应用数据目录: {e}"))?;
    let root = base.join("marketplace").join("staging");
    fs::create_dir_all(&root).map_err(|e| format!("创建缓存目录失败: {e}"))?;
    Ok(root)
}

fn cli_item_dir(platform: &str, item_type: &str, name: &str) -> Result<PathBuf, String> {
    let home = home_dir()?;
    let sub = match (platform, item_type) {
        ("claude", "plugin") => home.join(".claude/plugins"),
        ("claude", "skill") => home.join(".claude/skills"),
        ("codex", "plugin") => home.join(".codex/plugins/cache"),
        ("codex", "skill") => home.join(".agents/skills"),
        _ => return Err(format!("不支持的平台或类型: {platform}/{item_type}")),
    };
    Ok(sub.join(name))
}

fn item_key(platform: &str, item_type: &str, name: &str) -> String {
    format!("{platform}:{item_type}:{name}")
}

fn home_dir() -> Result<PathBuf, String> {
    dirs::home_dir().ok_or_else(|| "无法获取用户目录".to_string())
}

fn resolve_api_base_url(auth: &crate::store::StoredAuth) -> String {
    crate::user_api::resolve_api_base_url(auth.api_base_url.as_deref())
}

fn is_valid_item_dir(path: &Path, platform: &str, item_type: &str) -> bool {
    match item_type {
        "skill" => path.join("SKILL.md").is_file(),
        "plugin" => {
            if platform == "codex" {
                is_valid_codex_plugin_dir(path)
            } else {
                path.join(".claude-plugin/plugin.json").is_file()
            }
        }
        _ => false,
    }
}

fn read_local_description(path: &Path, platform: &str, item_type: &str) -> Option<String> {
    if item_type == "skill" {
        let content = fs::read_to_string(path.join("SKILL.md")).ok()?;
        return extract_frontmatter_field(&content, "description");
    }

    let manifest = if platform == "codex" {
        path.join(".codex-plugin/plugin.json")
    } else {
        path.join(".claude-plugin/plugin.json")
    };
    let value: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(manifest).ok()?).ok()?;
    value
        .get("description")
        .and_then(|v| v.as_str())
        .map(str::to_string)
}

fn read_local_version(path: &Path, platform: &str, item_type: &str) -> Option<String> {
    if item_type == "skill" {
        let content = fs::read_to_string(path.join("SKILL.md")).ok()?;
        return extract_frontmatter_field(&content, "version");
    }

    let manifest = if platform == "codex" {
        path.join(".codex-plugin/plugin.json")
    } else {
        path.join(".claude-plugin/plugin.json")
    };
    let value: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(manifest).ok()?).ok()?;
    value
        .get("version")
        .and_then(|v| v.as_str())
        .map(str::to_string)
}

fn extract_frontmatter_field(content: &str, field: &str) -> Option<String> {
    let trimmed = content.trim_start();
    if !trimmed.starts_with("---") {
        return None;
    }
    let rest = trimmed.strip_prefix("---")?;
    let end = rest.find("\n---")?;
    let frontmatter = &rest[..end];
    for line in frontmatter.lines() {
        let line = line.trim();
        if let Some((key, value)) = line.split_once(':') {
            if key.trim() == field {
                return Some(value.trim().trim_matches('"').to_string());
            }
        }
    }
    None
}

fn copy_dir_all(src: &Path, dst: &Path) -> Result<(), String> {
    fs::create_dir_all(dst).map_err(|e| format!("创建目录失败: {e}"))?;
    for entry in fs::read_dir(src).map_err(|e| format!("读取目录失败: {e}"))? {
        let entry = entry.map_err(|e| format!("读取目录项失败: {e}"))?;
        let target = dst.join(entry.file_name());
        let file_type = entry.file_type().map_err(|e| e.to_string())?;
        if file_type.is_dir() {
            copy_dir_all(&entry.path(), &target)?;
        } else {
            fs::copy(entry.path(), &target).map_err(|e| format!("复制文件失败: {e}"))?;
        }
    }
    Ok(())
}

fn remove_path_all(path: &Path) -> Result<(), String> {
    if path.exists() {
        if path.is_dir() {
            fs::remove_dir_all(path).map_err(|e| format!("删除目录失败: {e}"))?;
        } else {
            fs::remove_file(path).map_err(|e| format!("删除文件失败: {e}"))?;
        }
    }
    Ok(())
}

fn chrono_lite_now() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    secs.to_string()
}
