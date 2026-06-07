mod auth;
mod cli_tools;
mod codex_plugins;
mod config;
mod grayscale_api;
mod device;
mod macos_scheme;
mod marketplace;
mod proxy;
mod store;
mod updater;
mod usage;
mod usage_api;
mod user_api;

use std::sync::Mutex;

/// 后台定时刷新用户资料与临时凭证的间隔（秒）。
const SESSION_REFRESH_INTERVAL_SECS: u64 = 600;

use auth::AuthState;
use cli_tools::CliToolStatus;
use marketplace::{ExploreItemView, MarketplaceItemView, MarketplaceSyncResult};
use updater::{AppInfo, PendingUpdate, UpdateCheckResult};

#[tauri::command]
fn get_auth_state(app: tauri::AppHandle) -> Result<AuthState, String> {
    auth::get_auth_state(&app)
}

#[tauri::command]
fn open_login_window(app: tauri::AppHandle) -> Result<(), String> {
    auth::open_login_window(&app)
}

#[tauri::command]
fn logout(app: tauri::AppHandle) -> Result<(), String> {
    auth::logout(&app)
}

#[tauri::command]
fn get_cli_tools_status(app: tauri::AppHandle) -> Result<Vec<CliToolStatus>, String> {
    cli_tools::get_cli_tools_status(&app)
}

#[tauri::command]
fn get_proxy_enabled(app: tauri::AppHandle) -> Result<bool, String> {
    Ok(store::load_settings(&app)?.proxy_enabled)
}

#[tauri::command]
async fn set_proxy_enabled(app: tauri::AppHandle, enabled: bool) -> Result<(), String> {
    let mut settings = store::load_settings(&app)?;
    settings.proxy_enabled = enabled;
    store::save_settings(&app, &settings)?;
    cli_tools::apply_config_injection_async(&app).await
}

#[tauri::command]
async fn refresh_user_profile(app: tauri::AppHandle) -> Result<AuthState, String> {
    auth::refresh_user_profile(&app).await
}

#[tauri::command]
async fn apply_config_injection(app: tauri::AppHandle) -> Result<(), String> {
    cli_tools::apply_config_injection_async(&app).await
}

#[tauri::command]
async fn get_explore_items(app: tauri::AppHandle) -> Result<Vec<ExploreItemView>, String> {
    marketplace::get_explore_items(&app).await
}

#[tauri::command]
async fn install_marketplace_item(
    app: tauri::AppHandle,
    platform: String,
    item_type: String,
    name: String,
) -> Result<(), String> {
    marketplace::install_marketplace_item(&app, platform, item_type, name).await
}

#[tauri::command]
fn get_marketplace_items(app: tauri::AppHandle) -> Result<Vec<MarketplaceItemView>, String> {
    marketplace::get_marketplace_items(&app)
}

#[tauri::command]
async fn sync_marketplace(app: tauri::AppHandle) -> Result<MarketplaceSyncResult, String> {
    marketplace::sync_marketplace(&app).await
}

#[tauri::command]
fn set_marketplace_item_enabled(
    app: tauri::AppHandle,
    platform: String,
    item_type: String,
    name: String,
    enabled: bool,
) -> Result<(), String> {
    marketplace::set_marketplace_item_enabled(&app, platform, item_type, name, enabled)
}

#[tauri::command]
fn delete_marketplace_item(
    app: tauri::AppHandle,
    platform: String,
    item_type: String,
    name: String,
) -> Result<(), String> {
    marketplace::delete_marketplace_item(&app, platform, item_type, name)
}

#[tauri::command]
fn scan_local_marketplace(app: tauri::AppHandle) -> Result<u32, String> {
    marketplace::scan_local_marketplace(&app)
}

#[tauri::command]
fn get_app_info(app: tauri::AppHandle) -> Result<AppInfo, String> {
    updater::get_app_info(app)
}

#[tauri::command]
async fn check_for_update(
    app: tauri::AppHandle,
    pending: tauri::State<'_, PendingUpdate>,
) -> Result<UpdateCheckResult, String> {
    updater::check_for_update(app, pending).await
}

