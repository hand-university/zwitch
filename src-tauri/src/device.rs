use crate::store::get_store;
use tauri::AppHandle;
use uuid::Uuid;

const DEVICE_KEY: &str = "device";
const FINGERPRINT_FIELD: &str = "fingerprint";

/// 读取或生成设备指纹。
///
/// 指纹独立于登录态持久化，登出不会清除，保证同一设备在重新登录后仍能复用
/// 已与后端绑定的授权码语义（由后端按 fingerprint 关联）。
pub fn get_or_create_fingerprint(app: &AppHandle) -> Result<String, String> {
    let store = get_store(app)?;

    if let Some(existing) = store.get(DEVICE_KEY).and_then(|v| {
        v.get(FINGERPRINT_FIELD)
            .and_then(|f| f.as_str().map(str::to_string))
    }) {
        if !existing.is_empty() {
            return Ok(existing);
        }
    }

    let fingerprint = generate_fingerprint();
    store.set(
        DEVICE_KEY,
        serde_json::json!({ FINGERPRINT_FIELD: &fingerprint }),
    );
    store.save().map_err(|e| format!("保存设备指纹失败: {e}"))?;
    Ok(fingerprint)
}

fn generate_fingerprint() -> String {
    let os = std::env::consts::OS;
    let arch = std::env::consts::ARCH;
    let unique = Uuid::new_v4().simple().to_string();
    format!("{os}-{arch}-{unique}")
}
