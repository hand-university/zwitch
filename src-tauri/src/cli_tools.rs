use crate::config::LOCAL_PROXY_HOST;
use crate::grayscale_api::{
    fetch_grayscale_models, find_platform_models, grayscale_model_display_name,
    merge_opencode_platform_response, prioritize_grayscale_default_model,
    resolve_opencode_grayscale_platform, GrayscaleModelEntry, GrayscalePlatformModels,
    GRAYSCALE_DEFAULT_MODEL_ID,
};
use crate::store::StoredSettings;
use crate::store::{load_auth, load_backup, load_settings, save_backup};
use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::collections::HashSet;
use std::ffi::{OsStr, OsString};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use tauri::AppHandle;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CliToolStatus {
    pub id: String,
    pub name: String,
    pub supported: bool,
    /// 本机是否检测到 CLI 可执行文件。
    pub installed: bool,
    /// 终端安装命令（通常为 curl 脚本）。
    pub install_shell: String,
    /// 官方快速开始文档地址。
    pub quick_start_doc_url: String,
    /// 用户是否允许为该工具注入配置（默认开启）。
    pub config_enabled: bool,
    pub config_path: String,
    pub base_url_field: String,
    pub token_field: String,
}

enum ConfigFormat {
    Json,
    Toml,
    DotEnv,
    CodexProviderToml,
    OpencodeProviders,
}

/// 旧版注入使用的自定义 provider，还原时需清理。
const LEGACY_CODEX_MODEL_PROVIDER: &str = "zsdx_ai";
const CODEX_OPENAI_BASE_URL_KEY: &str = "openai_base_url";

const CODEX_CONFIG_FIELDS: &[&str] = &[
    "snapshot",
    "openai_base_url",
    "model",
    "model_provider",
    "model_field_order",
    "model_catalog_json",
    "model_providers.zsdx_ai",
];

const CLAUDE_CUSTOM_MODEL_ENV_PREFIX: &str = "ANTHROPIC_CUSTOM_MODEL_OPTION";
const CLAUDE_GATEWAY_DISCOVERY_ENV: &str = "CLAUDE_CODE_ENABLE_GATEWAY_MODEL_DISCOVERY";
const ZWITCH_MANAGED_MODEL_CATALOG_MARKER: &str = "zwitch-grayscale-catalog.json";
const CODEX_GRAYSCALE_CATALOG_RELATIVE: &str = ".codex/zwitch-grayscale-catalog.json";

/// 备份中标记「注入前不存在」的占位值。
const BACKUP_ABSENT: &str = "\0zwitch_absent\0";

enum FieldValue {
    BaseUrl,
}

struct FieldSpec {
    path: &'static str,
    value: FieldValue,
}

struct ConfigTarget {
    relative: &'static str,
    format: ConfigFormat,
    fields: &'static [FieldSpec],
}

struct ToolDefinition {
    id: &'static str,
    name: &'static str,
    binaries: &'static [&'static str],
    base_url_field: &'static str,
    token_field: &'static str,
    install_shell: &'static str,
    quick_start_doc_url: &'static str,
    targets: &'static [ConfigTarget],
}

const CODEX_TARGETS: &[ConfigTarget] = &[ConfigTarget {
    relative: ".codex/config.toml",
    format: ConfigFormat::CodexProviderToml,
    fields: &[],
}];

const CLAUDE_TARGETS: &[ConfigTarget] = &[ConfigTarget {
    relative: ".claude/settings.json",
    format: ConfigFormat::Json,
    fields: &[FieldSpec {
        path: "env.ANTHROPIC_BASE_URL",
        value: FieldValue::BaseUrl,
    }],
}];

const GEMINI_TARGETS: &[ConfigTarget] = &[ConfigTarget {
    relative: ".gemini/.env",
    format: ConfigFormat::DotEnv,
    fields: &[FieldSpec {
        path: "GOOGLE_GEMINI_BASE_URL",
        value: FieldValue::BaseUrl,
    }],
}];

const OPENCODE_TARGETS: &[ConfigTarget] = &[ConfigTarget {
    relative: ".config/opencode/opencode.json",
    format: ConfigFormat::OpencodeProviders,
    fields: &[],
}];

const OPENCODE_MANAGED_PROVIDERS_BACKUP_KEY: &str = "managed_providers";
/// OpenCode 自定义 Provider 需要 apiKey 字段；实际鉴权由本地代理注入，此处仅占位。
const OPENCODE_PROXY_API_KEY_PLACEHOLDER: &str = "zwitch";
/// OpenAI 兼容 Provider 使用的 AI SDK 适配器（`/v1/chat/completions`）。
const OPENCODE_OPENAI_COMPATIBLE_NPM: &str = "@ai-sdk/openai-compatible";
/// 使用 `/v1/responses` 的 Provider 改用官方 OpenAI 适配器。
const OPENCODE_OPENAI_RESPONSES_NPM: &str = "@ai-sdk/openai";
/// OpenCode 灰度模型未声明 `context_window` 时的默认上下文窗口（tokens）。
const OPENCODE_DEFAULT_CONTEXT_WINDOW: u64 = 1_000_000;
/// OpenCode schema 要求 `limit.output` 与 `limit.context` 同时存在时的默认最大输出（tokens）。
const OPENCODE_DEFAULT_MAX_OUTPUT: u64 = 65_536;

const TOOLS: &[ToolDefinition] = &[
    ToolDefinition {
        id: "codex",
        name: "Codex CLI",
        binaries: &["codex"],
        base_url_field: "openai_base_url",
        token_field: "OPENAI_API_KEY",
        install_shell: "curl -fsSL https://chatgpt.com/codex/install.sh | sh",
        quick_start_doc_url: "https://developers.openai.com/codex/cli",
        targets: CODEX_TARGETS,
    },
    ToolDefinition {
        id: "claude",
        name: "Claude Code",
        binaries: &["claude"],
        base_url_field: "env.ANTHROPIC_BASE_URL",
        token_field: "env.ANTHROPIC_AUTH_TOKEN",
        install_shell: "curl -fsSL https://claude.ai/install.sh | bash",
        quick_start_doc_url: "https://code.claude.com/docs/en/quickstart",
        targets: CLAUDE_TARGETS,
    },
    ToolDefinition {
        id: "opencode",
        name: "OpenCode",
        binaries: &["opencode"],
        base_url_field: "provider.*.options.baseURL",
        token_field: "provider.*.options.apiKey",
        install_shell: "curl -fsSL https://opencode.ai/install | bash",
        quick_start_doc_url: "https://opencode.ai/docs/",
        targets: OPENCODE_TARGETS,
    },
];

/// 暂时禁用的工具：不参与检测/注入，但会尝试还原已注入的配置。
const DISABLED_TOOLS: &[ToolDefinition] = &[ToolDefinition {
    id: "gemini",
    name: "Gemini CLI",
    binaries: &["gemini"],
    base_url_field: "GOOGLE_GEMINI_BASE_URL",
    token_field: "GEMINI_API_KEY",
    install_shell: "npm install -g @google/gemini-cli",
    quick_start_doc_url: "https://github.com/google-gemini/gemini-cli",
    targets: GEMINI_TARGETS,
}];

/// 暴露所有受支持工具的 id，供本地拦截服务建立上游路由映射。
pub fn tool_ids() -> Vec<&'static str> {
    TOOLS.iter().map(|tool| tool.id).collect()
}

pub fn is_tool_config_enabled(settings: &StoredSettings, tool_id: &str) -> bool {
    settings.tool_switches.get(tool_id).copied().unwrap_or(true)
}

fn tool_should_inject(settings: &StoredSettings, tool: &ToolDefinition) -> bool {
    settings.proxy_enabled && is_tool_config_enabled(settings, tool.id)
}

fn is_tool_installed(tool: &ToolDefinition) -> bool {
    find_binary(tool.binaries).is_some()
}

pub async fn set_tool_config_enabled_async(
    app: &AppHandle,
    tool_id: &str,
    enabled: bool,
) -> Result<(), String> {
    if !TOOLS.iter().any(|tool| tool.id == tool_id) {
        return Err(format!("未知 CLI 工具: {tool_id}"));
    }

    let mut settings = load_settings(app)?;
    if enabled {
        settings.tool_switches.remove(tool_id);
    } else {
        settings.tool_switches.insert(tool_id.to_string(), false);
    }
    crate::store::save_settings(app, &settings)?;
    apply_config_injection_async(app).await?;
    Ok(())
}

pub async fn get_cli_tools_status_async(app: &AppHandle) -> Result<Vec<CliToolStatus>, String> {
    let home = dirs::home_dir().ok_or_else(|| "无法获取用户目录".to_string())?;
    let settings = load_settings(app)?;

    let show_opencode = if load_auth(app)
        .ok()
        .and_then(|auth| auth.authorization_code)
        .is_some()
    {
        let grayscale_platforms = load_grayscale_platforms_by_tool(app).await;
        let grayscale_models: HashMap<String, Vec<GrayscaleModelEntry>> = grayscale_platforms
            .iter()
            .map(|(tool_id, platform)| (tool_id.clone(), platform.additional_models.clone()))
            .collect();
        crate::grayscale_api::update_grayscale_cache(grayscale_models);
        grayscale_platforms.contains_key("opencode")
    } else {
        false
    };

    Ok(TOOLS
        .iter()
        .filter(|tool| tool.id != "opencode" || show_opencode)
        .map(|tool| {
            let config_path = home.join(tool.targets[0].relative);
            CliToolStatus {
                id: tool.id.to_string(),
                name: tool.name.to_string(),
                supported: true,
                installed: is_tool_installed(tool),
                install_shell: tool.install_shell.to_string(),
                quick_start_doc_url: tool.quick_start_doc_url.to_string(),
                config_enabled: is_tool_config_enabled(&settings, tool.id),
                config_path: config_path.to_string_lossy().to_string(),
                base_url_field: tool.base_url_field.to_string(),
                token_field: tool.token_field.to_string(),
            }
        })
        .collect())
}

pub fn apply_config_injection(app: &AppHandle) -> Result<(), String> {
    tauri::async_runtime::block_on(apply_config_injection_async(app))
}

