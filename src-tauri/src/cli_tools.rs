use crate::config::LOCAL_PROXY_HOST;
use crate::grayscale_api::{
    fetch_grayscale_models, find_platform_models, prioritize_grayscale_default_model,
    GrayscaleModelEntry, GrayscalePlatformModels,
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
    /// 当前用户是否有灰度模型权限（Claude Code 使用）。
    #[serde(default)]
    pub has_grayscale: bool,
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
const CODEX_MODEL_PROVIDER: &str = "zwitch";
const CODEX_OPENAI_BASE_URL_KEY: &str = "openai_base_url";
const CODEX_LEGACY_TOP_LEVEL_BASE_URL_KEY: &str = "base_url";
const CODEX_AUTH_RELATIVE: &str = ".codex/auth.json";
const CODEX_PROXY_API_KEY_PLACEHOLDER: &str = "PROXY_MANAGED";

const CODEX_CONFIG_FIELDS: &[&str] = &[
    "snapshot",
    "openai_base_url",
    "base_url",
    "wire_api",
    "model",
    "model_provider",
    "model_field_order",
    "model_catalog_json",
    "model_providers.zwitch",
];

const CLAUDE_CUSTOM_MODEL_ENV_PREFIX: &str = "ANTHROPIC_CUSTOM_MODEL_OPTION";
const CLAUDE_GATEWAY_DISCOVERY_ENV: &str = "CLAUDE_CODE_ENABLE_GATEWAY_MODEL_DISCOVERY";
const CLAUDE_DISABLE_NONESSENTIAL_TRAFFIC_ENV: &str = "CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC";
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
];

/// 暂时禁用的工具：不参与检测/注入，但会尝试还原已注入的配置。
const DISABLED_TOOLS: &[ToolDefinition] = &[
    ToolDefinition {
        id: "gemini",
        name: "Gemini CLI",
        binaries: &["gemini"],
        base_url_field: "GOOGLE_GEMINI_BASE_URL",
        token_field: "GEMINI_API_KEY",
        install_shell: "npm install -g @google/gemini-cli",
        quick_start_doc_url: "https://github.com/google-gemini/gemini-cli",
        targets: GEMINI_TARGETS,
    },
    ToolDefinition {
        id: "opencode",
        name: "OpenCode",
        binaries: &["opencode"],
        base_url_field: "provider.*.options.baseURL",
        token_field: "provider.*.options.apiKey",
        install_shell: "",
        quick_start_doc_url: "",
        targets: OPENCODE_TARGETS,
    },
];

/// 暴露所有受支持工具的 id，供本地拦截服务建立上游路由映射。
pub fn tool_ids() -> Vec<&'static str> {
    TOOLS.iter().map(|tool| tool.id).collect()
}

pub fn is_tool_config_enabled(settings: &StoredSettings, tool_id: &str) -> bool {
    settings.tool_switches.get(tool_id).copied().unwrap_or(false)
}

fn tool_should_inject(settings: &StoredSettings, tool: &ToolDefinition) -> bool {
    settings.proxy_enabled && is_tool_config_enabled(settings, tool.id)
}

fn is_tool_installed(tool: &ToolDefinition) -> bool {
    if find_binary(tool.binaries).is_some() {
        return true;
    }
    tool_installed_at_known_paths(tool.id)
}

fn tool_installed_at_known_paths(tool_id: &str) -> bool {
    match tool_id {
        "claude" => claude_installed_at_known_paths(),
        "codex" => codex_installed_at_known_paths(),
        _ => false,
    }
}

fn path_is_existing_file(path: &Path) -> bool {
    path.is_file()
}

fn path_is_existing_dir(path: &Path) -> bool {
    path.is_dir()
}

fn any_existing_file(paths: &[PathBuf]) -> bool {
    paths.iter().any(|path| path_is_existing_file(path))
}

fn any_existing_dir(paths: &[PathBuf]) -> bool {
    paths.iter().any(|path| path_is_existing_dir(path))
}

