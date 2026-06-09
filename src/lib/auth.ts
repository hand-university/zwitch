import { getAuthState } from "@/lib/api";
import { message } from "@/components/ui/message";
import type { AuthState } from "@/types";

const AUTH_SESSION_EXPIRED_MARKERS = [
  "设备授权已失效",
  "登录态已过期",
  "登录会话已过期",
  "请重新登录",
  "请先登录",
  "未登录",
  "凭证无效",
] as const;

export function isAuthSessionExpiredMessage(text: string): boolean {
  return AUTH_SESSION_EXPIRED_MARKERS.some((marker) => text.includes(marker));
}

export async function reportApiError(
  error: unknown,
  onSessionExpired?: (auth: AuthState) => void,
): Promise<void> {
  const text = String(error);
  message.error(text);
  if (!onSessionExpired || !isAuthSessionExpiredMessage(text)) {
    return;
  }
  try {
    onSessionExpired(await getAuthState());
  } catch {
    onSessionExpired({
      is_logged_in: false,
      api_key: null,
      user_name: null,
      avatar: null,
      department: null,
      title: null,
    });
  }
}