pub async fn apply_config_injection_async(app: &AppHandle) -> Result<(), String> {
    let settings = load_settings(app)?;
    let auth = load_auth(app)?;
    let mut backup = load_backup(app)?;
    let mut backup_dirty = false;

    let home = dirs::home_dir().ok_or_else(|| "无法获取用户目录".to_string())?;

    let needs_inject = TOOLS.iter().any(|tool| tool_should_inject(&settings, tool));

    if needs_inject && auth.authorization_code.is_none() {
        return Err("请先登录并完成设备授权".to_string());
    }

    let grayscale_platforms = if needs_inject {
        load_grayscale_platforms_by_tool(app).await
    } else {
        HashMap::new()
    };
    let grayscale_models: HashMap<String, Vec<GrayscaleModelEntry>> = grayscale_platforms
        .iter()
        .map(|(tool_id, platform)| (tool_id.clone(), platform.additional_models.clone()))
        .collect();
    crate::grayscale_api::update_grayscale_cache(grayscale_models.clone());

    let mut proxy_port = crate::proxy::local_port();
    if needs_inject {
        let preferred = detect_preferred_proxy_port(&home, &settings)?;
        if let Ok(fingerprint) = crate::device::get_or_create_fingerprint(app) {
            if let Some(started_port) =
                crate::proxy::ensure_listening(app.clone(), fingerprint, preferred)
            {
                proxy_port = started_port;
            }
        }
    }

    for tool in TOOLS {
        let tool_enabled = tool_should_inject(&settings, tool);
        let additional_models = grayscale_models
            .get(tool.id)
            .cloned()
            .unwrap_or_default();

        for target in tool.targets {
            let config_path = home.join(target.relative);

            if tool.id == "opencode" {
                let platform = grayscale_platforms.get(tool.id);
                if !tool_enabled
                    || platform.is_none_or(|platform| platform.additional_models.is_empty())
                {
                    if restore_config(app, &mut backup, tool, target, &config_path)? {
                        backup_dirty = true;
                    }
                    continue;
                }

                if proxy_port == 0 {
                    eprintln!("OpenCode: 本地代理未启动，跳过 opencode.json 注入");
                    continue;
                }

                let already_injected = read_config_if_exists(&config_path)
                    .map(|content| is_target_injected(tool, target, &content))
                    .unwrap_or(false);

                if !already_injected {
                    backup_current_values(&mut backup, tool, target, &config_path)?;
                    backup_dirty = true;
                }

                let local_base = if proxy_port != 0 {
                    crate::proxy::local_proxy_base_for_port(tool.id, proxy_port)
                } else {
                    crate::proxy::local_proxy_base(tool.id)
                };
                inject_opencode_grayscale_models(
                    tool,
                    target,
                    &config_path,
                    &local_base,
                    platform.expect("opencode platform checked above"),
                )?;
                continue;
            }

            let local_base = if proxy_port != 0 {
                crate::proxy::local_proxy_base_for_port(tool.id, proxy_port)
            } else {
                crate::proxy::local_proxy_base(tool.id)
            };

            if !tool_enabled {
                if restore_config(app, &mut backup, tool, target, &config_path)? {
                    backup_dirty = true;
                }
                continue;
            }

            let already_injected = read_config_if_exists(&config_path)
                .map(|content| is_target_injected(tool, target, &content))
                .unwrap_or(false);

            if !already_injected {
                backup_current_values(&mut backup, tool, target, &config_path)?;
                backup_dirty = true;
            }

            inject_values(
                tool,
                target,
                &config_path,
                &local_base,
                &additional_models,
                &home,
            )?;
        }
    }

    for tool in DISABLED_TOOLS {
        for target in tool.targets {
            let config_path = home.join(target.relative);
            let Some(content) = read_config_if_exists(&config_path) else {
                continue;
            };
            if !is_target_injected(tool, target, &content) {
                continue;
            }
            if restore_config(app, &mut backup, tool, target, &config_path)? {
                backup_dirty = true;
            } else if matches!(target.format, ConfigFormat::DotEnv) {
                let mut content = fs::read_to_string(&config_path)
                    .map_err(|e| format!("读取配置失败: {e}"))?;
                for field in target.fields {
                    if read_dotenv_value(&content, field.path)
                        .is_some_and(|url| is_local_proxy_base_url(&url, tool.id))
                    {
                        content = remove_dotenv_key(&content, field.path)?;
                    }
                }
                write_config(&config_path, &content)?;
            }
        }
    }

    if backup_dirty {
        save_backup(app, &backup)?;
    }
    Ok(())
}

/// GUI 应用启动时 PATH 往往不含 Homebrew / fnm 等目录，需扩展后再查找。
fn executable_search_path() -> &'static OsStr {
    static PATH: OnceLock<OsString> = OnceLock::new();
    PATH.get_or_init(|| {
        OsString::from(
            collect_executable_search_dirs()
                .into_iter()
                .map(|dir| dir.to_string_lossy().into_owned())
                .collect::<Vec<_>>()
                .join(if cfg!(windows) { ";" } else { ":" }),
        )
    })
    .as_os_str()
}

fn collect_executable_search_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    let mut seen = HashSet::new();
    let mut push = |dir: PathBuf| {
        if dir.as_os_str().is_empty() || !seen.insert(dir.clone()) {
            return;
        }
        dirs.push(dir);
    };

    if let Some(path) = std::env::var_os("PATH") {
        for dir in std::env::split_paths(&path) {
            push(dir);
        }
    }

    #[cfg(target_os = "macos")]
    append_macos_system_paths(&mut push);

    if let Some(home) = dirs::home_dir() {
        for suffix in [
            ".local/bin",
            ".cargo/bin",
            ".bun/bin",
            ".npm-global/bin",
            "bin",
        ] {
            push(home.join(suffix));
        }
        append_version_manager_bin_dirs(&home, &mut push);
    }

    push(PathBuf::from("/opt/homebrew/bin"));
    push(PathBuf::from("/opt/homebrew/sbin"));
    push(PathBuf::from("/usr/local/bin"));

    #[cfg(target_os = "macos")]
    for dir in macos_login_shell_path_dirs() {
        push(dir);
    }

    dirs
}

#[cfg(target_os = "macos")]
fn append_macos_system_paths(push: &mut impl FnMut(PathBuf)) {
    if let Ok(content) = fs::read_to_string("/etc/paths") {
        for line in content.lines() {
            let line = line.trim();
            if !line.is_empty() {
                push(PathBuf::from(line));
            }
        }
    }
    if let Ok(entries) = fs::read_dir("/etc/paths.d") {
        for entry in entries.flatten() {
            if let Ok(content) = fs::read_to_string(entry.path()) {
                for line in content.lines() {
                    let line = line.trim();
                    if !line.is_empty() {
                        push(PathBuf::from(line));
                    }
                }
            }
        }
    }
}

/// 扫描 fnm / nvm / volta / mise / asdf 等工具链目录下的 bin。
fn append_version_manager_bin_dirs(home: &Path, push: &mut impl FnMut(PathBuf)) {
    append_nested_bin_dirs(
        &home.join(".local/share/fnm/node-versions"),
        &["installation", "bin"],
        push,
    );
    push(home.join(".local/share/fnm/aliases/default/bin"));
    append_nested_bin_dirs(&home.join(".local/state/fnm_multishells"), &["bin"], push);
    append_nested_bin_dirs(&home.join(".nvm/versions/node"), &["bin"], push);
    push(home.join(".volta/bin"));
    push(home.join(".local/share/mise/shims"));
    push(home.join(".asdf/shims"));
}

fn append_nested_bin_dirs(base: &Path, suffix: &[&str], push: &mut impl FnMut(PathBuf)) {
    let Ok(entries) = fs::read_dir(base) else {
        return;
    };
    for entry in entries.flatten() {
        if !entry.path().is_dir() {
            continue;
        }
        let mut path = entry.path();
        for part in suffix {
            path = path.join(part);
        }
        push(path);
    }
}

#[cfg(unix)]
fn find_binary_via_login_shell(name: &str) -> Option<String> {
    let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".to_string());
    let output = std::process::Command::new(&shell)
        .args(["-l", "-c", &format!("command -v -- {name}")])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let path = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if path.is_empty() {
        return None;
    }
    let candidate = PathBuf::from(&path);
    if candidate.is_file() {
        Some(path)
    } else {
        None
    }
}

#[cfg(not(unix))]
fn find_binary_via_login_shell(_name: &str) -> Option<String> {
    None
}

#[cfg(target_os = "macos")]
fn macos_login_shell_path_dirs() -> Vec<PathBuf> {
    let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/zsh".to_string());
    let output = match std::process::Command::new(&shell)
        .args(["-l", "-c", "printf %s \"$PATH\""])
        .output()
    {
        Ok(output) if output.status.success() => output,
        _ => return Vec::new(),
    };
    let path = String::from_utf8_lossy(&output.stdout).into_owned();
    std::env::split_paths(&path).collect()
}

fn find_binary(names: &[&str]) -> Option<String> {
    let paths = executable_search_path();
    for name in names {
        if let Ok(path) = which::which_in(name, Some(paths), ".") {
            return Some(path.to_string_lossy().to_string());
        }
        if let Some(path) = find_binary_via_login_shell(name) {
            return Some(path);
        }
    }
    None
}

fn backup_key(tool_id: &str, field: &str) -> String {
    format!("{tool_id}::{field}")
}

fn resolve_field_value(spec: &FieldSpec, base_url: &str) -> String {
    match spec.value {
        FieldValue::BaseUrl => base_url.to_string(),
    }
}

fn backup_absent_fields(
    backup: &mut crate::store::ConfigBackup,
    tool: &ToolDefinition,
    target: &ConfigTarget,
    config_path: &Path,
) {
    let file_key = config_path.to_string_lossy().to_string();
    let entry = backup.files.entry(file_key).or_default();

    match target.format {
        ConfigFormat::Json => {
            for field in target.fields {
                entry
                    .entry(backup_key(tool.id, field.path))
                    .or_insert_with(|| BACKUP_ABSENT.to_string());
            }
        }
        ConfigFormat::Toml => {}
        ConfigFormat::CodexProviderToml => {
            for field in codex_config_fields() {
                if *field == "snapshot" {
                    continue;
                }
                entry
                    .entry(backup_key(tool.id, field))
                    .or_insert_with(|| BACKUP_ABSENT.to_string());
            }
        }
        ConfigFormat::DotEnv => {
            for field in target.fields {
                entry
                    .entry(backup_key(tool.id, field.path))
                    .or_insert_with(|| BACKUP_ABSENT.to_string());
            }
        }
        ConfigFormat::OpencodeProviders => {
            entry
                .entry(backup_key(tool.id, "snapshot"))
                .or_insert_with(|| BACKUP_ABSENT.to_string());
            entry
                .entry(backup_key(tool.id, OPENCODE_MANAGED_PROVIDERS_BACKUP_KEY))
                .or_insert_with(|| String::new());
        }
    }
}

fn backup_current_values(
    backup: &mut crate::store::ConfigBackup,
    tool: &ToolDefinition,
    target: &ConfigTarget,
    config_path: &Path,
) -> Result<(), String> {
    if !config_path.exists() {
        backup_absent_fields(backup, tool, target, config_path);
        return Ok(());
    }

    let content = fs::read_to_string(config_path)
        .map_err(|e| format!("读取配置失败 {}: {e}", config_path.display()))?;

    let file_key = config_path.to_string_lossy().to_string();
    let entry = backup.files.entry(file_key).or_default();

    match target.format {
        ConfigFormat::Json => {
            let value: Value =
                serde_json::from_str(&content).map_err(|e| format!("解析 JSON 失败: {e}"))?;
            for field in target.fields {
                let backup_field_key = backup_key(tool.id, field.path);
                if entry.contains_key(&backup_field_key) {
                    continue;
                }
                let stored = read_json_field(&value, field.path)
                    .unwrap_or_else(|| BACKUP_ABSENT.to_string());
                entry.insert(backup_field_key, stored);
            }
            if tool.id == "claude" {
                backup_claude_grayscale_env(entry, tool.id, &value);
            }
        }
        ConfigFormat::Toml => {
            for field in target.fields {
                if let Some(current) = read_toml_value(&content, field.path) {
                    entry
                        .entry(backup_key(tool.id, field.path))
                        .or_insert(current);
                }
            }
        }
        ConfigFormat::CodexProviderToml => {
            backup_codex_provider_toml(entry, tool.id, &content);
        }
        ConfigFormat::DotEnv => {
            for field in target.fields {
                let backup_field_key = backup_key(tool.id, field.path);
                if entry.contains_key(&backup_field_key) {
                    continue;
                }
                let stored = read_dotenv_value(&content, field.path)
                    .unwrap_or_else(|| BACKUP_ABSENT.to_string());
                entry.insert(backup_field_key, stored);
            }
        }
        ConfigFormat::OpencodeProviders => {
            backup_opencode_providers(entry, tool.id, &content);
        }
    }

    Ok(())
}

fn read_config_if_exists(config_path: &Path) -> Option<String> {
    if !config_path.exists() {
        return None;
    }
    fs::read_to_string(config_path).ok()
}

fn is_local_proxy_base_url(url: &str, tool_id: &str) -> bool {
    let prefix = format!("http://{LOCAL_PROXY_HOST}:");
    url.starts_with(&prefix) && url.ends_with(&format!("/{tool_id}"))
}

