use serde::Deserialize;
use serde_json::Value;
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use toml::Value as TomlValue;

#[derive(Debug, Clone)]
pub struct CodexPluginInstallRef {
    pub plugin_name: String,
    pub marketplace_name: String,
    pub version: String,
    pub source: PathBuf,
}

#[derive(Debug, Deserialize)]
struct CodexMarketplaceManifest {
    name: String,
    plugins: Vec<CodexMarketplacePluginRef>,
}

#[derive(Debug, Deserialize)]
struct CodexMarketplacePluginRef {
    name: String,
    source: CodexPluginSourceRef,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum CodexPluginSourceRef {
    Object { source: String, path: String },
    String(String),
}

pub fn resolve_codex_plugin_installs(
    staging: &Path,
    _item_name: &str,
) -> Result<Vec<CodexPluginInstallRef>, String> {
    let marketplace_json = staging.join(".agents/plugins/marketplace.json");
    if marketplace_json.is_file() {
        let raw = fs::read_to_string(&marketplace_json)
            .map_err(|e| format!("读取 Codex marketplace.json 失败: {e}"))?;
        let manifest: CodexMarketplaceManifest = serde_json::from_str(&raw)
            .map_err(|e| format!("解析 Codex marketplace.json 失败: {e}"))?;
        let marketplace_name = manifest.name;
        return manifest
            .plugins
            .into_iter()
            .map(|entry| {
                let source_path = resolve_codex_source_path(staging, &entry.source);
                Ok(CodexPluginInstallRef {
                    plugin_name: entry.name,
                    marketplace_name: marketplace_name.clone(),
                    version: read_codex_plugin_version(&source_path, staging)?,
                    source: source_path,
                })
            })
            .collect();
    }

    if staging.join(".codex-plugin/plugin.json").is_file() {
        let plugin_name = read_codex_plugin_name(staging)?;
        return Ok(vec![CodexPluginInstallRef {
            plugin_name: plugin_name.clone(),
            marketplace_name: plugin_name,
            version: read_codex_plugin_version(staging, staging)?,
            source: staging.to_path_buf(),
        }]);
    }

    Err(format!("无法识别 Codex 插件结构: {}", staging.display()))
}

fn resolve_codex_source_path(staging: &Path, source: &CodexPluginSourceRef) -> PathBuf {
    let relative = match source {
        CodexPluginSourceRef::Object { path, .. } => {
            path.strip_prefix("./").unwrap_or(path).to_string()
        }
        CodexPluginSourceRef::String(value) => {
            value.strip_prefix("./").unwrap_or(value).to_string()
        }
    };
    staging.join(relative)
}

fn read_codex_plugin_name(src: &Path) -> Result<String, String> {
    let plugin_json = src.join(".codex-plugin/plugin.json");
    let value: Value = serde_json::from_str(
        &fs::read_to_string(&plugin_json).map_err(|e| format!("读取 plugin.json 失败: {e}"))?,
    )
    .map_err(|e| format!("解析 plugin.json 失败: {e}"))?;
    value
        .get("name")
        .and_then(|entry| entry.as_str())
        .map(str::to_string)
        .ok_or_else(|| format!("{} 缺少 name 字段", plugin_json.display()))
}

fn read_codex_plugin_version(src: &Path, staging: &Path) -> Result<String, String> {
    for candidate in [
        src.join(".codex-plugin/plugin.json"),
        staging.join(".codex-plugin/plugin.json"),
    ] {
        if !candidate.is_file() {
            continue;
        }
        let value: Value = serde_json::from_str(
            &fs::read_to_string(&candidate).map_err(|e| format!("读取 plugin.json 失败: {e}"))?,
        )
        .map_err(|e| format!("解析 plugin.json 失败: {e}"))?;
        if let Some(version) = value.get("version").and_then(|entry| entry.as_str()) {
            return Ok(version.to_string());
        }
    }
    Ok("local".to_string())
}

pub fn codex_config_path() -> Result<PathBuf, String> {
    Ok(home_dir()?.join(".codex/config.toml"))
}

pub fn read_codex_config() -> Result<TomlValue, String> {
    let path = codex_config_path()?;
    if path.is_file() {
        let content =
            fs::read_to_string(&path).map_err(|e| format!("读取 Codex config.toml 失败: {e}"))?;
        toml::from_str(&content).map_err(|e| format!("解析 Codex config.toml 失败: {e}"))
    } else {
        Ok(TomlValue::Table(Default::default()))
    }
}

pub fn write_codex_config(value: &TomlValue) -> Result<(), String> {
    let path = codex_config_path()?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("创建 .codex 目录失败: {e}"))?;
    }
    let body =
        toml::to_string_pretty(value).map_err(|e| format!("序列化 Codex config.toml 失败: {e}"))?;
    fs::write(&path, body).map_err(|e| format!("写入 Codex config.toml 失败: {e}"))
}

