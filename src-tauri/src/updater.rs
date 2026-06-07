use crate::config::UPDATE_ENDPOINT_PROD;
use crate::store;
use serde::Serialize;
use std::path::PathBuf;
use std::sync::Mutex;
use tauri::{AppHandle, Emitter, Manager, State};
use tauri_plugin_updater::UpdaterExt;
use url::Url;

const PENDING_UPDATE_FILE: &str = "pending-update.bin";
const PENDING_UPDATE_VERSION_KEY: &str = "pending_update_version";

pub struct UpdateState(Mutex<InnerUpdateState>);

impl UpdateState {
    pub fn new() -> Self {
        Self(Mutex::new(InnerUpdateState::default()))
    }
}

struct InnerUpdateState {
    pending: Option<tauri_plugin_updater::Update>,
    downloaded_bytes: Option<Vec<u8>>,
    downloading: bool,
}

impl Default for InnerUpdateState {
    fn default() -> Self {
        Self {
            pending: None,
            downloaded_bytes: None,
            downloading: false,
        }
    }
}

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

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateDownloadProgress {
    pub downloaded: u64,
    pub total: Option<u64>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DownloadedUpdateInfo {
    pub ready: bool,
    pub version: Option<String>,
    pub deferred: bool,
}

fn resolve_update_endpoint() -> Result<Url, String> {
    Url::parse(UPDATE_ENDPOINT_PROD).map_err(|e| format!("无效的更新地址: {e}"))
}

fn pending_update_path(app: &AppHandle) -> Result<PathBuf, String> {
    app.path()
        .app_data_dir()
        .map(|path| path.join(PENDING_UPDATE_FILE))
        .map_err(|e| e.to_string())
}

fn save_deferred_update(app: &AppHandle, bytes: &[u8], version: &str) -> Result<(), String> {
    let path = pending_update_path(app)?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    std::fs::write(&path, bytes).map_err(|e| e.to_string())?;

    let store = store::get_store(app)?;
    store.set(
        PENDING_UPDATE_VERSION_KEY,
        serde_json::Value::String(version.to_string()),
    );
    store.save().map_err(|e| e.to_string())
}

fn clear_deferred_update(app: &AppHandle) -> Result<(), String> {
    let path = pending_update_path(app)?;
    if path.exists() {
        std::fs::remove_file(&path).map_err(|e| e.to_string())?;
    }

    let store = store::get_store(app)?;
    store.delete(PENDING_UPDATE_VERSION_KEY);
    store.save().map_err(|e| e.to_string())
}

fn load_deferred_update_version(app: &AppHandle) -> Result<Option<String>, String> {
    let store = store::get_store(app)?;
    Ok(store
        .get(PENDING_UPDATE_VERSION_KEY)
        .and_then(|value| value.as_str().map(str::to_string)))
}

fn deferred_update_ready(app: &AppHandle) -> Result<Option<String>, String> {
    let path = pending_update_path(app)?;
    if !path.exists() {
        return Ok(None);
    }
    load_deferred_update_version(app)
}

async fn fetch_update(app: &AppHandle) -> Result<Option<tauri_plugin_updater::Update>, String> {
    let endpoint = resolve_update_endpoint()?;
    app.updater_builder()
        .endpoints(vec![endpoint])
        .map_err(|e| format!("更新端点配置无效: {e}"))?
        .build()
        .map_err(|e| format!("更新服务初始化失败: {e}"))?
        .check()
        .await
        .map_err(|e| format!("检查更新失败: {e}"))
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
    state: State<'_, UpdateState>,
) -> Result<UpdateCheckResult, String> {
    let current_version = app.package_info().version.to_string();
    let update = fetch_update(&app).await?;

    if let Some(update) = update {
        let result = UpdateCheckResult {
            available: true,
            current_version,
            version: Some(update.version.clone()),
            notes: update.body.clone(),
            date: update.date.map(|value| value.to_string()),
        };
        state.0.lock().map_err(|e| e.to_string())?.pending = Some(update);
        Ok(result)
    } else {
        state.0.lock().map_err(|e| e.to_string())?.pending = None;
        Ok(UpdateCheckResult {
            available: false,
            current_version,
            version: None,
            notes: None,
            date: None,
        })
    }
}

pub fn get_downloaded_update_info(
    app: AppHandle,
    state: State<'_, UpdateState>,
) -> Result<DownloadedUpdateInfo, String> {
    let inner = state.0.lock().map_err(|e| e.to_string())?;
    if inner.downloaded_bytes.is_some() {
        let version = inner
            .pending
            .as_ref()
            .map(|update| update.version.clone())
            .or_else(|| load_deferred_update_version(&app).ok().flatten());
        return Ok(DownloadedUpdateInfo {
            ready: true,
            version,
            deferred: false,
        });
    }

    if let Some(version) = deferred_update_ready(&app)? {
        return Ok(DownloadedUpdateInfo {
            ready: true,
            version: Some(version),
            deferred: true,
        });
    }

    Ok(DownloadedUpdateInfo {
        ready: false,
        version: None,
        deferred: false,
    })
}

pub async fn download_available_update(
    app: AppHandle,
    state: State<'_, UpdateState>,
) -> Result<(), String> {
    let update = {
        let mut inner = state.0.lock().map_err(|e| e.to_string())?;
        if inner.downloading {
            return Err("更新正在下载中".to_string());
        }
        if inner.downloaded_bytes.is_some() {
            return Ok(());
        }
        inner.downloading = true;
        inner.pending.clone().ok_or_else(|| {
            inner.downloading = false;
            "没有待下载的更新，请先检查更新".to_string()
        })?
    };

    let app_handle = app.clone();
    let mut downloaded: u64 = 0;
    let download_result = update
        .download(
            |chunk_len, total| {
                downloaded += chunk_len as u64;
                let _ = app_handle.emit(
                    "update-download-progress",
                    UpdateDownloadProgress { downloaded, total },
                );
            },
            || {},
        )
        .await;

    match download_result {
        Ok(bytes) => {
            let mut inner = state.0.lock().map_err(|e| e.to_string())?;
            inner.downloaded_bytes = Some(bytes);
            inner.downloading = false;
            Ok(())
        }
        Err(error) => {
            let mut inner = state.0.lock().map_err(|e| e.to_string())?;
            inner.downloading = false;
            Err(format!("下载更新失败: {error}"))
        }
    }
}

fn install_bytes(update: &tauri_plugin_updater::Update, bytes: Vec<u8>) -> Result<(), String> {
    update
        .install(bytes)
        .map_err(|e| format!("安装更新失败: {e}"))
}

async fn resolve_update_for_install(
    app: &AppHandle,
    state: &State<'_, UpdateState>,
    bytes: Vec<u8>,
) -> Result<tauri_plugin_updater::Update, String> {
    if let Some(update) = state.0.lock().map_err(|e| e.to_string())?.pending.clone() {
        return Ok(update);
    }

    let deferred_version = load_deferred_update_version(app)?;
    let update = fetch_update(app)
        .await?
        .ok_or_else(|| "没有可用的更新".to_string())?;

    if let Some(expected) = deferred_version {
        if update.version != expected {
            return Err(format!(
                "待安装版本 v{expected} 已过期，请重新检查更新"
            ));
        }
    }

    state.0.lock().map_err(|e| e.to_string())?.pending = Some(update.clone());
    let _ = bytes;
    Ok(update)
}

pub async fn install_downloaded_update(
    app: AppHandle,
    state: State<'_, UpdateState>,
) -> Result<(), String> {
    let bytes = {
        let mut inner = state.0.lock().map_err(|e| e.to_string())?;
        if let Some(bytes) = inner.downloaded_bytes.take() {
            bytes
        } else {
            drop(inner);
            let path = pending_update_path(&app)?;
            std::fs::read(&path).map_err(|e| format!("读取待安装更新失败: {e}"))?
        }
    };

    let update = resolve_update_for_install(&app, &state, bytes.clone()).await?;
    install_bytes(&update, bytes)?;
    let _ = clear_deferred_update(&app);
    app.request_restart();
    #[allow(unreachable_code)]
    Ok(())
}

pub async fn defer_downloaded_update(
    app: AppHandle,
    state: State<'_, UpdateState>,
) -> Result<(), String> {
    let (bytes, version) = {
        let mut inner = state.0.lock().map_err(|e| e.to_string())?;
        let bytes = inner
            .downloaded_bytes
            .take()
            .ok_or_else(|| "没有已下载的更新".to_string())?;
        let version = inner
            .pending
            .as_ref()
            .map(|update| update.version.clone())
            .ok_or_else(|| "没有待安装的更新信息".to_string())?;
        (bytes, version)
    };

    save_deferred_update(&app, &bytes, &version)
}

pub async fn try_install_deferred_update(app: &AppHandle) {
    let path = match pending_update_path(app) {
        Ok(path) => path,
        Err(_) => return,
    };
    if !path.exists() {
        return;
    }

    let bytes = match std::fs::read(&path) {
        Ok(bytes) => bytes,
        Err(error) => {
            eprintln!("读取待安装更新失败: {error}");
            let _ = clear_deferred_update(app);
            return;
        }
    };

    let deferred_version = match load_deferred_update_version(app) {
        Ok(version) => version,
        Err(error) => {
            eprintln!("读取待安装版本失败: {error}");
            return;
        }
    };

    let update = match fetch_update(app).await {
        Ok(Some(update)) => update,
        Ok(None) => {
            eprintln!("启动时跳过待安装更新：远端无可用更新");
            return;
        }
        Err(error) => {
            eprintln!("启动时检查待安装更新失败: {error}");
            return;
        }
    };

    if deferred_version.as_deref() != Some(update.version.as_str()) {
        eprintln!("启动时跳过待安装更新：版本不匹配");
        let _ = clear_deferred_update(app);
        return;
    }

    if let Err(error) = install_bytes(&update, bytes) {
        eprintln!("启动时安装待安装更新失败: {error}");
        return;
    }

    let _ = clear_deferred_update(app);
    app.request_restart();
}
