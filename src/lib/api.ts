import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import type { AuthState, CliToolStatus } from "@/types";

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

export async function setToolSwitch(
  toolId: string,
  enabled: boolean,
): Promise<void> {
  return invoke("set_tool_switch", { toolId, enabled });
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