fn cli_candidate_paths(home: &Path, binary: &str) -> Vec<PathBuf> {
    let mut paths = vec![
        home.join(".local/bin").join(binary),
        home.join("bin").join(binary),
    ];
    #[cfg(windows)]
    {
        paths.push(home.join(".local/bin").join(format!("{binary}.exe")));
        paths.push(home.join("bin").join(format!("{binary}.exe")));
    }
    paths
}

#[cfg(unix)]
fn unix_system_cli_paths(binary: &str) -> Vec<PathBuf> {
    vec![
        PathBuf::from(format!("/usr/bin/{binary}")),
        PathBuf::from(format!("/usr/local/bin/{binary}")),
    ]
}

#[cfg(not(unix))]
fn unix_system_cli_paths(_binary: &str) -> Vec<PathBuf> {
    Vec::new()
}

#[cfg(windows)]
fn windows_local_programs(relative: &str) -> Option<PathBuf> {
    std::env::var("LOCALAPPDATA")
        .ok()
        .map(|root| PathBuf::from(root).join("Programs").join(relative))
}

#[cfg(windows)]
fn windows_program_files(relative: &str) -> Option<PathBuf> {
    std::env::var("ProgramFiles")
        .ok()
        .map(|root| PathBuf::from(root).join(relative))
}

#[cfg(windows)]
fn windows_claude_msix_installed() -> bool {
    let Ok(local_app_data) = std::env::var("LOCALAPPDATA") else {
        return false;
    };
    let packages = PathBuf::from(local_app_data).join("Packages");
    let Ok(entries) = fs::read_dir(packages) else {
        return false;
    };
    entries.flatten().any(|entry| {
        entry
            .file_name()
            .to_string_lossy()
            .starts_with("Claude_")
            && entry.path().is_dir()
    })
}

#[cfg(not(windows))]
fn windows_local_programs(_relative: &str) -> Option<PathBuf> {
    None
}

#[cfg(not(windows))]
fn windows_program_files(_relative: &str) -> Option<PathBuf> {
    None
}

#[cfg(not(windows))]
fn windows_claude_msix_installed() -> bool {
    false
}

/// Electron/Tauri 桌面端常见数据目录名（跨平台）。
fn electron_app_data_installed(home: &Path, app_id: &str) -> bool {
    let mut candidates = vec![
        home.join(".config").join(app_id),
        home.join(".local/share").join(app_id),
    ];
    if let Some(data_dir) = dirs::data_dir() {
        candidates.push(data_dir.join(app_id));
    }
    if let Some(data_local_dir) = dirs::data_local_dir() {
        candidates.push(data_local_dir.join(app_id));
    }
    any_existing_dir(&candidates)
}

/// 在 `base/<version>/...suffix` 下查找可执行文件（桌面端常按版本号分目录发布）。
fn any_file_in_versioned_subdirs(base: &Path, suffix: &[&str]) -> bool {
    let Ok(entries) = fs::read_dir(base) else {
        return false;
    };
    for entry in entries.flatten() {
        if !entry.path().is_dir() {
            continue;
        }
        let mut path = entry.path();
        for part in suffix {
            path = path.join(part);
        }
        if path_is_existing_file(&path) {
            return true;
        }
    }
    false
}

