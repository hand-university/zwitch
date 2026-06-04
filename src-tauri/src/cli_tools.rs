use crate::store::{load_auth, load_backup, load_settings, save_backup};
use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::fs;
use std::path::Path;
use tauri::AppHandle;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CliToolStatus {
    pub id: String,
    pub name: String,
    pub installed: bool,
    pub binary: Option<String>,
    pub config_path: String,
    pub switch_enabled: bool,
    pub base_url_field: String,
    pub token_field: String,
    pub install_url: String,
}

enum ConfigFormat {
    Json,
    Toml,
    DotEnv,
    CodexProviderToml,
}

/// Codex 自定义 model provider，与官方 config.toml 结构一致。
const CODEX_MODEL_PROVIDER: &str = "zsdx_ai";
const CODEX_PROVIDER_NAME: &str = "ZSDX AI";
const CODEX_DEFAULT_MODEL: &str = "gpt-4";

const CODEX_CONFIG_FIELDS: &[&str] = &[
    "snapshot",
    "model",
    "model_provider",
    "model_field_order",
    "model_providers.zsdx_ai",
];

/// 备份中标记「注入前不存在」的占位值。
const BACKUP_ABSENT: &str = "\0zd_switch_absent\0";

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
    install_url: &'static str,
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

const TOOLS: &[ToolDefinition] = &[
    ToolDefinition {
        id: "codex",
        name: "Codex CLI",
        binaries: &["codex"],
        base_url_field: "model_providers.zsdx_ai.base_url",
        token_field: "OPENAI_API_KEY",
        install_url: "https://developers.openai.com/codex/cli",
        targets: CODEX_TARGETS,
    },
    ToolDefinition {
        id: "claude",
        name: "Claude Code",
        binaries: &["claude"],
        base_url_field: "env.ANTHROPIC_BASE_URL",
        token_field: "env.ANTHROPIC_AUTH_TOKEN",
        install_url: "https://docs.anthropic.com/en/docs/claude-code",
        targets: CLAUDE_TARGETS,
    },
    ToolDefinition {
        id: "gemini",
        name: "Gemini CLI",
        binaries: &["gemini"],
        base_url_field: "GOOGLE_GEMINI_BASE_URL",
        token_field: "GEMINI_API_KEY",
        install_url: "https://github.com/google-gemini/gemini-cli",
        targets: GEMINI_TARGETS,
    },
];

/// 暴露所有受支持工具的 id，供本地拦截服务建立上游路由映射。
pub fn tool_ids() -> Vec<&'static str> {
    TOOLS.iter().map(|tool| tool.id).collect()
}

pub fn get_cli_tools_status(app: &AppHandle) -> Result<Vec<CliToolStatus>, String> {
    let settings = load_settings(app)?;
    let home = dirs::home_dir().ok_or_else(|| "无法获取用户目录".to_string())?;

    Ok(TOOLS
        .iter()
        .map(|tool| {
            let binary = find_binary(tool.binaries);
            let config_path = home.join(tool.targets[0].relative);
            CliToolStatus {
                id: tool.id.to_string(),
                name: tool.name.to_string(),
                installed: binary.is_some(),
                binary,
                config_path: config_path.to_string_lossy().to_string(),
                switch_enabled: settings
                    .tool_switches
                    .get(tool.id)
                    .copied()
                    .unwrap_or(false),
                base_url_field: tool.base_url_field.to_string(),
                token_field: tool.token_field.to_string(),
                install_url: tool.install_url.to_string(),
            }
        })
        .collect())
}

pub fn apply_config_injection(app: &AppHandle) -> Result<(), String> {
    let settings = load_settings(app)?;
    let auth = load_auth(app)?;
    let mut backup = load_backup(app)?;

    let home = dirs::home_dir().ok_or_else(|| "无法获取用户目录".to_string())?;

    let needs_inject = TOOLS.iter().any(|tool| {
        settings.proxy_enabled
            && settings
                .tool_switches
                .get(tool.id)
                .copied()
                .unwrap_or(false)
            && find_binary(tool.binaries).is_some()
    });

    if needs_inject && auth.authorization_code.is_none() {
        return Err("请先登录并完成设备授权".to_string());
    }

    for tool in TOOLS {
        let tool_enabled = settings.proxy_enabled
            && settings
                .tool_switches
                .get(tool.id)
                .copied()
                .unwrap_or(false);

        let installed = find_binary(tool.binaries).is_some();
        let local_base = crate::proxy::local_proxy_base(tool.id);

        for target in tool.targets {
            let config_path = home.join(target.relative);

            if !tool_enabled {
                restore_config(app, &mut backup, tool, target, &config_path)?;
                continue;
            }

            if !installed {
                continue;
            }

            backup_current_values(&mut backup, tool, target, &config_path)?;
            inject_values(target, &config_path, &local_base)?;
        }
    }

    save_backup(app, &backup)?;
    Ok(())
}

