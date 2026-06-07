import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import type {
  AppInfo,
  AuthState,
  CliToolStatus,
  DownloadedUpdateInfo,
  ExploreItem,
  MarketplaceItem,
  MarketplaceSyncResult,
  UpdateCheckResult,
  UpdateDownloadProgress,
  UsageSummary,
} from "@/types";

export async function getAuthState(): Promise<AuthState> {
  return invoke("get_auth_state");
}

export async function openLoginWindow(): Promise<void> {
  return invoke("open_login_window");
}

export async function logout(): Promise<void> {
  return invoke("logout");
}

export async function refreshUserProfile(): Promise<AuthState> {
  return invoke("refresh_user_profile");
}

export async function getCliToolsStatus(): Promise<CliToolStatus[]> {
  return invoke("get_cli_tools_status");
}

export async function getProxyEnabled(): Promise<boolean> {
  return invoke("get_proxy_enabled");
}

export async function setProxyEnabled(enabled: boolean): Promise<void> {
  return invoke("set_proxy_enabled", { enabled });
}

export async function applyConfigInjection(): Promise<void> {
  return invoke("apply_config_injection");
}

export function onAuthChanged(callback: (state: AuthState) => void) {
  return listen<AuthState>("auth-changed", (event) => {
    callback(event.payload);
  });
}

export function onLoginFailed(callback: (message: string) => void) {
  return listen<string>("login-failed", (event) => {
    callback(event.payload);
  });
}

export function onProxyChanged(callback: (enabled: boolean) => void) {
  return listen<boolean>("proxy-changed", (event) => {
    callback(event.payload);
  });
}

export async function getExploreItems(): Promise<ExploreItem[]> {
  return invoke("get_explore_items");
}

export async function installMarketplaceItem(
  platform: string,
  itemType: string,
  name: string,
): Promise<void> {
  return invoke("install_marketplace_item", { platform, itemType, name });
}

export async function getMarketplaceItems(): Promise<MarketplaceItem[]> {
  return invoke("get_marketplace_items");
}

export async function syncMarketplace(): Promise<MarketplaceSyncResult> {
  return invoke("sync_marketplace");
}

export async function setMarketplaceItemEnabled(
  platform: string,
  itemType: string,
  name: string,
  enabled: boolean,
): Promise<void> {
  return invoke("set_marketplace_item_enabled", {
    platform,
    itemType,
    name,
    enabled,
  });
}

export async function deleteMarketplaceItem(
  platform: string,
  itemType: string,
  name: string,
): Promise<void> {
  return invoke("delete_marketplace_item", { platform, itemType, name });
}

export async function scanLocalMarketplace(): Promise<number> {
  return invoke("scan_local_marketplace");
}

export async function getAppInfo(): Promise<AppInfo> {
  return invoke("get_app_info");
}

export async function checkForUpdate(): Promise<UpdateCheckResult> {
  return invoke("check_for_update");
}

export async function getDownloadedUpdateInfo(): Promise<DownloadedUpdateInfo> {
  return invoke("get_downloaded_update_info");
}

export async function downloadAvailableUpdate(): Promise<void> {
  return invoke("download_available_update");
}

export async function installDownloadedUpdate(): Promise<void> {
  return invoke("install_downloaded_update");
}

export async function deferDownloadedUpdate(): Promise<void> {
  return invoke("defer_downloaded_update");
}

export function onUpdateDownloadProgress(
  callback: (progress: UpdateDownloadProgress) => void,
) {
  return listen<UpdateDownloadProgress>("update-download-progress", (event) => {
    callback(event.payload);
  });
}

export async function getUsageSummary(): Promise<UsageSummary> {
  return invoke("get_usage_summary");
}

export async function clearUsage(): Promise<void> {
  return invoke("clear_usage");
}