/// Claude Desktop 内置 CLI 落在 Application Support；独立 CLI 也可能不在 GUI PATH 中。
fn claude_installed_at_known_paths() -> bool {
    let Some(home) = dirs::home_dir() else {
        return false;
    };

    if any_existing_file(&cli_candidate_paths(&home, "claude")) {
        return true;
    }

    if any_nonempty_file_in_dir(&home.join(".local/share/claude/versions")) {
        return true;
    }

    #[cfg(target_os = "macos")]
    {
        if path_is_existing_dir(Path::new("/Applications/Claude.app")) {
            return true;
        }
        let claude_code_base = home.join("Library/Application Support/Claude/claude-code");
        if any_file_in_versioned_subdirs(&claude_code_base, &["claude"]) {
            return true;
        }
        if any_file_in_versioned_subdirs(
            &claude_code_base,
            &["claude.app", "Contents", "MacOS", "claude"],
        ) {
            return true;
        }
    }

    #[cfg(windows)]
    {
        if windows_claude_msix_installed() {
            return true;
        }
        if let Some(data_dir) = dirs::data_dir() {
            if path_is_existing_dir(&data_dir.join("Claude")) {
                return true;
            }
            let claude_code_base = data_dir.join("Claude").join("claude-code");
            if any_file_in_versioned_subdirs(&claude_code_base, &["claude.exe"])
                || any_file_in_versioned_subdirs(&claude_code_base, &["claude"])
            {
                return true;
            }
        }
        for relative in ["Claude/Claude.exe", "Claude Code/Claude.exe"] {
            if let Some(path) = windows_local_programs(relative) {
                if path_is_existing_file(&path) {
                    return true;
                }
            }
            if let Some(path) = windows_program_files(relative) {
                if path_is_existing_file(&path) {
                    return true;
                }
            }
        }
    }

    #[cfg(target_os = "linux")]
    {
        if any_existing_file(&unix_system_cli_paths("claude")) {
            return true;
        }
    }

    false
}

fn any_nonempty_file_in_dir(base: &Path) -> bool {
    let Ok(entries) = fs::read_dir(base) else {
        return false;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        if entry.metadata().map(|meta| meta.len() > 0).unwrap_or(false) {
            return true;
        }
    }
    false
}

/// Codex 桌面端 bundle 内携带 CLI；仅装 App 时 PATH 里可能没有 `codex`。
fn codex_installed_at_known_paths() -> bool {
    let Some(home) = dirs::home_dir() else {
        return false;
    };

    if any_existing_file(&cli_candidate_paths(&home, "codex")) {
        return true;
    }

    #[cfg(target_os = "macos")]
    {
        if path_is_existing_dir(Path::new("/Applications/Codex.app")) {
            return true;
        }
        if path_is_existing_file(Path::new(
            "/Applications/Codex.app/Contents/Resources/codex",
        )) {
            return true;
        }
    }

    #[cfg(windows)]
    {
        if electron_app_data_installed(&home, "com.openai.codex") {
            return true;
        }
        for relative in ["Codex/Codex.exe", "OpenAI/Codex/Codex.exe"] {
            if let Some(path) = windows_local_programs(relative) {
                if path_is_existing_file(&path) {
                    return true;
                }
            }
            if let Some(path) = windows_program_files(relative) {
                if path_is_existing_file(&path) {
                    return true;
                }
            }
        }
    }

    #[cfg(target_os = "linux")]
    {
        if any_existing_file(&unix_system_cli_paths("codex")) {
            return true;
        }
    }

    false
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
    settings
        .tool_switches
        .insert(tool_id.to_string(), enabled);
    crate::store::save_settings(app, &settings)?;
    apply_config_injection_async(app).await?;
    Ok(())
}

