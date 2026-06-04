use crate::config::DEEPLINK_SCHEME;
use std::path::{Path, PathBuf};
use std::process::Command;

const LSREGISTER: &str = "/System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework/Support/lsregister";
const DEV_APP_NAME: &str = "ZD Switch Dev.app";
const DEV_BUNDLE_ID: &str = "com.zhangqiang.zd-switch";

pub fn ensure_url_scheme_registered() -> Result<(), String> {
    #[cfg(not(target_os = "macos"))]
    {
        let _ = DEEPLINK_SCHEME;
        Ok(())
    }

    #[cfg(target_os = "macos")]
    {
        if cfg!(debug_assertions) {
            unregister_legacy_handlers();
            let exe = std::env::current_exe().map_err(|e| format!("无法获取当前可执行文件: {e}"))?;
            ensure_dev_app_bundle(&exe)?;
        }
        Ok(())
    }
}

/// Dev 模式下，若当前不是从 Dev.app 启动，则 re-exec 到 bundle 内，
/// 让 macOS 把 deeplink 交给正在运行的 dev 实例。
pub fn reexec_from_dev_app_if_needed() {
    #[cfg(all(target_os = "macos", debug_assertions))]
    {
        use std::os::unix::process::CommandExt;

        let exe = match std::env::current_exe() {
            Ok(path) => path,
            Err(_) => return,
        };

        if exe.to_string_lossy().contains(".app/Contents/MacOS") {
            return;
        }

        unregister_legacy_handlers();
        if ensure_dev_app_bundle(&exe).is_err() {
            return;
        }

        let Some(target_dir) = target_debug_dir(&exe) else {
            return;
        };

        let app_exe = target_dir
            .join(DEV_APP_NAME)
            .join("Contents/MacOS/zd-switch");

        if !app_exe.exists() {
            return;
        }

        let err = std::process::Command::new(&app_exe)
            .args(std::env::args().skip(1))
            .exec();

        eprintln!("无法从 Dev.app 启动: {err}");
    }
}

#[cfg(target_os = "macos")]
pub fn ensure_dev_app_bundle(exe: &Path) -> Result<(), String> {
    use std::fs;
    use std::os::unix::fs::symlink;

    let target_dir = target_debug_dir(exe)
        .ok_or_else(|| "无法定位 target/debug 目录".to_string())?;
    let binary = target_dir.join("zd-switch");

    if !binary.exists() {
        return Err(format!("Dev 二进制不存在: {}", binary.display()));
    }

    let app_dir = target_dir.join(DEV_APP_NAME);
    let contents = app_dir.join("Contents");
    let macos_dir = contents.join("MacOS");
    let exe_link = macos_dir.join("zd-switch");
    let info_plist = contents.join("Info.plist");

    fs::create_dir_all(&macos_dir).map_err(|e| format!("创建 Dev.app 目录失败: {e}"))?;

    let plist = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleExecutable</key>
  <string>zd-switch</string>
  <key>CFBundleIdentifier</key>
  <string>{DEV_BUNDLE_ID}</string>
  <key>CFBundleName</key>
  <string>ZD Switch Dev</string>
  <key>CFBundlePackageType</key>
  <string>APPL</string>
  <key>CFBundleVersion</key>
  <string>0.1.0</string>
  <key>CFBundleShortVersionString</key>
  <string>0.1.0</string>
  <key>CFBundleURLTypes</key>
  <array>
    <dict>
      <key>CFBundleURLName</key>
      <string>{DEV_BUNDLE_ID}</string>
      <key>CFBundleURLSchemes</key>
      <array>
        <string>{DEEPLINK_SCHEME}</string>
      </array>
    </dict>
  </array>
</dict>
</plist>
"#
    );
    fs::write(&info_plist, plist).map_err(|e| format!("写入 Info.plist 失败: {e}"))?;

    let _ = fs::remove_file(&exe_link);
    symlink(&binary, &exe_link).map_err(|e| format!("创建可执行文件链接失败: {e}"))?;

    register_app_bundle(&app_dir)
}

#[cfg(target_os = "macos")]
fn target_debug_dir(exe: &Path) -> Option<PathBuf> {
    if exe.to_string_lossy().contains(".app/Contents/MacOS") {
        exe.ancestors()
            .find(|path| path.ends_with("debug"))
            .map(|path| path.to_path_buf())
    } else {
        exe.parent().map(|path| path.to_path_buf())
    }
}

#[cfg(target_os = "macos")]
fn register_app_bundle(app_dir: &Path) -> Result<(), String> {
    let status = Command::new(LSREGISTER)
        .args(["-f", "-R", "-trusted", &app_dir.to_string_lossy()])
        .status()
        .map_err(|e| format!("执行 lsregister 失败: {e}"))?;

    if !status.success() {
        return Err("lsregister 注册 zd-switch:// URL scheme 失败".into());
    }

    Ok(())
}

#[cfg(target_os = "macos")]
fn unregister_legacy_handlers() {
    if let Some(home) = dirs::home_dir() {
        let legacy = home.join("Library/Application Support/zd-switch/ZD Switch.app");
        if legacy.exists() {
            let _ = Command::new(LSREGISTER)
                .args(["-u", &legacy.to_string_lossy()])
                .status();
        }
    }

    if let Ok(exe) = std::env::current_exe() {
        if let Some(target_dir) = target_debug_dir(&exe) {
            let url_handler = target_dir.join("deeplink").join("ZD Switch URL Handler.app");
            if url_handler.exists() {
                let _ = Command::new(LSREGISTER)
                    .args(["-u", &url_handler.to_string_lossy()])
                    .status();
            }
        }
    }
}