fn is_target_injected(tool: &ToolDefinition, target: &ConfigTarget, content: &str) -> bool {
    match target.format {
        ConfigFormat::CodexProviderToml => is_zwitch_injected(content),
        ConfigFormat::Json => serde_json::from_str::<Value>(content)
            .ok()
            .is_some_and(|value| {
                target.fields.iter().any(|field| {
                    read_json_field(&value, field.path)
                        .is_some_and(|url| is_local_proxy_base_url(&url, tool.id))
                })
            }),
        ConfigFormat::DotEnv => target.fields.iter().any(|field| {
            read_dotenv_value(content, field.path)
                .is_some_and(|url| is_local_proxy_base_url(&url, tool.id))
        }),
        ConfigFormat::Toml => target.fields.iter().any(|field| {
            read_toml_value(content, field.path)
                .is_some_and(|url| is_local_proxy_base_url(&url, tool.id))
        }),
        ConfigFormat::OpencodeProviders => opencode_managed_provider_keys(content)
            .map(|keys| !keys.is_empty())
            .unwrap_or(false),
    }
}

fn read_injected_proxy_port(
    tool: &ToolDefinition,
    target: &ConfigTarget,
    content: &str,
) -> Option<u16> {
    if !is_target_injected(tool, target, content) {
        return None;
    }

    match target.format {
        ConfigFormat::CodexProviderToml => read_top_level_toml_value(content, CODEX_OPENAI_BASE_URL_KEY)
            .and_then(|url| crate::proxy::parse_local_proxy_port(&url))
            .or_else(|| {
                extract_toml_section(content, legacy_codex_provider_table_header())
                    .and_then(|body| read_toml_assignment(&body, "base_url"))
                    .and_then(|url| crate::proxy::parse_local_proxy_port(&url))
            }),
        ConfigFormat::Json => serde_json::from_str::<Value>(content)
            .ok()
            .and_then(|value| {
                target.fields.iter().find_map(|field| {
                    read_json_field(&value, field.path)
                        .and_then(|url| crate::proxy::parse_local_proxy_port(&url))
                })
            }),
        ConfigFormat::DotEnv => target.fields.iter().find_map(|field| {
            read_dotenv_value(content, field.path)
                .and_then(|url| crate::proxy::parse_local_proxy_port(&url))
        }),
        ConfigFormat::Toml => target.fields.iter().find_map(|field| {
            read_toml_value(content, field.path)
                .and_then(|url| crate::proxy::parse_local_proxy_port(&url))
        }),
        ConfigFormat::OpencodeProviders => serde_json::from_str::<Value>(content)
            .ok()
            .and_then(|value| {
                value
                    .get("provider")
                    .and_then(Value::as_object)
                    .and_then(|providers| {
                        providers.values().find_map(|provider| {
                            opencode_provider_base_url(provider).and_then(|url| {
                                if is_local_proxy_opencode_provider_base_url(url) {
                                    crate::proxy::parse_local_proxy_port(url)
                                } else {
                                    None
                                }
                            })
                        })
                    })
            }),
    }
}

fn detect_preferred_proxy_port(
    home: &Path,
    settings: &StoredSettings,
) -> Result<Option<u16>, String> {
    let mut ports = Vec::new();

    for tool in TOOLS {
        if !tool_should_inject(settings, tool) {
            continue;
        }

        for target in tool.targets {
            let config_path = home.join(target.relative);
            let Some(content) = read_config_if_exists(&config_path) else {
                continue;
            };
            if let Some(port) = read_injected_proxy_port(tool, target, &content) {
                ports.push(port);
            }
        }
    }

    if ports.is_empty() {
        return Ok(None);
    }

    let first = ports[0];
    if ports.iter().all(|port| *port == first) {
        Ok(Some(first))
    } else {
        Ok(Some(first))
    }
}

fn restore_config(
    app: &AppHandle,
    backup: &mut crate::store::ConfigBackup,
    tool: &ToolDefinition,
    target: &ConfigTarget,
    config_path: &Path,
) -> Result<bool, String> {
    if !config_path.exists() {
        return Ok(false);
    }

    let file_key = config_path.to_string_lossy().to_string();
    let file_backup = match backup.files.get(&file_key) {
        Some(entry) => entry.clone(),
        None if matches!(target.format, ConfigFormat::CodexProviderToml) => HashMap::new(),
        None => return Ok(false),
    };

    let mut content = fs::read_to_string(config_path).map_err(|e| format!("读取配置失败: {e}"))?;

    for field in target.fields {
        let key = backup_key(tool.id, field.path);
        if let Some(original) = file_backup.get(&key) {
            content = match target.format {
                ConfigFormat::Json => {
                    let mut value: Value = serde_json::from_str(&content)
                        .map_err(|e| format!("解析 JSON 失败: {e}"))?;
                    if original == BACKUP_ABSENT {
                        remove_json_field(&mut value, field.path);
                    } else {
                        write_json_field(&mut value, field.path, original);
                    }
                    serde_json::to_string_pretty(&value).map_err(|e| e.to_string())?
                }
                ConfigFormat::Toml => write_toml_value(&content, field.path, original)?,
                ConfigFormat::DotEnv => {
                    if original == BACKUP_ABSENT {
                        remove_dotenv_key(&content, field.path)?
                    } else {
                        write_dotenv_value(&content, field.path, original)?
                    }
                }
                ConfigFormat::CodexProviderToml | ConfigFormat::OpencodeProviders => {
                    unreachable!()
                }
            };
        }
    }

    if matches!(target.format, ConfigFormat::OpencodeProviders) {
        content = restore_opencode_providers(&content, tool.id, &file_backup)?;
    }

    if matches!(target.format, ConfigFormat::CodexProviderToml) {
        content = restore_codex_provider_toml(&content, tool.id, &file_backup)?;
        if let Some(home) = dirs::home_dir() {
            let _ = remove_managed_grayscale_artifacts(tool, &home);
        }
    }

    if matches!(target.format, ConfigFormat::Json) && tool.id == "claude" {
        content = restore_claude_grayscale_env(&content, tool.id, &file_backup)?;
    }

    write_config(config_path, &content)?;
    save_backup(app, backup)?;
    Ok(true)
}

fn inject_values(
    _tool: &ToolDefinition,
    target: &ConfigTarget,
    config_path: &Path,
    base_url: &str,
    additional_models: &[GrayscaleModelEntry],
    _home: &Path,
) -> Result<(), String> {
    ensure_parent(config_path)?;

    let content = if config_path.exists() {
        fs::read_to_string(config_path).map_err(|e| format!("读取配置失败: {e}"))?
    } else {
        match target.format {
            ConfigFormat::Json => "{}".to_string(),
            ConfigFormat::Toml
            | ConfigFormat::DotEnv
            | ConfigFormat::CodexProviderToml
            | ConfigFormat::OpencodeProviders => String::new(),
        }
    };

    let new_content = match target.format {
        ConfigFormat::Json => {
            let mut value: Value = if content.trim().is_empty() {
                serde_json::json!({})
            } else {
                serde_json::from_str(&content).map_err(|e| format!("解析 JSON 失败: {e}"))?
            };

            for field in target.fields {
                write_json_field(
                    &mut value,
                    field.path,
                    &resolve_field_value(field, base_url),
                );
            }

            inject_claude_grayscale_models(&mut value, additional_models);
            serde_json::to_string_pretty(&value).map_err(|e| e.to_string())?
        }
        ConfigFormat::Toml => {
            let mut result = content;
            for field in target.fields {
                result =
                    write_toml_value(&result, field.path, &resolve_field_value(field, base_url))?;
            }
            result
        }
        ConfigFormat::CodexProviderToml => inject_codex_openai_proxy_config(&content, base_url)?,
        ConfigFormat::DotEnv => {
            let mut result = content;
            for field in target.fields {
                result =
                    write_dotenv_value(&result, field.path, &resolve_field_value(field, base_url))?;
            }
            result
        }
        ConfigFormat::OpencodeProviders => {
            unreachable!("OpenCode 配置在 apply_config_injection_async 中单独处理")
        }
    };

    write_config(config_path, &new_content)
}

fn opencode_provider_id(provider: &str) -> String {
    if provider.is_empty() {
        "openai".to_string()
    } else {
        provider.to_string()
    }
}

fn normalize_opencode_base_path(base_path: &str) -> String {
    let trimmed = base_path.trim();
    if trimmed.is_empty() {
        return "/openai".to_string();
    }
    if trimmed.starts_with('/') {
        trimmed.to_string()
    } else {
        format!("/{trimmed}")
    }
}

fn default_opencode_base_path_for_provider(provider: &str) -> String {
    match provider {
        "gemini" | "google" | "google-generative-ai" => "/genai".to_string(),
        "anthropic" => "/anthropic".to_string(),
        _ => "/openai".to_string(),
    }
}

fn infer_opencode_api_from_base_path(base_path: &str) -> &'static str {
    match normalize_opencode_base_path(base_path).as_str() {
        "/anthropic" => "anthropic-messages",
        "/genai" => "google-generative-ai",
        _ => "openai-responses",
    }
}

/// 根据灰度模型声明的 API 类型选择 OpenCode 使用的 AI SDK 适配器。
fn opencode_npm_for_api(api: &str) -> &'static str {
    match api {
        "openai-responses" => OPENCODE_OPENAI_RESPONSES_NPM,
        _ => OPENCODE_OPENAI_COMPATIBLE_NPM,
    }
}

fn resolve_opencode_provider_base_path(
    provider: &str,
    models: &[GrayscaleModelEntry],
    platform: &GrayscalePlatformModels,
) -> String {
    models
        .iter()
        .find_map(|model| model.base_path.clone())
        .or_else(|| {
            if platform.base_path.is_empty() {
                None
            } else {
                Some(platform.base_path.clone())
            }
        })
        .map(|path| normalize_opencode_base_path(&path))
        .unwrap_or_else(|| default_opencode_base_path_for_provider(provider))
}

fn resolve_opencode_provider_api(base_path: &str, models: &[GrayscaleModelEntry]) -> String {
    models
        .iter()
        .find_map(|model| model.api.clone())
        .unwrap_or_else(|| infer_opencode_api_from_base_path(base_path).to_string())
}

/// 构造 OpenCode `provider.<key>.models.<id>` 的取值。
fn build_opencode_model_entry(model: &GrayscaleModelEntry) -> Value {
    let mut entry = serde_json::Map::new();
    entry.insert(
        "name".to_string(),
        Value::String(grayscale_model_display_name(&model.id)),
    );

    let mut limit = serde_json::Map::new();
    let context_window = model
        .context_window
        .filter(|value| *value > 0)
        .unwrap_or(OPENCODE_DEFAULT_CONTEXT_WINDOW);
    let max_output = model
        .max_tokens
        .filter(|value| *value > 0)
        .unwrap_or(OPENCODE_DEFAULT_MAX_OUTPUT);
    limit.insert("context".to_string(), Value::Number(context_window.into()));
    limit.insert("output".to_string(), Value::Number(max_output.into()));
    entry.insert("limit".to_string(), Value::Object(limit));
    Value::Object(entry)
}

fn build_opencode_provider_base_url(local_proxy_base: &str, base_path: &str) -> String {
    let local_proxy_base = local_proxy_base.trim_end_matches('/');
    let base_path = normalize_opencode_base_path(base_path);
    format!("{local_proxy_base}{base_path}/v1")
}

fn is_local_proxy_opencode_provider_base_url(url: &str) -> bool {
    let prefix = format!("http://{LOCAL_PROXY_HOST}:");
    let Some(rest) = url.strip_prefix(&prefix) else {
        return false;
    };
    let Some(path) = rest.split_once('/').map(|(_, path)| path) else {
        return false;
    };
    path.starts_with("opencode/")
}

