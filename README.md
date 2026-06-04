# ZD Switch

Tauri 桌面应用，用于 OAuth 登录后获取 API Key，并通过开关向 Codex、Claude Code、Gemini CLI 注入 `base_url` 与 `token`。

## 登录流程

1. 使用系统默认浏览器打开登录页，并附带 `source=zd-switch` 参数，例如：
   `http://localhost:8080/login?source=zd-switch`
2. 登录完成后，登录服务通过 Deep Link 唤起应用并回传 `access_token`
3. 应用用该 `access_token` + 设备指纹向后端注册设备，换取一个**持久授权码**（authorization_code）
4. 之后应用用授权码独立续期 `access_token` / `api_key`，不再依赖与 web 共用的短期 token

> `authorization_code` 才是桌面端的长期凭证；`access_token` 仅作可随时丢弃的工作态缓存。当后端拒绝授权码（登出/禁用/解绑）时，应用会强制重新登录。

### 后端需要支持的设备授权接口

| 用途 | 方法 / 路径 | 入参 | 返回 |
|------|-------------|------|------|
| 注册设备 | `POST /api/aone/devices/authorize` | Header `Authorization: Bearer {access_token}`；Body `{ device_fingerprint, device_name }` | `{ authorization_code }` |
| 换取 token | `POST /api/aone/devices/token` | Body `{ authorization_code, device_fingerprint }` | `{ access_token, base_url? }`（授权码失效返回 401/403） |
| 吊销授权 | `POST /api/aone/devices/revoke` | Body `{ authorization_code, device_fingerprint }` | — |

后端需记录 `authorization_code ↔ device_fingerprint ↔ user_id` 关联，并在用户登出或被禁用时让对应授权码失效。接口路径定义在 `src-tauri/src/config.rs`。

### 登录页需要配合的实现

登录成功后，请重定向到 Deep Link：

```
zd-switch://open?access_token=YOUR_ACCESS_TOKEN
```

首次启动应用时会自动向 macOS 注册 `zd-switch://` scheme（`tauri dev` 同样生效）。若浏览器仍提示未注册 handler，请完全退出应用后重新运行 `npm run tauri dev`。

## 功能

- 未登录时使用系统浏览器 OAuth 登录
- Deep Link 接收 `access_token` 并持久化
- 检测本地 Codex / Claude Code / Gemini CLI 安装状态
- 总开关 + 各工具独立开关，精准替换配置文件中的 `base_url` / `token`
- 关闭开关时自动恢复原始配置

## 开发

```bash
npm install
npm run tauri dev
```

## 配置说明

地址与 Deep Link scheme 写死在以下文件中，按需直接修改：

- `src-tauri/src/config.rs`（Rust 后端）
- `src/config/env.ts`（前端展示）

## 配置文件映射

| 工具 | 配置文件 | base_url 字段 | token 字段 |
|------|----------|---------------|------------|
| Codex | `~/.codex/config.toml` + `~/.codex/auth.json` | `model_providers.zsdx_ai.base_url` | `OPENAI_API_KEY`（auth.json） |
| Claude Code | `~/.claude/settings.json` | `env.ANTHROPIC_BASE_URL` | `env.ANTHROPIC_AUTH_TOKEN` |
| Gemini CLI | `~/.gemini/.env` | `GOOGLE_GEMINI_BASE_URL` | `GEMINI_API_KEY` |

## 线上环境

生产环境地址在以下文件中预留占位符，部署前请替换：

- `src/config/env.ts`
- `src-tauri/src/config.rs`

将 `YOUR_PROD_LOGIN_DOMAIN`、`YOUR_PROD_PROXY_DOMAIN` 等替换为实际域名。
