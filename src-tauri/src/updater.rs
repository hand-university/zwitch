use crate::config::UPDATE_ENDPOINT_PROD;
use serde::Serialize;
use std::sync::Mutex;
use tauri::{AppHandle, State};
use tauri_plugin_updater::UpdaterExt;
use url::Url;

pub struct PendingUpdate(pub Mutex<Option<tauri_plugin_updater::Update>>);

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppInfo {
    pub name: String,
    pub version: String,
    pub identifier: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateCheckResult {
    pub available: bool,
    pub current_version: String,
    pub version: Option<String>,
    pub notes: Option<String>,
    pub date: Option<String>,
}

fn resolve_update_endpoint() -> Result<Url, String> {
    Url::parse(UPDATE_ENDPOINT_PROD).map_err(|e| format!("无效的更新地址: {e}"))
}

pub fn get_app_info(app: AppHandle) -> Result<AppInfo, String> {
    let info = app.package_info();
    Ok(AppInfo {
        name: info.name.clone(),
        version: info.version.to_string(),
        identifier: app.config().identifier.clone(),
    })
}

pub async fn check_for_update(
    app: AppHandle,
    pending: State<'_, PendingUpdate>,
) -> Result<UpdateCheckResult, String> {
    let current_version = app.package_info().version.to_string();
    let endpoint = resolve_update_endpoint()?;

    let update = app
        .updater_builder()
        .endpoints(vec![endpoint])
        .map_err(|e| format!("更新端点配置无效: {e}"))?
        .build()
        .map_err(|e| format!("更新服务初始化失败: {e}"))?
        .check()
        .await
        .map_err(|e| format!("检查更新失败: {e}"))?;

    if let Some(update) = update {
        let result = UpdateCheckResult {
            available: true,
            current_version,
            version: Some(update.version.clone()),
            notes: update.body.clone(),
            date: update.date.map(|value| value.to_string()),
        };
        *pending.0.lock().map_err(|e| e.to_string())? = Some(update);
        Ok(result)
    } else {
        *pending.0.lock().map_err(|e| e.to_string())? = None;
        Ok(UpdateCheckResult {
            available: false,
            current_version,
            version: None,
            notes: None,
            date: None,
        })
    }
}

pub async fn install_available_update(
    app: AppHandle,
    pending: State<'_, PendingUpdate>,
) -> Result<(), String> {
    let update = pending
        .0
        .lock()
        .map_err(|e| e.to_string())?
        .take()
        .ok_or_else(|| "没有待安装的更新，请先检查更新".to_string())?;

    update
        .download_and_install(|_chunk, _total| {}, || {})
        .await
        .map_err(|e| format!("安装更新失败: {e}"))?;

    app.restart();
    #[allow(unreachable_code)]
    Ok(())
}