/// 读取 OpenCode provider 节点上的 `options.baseURL`。
fn opencode_provider_base_url(provider: &Value) -> Option<&str> {
    provider
        .get("options")
        .and_then(Value::as_object)
        .and_then(|options| options.get("baseURL"))
        .and_then(Value::as_str)
}

fn group_opencode_models_by_key(
    models: &[GrayscaleModelEntry],
) -> HashMap<String, Vec<GrayscaleModelEntry>> {
    let mut grouped: HashMap<String, Vec<GrayscaleModelEntry>> = HashMap::new();
    for model in models {
        let group_key = if !model.key_id.is_empty() {
            model.key_id.clone()
        } else if !model.key_name.is_empty() {
            model.key_name.clone()
        } else if !model.provider.is_empty() {
            model.provider.clone()
        } else {
            "openai".to_string()
        };
        grouped.entry(group_key).or_default().push(model.clone());
    }
    grouped
}

fn opencode_provider_key_from_models(models: &[GrayscaleModelEntry]) -> String {
    models
        .iter()
        .find_map(|model| {
            if model.key_name.is_empty() {
                None
            } else {
                Some(model.key_name.clone())
            }
        })
        .or_else(|| {
            models.first().and_then(|model| {
                if model.key_id.is_empty() {
                    None
                } else {
                    Some(model.key_id.clone())
                }
            })
        })
        .unwrap_or_else(|| {
            opencode_provider_id(
                models
                    .first()
                    .map(|model| model.provider.as_str())
                    .unwrap_or("openai"),
            )
        })
}

fn bifrost_provider_for_opencode_models(models: &[GrayscaleModelEntry]) -> String {
    models
        .first()
        .map(|model| model.provider.as_str())
        .filter(|provider| !provider.is_empty())
        .unwrap_or("openai")
        .to_string()
}

fn build_opencode_providers_config(
    local_proxy_base: &str,
    platform: &GrayscalePlatformModels,
) -> (Value, Vec<String>) {
    let mut providers = serde_json::Map::new();
    let mut managed_keys = Vec::new();

    for (_, models) in group_opencode_models_by_key(&platform.additional_models) {
        let provider_key = opencode_provider_key_from_models(&models);
        let bifrost_provider = bifrost_provider_for_opencode_models(&models);
        let base_path = resolve_opencode_provider_base_path(&bifrost_provider, &models, platform);
        let api = resolve_opencode_provider_api(&base_path, &models);
        let npm = opencode_npm_for_api(&api);
        let base_url = build_opencode_provider_base_url(local_proxy_base, &base_path);

        let mut model_entries = serde_json::Map::new();
        for model in &models {
            model_entries.insert(model.id.clone(), build_opencode_model_entry(model));
        }

        providers.insert(
            provider_key.clone(),
            serde_json::json!({
                "npm": npm,
                "name": provider_key,
                "options": {
                    "baseURL": base_url,
                    "apiKey": OPENCODE_PROXY_API_KEY_PLACEHOLDER,
                },
                "models": Value::Object(model_entries),
            }),
        );
        managed_keys.push(provider_key);
    }

    (Value::Object(providers), managed_keys)
}

fn opencode_managed_provider_keys_from_providers(
    providers: &serde_json::Map<String, Value>,
) -> Vec<String> {
    providers
        .iter()
        .filter_map(|(key, provider)| {
            opencode_provider_base_url(provider)
                .filter(|url| is_local_proxy_opencode_provider_base_url(url))
                .map(|_| key.clone())
        })
        .collect()
}

fn opencode_provider_display_name<'a>(provider: &'a Value, key: &'a str) -> &'a str {
    provider
        .get("name")
        .and_then(Value::as_str)
        .filter(|name| !name.is_empty())
        .unwrap_or(key)
}

fn opencode_provider_keys_matching_name(
    providers: &serde_json::Map<String, Value>,
    name: &str,
) -> Vec<String> {
    providers
        .iter()
        .filter_map(|(key, provider)| {
            if opencode_provider_display_name(provider, key) == name || key == name {
                Some(key.clone())
            } else {
                None
            }
        })
        .collect()
}

/// 将灰度 Provider 合并进 `opencode.json`：移除此前注入项，并按 `name`/key 替换同名 Provider。
fn merge_opencode_injected_providers(
    providers: &mut serde_json::Map<String, Value>,
    injected_providers: &Value,
) {
    let Some(injected) = injected_providers.as_object() else {
        return;
    };

    let mut keys_to_remove: HashSet<String> =
        opencode_managed_provider_keys_from_providers(providers)
            .into_iter()
            .collect();

    for (key, provider_value) in injected {
        let injected_name = opencode_provider_display_name(provider_value, key);
        for existing_key in opencode_provider_keys_matching_name(providers, injected_name) {
            keys_to_remove.insert(existing_key);
        }
    }

    for key in keys_to_remove {
        providers.remove(&key);
    }

    for (key, provider_value) in injected {
        providers.insert(key.clone(), provider_value.clone());
    }
}

fn resolve_opencode_default_model(platform: &GrayscalePlatformModels) -> Option<String> {
    for (_, models) in group_opencode_models_by_key(&platform.additional_models) {
        if !models
            .iter()
            .any(|model| model.id == GRAYSCALE_DEFAULT_MODEL_ID)
        {
            continue;
        }
        let provider_key = opencode_provider_key_from_models(&models);
        return Some(format!("{provider_key}/{GRAYSCALE_DEFAULT_MODEL_ID}"));
    }
    None
}

fn opencode_managed_provider_keys_from_value(value: &Value) -> Vec<String> {
    let Some(providers) = value.get("provider").and_then(Value::as_object) else {
        return Vec::new();
    };

    opencode_managed_provider_keys_from_providers(providers)
}

fn opencode_managed_provider_keys(content: &str) -> Option<Vec<String>> {
    let value: Value = serde_json::from_str(content).ok()?;
    Some(opencode_managed_provider_keys_from_value(&value))
}

fn backup_opencode_providers(entry: &mut HashMap<String, String>, tool_id: &str, content: &str) {
    if is_zwitch_injected_opencode_providers(content) {
        return;
    }

    entry
        .entry(backup_key(tool_id, "snapshot"))
        .or_insert_with(|| content.to_string());

    let managed_keys = opencode_managed_provider_keys(content).unwrap_or_default();
    entry
        .entry(backup_key(tool_id, OPENCODE_MANAGED_PROVIDERS_BACKUP_KEY))
        .or_insert_with(|| managed_keys.join(","));
}

fn is_zwitch_injected_opencode_providers(content: &str) -> bool {
    opencode_managed_provider_keys(content)
        .is_some_and(|keys| !keys.is_empty())
}

fn restore_opencode_providers(
    content: &str,
    tool_id: &str,
    file_backup: &HashMap<String, String>,
) -> Result<String, String> {
    if let Some(snapshot) = file_backup.get(&backup_key(tool_id, "snapshot")) {
        if snapshot != BACKUP_ABSENT {
            return Ok(snapshot.clone());
        }
    }

    let mut value: Value = if content.trim().is_empty() {
        serde_json::json!({})
    } else {
        serde_json::from_str(content).map_err(|e| format!("解析 opencode.json 失败: {e}"))?
    };

    let managed_keys: Vec<String> = file_backup
        .get(&backup_key(tool_id, OPENCODE_MANAGED_PROVIDERS_BACKUP_KEY))
        .map(|raw| {
            raw.split(',')
                .map(str::trim)
                .filter(|key| !key.is_empty())
                .map(str::to_string)
                .collect::<Vec<_>>()
        })
        .filter(|keys: &Vec<String>| !keys.is_empty())
        .unwrap_or_else(|| opencode_managed_provider_keys(content).unwrap_or_default());

    if let Some(providers) = value
        .as_object_mut()
        .and_then(|root| root.get_mut("provider"))
        .and_then(Value::as_object_mut)
    {
        for key in managed_keys {
            providers.remove(&key);
        }
        if providers.is_empty() {
            if let Some(root) = value.as_object_mut() {
                root.remove("provider");
            }
        }
    }

    if value.as_object().is_some_and(|root| root.is_empty()) {
        return Ok(String::new());
    }

    serde_json::to_string_pretty(&value).map_err(|e| e.to_string())
}

fn inject_opencode_grayscale_models(
    _tool: &ToolDefinition,
    _target: &ConfigTarget,
    config_path: &Path,
    local_proxy_base: &str,
    platform: &GrayscalePlatformModels,
) -> Result<(), String> {
    ensure_parent(config_path)?;

    let content = if config_path.exists() {
        fs::read_to_string(config_path).map_err(|e| format!("读取配置失败: {e}"))?
    } else {
        String::new()
    };

    let mut value: Value = if content.trim().is_empty() {
        serde_json::json!({})
    } else {
        serde_json::from_str(&content).map_err(|e| format!("解析 opencode.json 失败: {e}"))?
    };

    let (injected_providers, _managed_keys) =
        build_opencode_providers_config(local_proxy_base, platform);

    let root = value
        .as_object_mut()
        .ok_or_else(|| "opencode.json 根节点必须是对象".to_string())?;

    if !root.contains_key("provider") {
        root.insert("provider".to_string(), Value::Object(Default::default()));
    }

    let providers = root
        .get_mut("provider")
        .and_then(Value::as_object_mut)
        .ok_or_else(|| "opencode.json provider 必须是对象".to_string())?;
    merge_opencode_injected_providers(providers, &injected_providers);

    if let Some(model) = resolve_opencode_default_model(platform) {
        root.insert("model".to_string(), Value::String(model));
    }

    let new_content = serde_json::to_string_pretty(&value).map_err(|e| e.to_string())?;
    write_config(config_path, &new_content)
}

async fn resolve_grayscale_api_credential(
    app: &AppHandle,
    auth: &crate::store::StoredAuth,
) -> Result<String, String> {
    if let Some(token) = crate::auth::resolve_aone_session_token(auth) {
        return Ok(token);
    }

    eprintln!("灰度模型 API: 浏览器会话不可用，尝试使用设备临时凭证");
    let fingerprint = crate::device::get_or_create_fingerprint(app)?;
    let code = auth
        .authorization_code
        .clone()
        .ok_or_else(|| "未登录，无法拉取灰度模型".to_string())?;
    let base = crate::user_api::resolve_api_base_url(auth.api_base_url.as_deref());
    let resp = crate::user_api::exchange_device_code(&code, &fingerprint, &base)
        .await
        .map_err(|e| e.message())?;
    Ok(resp.credential().to_string())
}

async fn load_grayscale_platforms_by_tool(
    app: &AppHandle,
) -> HashMap<String, GrayscalePlatformModels> {
    let auth = match load_auth(app) {
        Ok(auth) => auth,
        Err(_) => return HashMap::new(),
    };
    let api_base = crate::user_api::resolve_api_base_url(auth.api_base_url.as_deref());

    let credential = match resolve_grayscale_api_credential(app, &auth).await {
        Ok(credential) => credential,
        Err(error) => {
            eprintln!("拉取灰度模型失败: {error}");
            return HashMap::new();
        }
    };

    let mut response = match fetch_grayscale_models(None, &credential, &api_base).await {
        Ok(response) => response,
        Err(crate::user_api::ApiError::Unauthorized) => {
            eprintln!("拉取灰度模型失败: 凭证无效或已过期，请重新登录");
            return HashMap::new();
        }
        Err(error) => {
            eprintln!("拉取灰度模型失败: {}", error.message());
            return HashMap::new();
        }
    };

    if let Ok(opencode_only) =
        fetch_grayscale_models(Some("opencode"), &credential, &api_base).await
    {
        response = merge_opencode_platform_response(response, opencode_only);
    }

    let mut by_platform = HashMap::new();
    for tool in TOOLS {
        if tool.id == "opencode" {
            if let Some(platform) = resolve_opencode_grayscale_platform(&response) {
                eprintln!(
                    "OpenCode: 找到 {} 个灰度模型待注入",
                    platform.additional_models.len()
                );
                by_platform.insert(tool.id.to_string(), platform);
            } else {
                eprintln!(
                    "OpenCode: 未找到可注入的灰度模型（检查 platform=opencode 或 codex 自定义 Provider）"
                );
            }
            continue;
        }

        let Some(platform) = find_platform_models(&response, tool.id) else {
            continue;
        };
        if !platform.additional_models.is_empty() {
            by_platform.insert(tool.id.to_string(), platform.clone());
        }
    }

    by_platform
}