pub async fn get_cli_tools_status_async(app: &AppHandle) -> Result<Vec<CliToolStatus>, String> {
    let home = dirs::home_dir().ok_or_else(|| "无法获取用户目录".to_string())?;
    let settings = load_settings(app)?;

    let show_claude_grayscale = if load_auth(app)
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
        grayscale_platforms.contains_key("claude")
    } else {
        false
    };

    Ok(TOOLS
        .iter()
        .map(|tool| {
            let config_path = home.join(tool.targets[0].relative);
            let is_claude = tool.id == "claude";
            CliToolStatus {
                id: tool.id.to_string(),
                name: tool.name.to_string(),
                supported: true,
                installed: is_tool_installed(tool),
                install_shell: tool.install_shell.to_string(),
                quick_start_doc_url: tool.quick_start_doc_url.to_string(),
                config_enabled: is_tool_config_enabled(&settings, tool.id),
                has_grayscale: is_claude && show_claude_grayscale,
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

            let local_base = if proxy_port != 0 {
                crate::proxy::local_proxy_base_for_port(tool.id, proxy_port)
            } else {
                crate::proxy::local_proxy_base(tool.id)
            };

            if !tool_enabled {
                if restore_config(app, &mut backup, tool, target, &config_path)? {
                    backup_dirty = true;
                }
                if tool.id == "codex" {
                    let auth_path = home.join(CODEX_AUTH_RELATIVE);
                    if restore_codex_auth(app, &mut backup, &auth_path)? {
                        backup_dirty = true;
                    }
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
            "go/bin",
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
    resolve_binary_from_login_shell(&format!("command -v -- {name}"))
}

#[cfg(unix)]
fn find_binary_via_which(name: &str) -> Option<String> {
    resolve_binary_from_login_shell(&format!("which -- {name}"))
}

#[cfg(unix)]
fn resolve_binary_from_login_shell(command: &str) -> Option<String> {
    let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".to_string());
    let output = std::process::Command::new(&shell)
        .args(["-l", "-c", command])
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
    if candidate.is_file() || (candidate.is_symlink() && candidate.exists()) {
        Some(path)
    } else {
        None
    }
}

#[cfg(not(unix))]
fn find_binary_via_which(_name: &str) -> Option<String> {
    None
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
        if let Some(path) = find_binary_via_which(name) {
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
                backup_claude_managed_env(entry, tool.id, &value);
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

/// Codex 桌面端有时会把 provider 字段展平到顶层 `base_url`（如 `http://127.0.0.1:PORT/v1`）。
fn is_zwitch_managed_codex_top_level_base_url(url: &str) -> bool {
    is_local_proxy_base_url(url, "codex") || parse_codex_legacy_local_proxy_port(url).is_some()
}

fn parse_codex_legacy_local_proxy_port(url: &str) -> Option<u16> {
    let prefix = format!("http://{LOCAL_PROXY_HOST}:");
    let rest = url.strip_prefix(&prefix)?;
    let (port_str, path) = rest.split_once('/')?;
    let port = port_str.parse().ok()?;
    if path.is_empty() || path == "v1" || path.starts_with("codex") {
        Some(port)
    } else {
        None
    }
}

fn read_codex_injected_proxy_port(content: &str) -> Option<u16> {
    read_top_level_toml_value(content, CODEX_OPENAI_BASE_URL_KEY)
        .and_then(|url| crate::proxy::parse_local_proxy_port(&url))
        .or_else(|| {
            read_top_level_toml_value(content, CODEX_LEGACY_TOP_LEVEL_BASE_URL_KEY)
                .filter(|url| is_local_proxy_base_url(url, "codex"))
                .and_then(|url| crate::proxy::parse_local_proxy_port(&url))
        })
        .or_else(|| {
            extract_toml_section(content, legacy_codex_provider_table_header())
                .and_then(|body| read_toml_assignment(&body, "base_url"))
                .and_then(|url| crate::proxy::parse_local_proxy_port(&url))
        })
}

fn codex_auth_injected(path: &Path) -> bool {
    read_config_if_exists(path)
        .and_then(|content| serde_json::from_str::<Value>(&content).ok())
        .and_then(|value| {
            value
                .get("OPENAI_API_KEY")
                .and_then(Value::as_str)
                .map(str::to_string)
        })
        .is_some_and(|key| key == CODEX_PROXY_API_KEY_PLACEHOLDER)
}

fn backup_codex_auth(
    backup: &mut crate::store::ConfigBackup,
    config_path: &Path,
) -> Result<(), String> {
    let file_key = config_path.to_string_lossy().to_string();
    let entry = backup.files.entry(file_key).or_default();
    let backup_field_key = backup_key("codex", "auth.json::OPENAI_API_KEY");
    if entry.contains_key(&backup_field_key) {
        return Ok(());
    }

    let stored = if config_path.exists() {
        let content = fs::read_to_string(config_path)
            .map_err(|e| format!("读取配置失败 {}: {e}", config_path.display()))?;
        serde_json::from_str::<Value>(&content)
            .ok()
            .and_then(|value| {
                value
                    .get("OPENAI_API_KEY")
                    .and_then(Value::as_str)
                    .map(str::to_string)
            })
            .unwrap_or_else(|| BACKUP_ABSENT.to_string())
    } else {
        BACKUP_ABSENT.to_string()
    };
    entry.insert(backup_field_key, stored);
    Ok(())
}

fn inject_codex_auth(config_path: &Path) -> Result<(), String> {
    ensure_parent(config_path)?;
    let value = serde_json::json!({
        "OPENAI_API_KEY": CODEX_PROXY_API_KEY_PLACEHOLDER,
    });
    write_config(
        config_path,
        &serde_json::to_string_pretty(&value).map_err(|e| e.to_string())?,
    )
}

fn restore_codex_auth(
    app: &AppHandle,
    backup: &mut crate::store::ConfigBackup,
    config_path: &Path,
) -> Result<bool, String> {
    let file_key = config_path.to_string_lossy().to_string();
    let Some(file_backup) = backup.files.get(&file_key) else {
        return Ok(false);
    };
    let Some(original) = file_backup.get(&backup_key("codex", "auth.json::OPENAI_API_KEY")) else {
        return Ok(false);
    };
    if !config_path.exists() {
        return Ok(false);
    }

    let mut value: Value = serde_json::from_str(
        &fs::read_to_string(config_path).map_err(|e| format!("读取配置失败: {e}"))?,
    )
    .map_err(|e| format!("解析 JSON 失败: {e}"))?;

    if original == BACKUP_ABSENT {
        if let Some(obj) = value.as_object_mut() {
            obj.remove("OPENAI_API_KEY");
        }
        if value.as_object().is_some_and(|obj| obj.is_empty()) {
            fs::remove_file(config_path).map_err(|e| format!("删除配置失败: {e}"))?;
            backup.files.remove(&file_key);
            save_backup(app, backup)?;
            return Ok(true);
        }
    } else {
        write_json_field(&mut value, "OPENAI_API_KEY", original);
    }

    write_config(
        config_path,
        &serde_json::to_string_pretty(&value).map_err(|e| e.to_string())?,
    )?;
    save_backup(app, backup)?;
    Ok(true)
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
        ConfigFormat::CodexProviderToml => read_codex_injected_proxy_port(content),
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
    tool: &ToolDefinition,
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

            if tool.id == "claude" {
                strip_injected_claude_env(&mut value);
            }

            for field in target.fields {
                write_json_field(
                    &mut value,
                    field.path,
                    &resolve_field_value(field, base_url),
                );
            }

            if tool.id == "claude" {
                inject_claude_grayscale_models(&mut value, additional_models);
            }

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

fn is_local_proxy_opencode_provider_base_url(url: &str) -> bool {
    let prefix = format!("http://{LOCAL_PROXY_HOST}:");
    let Some(rest) = url.strip_prefix(&prefix) else {
        return false;
    };
    let Some(path) = rest.split_once('/').map(|(_, path)| path) else {
        return false;
    };
    path.starts_with("opencode/") || path.starts_with("codex/")
}

fn opencode_provider_base_url(provider: &Value) -> Option<&str> {
    provider
        .get("options")
        .and_then(Value::as_object)
        .and_then(|options| options.get("baseURL"))
        .and_then(Value::as_str)
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

    let response = match fetch_grayscale_models(None, &credential, &api_base).await {
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

    let mut by_platform = HashMap::new();
    for tool in TOOLS {
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
        || key == CLAUDE_DISABLE_NONESSENTIAL_TRAFFIC_ENV
}

fn strip_injected_claude_env(value: &mut Value) {
    let Some(env) = value.get_mut("env").and_then(Value::as_object_mut) else {
        return;
    };
    let managed_keys: Vec<String> = env
        .keys()
        .filter(|key| is_zwitch_managed_custom_model_env_key(key))
        .cloned()
        .collect();
    for key in managed_keys {
        env.remove(&key);
    }
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

fn ensure_json_env_object(value: &mut Value) -> &mut serde_json::Map<String, Value> {
    let root = value.as_object_mut().expect("settings root must be object");
    if !root.contains_key("env") {
        root.insert("env".to_string(), Value::Object(Default::default()));
    }
    root.get_mut("env")
        .and_then(Value::as_object_mut)
        .expect("env must be object")
}

fn inject_claude_grayscale_models(value: &mut Value, additional_models: &[GrayscaleModelEntry]) {
    if additional_models.is_empty() {
        return;
    }

    let env = ensure_json_env_object(value);
    env.insert(
        CLAUDE_DISABLE_NONESSENTIAL_TRAFFIC_ENV.to_string(),
        Value::String("1".to_string()),
    );
    env.insert(
        CLAUDE_GATEWAY_DISCOVERY_ENV.to_string(),
        Value::String("1".to_string()),
    );

    for (index, model) in prioritize_grayscale_default_model(additional_models)
        .iter()
        .enumerate()
    {
        let (id_key, name_key, description_key) = claude_custom_model_env_keys(index);
        env.insert(id_key, Value::String(model.id.clone()));
        env.insert(name_key, Value::String(model.id.clone()));
        env.insert(description_key, Value::String(model.id.clone()));
    }
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

fn codex_provider_table_header() -> &'static str {
    "[model_providers.zwitch]"
}

fn is_managed_codex_model_provider(provider: &str) -> bool {
    provider == CODEX_MODEL_PROVIDER || provider == LEGACY_CODEX_MODEL_PROVIDER
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
    let base_url_backup = read_top_level_toml_value(content, CODEX_LEGACY_TOP_LEVEL_BASE_URL_KEY)
        .unwrap_or_else(|| BACKUP_ABSENT.to_string());
    let wire_api_backup = read_top_level_toml_value(content, "wire_api")
        .unwrap_or_else(|| BACKUP_ABSENT.to_string());
    let catalog_backup = read_top_level_toml_value(content, "model_catalog_json")
        .unwrap_or_else(|| BACKUP_ABSENT.to_string());
    let section_backup = extract_toml_section(content, codex_provider_table_header())
        .or_else(|| extract_toml_section(content, legacy_codex_provider_table_header()))
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
            "base_url" => base_url_backup.clone(),
            "wire_api" => wire_api_backup.clone(),
            "model" => model_backup.clone(),
            "model_provider" => provider_backup.clone(),
            "model_field_order" => order_backup.clone(),
            "model_catalog_json" => catalog_backup.clone(),
            "model_providers.zwitch" => section_backup.clone(),
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

    if read_top_level_toml_value(content, CODEX_LEGACY_TOP_LEVEL_BASE_URL_KEY)
        .is_some_and(|url| is_local_proxy_base_url(&url, "codex"))
    {
        return true;
    }

    if read_top_level_toml_value(content, "model_provider")
        .as_deref()
        .is_some_and(is_managed_codex_model_provider)
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
    let mut without_managed = remove_toml_section(content, codex_provider_table_header());
    without_managed = remove_toml_section(&without_managed, legacy_codex_provider_table_header());
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
        out.push(codex_provider_table_header().to_string());
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

fn backup_claude_managed_env(
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
    let (model, model_provider, field_order, catalog_json, openai_base_url, base_url, wire_api) =
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

    if let Some(url) = base_url {
        restored = upsert_top_level_toml_key(&restored, CODEX_LEGACY_TOP_LEVEL_BASE_URL_KEY, &url)?;
    }

    if let Some(value) = wire_api {
        restored = upsert_top_level_toml_key(&restored, "wire_api", &value)?;
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
    Option<String>,
    Option<String>,
) {
    let model_raw = file_backup.get(&backup_key(tool_id, "model"));
    let provider_raw = file_backup.get(&backup_key(tool_id, "model_provider"));

    let provider_was_injected = provider_raw
        .map(String::as_str)
        .is_some_and(|value| is_managed_codex_model_provider(value));

    let model = model_raw.and_then(|value| {
        if value == BACKUP_ABSENT || provider_was_injected {
            return None;
        }
        Some(value.clone())
    });

    let model_provider = provider_raw.and_then(|value| {
        if value == BACKUP_ABSENT || is_managed_codex_model_provider(value) {
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

    let base_url = file_backup
        .get(&backup_key(tool_id, "base_url"))
        .and_then(|value| {
            if value == BACKUP_ABSENT {
                None
            } else {
                Some(value.clone())
            }
        });

    let wire_api = file_backup
        .get(&backup_key(tool_id, "wire_api"))
        .and_then(|value| {
            if value == BACKUP_ABSENT {
                None
            } else {
                Some(value.clone())
            }
        });

    (
        model,
        model_provider,
        field_order,
        catalog_json,
        openai_base_url,
        base_url,
        wire_api,
    )
}

fn strip_injected_codex_config(content: &str) -> String {
    let mut result = remove_toml_section(content, codex_provider_table_header());
    result = remove_toml_section(&result, legacy_codex_provider_table_header());

    if read_top_level_toml_value(&result, CODEX_OPENAI_BASE_URL_KEY)
        .is_some_and(|url| is_local_proxy_base_url(&url, "codex"))
    {
        result = remove_top_level_toml_key(&result, CODEX_OPENAI_BASE_URL_KEY);
    }

    let had_managed_top_level_base_url = read_top_level_toml_value(&result, CODEX_LEGACY_TOP_LEVEL_BASE_URL_KEY)
        .is_some_and(|url| is_zwitch_managed_codex_top_level_base_url(&url));
    if had_managed_top_level_base_url {
        result = remove_top_level_toml_key(&result, CODEX_LEGACY_TOP_LEVEL_BASE_URL_KEY);
        result = remove_top_level_toml_key(&result, "wire_api");
    }

    if read_top_level_toml_value(&result, "model_provider")
        .as_deref()
        .is_some_and(is_managed_codex_model_provider)
    {
        result = remove_top_level_toml_key(&result, "model_provider");
        result = remove_top_level_toml_key(&result, "model");
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

    fn grayscale_model(id: &str, provider: &str, key_name: &str) -> GrayscaleModelEntry {
        GrayscaleModelEntry {
            id: id.into(),
            provider: provider.into(),
            key_id: String::new(),
            key_name: key_name.into(),
            source: "grayscale".into(),
            api: None,
            base_path: None,
            context_window: None,
            max_tokens: None,
        }
    }

    #[test]
    fn opencode_restore_removes_injected_providers() {
        let original = r#"{"provider":{"local":{"npm":"@ai-sdk/openai-compatible"}}}"#;
        let injected = r#"{"provider":{"local":{"npm":"@ai-sdk/openai-compatible"},"灰度":{"npm":"@ai-sdk/openai","options":{"baseURL":"http://127.0.0.1:51805/opencode/openai/v1"}}}}"#;

        let mut backup = HashMap::new();
        backup_opencode_providers(&mut backup, "opencode", original);
        let restored = restore_opencode_providers(injected, "opencode", &backup).unwrap();
        let restored_value: Value = serde_json::from_str(&restored).unwrap();
        let providers = restored_value.get("provider").unwrap().as_object().unwrap();
        assert!(providers.contains_key("local"));
        assert!(!providers.contains_key("灰度"));
    }

    #[test]
    fn claude_injects_grayscale_models_without_touching_auth_token() {
        let mut value = serde_json::json!({
            "env": {
                "ANTHROPIC_BASE_URL": "https://api.anthropic.com",
                "ANTHROPIC_AUTH_TOKEN": "user-secret-token"
            }
        });
        let models = vec![GrayscaleModelEntry {
            id: "claude-fable-5".into(),
            provider: "anthropic".into(),
            key_id: String::new(),
            key_name: String::new(),
            source: "grayscale".into(),
            api: None,
            base_path: None,
            context_window: None,
            max_tokens: None,
        }];
        inject_claude_grayscale_models(&mut value, &models);
        let env = value.get("env").unwrap().as_object().unwrap();
        assert_eq!(
            env.get("ANTHROPIC_AUTH_TOKEN").and_then(Value::as_str),
            Some("user-secret-token")
        );
        assert_eq!(
            env.get("ANTHROPIC_CUSTOM_MODEL_OPTION").and_then(Value::as_str),
            Some("claude-fable-5")
        );
        assert_eq!(
            env.get("ANTHROPIC_CUSTOM_MODEL_OPTION_NAME")
                .and_then(Value::as_str),
            Some("claude-fable-5")
        );
        assert_eq!(
            env.get("CLAUDE_CODE_ENABLE_GATEWAY_MODEL_DISCOVERY")
                .and_then(Value::as_str),
            Some("1")
        );
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

    #[test]
    fn codex_inject_only_updates_openai_base_url() {
        let injected =
            inject_codex_openai_proxy_config("", "http://127.0.0.1:51805/codex").unwrap();
        assert_eq!(
            injected.trim(),
            r#"openai_base_url = "http://127.0.0.1:51805/codex""#
        );
    }

    #[test]
    fn codex_inject_migrates_legacy_top_level_base_url() {
        let original = r#"base_url = "http://127.0.0.1:15721/v1"
wire_api = "responses"
model = "gpt-5.5"

[model_providers.ai]
name = "ai"
base_url = "https://ft-app.wxhand.com/cc"
wire_api = "responses"
"#;
        let mut backup = HashMap::new();
        backup_codex_provider_toml(&mut backup, "codex", original);
        let injected =
            inject_codex_openai_proxy_config(original, "http://127.0.0.1:59301/codex").unwrap();
        assert!(injected.contains(r#"openai_base_url = "http://127.0.0.1:59301/codex""#));
        assert!(!injected.contains(r#"base_url = "http://127.0.0.1:15721/v1""#));
        let (top, _) = split_toml_top_level(&injected);
        assert!(!top.iter().any(|line| line.trim().starts_with("wire_api =")));
        assert!(injected.contains(r#"model = "gpt-5.5""#));
        assert!(injected.contains("[model_providers.ai]"));
        assert!(injected.contains(r#"base_url = "https://ft-app.wxhand.com/cc""#));

        let restored = restore_codex_provider_toml(&injected, "codex", &backup).unwrap();
        assert_eq!(restored, original);
    }

    #[test]
    fn codex_legacy_top_level_proxy_is_not_treated_as_injected() {
        let content = r#"base_url = "http://127.0.0.1:15721/v1"
"#;
        assert!(!is_zwitch_injected(content));
        assert_eq!(read_codex_injected_proxy_port(content), None);
    }

    #[test]
    fn codex_auth_injection_writes_proxy_managed_key() {
        let dir = std::env::temp_dir().join(format!("zwitch-codex-auth-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let auth_path = dir.join("auth.json");

        inject_codex_auth(&auth_path).unwrap();
        assert!(codex_auth_injected(&auth_path));

        let mut backup = crate::store::ConfigBackup::default();
        fs::write(&auth_path, r#"{"OPENAI_API_KEY":"sk-user-key"}"#).unwrap();
        backup_codex_auth(&mut backup, &auth_path).unwrap();
        inject_codex_auth(&auth_path).unwrap();
        let stored = backup
            .files
            .get(&auth_path.to_string_lossy().to_string())
            .and_then(|entry| entry.get(&backup_key("codex", "auth.json::OPENAI_API_KEY")))
            .cloned()
            .unwrap();
        assert_eq!(stored, "sk-user-key");

        let _ = fs::remove_dir_all(&dir);
    }
}
