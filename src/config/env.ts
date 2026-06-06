/** Dev / prod endpoint configuration */
export const isDev = import.meta.env.DEV;

/** OAuth login page base URL */
export const LOGIN_BASE_URL = isDev
  ? "http://localhost:8080"
  : "https://ft-wxhand.com/zai";

/** OAuth login page，携带 source=zwitch */
export const LOGIN_URL = `${LOGIN_BASE_URL}/login?source=zwitch`;

/** Injected proxy base URL for CLI tools */
export const PROXY_BASE_URL = isDev
  ? "http://localhost:8080/v1"
  : "https://ft-wxhand.com/zai/v1";

/** Deep link 回调地址 */
export const DEEPLINK_SCHEME = "zwitch";
export const DEEPLINK_HOST = "open";
export const DEEPLINK_CALLBACK_URL = `${DEEPLINK_SCHEME}://${DEEPLINK_HOST}`;

export const CLI_INSTALL_LINKS = {
  codex: "https://developers.openai.com/codex/cli",
  claude: "https://docs.anthropic.com/en/docs/claude-code",
  gemini: "https://github.com/google-gemini/gemini-cli",
} as const;

export type CliToolId = keyof typeof CLI_INSTALL_LINKS;