fn is_zwitch_managed_custom_model_env_key(key: &str) -> bool {
    key == CLAUDE_CUSTOM_MODEL_ENV_PREFIX
        || key.starts_with(&format!("{CLAUDE_CUSTOM_MODEL_ENV_PREFIX}_"))
        || key == CLAUDE_GATEWAY_DISCOVERY_ENV
}

fn claude_custom_model_env_keys(index: usize) -> (String, String, String) {
    if index == 0 {
        (
            CLAUDE_CUSTOM_MODEL_ENV_PREFIX.to_string(),
            format!("{CLAUDE_CUSTOM_MODEL_ENV_PREFIX}_NAME"),
            format!("{CLAUDE_CUSTOM_MODEL_ENV_PREFIX}_DESCRIPTION"),
        )
    } else {
        let suffix = index + 1;
        (
            format!("{CLAUDE_CUSTOM_MODEL_ENV_PREFIX}_{suffix}"),
            format!("{CLAUDE_CUSTOM_MODEL_ENV_PREFIX}_NAME_{suffix}"),
            format!("{CLAUDE_CUSTOM_MODEL_ENV_PREFIX}_DESCRIPTION_{suffix}"),
        )
    }
}

fn inject_claude_grayscale_models(value: &mut Value, additional_models: &[GrayscaleModelEntry]) {
    let env = ensure_json_env_object(value);

    let managed_keys: Vec<String> = env
        .keys()
        .filter(|key| is_zwitch_managed_custom_model_env_key(key))
        .cloned()
        .collect();
    for key in managed_keys {
        env.remove(&key);
    }

    if !additional_models.is_empty() {
        env.insert(
            CLAUDE_GATEWAY_DISCOVERY_ENV.to_string(),
            Value::String("1".to_string()),
        );
    } else {
        env.remove(CLAUDE_GATEWAY_DISCOVERY_ENV);
    }

    for (index, model) in prioritize_grayscale_default_model(additional_models)
        .iter()
        .enumerate()
    {
        let (id_key, name_key, description_key) = claude_custom_model_env_keys(index);
        let description = if model.key_name.is_empty() {
            "灰度模型".to_string()
        } else {
            format!("{}（灰度）", model.key_name)
        };
        env.insert(id_key, Value::String(model.id.clone()));
        env.insert(
            name_key,
            Value::String(grayscale_model_display_name(&model.id)),
        );
        env.insert(description_key, Value::String(description));
    }
}

fn ensure_json_env_object(value: &mut Value) -> &mut serde_json::Map<String, Value> {
    let root = value.as_object_mut().expect("settings root must be object");
    if !root.contains_key("env") {
        root.insert("env".to_string(), Value::Object(Default::default()));
    }
    root.get_mut("env")
        .and_then(Value::as_object_mut)
        .expect("env must be object")
}

fn codex_grayscale_catalog_path(home: &Path) -> PathBuf {
    home.join(CODEX_GRAYSCALE_CATALOG_RELATIVE)
}

fn remove_managed_grayscale_artifacts(tool: &ToolDefinition, home: &Path) -> Result<(), String> {
    if tool.id == "codex" {
        let catalog_path = codex_grayscale_catalog_path(home);
        if catalog_path.exists() {
            fs::remove_file(&catalog_path)
                .map_err(|e| format!("删除灰度模型目录失败 {}: {e}", catalog_path.display()))?;
        }
    }
    Ok(())
}

fn is_zwitch_managed_model_catalog_path(path: &str) -> bool {
    path.contains(ZWITCH_MANAGED_MODEL_CATALOG_MARKER)
}

fn read_json_field(value: &Value, path: &str) -> Option<String> {
    let parts: Vec<&str> = path.split('.').collect();
    let mut current = value;
    for part in parts {
        current = current.get(part)?;
    }
    current.as_str().map(|s| s.to_string())
}

fn write_json_field(value: &mut Value, path: &str, new_value: &str) {
    let parts: Vec<&str> = path.split('.').collect();
    if parts.is_empty() {
        return;
    }

    let mut current = value;
    for (index, part) in parts.iter().enumerate() {
        if index == parts.len() - 1 {
            if let Value::Object(map) = current {
                map.insert(part.to_string(), Value::String(new_value.to_string()));
            }
            return;
        }

        if !current.get(*part).is_some() {
            if let Value::Object(map) = current {
                map.insert(part.to_string(), Value::Object(Default::default()));
            }
        }
        current = current.get_mut(*part).unwrap();
    }
}

fn remove_json_field(value: &mut Value, path: &str) {
    let parts: Vec<&str> = path.split('.').collect();
    if parts.is_empty() {
        return;
    }

    let mut current = value;
    for (index, part) in parts.iter().enumerate() {
        if index == parts.len() - 1 {
            if let Value::Object(map) = current {
                map.remove(*part);
            }
            return;
        }
        let Some(next) = current.get_mut(*part) else {
            return;
        };
        current = next;
    }
}