fn codex_plugin_cache_dir(
    marketplace_name: &str,
    plugin_name: &str,
    version: &str,
) -> Result<PathBuf, String> {
    Ok(home_dir()?
        .join(".codex/plugins/cache")
        .join(marketplace_name)
        .join(plugin_name)
        .join(version))
}

fn codex_marketplace_dir(marketplace_name: &str) -> Result<PathBuf, String> {
    Ok(home_dir()?
        .join(".codex/plugins/marketplaces")
        .join(marketplace_name))
}

fn persist_codex_marketplace(staging: &Path, marketplace_name: &str) -> Result<PathBuf, String> {
    let dest = codex_marketplace_dir(marketplace_name)?;
    remove_path_all(&dest)?;
    copy_dir_all(staging, &dest)?;
    Ok(dest)
}

fn register_codex_marketplace(
    marketplace_root: &Path,
    marketplace_name: &str,
) -> Result<(), String> {
    let mut config = read_codex_config()?;
    let root = config
        .as_table_mut()
        .ok_or_else(|| "Codex config.toml 格式无效".to_string())?;
    let marketplaces = root
        .entry("marketplaces")
        .or_insert(TomlValue::Table(Default::default()));
    let marketplaces = marketplaces
        .as_table_mut()
        .ok_or_else(|| "marketplaces 格式无效".to_string())?;
    let install_location = canonical_path(marketplace_root)?;
    let install_location_str = install_location.to_string_lossy().to_string();
    let mut entry = toml::map::Map::new();
    entry.insert(
        "last_updated".to_string(),
        TomlValue::String(iso_timestamp_now()),
    );
    entry.insert(
        "source_type".to_string(),
        TomlValue::String("local".to_string()),
    );
    entry.insert(
        "source".to_string(),
        TomlValue::String(install_location_str),
    );
    marketplaces.insert(marketplace_name.to_string(), TomlValue::Table(entry));
    ensure_codex_features_plugins(&mut config);
    write_codex_config(&config)
}

fn ensure_codex_features_plugins(config: &mut TomlValue) {
    let root = config.as_table_mut().expect("config table");
    let features = root
        .entry("features")
        .or_insert(TomlValue::Table(Default::default()));
    if let Some(table) = features.as_table_mut() {
        table.insert("plugins".to_string(), TomlValue::Boolean(true));
    }
}

fn install_codex_marketplace_plugin(install: &CodexPluginInstallRef) -> Result<(), String> {
    let cache_dir = codex_plugin_cache_dir(
        &install.marketplace_name,
        &install.plugin_name,
        &install.version,
    )?;
    remove_path_all(&cache_dir)?;
    copy_dir_all(&install.source, &cache_dir)?;

    let plugin_key = format!("{}@{}", install.plugin_name, install.marketplace_name);
    let mut config = read_codex_config()?;
    let root = config
        .as_table_mut()
        .ok_or_else(|| "Codex config.toml 格式无效".to_string())?;
    let plugins = root
        .entry("plugins")
        .or_insert(TomlValue::Table(Default::default()));
    let plugins = plugins
        .as_table_mut()
        .ok_or_else(|| "plugins 格式无效".to_string())?;
    let mut entry = toml::map::Map::new();
    entry.insert("enabled".to_string(), TomlValue::Boolean(true));
    plugins.insert(plugin_key, TomlValue::Table(entry));
    ensure_codex_features_plugins(&mut config);
    write_codex_config(&config)
}