#[tauri::command]
async fn install_available_update(
    app: tauri::AppHandle,
    pending: tauri::State<'_, PendingUpdate>,
) -> Result<(), String> {
    updater::install_available_update(app, pending).await
}

#[tauri::command]
fn get_usage_summary(app: tauri::AppHandle) -> Result<usage_api::UsageSummary, String> {
    usage_api::get_usage_summary(&app)
}

#[tauri::command]
fn clear_usage(app: tauri::AppHandle) -> Result<(), String> {
    usage_api::clear_usage(&app)
}

#[cfg(desktop)]
use tauri::{Manager, RunEvent, WindowEvent};

#[cfg(desktop)]
fn focus_main_window(app: &tauri::AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    macos_scheme::reexec_from_dev_app_if_needed();

    let mut builder = tauri::Builder::default()
        .plugin(tauri_plugin_process::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .manage(PendingUpdate(Mutex::new(None)));

    #[cfg(desktop)]
    {
        builder = builder.plugin(tauri_plugin_single_instance::init(|app, _argv, _cwd| {
            focus_main_window(app);
        }));
    }

    builder
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_store::Builder::new().build())
        .plugin(tauri_plugin_deep_link::init())
        .invoke_handler(tauri::generate_handler![
            get_auth_state,
            open_login_window,
            logout,
            refresh_user_profile,
            get_cli_tools_status,
            get_proxy_enabled,
            set_proxy_enabled,
            apply_config_injection,
            get_explore_items,
            install_marketplace_item,
            get_marketplace_items,
            sync_marketplace,
            set_marketplace_item_enabled,
            delete_marketplace_item,
            scan_local_marketplace,
            get_app_info,
            check_for_update,
            install_available_update,
            get_usage_summary,
            clear_usage,
        ])
        .setup(|app| {
            macos_scheme::ensure_url_scheme_registered()?;
            if let Ok(mut auth) = store::load_auth(app.handle()) {
                let effective =
                    user_api::resolve_api_base_url(auth.api_base_url.as_deref());
                if auth.api_base_url.as_deref() != Some(effective.as_str()) {
                    auth.api_base_url = Some(effective);
                    let _ = store::save_auth(app.handle(), &auth);
                }
            }
            // 总开关每次启动默认为关闭，并还原 CLI 配置。
            {
                let mut settings = store::load_settings(app.handle())?;
                settings.proxy_enabled = false;
                store::save_settings(app.handle(), &settings)?;
                let _ = cli_tools::apply_config_injection(app.handle());
            }
            auth::setup_deep_link(app.handle())?;

            // 启动本地拦截服务：CLI 请求先到本地，注入设备指纹后转发到上游。
            // 端口在启动时动态分配，随后的配置注入会写入实际端口。
            if let Ok(fingerprint) = device::get_or_create_fingerprint(app.handle()) {
                proxy::start(app.handle().clone(), fingerprint);
            }

            let handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                // 只要拥有授权码或既有登录态即可恢复会话，access_token 过期会自动续期。
                if let Ok(auth) = store::load_auth(&handle) {
                    if auth.authorization_code.is_some() {
                        let _ = auth::refresh_user_profile(&handle).await;
                    }
                }
            });

            // 后台定时刷新用户资料；临时凭证由本地拦截服务按需换取并缓存。
            let timer_handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                let interval = std::time::Duration::from_secs(SESSION_REFRESH_INTERVAL_SECS);
                loop {
                    tokio::time::sleep(interval).await;
                    if let Ok(auth) = store::load_auth(&timer_handle) {
                        if auth.authorization_code.is_some() {
                            let _ = auth::refresh_user_profile(&timer_handle).await;
                        }
                    }
                }
            });

            #[cfg(any(windows, target_os = "linux"))]
            {
                use tauri_plugin_deep_link::DeepLinkExt;
                app.deep_link().register_all()?;
            }

            Ok(())
        })
        .on_window_event(|window, event| {
            if let WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                let _ = window.hide();
            }
        })
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|app_handle, event| {
            #[cfg(target_os = "macos")]
            if let RunEvent::Reopen {
                has_visible_windows,
                ..
            } = event
            {
                if !has_visible_windows {
                    focus_main_window(app_handle);
                }
            }
        });
}