fn find_binary(names: &[&str]) -> Option<String> {
    for name in names {
        if let Ok(path) = which::which(name) {
            return Some(path.to_string_lossy().to_string());
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

fn backup_current_values(
    backup: &mut crate::store::ConfigBackup,
    tool: &ToolDefinition,
    target: &ConfigTarget,
    config_path: &Path,
) -> Result<(), String> {
    if !config_path.exists() {
        return Ok(());
    }

    let content = fs::read_to_string(config_path)
        .map_err(|e| format!("读取配置失败 {}: {e}", config_path.display()))?;

    let file_key = config_path.to_string_lossy().to_string();
    let entry = backup.files.entry(file_key).or_default();

    match target.format {
        ConfigFormat::Json => {
            let value: Value = serde_json::from_str(&content)
                .map_err(|e| format!("解析 JSON 失败: {e}"))?;
            for field in target.fields {
                let backup_field_key = backup_key(tool.id, field.path);
                if entry.contains_key(&backup_field_key) {
                    continue;
                }
                let stored = read_json_field(&value, field.path)
                    .unwrap_or_else(|| BACKUP_ABSENT.to_string());
                entry.insert(backup_field_key, stored);
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
    }

    Ok(())
}

fn restore_config(
    app: &AppHandle,
    backup: &mut crate::store::ConfigBackup,
    tool: &ToolDefinition,
    target: &ConfigTarget,
    config_path: &Path,
) -> Result<(), String> {
    if !config_path.exists() {
        return Ok(());
    }

    let file_key = config_path.to_string_lossy().to_string();
    let file_backup = match backup.files.get(&file_key) {
        Some(entry) => entry.clone(),
        None if matches!(target.format, ConfigFormat::CodexProviderToml) => HashMap::new(),
        None => return Ok(()),
    };

    let mut content = fs::read_to_string(config_path)
        .map_err(|e| format!("读取配置失败: {e}"))?;

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
                ConfigFormat::CodexProviderToml => unreachable!(),
            };
        }
    }

    if matches!(target.format, ConfigFormat::CodexProviderToml) {
        content = restore_codex_provider_toml(&content, tool.id, &file_backup)?;
    }

    write_config(config_path, &content)?;
    save_backup(app, backup)?;
    Ok(())
}

fn inject_values(
    target: &ConfigTarget,
    config_path: &Path,
    base_url: &str,
) -> Result<(), String> {
    ensure_parent(config_path)?;

    let content = if config_path.exists() {
        fs::read_to_string(config_path)
            .map_err(|e| format!("读取配置失败: {e}"))?
    } else {
        match target.format {
            ConfigFormat::Json => "{}".to_string(),
            ConfigFormat::Toml | ConfigFormat::DotEnv | ConfigFormat::CodexProviderToml => {
                String::new()
            }
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

            serde_json::to_string_pretty(&value).map_err(|e| e.to_string())?
        }
        ConfigFormat::Toml => {
            let mut result = content;
            for field in target.fields {
                result = write_toml_value(
                    &result,
                    field.path,
                    &resolve_field_value(field, base_url),
                )?;
            }
            result
        }
        ConfigFormat::CodexProviderToml => inject_codex_provider_config(&content, base_url)?,
        ConfigFormat::DotEnv => {
            let mut result = content;
            for field in target.fields {
                result = write_dotenv_value(
                    &result,
                    field.path,
                    &resolve_field_value(field, base_url),
                )?;
            }
            result
        }
    };

    write_config(config_path, &new_content)
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

fn codex_provider_table_header() -> &'static str {
    "[model_providers.zsdx_ai]"
}

fn codex_config_fields() -> &'static [&'static str] {
    CODEX_CONFIG_FIELDS
}

fn backup_codex_provider_toml(
    entry: &mut HashMap<String, String>,
    tool_id: &str,
    content: &str,
) {
    if is_zd_switch_injected(content) {
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
    let section_backup = extract_toml_section(content, codex_provider_table_header())
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
            "model" => model_backup.clone(),
            "model_provider" => provider_backup.clone(),
            "model_field_order" => order_backup.clone(),
            "model_providers.zsdx_ai" => section_backup.clone(),
            _ => BACKUP_ABSENT.to_string(),
        };
        entry.insert(backup_field_key, stored);
    }
}

