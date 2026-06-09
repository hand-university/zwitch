use tauri::{
    menu::{CheckMenuItem, Menu, MenuItem, PredefinedMenuItem},
    tray::TrayIconBuilder,
    AppHandle, Emitter, Manager, Runtime,
};

const TRAY_ID: &str = "main-tray";
const MENU_SHOW: &str = "show_main";
const MENU_PROXY: &str = "toggle_proxy";
const MENU_QUIT: &str = "quit_app";

pub struct TrayState<R: Runtime> {
    proxy_item: CheckMenuItem<R>,
}

impl<R: Runtime> TrayState<R> {
    pub fn set_proxy_checked(&self, checked: bool) -> Result<(), String> {
        self.proxy_item
            .set_checked(checked)
            .map_err(|e| e.to_string())
    }
}

fn show_main_window(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
    }
}

pub async fn set_proxy_enabled_internal(app: &AppHandle, enabled: bool) -> Result<(), String> {
    let mut settings = crate::store::load_settings(app)?;
    settings.proxy_enabled = enabled;
    crate::store::save_settings(app, &settings)?;
    crate::cli_tools::apply_config_injection_async(app).await?;

    if let Some(state) = app.try_state::<TrayState<tauri::Wry>>() {
        let _ = state.set_proxy_checked(enabled);
    }

    let _ = app.emit("proxy-changed", enabled);

    Ok(())
}

pub fn setup(app: &AppHandle) -> Result<(), String> {
    let proxy_enabled = crate::store::load_settings(app)?.proxy_enabled;

    let show_main = MenuItem::with_id(app, MENU_SHOW, "显示主界面", true, None::<&str>)
        .map_err(|e| e.to_string())?;
    let proxy_item = CheckMenuItem::with_id(
        app,
        MENU_PROXY,
        "开启代理",
        true,
        proxy_enabled,
        None::<&str>,
    )
    .map_err(|e| e.to_string())?;

    let separator = PredefinedMenuItem::separator(app).map_err(|e| e.to_string())?;
    let quit_item = MenuItem::with_id(app, MENU_QUIT, "退出", true, None::<&str>)
        .map_err(|e| e.to_string())?;

    let menu = Menu::with_items(app, &[&show_main, &proxy_item, &separator, &quit_item])
        .map_err(|e| e.to_string())?;

    let proxy_for_state = proxy_item.clone();

    let _tray = TrayIconBuilder::with_id(TRAY_ID)
        .icon(
            app.default_window_icon()
                .ok_or("无法加载托盘图标")?
                .clone(),
        )
        .tooltip("ZWitch")
        .menu(&menu)
        .show_menu_on_left_click(true)
        .on_menu_event(|app, event| match event.id.as_ref() {
            MENU_SHOW => show_main_window(app),
            MENU_PROXY => {
                let app = app.clone();
                let desired = app
                    .try_state::<TrayState<tauri::Wry>>()
                    .and_then(|state| state.proxy_item.is_checked().ok())
                    .unwrap_or(false);

                tauri::async_runtime::spawn(async move {
                    if let Err(e) = set_proxy_enabled_internal(&app, desired).await {
                        eprintln!("托盘切换代理失败: {e}");
                        if let Some(state) = app.try_state::<TrayState<tauri::Wry>>() {
                            let _ = state.set_proxy_checked(!desired);
                        }
                    }
                });
            }
            MENU_QUIT => {
                app.exit(0);
            }
            _ => {}
        })
        .build(app)
        .map_err(|e| e.to_string())?;

    app.manage(TrayState {
        proxy_item: proxy_for_state,
    });

    Ok(())
}