fn read_toml_assignment(content: &str, key: &str) -> Option<String> {
    let re = Regex::new(&format!(r#"(?m)^\s*{key}\s*=\s*(.+?)\s*(?:#.*)?$"#)).ok()?;
    let raw = re.captures(content)?.get(1)?.as_str().trim();
    if raw.starts_with('"') {
        let mut value = String::new();
        let mut chars = raw.chars().peekable();
        chars.next();
        while let Some(ch) = chars.next() {
            if ch == '\\' {
                if let Some(next) = chars.next() {
                    value.push(next);
                }
            } else if ch == '"' {
                break;
            } else {
                value.push(ch);
            }
        }
        Some(value)
    } else {
        Some(raw.to_string())
    }
}

fn read_toml_value(content: &str, key: &str) -> Option<String> {
    read_toml_assignment(content, key)
}

fn write_toml_value(content: &str, key: &str, value: &str) -> Result<String, String> {
    let escaped = escape_toml_string(value);
    let line = format!(r#"{key} = "{escaped}""#);
    let re = Regex::new(&format!(r#"(?m)^\s*{key}\s*=.*$"#)).map_err(|e| e.to_string())?;

    if re.is_match(content) {
        Ok(re.replace(content, line.as_str()).to_string())
    } else if content.trim().is_empty() {
        Ok(format!("{line}\n"))
    } else {
        Ok(format!("{content}\n{line}\n"))
    }
}

fn escape_toml_string(value: &str) -> String {
    value.replace('\\', "\\\\").replace('"', "\\\"")
}

fn toml_quote(value: &str) -> String {
    format!("\"{}\"", escape_toml_string(value))
}

fn legacy_codex_provider_table_header() -> &'static str {
    "[model_providers.zsdx_ai]"
}

fn codex_config_fields() -> &'static [&'static str] {
    CODEX_CONFIG_FIELDS
}

fn backup_codex_provider_toml(entry: &mut HashMap<String, String>, tool_id: &str, content: &str) {
    if is_zwitch_injected(content) {
        return;
    }

    entry
        .entry(backup_key(tool_id, "snapshot"))
        .or_insert_with(|| content.to_string());

    let (model, model_provider, order) = scan_top_level_model_fields(content);
    let model_backup = model.unwrap_or_else(|| BACKUP_ABSENT.to_string());
    let provider_backup = model_provider.unwrap_or_else(|| BACKUP_ABSENT.to_string());
    let order_backup = if order.is_empty() {
        BACKUP_ABSENT.to_string()
    } else {
        order.join(",")
    };
    let openai_base_url_backup = read_top_level_toml_value(content, CODEX_OPENAI_BASE_URL_KEY)
        .unwrap_or_else(|| BACKUP_ABSENT.to_string());
    let catalog_backup = read_top_level_toml_value(content, "model_catalog_json")
        .unwrap_or_else(|| BACKUP_ABSENT.to_string());
    let section_backup = extract_toml_section(content, legacy_codex_provider_table_header())
        .unwrap_or_else(|| BACKUP_ABSENT.to_string());

    for field in codex_config_fields() {
        if *field == "snapshot" {
            continue;
        }
        let backup_field_key = backup_key(tool_id, field);
        if entry.contains_key(&backup_field_key) {
            continue;
        }
        let stored = match *field {
            "openai_base_url" => openai_base_url_backup.clone(),
            "model" => model_backup.clone(),
            "model_provider" => provider_backup.clone(),
            "model_field_order" => order_backup.clone(),
            "model_catalog_json" => catalog_backup.clone(),
            "model_providers.zsdx_ai" => section_backup.clone(),
            _ => BACKUP_ABSENT.to_string(),
        };
        entry.insert(backup_field_key, stored);
    }
}

fn is_zwitch_injected(content: &str) -> bool {
    if read_top_level_toml_value(content, CODEX_OPENAI_BASE_URL_KEY)
        .is_some_and(|url| is_local_proxy_base_url(&url, "codex"))
    {
        return true;
    }

    extract_toml_section(content, legacy_codex_provider_table_header()).is_some()
        && read_top_level_toml_value(content, "model_provider").as_deref()
            == Some(LEGACY_CODEX_MODEL_PROVIDER)
}

fn scan_top_level_model_fields(content: &str) -> (Option<String>, Option<String>, Vec<String>) {
    let (top, _) = split_toml_top_level(content);
    let mut model = None;
    let mut model_provider = None;
    let mut order = Vec::new();

    for line in top {
        if read_toml_assignment(&line, "model").is_some() {
            model = read_toml_assignment(&line, "model");
            order.push("model".to_string());
        } else if read_toml_assignment(&line, "model_provider").is_some() {
            model_provider = read_toml_assignment(&line, "model_provider");
            order.push("model_provider".to_string());
        }
    }

    (model, model_provider, order)
}

fn parse_model_field_order(raw: &str) -> Vec<String> {
    raw.split(',')
        .map(str::trim)
        .filter(|part| *part == "model" || *part == "model_provider")
        .map(str::to_string)
        .collect()
}

fn format_top_level_assignment(key: &str, value: &str) -> String {
    format!("{key} = {}", toml_quote(value))
}

fn append_model_fields(
    out: &mut Vec<String>,
    model: Option<&str>,
    model_provider: Option<&str>,
    field_order: &[String],
) {
    for key in field_order {
        match key.as_str() {
            "model" => {
                if let Some(value) = model {
                    out.push(format_top_level_assignment("model", value));
                }
            }
            "model_provider" => {
                if let Some(value) = model_provider {
                    out.push(format_top_level_assignment("model_provider", value));
                }
            }
            _ => {}
        }
    }
}

fn rebuild_codex_config(
    content: &str,
    model: Option<&str>,
    model_provider: Option<&str>,
    field_order: &[String],
    provider_section_body: Option<&str>,
) -> Result<String, String> {
    let header = legacy_codex_provider_table_header();
    let mut without_managed = remove_toml_section(content, header);
    for key in ["model", "model_provider"] {
        without_managed = remove_top_level_toml_key(&without_managed, key);
    }

    let (top_lines, table_lines) = split_toml_top_level(&without_managed);
    let remaining_top: Vec<String> = top_lines
        .into_iter()
        .filter(|line| {
            let trimmed = line.trim();
            !trimmed.is_empty()
                && !trimmed.starts_with('#')
                && read_toml_assignment(line, "model").is_none()
                && read_toml_assignment(line, "model_provider").is_none()
        })
        .collect();

    let mut out = Vec::new();
    if !remaining_top.is_empty() {
        out.extend(remaining_top);
        out.push(String::new());
    }

    append_model_fields(&mut out, model, model_provider, field_order);

    if let Some(body) = provider_section_body {
        if out.last().map(|line| line.trim().is_empty()) != Some(true) {
            out.push(String::new());
        }
        out.push(header.to_string());
        out.extend(
            body.lines()
                .filter(|line| !line.trim().is_empty())
                .map(str::to_string),
        );
    }

    if !table_lines.is_empty() {
        if out.last().map(|line| line.trim().is_empty()) != Some(true) {
            out.push(String::new());
        }
        out.extend(table_lines);
    }

    Ok(normalize_toml_newlines(&out.join("\n")))
}

fn backup_claude_grayscale_env(
    entry: &mut HashMap<String, String>,
    tool_id: &str,
    value: &Value,
) {
    let Some(env) = value.get("env").and_then(Value::as_object) else {
        return;
    };
    for (key, env_value) in env {
        if !is_zwitch_managed_custom_model_env_key(key) {
            continue;
        }
        let backup_field_key = backup_key(tool_id, &format!("env.{key}"));
        if entry.contains_key(&backup_field_key) {
            continue;
        }
        let stored = env_value
            .as_str()
            .map(str::to_string)
            .unwrap_or_else(|| BACKUP_ABSENT.to_string());
        entry.insert(backup_field_key, stored);
    }
}

fn restore_claude_grayscale_env(
    content: &str,
    tool_id: &str,
    file_backup: &HashMap<String, String>,
) -> Result<String, String> {
    let mut value: Value = serde_json::from_str(content).map_err(|e| format!("解析 JSON 失败: {e}"))?;
    let Some(env) = value
        .as_object_mut()
        .and_then(|root| root.get_mut("env"))
        .and_then(Value::as_object_mut)
    else {
        return Ok(content.to_string());
    };

    let had_managed_keys = env
        .keys()
        .any(|key| is_zwitch_managed_custom_model_env_key(key));
    env.retain(|key, _| !is_zwitch_managed_custom_model_env_key(key));

    let mut restored_any = false;
    for (backup_key, original) in file_backup {
        let prefix = format!("{tool_id}::env.");
        let Some(env_key) = backup_key.strip_prefix(&prefix) else {
            continue;
        };
        if !is_zwitch_managed_custom_model_env_key(env_key) {
            continue;
        }
        if original == BACKUP_ABSENT {
            continue;
        }
        env.insert(env_key.to_string(), Value::String(original.clone()));
        restored_any = true;
    }

    if !had_managed_keys && !restored_any {
        return Ok(content.to_string());
    }

    serde_json::to_string_pretty(&value).map_err(|e| e.to_string())
}

fn restore_codex_provider_toml(
    content: &str,
    tool_id: &str,
    file_backup: &HashMap<String, String>,
) -> Result<String, String> {
    if let Some(snapshot) = file_backup.get(&backup_key(tool_id, "snapshot")) {
        return Ok(normalize_toml_newlines(snapshot));
    }

    let stripped = strip_injected_codex_config(content);
    let (model, model_provider, field_order, catalog_json, openai_base_url) =
        sanitized_codex_restore_fields(file_backup, tool_id);

    let mut restored = if model.is_none() && model_provider.is_none() {
        stripped
    } else {
        rebuild_codex_config(
            &stripped,
            model.as_deref(),
            model_provider.as_deref(),
            &field_order,
            None,
        )?
    };

    if let Some(url) = openai_base_url {
        restored = upsert_top_level_toml_key(&restored, CODEX_OPENAI_BASE_URL_KEY, &url)?;
    }

    if let Some(catalog_json) = catalog_json {
        restored = upsert_top_level_toml_key(&restored, "model_catalog_json", &catalog_json)?;
    } else {
        restored = remove_managed_codex_model_catalog(&restored);
    }

    Ok(restored)
}

fn sanitized_codex_restore_fields(
    file_backup: &HashMap<String, String>,
    tool_id: &str,
) -> (
    Option<String>,
    Option<String>,
    Vec<String>,
    Option<String>,
    Option<String>,
) {
    let model_raw = file_backup.get(&backup_key(tool_id, "model"));
    let provider_raw = file_backup.get(&backup_key(tool_id, "model_provider"));

    let provider_was_injected = provider_raw
        .map(String::as_str)
        .is_some_and(|value| value == LEGACY_CODEX_MODEL_PROVIDER);

    let model = model_raw.and_then(|value| {
        if value == BACKUP_ABSENT || provider_was_injected {
            return None;
        }
        Some(value.clone())
    });

    let model_provider = provider_raw.and_then(|value| {
        if value == BACKUP_ABSENT || value == LEGACY_CODEX_MODEL_PROVIDER {
            return None;
        }
        Some(value.clone())
    });

    let field_order = file_backup
        .get(&backup_key(tool_id, "model_field_order"))
        .filter(|value| *value != BACKUP_ABSENT)
        .map(|value| parse_model_field_order(value))
        .filter(|order| !order.is_empty())
        .unwrap_or_else(|| vec!["model".to_string(), "model_provider".to_string()]);

    let catalog_json = file_backup
        .get(&backup_key(tool_id, "model_catalog_json"))
        .and_then(|value| {
            if value == BACKUP_ABSENT {
                None
            } else {
                Some(value.clone())
            }
        });

    let openai_base_url = file_backup
        .get(&backup_key(tool_id, "openai_base_url"))
        .and_then(|value| {
            if value == BACKUP_ABSENT {
                None
            } else {
                Some(value.clone())
            }
        });

    (model, model_provider, field_order, catalog_json, openai_base_url)
}

fn strip_injected_codex_config(content: &str) -> String {
    let mut result = remove_toml_section(content, legacy_codex_provider_table_header());

    if read_top_level_toml_value(&result, CODEX_OPENAI_BASE_URL_KEY)
        .is_some_and(|url| is_local_proxy_base_url(&url, "codex"))
    {
        result = remove_top_level_toml_key(&result, CODEX_OPENAI_BASE_URL_KEY);
    }

    if read_top_level_toml_value(&result, "model_provider").as_deref()
        == Some(LEGACY_CODEX_MODEL_PROVIDER)
    {
        result = remove_top_level_toml_key(&result, "model_provider");
    }

    result = remove_managed_codex_model_catalog(&result);
    normalize_toml_newlines(&result)
}

fn inject_codex_openai_proxy_config(content: &str, base_url: &str) -> Result<String, String> {
    let cleaned = strip_injected_codex_config(content);
    upsert_top_level_toml_key(&cleaned, CODEX_OPENAI_BASE_URL_KEY, base_url)
}

fn remove_managed_codex_model_catalog(content: &str) -> String {
    let current = read_top_level_toml_value(content, "model_catalog_json");
    if current
        .as_deref()
        .is_some_and(is_zwitch_managed_model_catalog_path)
    {
        remove_top_level_toml_key(content, "model_catalog_json")
    } else {
        content.to_string()
    }
}

fn split_toml_top_level(content: &str) -> (Vec<String>, Vec<String>) {
    let mut top = Vec::new();
    let mut tables = Vec::new();
    let mut in_tables = false;
    for line in content.lines() {
        if !in_tables && is_toml_table_header(line) {
            in_tables = true;
        }
        if in_tables {
            tables.push(line.to_string());
        } else {
            top.push(line.to_string());
        }
    }
    (top, tables)
}

fn read_top_level_toml_value(content: &str, key: &str) -> Option<String> {
    let (top, _) = split_toml_top_level(content);
    for line in top {
        if read_toml_assignment(&line, key).is_some() {
            return read_toml_assignment(&line, key);
        }
    }
    None
}

fn remove_top_level_toml_key(content: &str, key: &str) -> String {
    let (top, tables) = split_toml_top_level(content);
    let filtered_top: Vec<String> = top
        .into_iter()
        .filter(|line| read_toml_assignment(line, key).is_none())
        .collect();
    join_toml_parts(&filtered_top, &tables)
}

fn upsert_top_level_toml_key(content: &str, key: &str, value: &str) -> Result<String, String> {
    let mut top: Vec<String> = split_toml_top_level(content).0;
    let tables = split_toml_top_level(content).1;
    top.retain(|line| read_toml_assignment(line, key).is_none());
    top.push(format!("{key} = {}", toml_quote(value)));
    Ok(join_toml_parts(&top, &tables))
}

fn insert_toml_section_after_top_level(
    content: &str,
    header: &str,
    section_body: &str,
) -> Result<String, String> {
    let (top, tables) = split_toml_top_level(content);
    let mut out = top;
    if !out.is_empty() && out.last().map(|line| line.trim().is_empty()) != Some(true) {
        out.push(String::new());
    }
    out.push(header.to_string());
    out.extend(
        section_body
            .lines()
            .filter(|line| !line.trim().is_empty())
            .map(str::to_string),
    );
    if !tables.is_empty() {
        if out.last().map(|line| line.trim().is_empty()) != Some(true) {
            out.push(String::new());
        }
        out.extend(tables);
    }
    Ok(out.join("\n"))
}

fn join_toml_parts(top: &[String], tables: &[String]) -> String {
    let mut out = top.to_vec();
    if !tables.is_empty() {
        if !out.is_empty() && out.last().map(|line| line.trim().is_empty()) != Some(true) {
            out.push(String::new());
        }
        out.extend_from_slice(tables);
    }
    normalize_toml_newlines(&out.join("\n"))
}

fn remove_toml_section(content: &str, header: &str) -> String {
    let lines: Vec<&str> = content.lines().collect();
    let Some(start) = lines.iter().position(|line| line.trim() == header) else {
        return content.to_string();
    };
    let mut end = lines.len();
    for (index, line) in lines.iter().enumerate().skip(start + 1) {
        if is_toml_table_header(line) {
            end = index;
            break;
        }
    }
    let mut out: Vec<&str> = lines[..start].to_vec();
    out.extend_from_slice(&lines[end..]);
    normalize_toml_newlines(&out.join("\n"))
}

fn extract_toml_section(content: &str, header: &str) -> Option<String> {
    let lines: Vec<&str> = content.lines().collect();
    let start = lines.iter().position(|line| line.trim() == header)?;
    let mut end = lines.len();
    for (index, line) in lines.iter().enumerate().skip(start + 1) {
        if is_toml_table_header(line) {
            end = index;
            break;
        }
    }
    let body = lines[start + 1..end]
        .iter()
        .copied()
        .collect::<Vec<_>>()
        .join("\n");
    if body.trim().is_empty() {
        None
    } else {
        Some(body)
    }
}

fn normalize_toml_newlines(content: &str) -> String {
    let mut out = Vec::new();
    let mut prev_blank = false;
    for line in content.lines() {
        let blank = line.trim().is_empty();
        if blank {
            if prev_blank {
                continue;
            }
            prev_blank = true;
        } else {
            prev_blank = false;
        }
        out.push(line);
    }
    while out.last().map(|line| line.trim().is_empty()) == Some(true) {
        out.pop();
    }
    if out.is_empty() {
        String::new()
    } else {
        format!("{}\n", out.join("\n"))
    }
}

fn is_toml_table_header(line: &str) -> bool {
    let trimmed = line.trim();
    if !trimmed.starts_with('[') {
        return false;
    }
    let Some(idx) = trimmed.rfind(']') else {
        return false;
    };
    let rest = trimmed[idx + 1..].trim();
    rest.is_empty() || rest.starts_with('#')
}

fn read_dotenv_value(content: &str, key: &str) -> Option<String> {
    let re = Regex::new(&format!(r#"(?m)^\s*{key}\s*=\s*(.*)$"#)).ok()?;
    re.captures(content)
        .and_then(|caps| caps.get(1))
        .map(|m| m.as_str().trim().trim_matches('"').to_string())
}

fn write_dotenv_value(content: &str, key: &str, value: &str) -> Result<String, String> {
    let line = format!("{key}={value}");
    let re = Regex::new(&format!(r#"(?m)^\s*{key}\s*=.*$"#)).map_err(|e| e.to_string())?;

    if re.is_match(content) {
        Ok(re.replace(content, line.as_str()).to_string())
    } else if content.trim().is_empty() {
        Ok(format!("{line}\n"))
    } else {
        Ok(format!("{content}\n{line}\n"))
    }
}

fn remove_dotenv_key(content: &str, key: &str) -> Result<String, String> {
    let re = Regex::new(&format!(r#"(?m)^\s*{key}\s*=.*\n?"#)).map_err(|e| e.to_string())?;
    Ok(re.replace(content, "").to_string())
}

fn ensure_parent(path: &Path) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("创建目录失败: {e}"))?;
    }
    Ok(())
}

fn write_config(path: &Path, content: &str) -> Result<(), String> {
    ensure_parent(path)?;
    fs::write(path, content).map_err(|e| format!("写入配置失败 {}: {e}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cli_tool_status_reports_install_metadata() {
        let tool = &TOOLS[0];
        let status = CliToolStatus {
            id: tool.id.to_string(),
            name: tool.name.to_string(),
            supported: true,
            installed: is_tool_installed(tool),
            install_shell: tool.install_shell.to_string(),
            quick_start_doc_url: tool.quick_start_doc_url.to_string(),
            config_enabled: true,
            config_path: "/tmp/config".to_string(),
            base_url_field: tool.base_url_field.to_string(),
            token_field: tool.token_field.to_string(),
        };
        assert_eq!(
            status.install_shell,
            "curl -fsSL https://chatgpt.com/codex/install.sh | sh"
        );
        assert_eq!(
            status.quick_start_doc_url,
            "https://developers.openai.com/codex/cli"
        );
    }

    #[test]
    fn tool_config_defaults_to_enabled() {
        let settings = StoredSettings::default();
        assert!(is_tool_config_enabled(&settings, "codex"));
        assert!(is_tool_config_enabled(&settings, "claude"));
        assert!(!tool_should_inject(&settings, &TOOLS[0]));
    }

    #[test]
    fn tool_config_switch_disables_injection() {
        let mut settings = StoredSettings {
            proxy_enabled: true,
            tool_switches: HashMap::from([(String::from("codex"), false)]),
        };
        assert!(!is_tool_config_enabled(&settings, "codex"));
        assert!(is_tool_config_enabled(&settings, "claude"));
        assert!(!tool_should_inject(&settings, &TOOLS[0]));
        assert!(tool_should_inject(&settings, &TOOLS[1]));
    }

    #[test]
    fn codex_uses_openai_base_url_field() {
        assert_eq!(TOOLS[0].base_url_field, "openai_base_url");
        assert_eq!(TOOLS[0].token_field, "OPENAI_API_KEY");
        assert!(matches!(
            CODEX_TARGETS[0].format,
            ConfigFormat::CodexProviderToml
        ));
    }

    #[test]
    fn codex_inject_writes_openai_base_url() {
        let injected =
            inject_codex_openai_proxy_config("", "http://127.0.0.1:51805/codex").unwrap();
        assert_eq!(
            injected,
            r#"openai_base_url = "http://127.0.0.1:51805/codex"
"#
        );
    }

    #[test]
    fn codex_inject_preserves_model_provider_and_other_tables() {
        let original = r#"model_provider = "bifrost"
model = "gpt-5.4"

[model_providers.bifrost]
base_url = "http://127.0.0.1:51805/codex"
"#;
        let injected =
            inject_codex_openai_proxy_config(original, "http://127.0.0.1:9999/codex").unwrap();
        assert!(injected.contains(r#"openai_base_url = "http://127.0.0.1:9999/codex""#));
        assert!(injected.contains(r#"model_provider = "bifrost""#));
        assert!(injected.contains(r#"model = "gpt-5.4""#));
        assert!(injected.contains("[model_providers.bifrost]"));
        assert!(!injected.contains("[model_providers.zsdx_ai]"));
    }

    #[test]
    fn codex_inject_migrates_legacy_zsdx_ai_provider() {
        let legacy = r#"model_provider = "zsdx_ai"

[model_providers.zsdx_ai]
name = "ZSDX AI"
base_url = "http://127.0.0.1:51805/codex"
wire_api = "responses"
"#;
        let injected =
            inject_codex_openai_proxy_config(legacy, "http://127.0.0.1:9999/codex").unwrap();
        assert!(injected.contains(r#"openai_base_url = "http://127.0.0.1:9999/codex""#));
        assert!(!injected.contains("model_provider"));
        assert!(!injected.contains("[model_providers.zsdx_ai]"));
    }

    #[test]
    fn codex_restore_removes_injected_config() {
        let original = r#"disable_response_storage = true

[model_providers.bifrost]
base_url = "http://old.example/codex"
"#;
        let mut backup = HashMap::new();
        backup_codex_provider_toml(&mut backup, "codex", original);
        let injected =
            inject_codex_openai_proxy_config(original, "http://127.0.0.1:51805/codex").unwrap();
        let restored = restore_codex_provider_toml(&injected, "codex", &backup).unwrap();
        assert_eq!(restored, original);
    }

    #[test]
    fn codex_restore_puts_back_replaced_top_level_keys() {
        let original = r#"model = "gpt-5.4"
model_provider = "bifrost"
"#;
        let mut backup = HashMap::new();
        backup_codex_provider_toml(&mut backup, "codex", original);
        let injected =
            inject_codex_openai_proxy_config(original, "http://127.0.0.1:51805/codex").unwrap();
        let restored = restore_codex_provider_toml(&injected, "codex", &backup).unwrap();
        assert_eq!(restored, original);
    }

    #[test]
    fn codex_restore_preserves_model_field_order() {
        let original = r#"model_provider = "bifrost"
model = "gpt-5.4"
"#;
        let mut backup = HashMap::new();
        backup_codex_provider_toml(&mut backup, "codex", original);
        let injected =
            inject_codex_openai_proxy_config(original, "http://127.0.0.1:51805/codex").unwrap();
        let restored = restore_codex_provider_toml(&injected, "codex", &backup).unwrap();
        assert_eq!(restored, original);
    }

    #[test]
    fn codex_restore_without_backup_strips_injected_values() {
        let injected = inject_codex_openai_proxy_config(
            r#"disable_response_storage = true
"#,
            "http://127.0.0.1:51805/codex",
        )
        .unwrap();
        let restored = restore_codex_provider_toml(&injected, "codex", &HashMap::new()).unwrap();
        assert_eq!(restored, "disable_response_storage = true\n");
    }

    #[test]
    fn codex_restore_clears_stale_injected_backup() {
        let original = r#"disable_response_storage = true

[model_providers.bifrost]
base_url = "http://old.example/codex"
"#;
        let injected =
            inject_codex_openai_proxy_config(original, "http://127.0.0.1:51805/codex").unwrap();

        let mut stale_backup = HashMap::new();
        stale_backup.insert("codex::model_provider".into(), LEGACY_CODEX_MODEL_PROVIDER.into());

        let restored = restore_codex_provider_toml(&injected, "codex", &stale_backup).unwrap();
        assert_eq!(restored, original);
    }

    #[test]
    fn codex_backup_skips_already_injected_config() {
        let injected =
            inject_codex_openai_proxy_config("", "http://127.0.0.1:51805/codex").unwrap();
        let mut backup = HashMap::new();
        backup.insert("codex::model".into(), BACKUP_ABSENT.into());
        backup_codex_provider_toml(&mut backup, "codex", &injected);
        assert_eq!(
            backup.get("codex::model").map(String::as_str),
            Some(BACKUP_ABSENT)
        );
        assert!(!backup.contains_key("codex::snapshot"));
    }

    #[test]
    fn codex_restore_uses_snapshot_when_available() {
        let original = "disable_response_storage = true\n";
        let mut backup = HashMap::new();
        backup.insert("codex::snapshot".into(), original.into());
        let injected =
            inject_codex_openai_proxy_config(original, "http://127.0.0.1:51805/codex").unwrap();
        let restored = restore_codex_provider_toml(&injected, "codex", &backup).unwrap();
        assert_eq!(restored, original);
    }

    #[test]
    fn codex_restore_puts_back_openai_base_url() {
        let original = r#"openai_base_url = "https://api.openai.com/v1"
"#;
        let mut backup = HashMap::new();
        backup_codex_provider_toml(&mut backup, "codex", original);
        let injected =
            inject_codex_openai_proxy_config(original, "http://127.0.0.1:51805/codex").unwrap();
        let restored = restore_codex_provider_toml(&injected, "codex", &backup).unwrap();
        assert_eq!(restored, original);
    }

    #[test]
    fn detects_claude_local_proxy_injection() {
        let content = r#"{"env":{"ANTHROPIC_BASE_URL":"http://127.0.0.1:51805/claude"}}"#;
        assert!(is_target_injected(&TOOLS[1], &CLAUDE_TARGETS[0], content));
        assert_eq!(
            read_injected_proxy_port(&TOOLS[1], &CLAUDE_TARGETS[0], content),
            Some(51805)
        );
    }

    #[test]
    fn claude_only_uses_env_block_fields() {
        assert_eq!(CLAUDE_TARGETS[0].fields.len(), 1);
        assert_eq!(CLAUDE_TARGETS[0].fields[0].path, "env.ANTHROPIC_BASE_URL");
    }

    #[test]
    fn executable_search_dirs_include_common_locations() {
        let dirs = collect_executable_search_dirs();
        assert!(dirs.iter().any(|dir| dir.ends_with("homebrew/bin")));
        assert!(dirs.iter().any(|dir| dir.ends_with(".local/bin")));
    }

    #[test]
    fn append_nested_bin_dirs_collects_fnm_node_versions() {
        let base = std::env::temp_dir().join(format!("zwitch-fnm-test-{}", std::process::id()));
        let bin_dir = base.join("v22.0.0/installation/bin");
        fs::create_dir_all(&bin_dir).expect("create fnm test dir");
        let mut dirs = Vec::new();
        let mut push = |dir: PathBuf| dirs.push(dir);
        append_nested_bin_dirs(&base, &["installation", "bin"], &mut push);
        assert!(dirs.iter().any(|dir| dir == &bin_dir));
        let _ = fs::remove_dir_all(base);
    }

    #[test]
    fn find_binary_resolves_known_cli_tools() {
        if find_binary(&["codex"]).is_none() && find_binary(&["claude"]).is_none() {
            return;
        }
        assert!(
            find_binary(&["codex"]).is_some() || find_binary(&["claude"]).is_some()
        );
    }

    #[test]
    fn claude_prioritizes_default_grayscale_model() {
        let mut value = serde_json::json!({ "env": {} });
        inject_claude_grayscale_models(
            &mut value,
            &[
                GrayscaleModelEntry {
                    id: "claude-mythos-preview".into(),
                    provider: "anthropic".into(),
                    key_id: String::new(),
                    key_name: String::new(),
                    source: "grayscale".into(),
                    api: None,
                    base_path: None,
                    context_window: None,
                    max_tokens: None,
                },
                GrayscaleModelEntry {
                    id: "claude-mythos-preview-fast".into(),
                    provider: "anthropic".into(),
                    key_id: String::new(),
                    key_name: String::new(),
                    source: "grayscale".into(),
                    api: None,
                    base_path: None,
                    context_window: None,
                    max_tokens: None,
                },
            ],
        );
        let env = value.get("env").unwrap().as_object().unwrap();
        assert_eq!(
            env.get("ANTHROPIC_CUSTOM_MODEL_OPTION")
                .and_then(Value::as_str),
            Some("claude-mythos-preview-fast")
        );
    }

    #[test]
    fn opencode_sets_default_model_selector() {
        let platform = opencode_test_platform(vec![
            GrayscaleModelEntry {
                id: "claude-mythos-preview".into(),
                provider: "claude".into(),
                key_id: "gray-claude-key".into(),
                key_name: "claude".into(),
                source: "grayscale".into(),
                api: Some("openai-responses".into()),
                base_path: Some("/openai".into()),
                context_window: None,
                max_tokens: None,
            },
            GrayscaleModelEntry {
                id: "claude-mythos-preview-fast".into(),
                provider: "claude".into(),
                key_id: "gray-claude-key".into(),
                key_name: "claude".into(),
                source: "grayscale".into(),
                api: Some("openai-responses".into()),
                base_path: Some("/openai".into()),
                context_window: None,
                max_tokens: None,
            },
        ]);
        assert_eq!(
            resolve_opencode_default_model(&platform).as_deref(),
            Some("claude/claude-mythos-preview-fast")
        );
    }

    #[test]
    fn claude_injects_grayscale_models_into_env() {
        let mut value = serde_json::json!({ "env": { "ANTHROPIC_BASE_URL": "http://example" } });
        let models = vec![GrayscaleModelEntry {
            id: "claude-sonnet-4-5".into(),
            provider: "anthropic".into(),
            key_id: "gray-anthropic".into(),
            key_name: "灰度 Anthropic".into(),
            source: "grayscale".into(),
            api: None,
            base_path: None,
            context_window: None,
            max_tokens: None,
        }];
        inject_claude_grayscale_models(&mut value, &models);
        let env = value.get("env").unwrap().as_object().unwrap();
        assert_eq!(
            env.get("CLAUDE_CODE_ENABLE_GATEWAY_MODEL_DISCOVERY")
                .and_then(Value::as_str),
            Some("1")
        );
        assert_eq!(
            env.get("ANTHROPIC_CUSTOM_MODEL_OPTION").and_then(Value::as_str),
            Some("claude-sonnet-4-5")
        );
        assert_eq!(
            env.get("ANTHROPIC_CUSTOM_MODEL_OPTION_NAME")
                .and_then(Value::as_str),
            Some("Claude Sonnet 4 5")
        );
    }

    #[test]
    fn claude_skips_gateway_discovery_without_grayscale_models() {
        let mut value = serde_json::json!({ "env": { "ANTHROPIC_BASE_URL": "http://example" } });
        inject_claude_grayscale_models(&mut value, &[]);
        let env = value.get("env").unwrap().as_object().unwrap();
        assert!(!env.contains_key("CLAUDE_CODE_ENABLE_GATEWAY_MODEL_DISCOVERY"));
        assert!(!env.contains_key("ANTHROPIC_CUSTOM_MODEL_OPTION"));
    }

    fn opencode_test_platform(models: Vec<GrayscaleModelEntry>) -> GrayscalePlatformModels {
        GrayscalePlatformModels {
            id: "opencode".into(),
            label: String::new(),
            base_path: "/openai".into(),
            injection_mode: String::new(),
            models: vec![],
            additional_models: models,
        }
    }

    #[test]
    fn opencode_provider_key_uses_key_name_not_provider_id() {
        let models = vec![GrayscaleModelEntry {
            id: "claude-mythos-preview".into(),
            provider: "测试".into(),
            key_id: "gray-key-1".into(),
            key_name: "灰度 Mythos".into(),
            source: "grayscale".into(),
            api: None,
            base_path: None,
            context_window: None,
            max_tokens: None,
        }];
        assert_eq!(opencode_provider_key_from_models(&models), "灰度 Mythos");
    }

    #[test]
    fn opencode_builds_custom_provider_config_from_grayscale_models() {
        let platform = opencode_test_platform(vec![
            GrayscaleModelEntry {
                id: "claude-mythos-preview".into(),
                provider: "claude".into(),
                key_id: "gray-claude-key".into(),
                key_name: "灰度 Claude".into(),
                source: "grayscale".into(),
                api: Some("openai-responses".into()),
                base_path: Some("/openai".into()),
                context_window: None,
                max_tokens: None,
            },
            GrayscaleModelEntry {
                id: "claude-mythos-preview-fast".into(),
                provider: "claude".into(),
                key_id: "gray-claude-key".into(),
                key_name: "灰度 Claude".into(),
                source: "grayscale".into(),
                api: Some("openai-responses".into()),
                base_path: Some("/openai".into()),
                context_window: None,
                max_tokens: None,
            },
        ]);

        let (providers, managed_keys) =
            build_opencode_providers_config("http://127.0.0.1:51805/opencode", &platform);
        let providers = providers.as_object().unwrap();
        let claude = providers.get("灰度 Claude").unwrap().as_object().unwrap();

        assert_eq!(managed_keys, vec!["灰度 Claude"]);
        assert_eq!(
            claude.get("npm").and_then(Value::as_str),
            Some(OPENCODE_OPENAI_RESPONSES_NPM)
        );
        assert_eq!(
            claude.get("name").and_then(Value::as_str),
            Some("灰度 Claude")
        );
        let options = claude.get("options").unwrap().as_object().unwrap();
        assert_eq!(
            options.get("baseURL").and_then(Value::as_str),
            Some("http://127.0.0.1:51805/opencode/openai/v1")
        );
        assert_eq!(
            options.get("apiKey").and_then(Value::as_str),
            Some(OPENCODE_PROXY_API_KEY_PLACEHOLDER)
        );
        let models = claude.get("models").unwrap().as_object().unwrap();
        assert_eq!(models.len(), 2);
        assert!(models.contains_key("claude-mythos-preview"));
        assert!(models.contains_key("claude-mythos-preview-fast"));
    }

    #[test]
    fn opencode_uses_openai_compatible_npm_for_completions() {
        let platform = opencode_test_platform(vec![GrayscaleModelEntry {
            id: "glm-4.6".into(),
            provider: "zhipu".into(),
            key_id: "gray-zhipu".into(),
            key_name: "灰度 GLM".into(),
            source: "grayscale".into(),
            api: Some("openai-completions".into()),
            base_path: Some("/openai".into()),
            context_window: None,
            max_tokens: None,
        }]);
        let (providers, _) =
            build_opencode_providers_config("http://127.0.0.1:51805/opencode", &platform);
        let provider = providers
            .as_object()
            .unwrap()
            .get("灰度 GLM")
            .unwrap()
            .as_object()
            .unwrap();
        assert_eq!(
            provider.get("npm").and_then(Value::as_str),
            Some(OPENCODE_OPENAI_COMPATIBLE_NPM)
        );
    }

    #[test]
    fn opencode_model_entry_uses_api_context_limits_not_hardcoded_default() {
        let entry = build_opencode_model_entry(&GrayscaleModelEntry {
            id: "claude-mythos-preview".into(),
            provider: "claude".into(),
            key_id: "gray-claude-key".into(),
            key_name: "灰度 Claude".into(),
            source: "grayscale".into(),
            api: None,
            base_path: None,
            context_window: Some(200_000),
            max_tokens: Some(32_000),
        });
        let limit = entry.get("limit").unwrap().as_object().unwrap();
        assert_eq!(limit.get("context").and_then(Value::as_u64), Some(200_000));
        assert_eq!(limit.get("output").and_then(Value::as_u64), Some(32_000));
        assert_eq!(
            entry.get("name").and_then(Value::as_str),
            Some("Claude Mythos Preview")
        );
    }

    #[test]
    fn opencode_model_entry_defaults_context_when_api_missing() {
        let entry = build_opencode_model_entry(&GrayscaleModelEntry {
            id: "claude-mythos-preview".into(),
            provider: "claude".into(),
            key_id: String::new(),
            key_name: String::new(),
            source: "grayscale".into(),
            api: None,
            base_path: None,
            context_window: None,
            max_tokens: None,
        });
        let limit = entry.get("limit").unwrap().as_object().unwrap();
        assert_eq!(
            limit.get("context").and_then(Value::as_u64),
            Some(OPENCODE_DEFAULT_CONTEXT_WINDOW)
        );
        assert_eq!(
            limit.get("output").and_then(Value::as_u64),
            Some(OPENCODE_DEFAULT_MAX_OUTPUT)
        );
    }

    #[test]
    fn opencode_injection_replaces_existing_provider_with_same_name() {
        let mut providers = serde_json::Map::new();
        providers.insert(
            "custom-claude".into(),
            serde_json::json!({
                "name": "claude",
                "npm": "@ai-sdk/openai",
                "options": { "baseURL": "https://api.example.com/v1" }
            }),
        );

        let (injected, _) = build_opencode_providers_config(
            "http://127.0.0.1:51805/opencode",
            &opencode_test_platform(vec![GrayscaleModelEntry {
                id: "claude-mythos-preview".into(),
                provider: "claude".into(),
                key_id: "gray-claude-key".into(),
                key_name: "claude".into(),
                source: "grayscale".into(),
                api: Some("openai-responses".into()),
                base_path: Some("/openai".into()),
                context_window: None,
                max_tokens: None,
            }]),
        );

        merge_opencode_injected_providers(&mut providers, &injected);

        assert_eq!(providers.len(), 1);
        assert!(providers.contains_key("claude"));
        assert!(!providers.contains_key("custom-claude"));
        let provider = providers.get("claude").unwrap();
        assert_eq!(
            provider
                .get("options")
                .and_then(|options| options.get("baseURL"))
                .and_then(Value::as_str),
            Some("http://127.0.0.1:51805/opencode/openai/v1")
        );
    }

    #[test]
    fn opencode_restore_removes_injected_providers() {
        let original = r#"{"provider":{"local":{"npm":"@ai-sdk/openai-compatible"}}}"#;
        let (providers, _) = build_opencode_providers_config(
            "http://127.0.0.1:51805/opencode",
            &opencode_test_platform(vec![GrayscaleModelEntry {
                id: "claude-mythos-preview".into(),
                provider: "claude".into(),
                key_id: "gray-claude-key".into(),
                key_name: "灰度 Claude".into(),
                source: "grayscale".into(),
                api: Some("openai-responses".into()),
                base_path: Some("/openai".into()),
                context_window: None,
                max_tokens: None,
            }]),
        );
        let mut value: Value = serde_json::from_str(original).unwrap();
        merge_opencode_injected_providers(
            value
                .as_object_mut()
                .unwrap()
                .get_mut("provider")
                .and_then(Value::as_object_mut)
                .unwrap(),
            &providers,
        );
        let injected = serde_json::to_string_pretty(&value).unwrap();

        let mut backup = HashMap::new();
        backup_opencode_providers(&mut backup, "opencode", original);
        let restored = restore_opencode_providers(&injected, "opencode", &backup).unwrap();
        let restored_value: Value = serde_json::from_str(&restored).unwrap();
        let providers = restored_value.get("provider").unwrap().as_object().unwrap();
        assert!(providers.contains_key("local"));
        assert!(!providers.contains_key("灰度 Claude"));
    }

    #[test]
    fn detects_codex_local_proxy_injection() {
        let content = r#"openai_base_url = "http://127.0.0.1:51805/codex"
"#;
        assert!(is_target_injected(&TOOLS[0], &CODEX_TARGETS[0], content));
        assert_eq!(
            read_injected_proxy_port(&TOOLS[0], &CODEX_TARGETS[0], content),
            Some(51805)
        );
    }
}