fn is_zd_switch_injected(content: &str) -> bool {
    extract_toml_section(content, codex_provider_table_header()).is_some()
        && read_top_level_toml_value(content, "model_provider").as_deref()
            == Some(CODEX_MODEL_PROVIDER)
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
    let header = codex_provider_table_header();
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

fn restore_codex_provider_toml(
    content: &str,
    tool_id: &str,
    file_backup: &HashMap<String, String>,
) -> Result<String, String> {
    if let Some(snapshot) = file_backup.get(&backup_key(tool_id, "snapshot")) {
        return Ok(normalize_toml_newlines(snapshot));
    }

    let stripped = strip_injected_codex_config(content);
    let (model, model_provider, field_order) = sanitized_codex_restore_fields(file_backup, tool_id);

    if model.is_none() && model_provider.is_none() {
        return Ok(stripped);
    }

    rebuild_codex_config(
        &stripped,
        model.as_deref(),
        model_provider.as_deref(),
        &field_order,
        None,
    )
}

fn sanitized_codex_restore_fields(
    file_backup: &HashMap<String, String>,
    tool_id: &str,
) -> (Option<String>, Option<String>, Vec<String>) {
    let model_raw = file_backup.get(&backup_key(tool_id, "model"));
    let provider_raw = file_backup.get(&backup_key(tool_id, "model_provider"));

    let provider_was_injected = provider_raw
        .map(String::as_str)
        .is_some_and(|value| value == CODEX_MODEL_PROVIDER);

    let model = model_raw.and_then(|value| {
        if value == BACKUP_ABSENT {
            return None;
        }
        if provider_was_injected && value == CODEX_DEFAULT_MODEL {
            return None;
        }
        Some(value.clone())
    });

    let model_provider = provider_raw.and_then(|value| {
        if value == BACKUP_ABSENT || value == CODEX_MODEL_PROVIDER {
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

    (model, model_provider, field_order)
}

fn strip_injected_codex_config(content: &str) -> String {
    let header = codex_provider_table_header();
    let mut result = remove_toml_section(content, header);
    for key in ["model", "model_provider"] {
        result = remove_top_level_toml_key(&result, key);
    }
    normalize_toml_newlines(&result)
}

fn inject_codex_provider_config(content: &str, base_url: &str) -> Result<String, String> {
    let (existing_model, _, existing_order) = scan_top_level_model_fields(content);
    let model = existing_model.as_deref();
    let section_body = format_codex_provider_section_body(base_url);

    let mut field_order: Vec<String> = if existing_order.is_empty() {
        vec!["model_provider".to_string()]
    } else {
        existing_order
            .into_iter()
            .filter(|key| key != "model" || model.is_some())
            .collect()
    };
    if !field_order.iter().any(|key| key == "model_provider") {
        field_order.push("model_provider".to_string());
    }

    rebuild_codex_config(
        content,
        model,
        Some(CODEX_MODEL_PROVIDER),
        &field_order,
        Some(&section_body),
    )
}

fn format_codex_provider_section_body(base_url: &str) -> String {
    format!(
        r#"name = "{CODEX_PROVIDER_NAME}"
base_url = {base_url}
wire_api = "responses""#,
        base_url = toml_quote(base_url),
    )
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
    fn codex_uses_custom_model_provider() {
        assert_eq!(TOOLS[0].base_url_field, "model_providers.zsdx_ai.base_url");
        assert_eq!(TOOLS[0].token_field, "OPENAI_API_KEY");
        assert!(matches!(
            CODEX_TARGETS[0].format,
            ConfigFormat::CodexProviderToml
        ));
    }

    #[test]
    fn codex_inject_writes_provider_section() {
        let injected =
            inject_codex_provider_config("", "http://127.0.0.1:51805/codex").unwrap();
        assert_eq!(
            injected,
            r#"model_provider = "zsdx_ai"

[model_providers.zsdx_ai]
name = "ZSDX AI"
base_url = "http://127.0.0.1:51805/codex"
wire_api = "responses"
"#
        );
    }

    #[test]
    fn codex_inject_replaces_existing_provider_and_preserves_other_tables() {
        let original = r#"model_provider = "bifrost"
model = "gpt-5.4"

[model_providers.bifrost]
base_url = "http://127.0.0.1:51805/codex"
"#;
        let injected =
            inject_codex_provider_config(original, "http://127.0.0.1:9999/codex").unwrap();
        assert!(injected.starts_with(
            r#"model_provider = "zsdx_ai"
model = "gpt-5.4"

[model_providers.zsdx_ai]
name = "ZSDX AI"
base_url = "http://127.0.0.1:9999/codex"
wire_api = "responses"
"#
        ));
        assert!(injected.contains("[model_providers.bifrost]"));
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
            inject_codex_provider_config(original, "http://127.0.0.1:51805/codex").unwrap();
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
            inject_codex_provider_config(original, "http://127.0.0.1:51805/codex").unwrap();
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
            inject_codex_provider_config(original, "http://127.0.0.1:51805/codex").unwrap();
        let restored = restore_codex_provider_toml(&injected, "codex", &backup).unwrap();
        assert_eq!(restored, original);
    }

    #[test]
    fn codex_restore_without_backup_strips_injected_values() {
        let injected = inject_codex_provider_config(
            r#"disable_response_storage = true
"#,
            "http://127.0.0.1:51805/codex",
        )
        .unwrap();
        let restored = restore_codex_provider_toml(&injected, "codex", &HashMap::new()).unwrap();
        assert_eq!(
            restored,
            "disable_response_storage = true\n"
        );
    }

    #[test]
    fn codex_restore_clears_stale_injected_backup() {
        let original = r#"disable_response_storage = true

[model_providers.bifrost]
base_url = "http://old.example/codex"
"#;
        let injected =
            inject_codex_provider_config(original, "http://127.0.0.1:51805/codex").unwrap();

        let mut stale_backup = HashMap::new();
        stale_backup.insert("codex::model".into(), CODEX_DEFAULT_MODEL.into());
        stale_backup.insert("codex::model_provider".into(), CODEX_MODEL_PROVIDER.into());

        let restored = restore_codex_provider_toml(&injected, "codex", &stale_backup).unwrap();
        assert_eq!(restored, original);
    }

    #[test]
    fn codex_backup_skips_already_injected_config() {
        let injected =
            inject_codex_provider_config("", "http://127.0.0.1:51805/codex").unwrap();
        let mut backup = HashMap::new();
        backup.insert("codex::model".into(), BACKUP_ABSENT.into());
        backup_codex_provider_toml(&mut backup, "codex", &injected);
        assert_eq!(backup.get("codex::model").map(String::as_str), Some(BACKUP_ABSENT));
        assert!(!backup.contains_key("codex::snapshot"));
    }

    #[test]
    fn codex_restore_uses_snapshot_when_available() {
        let original = "disable_response_storage = true\n";
        let mut backup = HashMap::new();
        backup.insert("codex::snapshot".into(), original.into());
        let injected =
            inject_codex_provider_config(original, "http://127.0.0.1:51805/codex").unwrap();
        let restored = restore_codex_provider_toml(&injected, "codex", &backup).unwrap();
        assert_eq!(restored, original);
    }

    #[test]
    fn gemini_uses_env_file_not_settings_json() {
        assert_eq!(GEMINI_TARGETS[0].relative, ".gemini/.env");
        assert_eq!(GEMINI_TARGETS[0].fields[0].path, "GOOGLE_GEMINI_BASE_URL");
        assert_eq!(GEMINI_TARGETS[0].fields.len(), 1);
    }

    #[test]
    fn claude_only_uses_env_block_fields() {
        assert_eq!(CLAUDE_TARGETS[0].fields.len(), 1);
        assert_eq!(CLAUDE_TARGETS[0].fields[0].path, "env.ANTHROPIC_BASE_URL");
    }
}