fn uninstall_codex_marketplace_plugin(install: &CodexPluginInstallRef) -> Result<(), String> {
    let plugin_key = format!("{}@{}", install.plugin_name, install.marketplace_name);
    let cache_base = home_dir()?
        .join(".codex/plugins/cache")
        .join(&install.marketplace_name)
        .join(&install.plugin_name);
    remove_path_all(&cache_base)?;

    let mut config = read_codex_config()?;
    if let Some(plugins) = config
        .get_mut("plugins")
        .and_then(|value| value.as_table_mut())
    {
        plugins.remove(&plugin_key);
    }
    write_codex_config(&config)
}

pub fn codex_installs_from_plugin_keys(
    keys: &[String],
    staging: &Path,
    item_name: &str,
) -> Result<Vec<CodexPluginInstallRef>, String> {
    if keys.is_empty() {
        return resolve_codex_plugin_installs(staging, item_name);
    }

    if staging.join(".agents/plugins/marketplace.json").is_file() {
        let key_set: HashSet<String> = keys.iter().cloned().collect();
        return resolve_codex_plugin_installs(staging, item_name).map(|installs| {
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

    keys.iter()
        .map(|plugin_key| {
            let (plugin_name, marketplace_name) = plugin_key
                .rsplit_once('@')
                .ok_or_else(|| format!("无效的 Codex 插件 ID: {plugin_key}"))?;
            let version = find_codex_plugin_cache_path(marketplace_name, plugin_name)
                .and_then(|path| {
                    path.file_name()
                        .map(|name| name.to_string_lossy().to_string())
                })
                .unwrap_or_else(|| "local".to_string());
            let source = if staging.join(".codex-plugin/plugin.json").is_file() {
                staging.to_path_buf()
            } else if let Some(path) = find_codex_plugin_cache_path(marketplace_name, plugin_name) {
                path
            } else {
                codex_plugin_cache_dir(marketplace_name, plugin_name, &version)?
            };
            Ok(CodexPluginInstallRef {
                plugin_name: plugin_name.to_string(),
                marketplace_name: marketplace_name.to_string(),
                version,
                source,
            })
        })
        .collect()
}

pub fn apply_codex_plugin(
    staging: &Path,
    item_name: &str,
    codex_plugin_keys: &[String],
) -> Result<(), String> {
    if !staging.exists() {
        return Err("本地缓存不存在".into());
    }

    let installs = codex_installs_from_plugin_keys(codex_plugin_keys, staging, item_name)?;
    if installs.is_empty() {
        return Err("未找到可安装的 Codex 插件".into());
    }

    if let Some(first) = installs.first() {
        let marketplace_root = if staging.join(".agents/plugins/marketplace.json").is_file() {
            persist_codex_marketplace(staging, &first.marketplace_name)?
        } else {
            staging.to_path_buf()
        };
        register_codex_marketplace(&marketplace_root, &first.marketplace_name)?;
    }

    for install in &installs {
        install_codex_marketplace_plugin(install)?;
    }

    remove_legacy_codex_plugin_path(item_name)
}

pub fn remove_codex_plugin_install(staging: &Path, item_name: &str) -> Result<(), String> {
    let installs = resolve_codex_plugin_installs(staging, item_name)
        .unwrap_or_else(|_| infer_codex_installs_from_config(item_name));
    let marketplace_names: HashSet<String> = installs
        .iter()
        .map(|install| install.marketplace_name.clone())
        .collect();

    for install in &installs {
        uninstall_codex_marketplace_plugin(install)?;
    }

    for marketplace_name in marketplace_names {
        unregister_codex_marketplace_if_empty(&marketplace_name)?;
    }

    cleanup_codex_marketplace_refs_for_item(item_name)?;
    Ok(())
}

fn infer_codex_installs_from_config(item_name: &str) -> Vec<CodexPluginInstallRef> {
    let Ok(config) = read_codex_config() else {
        return Vec::new();
    };
    let Some(plugins) = config.get("plugins").and_then(|value| value.as_table()) else {
        return Vec::new();
    };

    plugins
        .keys()
        .filter_map(|plugin_key| {
            let (plugin_name, marketplace_name) = plugin_key.rsplit_once('@')?;
            if plugin_name != item_name && !plugin_key.contains(item_name) {
                return None;
            }
            let version = find_codex_plugin_cache_path(marketplace_name, plugin_name)
                .and_then(|path| {
                    path.file_name()
                        .map(|name| name.to_string_lossy().to_string())
                })
                .unwrap_or_else(|| "local".to_string());
            let source = find_codex_plugin_cache_path(marketplace_name, plugin_name)
                .unwrap_or_else(|| PathBuf::from("."));
            Some(CodexPluginInstallRef {
                plugin_name: plugin_name.to_string(),
                marketplace_name: marketplace_name.to_string(),
                version,
                source,
            })
        })
        .collect()
}

fn codex_marketplace_has_installed_plugins(marketplace_name: &str) -> Result<bool, String> {
    let suffix = format!("@{marketplace_name}");
    let config = read_codex_config()?;
    Ok(config
        .get("plugins")
        .and_then(|value| value.as_table())
        .is_some_and(|plugins| plugins.keys().any(|key| key.ends_with(&suffix))))
}

fn unregister_codex_marketplace(marketplace_name: &str) -> Result<(), String> {
    let mut config = read_codex_config()?;
    if let Some(marketplaces) = config
        .get_mut("marketplaces")
        .and_then(|value| value.as_table_mut())
    {
        marketplaces.remove(marketplace_name);
    }
    write_codex_config(&config)?;
    remove_path_all(&codex_marketplace_dir(marketplace_name)?)
}

fn unregister_codex_marketplace_if_empty(marketplace_name: &str) -> Result<(), String> {
    if codex_marketplace_has_installed_plugins(marketplace_name)? {
        return Ok(());
    }
    unregister_codex_marketplace(marketplace_name)
}

pub fn cleanup_codex_marketplace_refs_for_item(item_name: &str) -> Result<(), String> {
    let config = read_codex_config()?;
    let Some(marketplaces) = config
        .get("marketplaces")
        .and_then(|value| value.as_table())
    else {
        return Ok(());
    };

    let marker = format!("/{item_name}");
    let to_remove: Vec<String> = marketplaces
        .iter()
        .filter_map(|(marketplace_name, entry)| {
            let source = entry.get("source").and_then(|value| value.as_str())?;
            if !source.contains(&marker) && !source.ends_with(&format!("/{item_name}")) {
                return None;
            }
            let no_plugins =
                !codex_marketplace_has_installed_plugins(marketplace_name).unwrap_or(true);
            if no_plugins {
                Some(marketplace_name.clone())
            } else {
                None
            }
        })
        .collect();

    for marketplace_name in to_remove {
        unregister_codex_marketplace(&marketplace_name)?;
    }
    Ok(())
}

pub fn find_codex_plugin_cache_path(marketplace_name: &str, plugin_name: &str) -> Option<PathBuf> {
    let base = home_dir()
        .ok()?
        .join(".codex/plugins/cache")
        .join(marketplace_name)
        .join(plugin_name);
    if !base.is_dir() {
        return None;
    }
    fs::read_dir(&base).ok()?.find_map(|entry| {
        let entry = entry.ok()?;
        let path = entry.path();
        if path.is_dir() {
            Some(path)
        } else {
            None
        }
    })
}

pub fn codex_plugin_keys_installed(keys: &[String]) -> bool {
    if keys.is_empty() {
        return false;
    }
    keys.iter().all(|plugin_key| {
        let Some((plugin_name, marketplace_name)) = plugin_key.rsplit_once('@') else {
            return false;
        };
        find_codex_plugin_cache_path(marketplace_name, plugin_name)
            .map(|path| {
                path.join(".codex-plugin/plugin.json").is_file()
                    || path.join(".claude-plugin/plugin.json").is_file()
                    || path.join("SKILL.md").is_file()
            })
            .unwrap_or(false)
    })
}

pub fn codex_plugin_enabled_in_config(keys: &[String]) -> bool {
    let Ok(config) = read_codex_config() else {
        return false;
    };
    let Some(plugins) = config.get("plugins").and_then(|value| value.as_table()) else {
        return false;
    };
    keys.iter().all(|key| {
        plugins
            .get(key)
            .and_then(|value| value.get("enabled"))
            .and_then(|value| value.as_bool())
            .unwrap_or(false)
    })
}

pub fn scan_codex_installed_plugins(
    registry: &mut crate::store::MarketplaceRegistry,
    copy_to_staging: impl Fn(&str, &Path) -> Result<PathBuf, String>,
) -> Result<u32, String> {
    let mut discovered = 0u32;
    let config = read_codex_config()?;
    let plugins = match config.get("plugins").and_then(|value| value.as_table()) {
        Some(plugins) => plugins,
        None => return Ok(0),
    };

    for (plugin_key, entry) in plugins {
        let Some((plugin_name, _marketplace_name)) = plugin_key.rsplit_once('@') else {
            continue;
        };
        let key = format!("codex:plugin:{plugin_name}");
        if registry.suppressed_keys.contains(&key) {
            continue;
        }

        let enabled = entry
            .get("enabled")
            .and_then(|value| value.as_bool())
            .unwrap_or(false);
        let codex_plugin_keys = vec![plugin_key.clone()];
        let (marketplace_name, _) = plugin_key.rsplit_once('@').unwrap_or((plugin_name, ""));
        let cache_path = find_codex_plugin_cache_path(marketplace_name, plugin_name);

        if let Some(record) = registry.items.get_mut(&key) {
            record.codex_plugin_keys = codex_plugin_keys;
            record.enabled = enabled;
            continue;
        }

        let staging =
            copy_to_staging(plugin_name, cache_path.as_deref().unwrap_or(Path::new(".")))?;
        if let Some(ref cache) = cache_path {
            if !staging.exists() {
                copy_dir_all(cache, &staging)?;
            }
        }

        registry.items.insert(
            key,
            crate::store::MarketplaceItemRecord {
                remote_id: None,
                name: plugin_name.to_string(),
                platform: "codex".to_string(),
                item_type: "plugin".to_string(),
                description: cache_path
                    .as_ref()
                    .and_then(|path| read_local_plugin_description(path)),
                version: cache_path
                    .as_ref()
                    .and_then(|path| read_local_plugin_version(path)),
                enabled,
                remote_available: false,
                origin: "local".to_string(),
                updated_at: None,
                claude_plugin_keys: Vec::new(),
                codex_plugin_keys,
            },
        );
        discovered += 1;
    }

    Ok(discovered)
}

pub fn sync_codex_plugin_enabled_states(registry: &mut crate::store::MarketplaceRegistry) -> bool {
    let Ok(config) = read_codex_config() else {
        return false;
    };
    let Some(plugins) = config.get("plugins").and_then(|value| value.as_table()) else {
        return false;
    };

    let mut changed = false;
    for record in registry.items.values_mut() {
        if record.platform != "codex" || record.item_type != "plugin" {
            continue;
        }
        if record.codex_plugin_keys.is_empty() {
            continue;
        }
        let enabled = record.codex_plugin_keys.iter().all(|key| {
            plugins
                .get(key)
                .and_then(|value| value.get("enabled"))
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

pub fn is_valid_codex_plugin_dir(path: &Path) -> bool {
    path.join(".codex-plugin/plugin.json").is_file()
        || path.join(".agents/plugins/marketplace.json").is_file()
}

fn read_local_plugin_description(path: &Path) -> Option<String> {
    let manifest = if path.join(".codex-plugin/plugin.json").is_file() {
        path.join(".codex-plugin/plugin.json")
    } else {
        path.join(".claude-plugin/plugin.json")
    };
    let value: Value = serde_json::from_str(&fs::read_to_string(manifest).ok()?).ok()?;
    value
        .get("description")
        .or_else(|| value.pointer("/interface/shortDescription"))
        .and_then(|entry| entry.as_str())
        .map(str::to_string)
}

fn read_local_plugin_version(path: &Path) -> Option<String> {
    let manifest = if path.join(".codex-plugin/plugin.json").is_file() {
        path.join(".codex-plugin/plugin.json")
    } else {
        path.join(".claude-plugin/plugin.json")
    };
    let value: Value = serde_json::from_str(&fs::read_to_string(manifest).ok()?).ok()?;
    value
        .get("version")
        .and_then(|entry| entry.as_str())
        .map(str::to_string)
}

fn remove_legacy_codex_plugin_path(item_name: &str) -> Result<(), String> {
    let legacy = home_dir()?.join(".codex/plugins").join(item_name);
    remove_path_all(&legacy)
}

fn home_dir() -> Result<PathBuf, String> {
    dirs::home_dir().ok_or_else(|| "无法获取用户目录".to_string())
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
