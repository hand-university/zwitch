# 灰度模型对接设计文档

本文面向 **Zwitch 客户端开发者** 与 **Bifrost 管理员**，说明灰度 API Key 的能力、网关侧访问控制，以及客户端如何通过灰度模型接口将额外模型**追加注入**到模型选择器（不替换原有列表）。

> 设备鉴权与临时凭证流程见 [客户端对接文档：设备码换取临时凭证调用 AI](./aone-device-client-integration.md)。

---

## 1. 背景与目标

部分模型仅对少量用户开放灰度验证。管理员可在某个 Provider 的 API Key 上开启**灰度模式**，并配置允许访问的 Aone 用户白名单。

| 角色 | 行为 |
| --- | --- |
| 管理员 | 在 Key 上开启灰度、配置白名单用户与可用模型列表 |
| 非白名单用户 | 无法通过该灰度 Key 发起推理；模型列表中也不展示灰度 Key 上的模型 |
| 白名单用户 | 可正常使用灰度 Key；客户端可拉取并**追加**灰度专属模型到选择器 |

客户端的核心诉求：**保留 CLI 自带模型列表，仅将灰度专属模型作为额外项注入**，避免覆盖用户熟悉的默认选项。

---

## 2. 名词约定

| 名词 | 说明 |
| --- | --- |
| 灰度 Key | `grayscale_enabled: true` 的 API Key，默认全员不可见/不可用 |
| 白名单用户 | `grayscale_users` 中的 Aone 用户 ID |
| 常规 Key | 未开启灰度的 API Key，所有已认证用户均可使用 |
| 平台 `platform` | 客户端产品标识：`claude`、`codex`、`gemini`、`opencode` |
| `injection_mode` | 模型注入方式，当前固定为 `append`（追加，不替换） |
| `models` | 来自常规 Key 的模型（builtin） |
| `additional_models` | 仅来自灰度 Key、且不在 `models` 中的额外模型 |

---

## 3. 管理员配置

### 3.1 Web UI

路径：**工作区 → Providers → 选择 Provider → API Keys → 编辑 Key**

在 **灰度访问** 区域可配置：

- **灰度模式**：开关，默认关闭
- **允许访问的用户**：Aone 用户多选（仅灰度开启时显示）

开启后语义：

- 仅白名单中的 Aone 用户可使用该 Key 及其模型
- 其他用户在选择 Key、列出模型、发起推理时均不可见/不可用该 Key

### 3.2 config.json

```json
{
  "keys": [
    {
      "id": "gray-anthropic",
      "name": "灰度 Anthropic",
      "value": "env.ANTHROPIC_GRAY_KEY",
      "models": ["claude-sonnet-4-5"],
      "grayscale_enabled": true,
      "grayscale_users": ["aone-user-id-1", "aone-user-id-2"]
    },
    {
      "id": "open-anthropic",
      "name": "常规 Anthropic",
      "value": "env.ANTHROPIC_KEY",
      "models": ["claude-opus-4"]
    }
  ]
}
```

| 字段 | 类型 | 默认 | 说明 |
| --- | --- | --- | --- |
| `grayscale_enabled` | `boolean` | `false` | 为 `true` 时启用灰度限制 |
| `grayscale_users` | `string[]` | `[]` | 允许使用的 Aone 用户 ID 列表 |

`models` 字段语义不变：可指定具体模型，或使用 `["*"]` 表示该 Key 可访问 Provider 下全部模型（受 `blacklisted_models` 约束）。

---

## 4. 网关侧访问控制

灰度策略在以下链路生效（均基于请求关联的 **Aone 用户 ID**）：

```
客户端请求（携带 session / bf-tmp- 凭证）
  → 解析 Aone 用户 ID
  → Key 选择：跳过当前用户无权访问的灰度 Key
  → 模型列表：仅展示至少有一个可访问 Key 能提供的模型
  → 灰度模型 API：按用户返回 builtin + additional 分组
```

### 4.1 判定规则

```text
grayscale_enabled == false  → 所有用户可访问
grayscale_enabled == true   → 仅 grayscale_users 中的用户可访问
                            → 无用户 ID 的请求一律拒绝
```

### 4.2 与虚拟 Key 的关系

通过个人 Aone 虚拟 Key 发起的请求，网关会将虚拟 Key 的 `created_by_user_id` 作为 `AoneUserID` 参与灰度过滤。设备临时凭证（`bf-tmp-...`）则绑定注册设备时的 Aone 用户。

---

## 5. 客户端平台映射

灰度模型 API 按**客户端平台**分组，而非直接暴露 Bifrost Provider 名称。映射关系如下：

| 平台 ID | 客户端 | 网关转发前缀 `base_path` | 对应 Provider |
| --- | --- | --- | --- |
| `claude` | Claude Code | `/anthropic` | `anthropic` |
| `codex` | Codex CLI | `/openai` | `openai` |
| `gemini` | Gemini CLI | `/genai` | `gemini` |
| `opencode` | Opencode | `/openai` | `openai` |

> Custom Provider 以 `custom_provider_config.base_provider_type` 作为映射依据。例如 base 为 `anthropic` 的自定义 Provider，其模型归入 `claude` 平台。

同一 OpenAI 兼容模型会同时出现在 `codex` 与 `opencode` 两个平台条目中（计数分别统计）。

---

## 6. 灰度模型 API

### 6.1 接口

```http
GET {BASE_URL}/api/aone/zwitch/grayscale-models?platform={platform}
```

| 参数 | 必填 | 说明 |
| --- | --- | --- |
| `platform` | 否 | 过滤单个平台：`claude`、`codex`、`gemini`、`opencode`。省略时返回全部平台 |

### 6.2 鉴权

与 `GET /api/aone/users/me` 相同，支持以下任一方式：

| 方式 | 示例 |
| --- | --- |
| Bearer 会话令牌 | 浏览器登录后的 session token |
| Bearer 设备临时凭证 | `bf-tmp-...`（换取方式见设备对接文档） |
| Cookie | 携带 session token 的 cookie |

使用设备临时凭证时，**不需要**额外携带 `X-Device-Fingerprint`（该头仅用于 AI 推理转发）。

### 6.3 响应结构

```json
{
  "injection_mode": "append",
  "platforms": [
    {
      "id": "claude",
      "label": "Claude Code",
      "base_path": "/anthropic",
      "injection_mode": "append",
      "models": [
        {
          "id": "claude-opus-4",
          "provider": "anthropic",
          "key_id": "open-anthropic",
          "key_name": "常规 Anthropic",
          "source": "builtin"
        }
      ],
      "additional_models": [
        {
          "id": "claude-sonnet-4-5",
          "provider": "anthropic",
          "key_id": "gray-anthropic",
          "key_name": "灰度 Anthropic",
          "source": "grayscale"
        }
      ]
    }
  ],
  "total": 2,
  "total_builtin": 1,
  "total_additional": 1
}
```

#### 顶层字段

| 字段 | 说明 |
| --- | --- |
| `injection_mode` | 固定 `"append"`，告知客户端注入策略 |
| `platforms` | 按平台分组的模型列表 |
| `total` | 全部平台的 `models` + `additional_models` 条目总数 |
| `total_builtin` | `models` 条目总数 |
| `total_additional` | `additional_models` 条目总数 |

#### 平台对象字段

| 字段 | 说明 |
| --- | --- |
| `id` | 平台标识 |
| `label` | 平台展示名 |
| `base_path` | 调用 AI 时使用的网关路径前缀 |
| `injection_mode` | 平台级注入策略，固定 `"append"` |
| `models` | 常规 Key 提供的模型（`source: "builtin"`） |
| `additional_models` | 灰度 Key 提供的额外模型（`source: "grayscale"`） |

#### 模型条目字段

| 字段 | 说明 |
| --- | --- |
| `id` | 模型 ID，推理时作为 `model` 参数 |
| `provider` | Bifrost Provider 名称（如 `anthropic`、`openai`） |
| `key_id` | 来源 Key ID（调试用，客户端通常不需要展示） |
| `key_name` | 来源 Key 名称（可选展示） |
| `source` | `"builtin"` 或 `"grayscale"` |

### 6.4 收集规则

1. **第一遍（常规 Key）**：遍历所有未开启灰度的已启用 Key，将其模型写入对应平台的 `models`。
2. **第二遍（灰度 Key）**：遍历当前用户可访问的灰度 Key，将其模型写入 `additional_models`。
3. **去重**：若某模型 ID 已存在于 `models`，则不再出现在 `additional_models`（即使灰度 Key 也配置了该模型）。
4. **模型展开**：Key 的 `models` 为 `["*"]` 时，从 Model Catalog 展开该 Provider 的全部模型；空列表或 Key 被禁用则不贡献模型。

### 6.5 错误响应

| HTTP | 场景 |
| --- | --- |
| `401` | 未鉴权、会话/凭证无效或过期 |
| `400` | `platform` 参数不在支持列表中 |
| `503` | 配置存储不可用 |

---

## 7. 客户端集成指南

### 7.1 推荐流程

```text
① 用户完成 Aone 登录 / 换取 bf-tmp- 凭证
② 确定当前运行的平台（如 claude）
③ GET /api/aone/zwitch/grayscale-models?platform=claude
④ 合并模型列表（见 7.2）
⑤ 用户选择模型后，按 base_path 调用 AI 转发接口
```

建议在以下时机刷新灰度模型列表：

- 应用启动 / 用户登录成功后
- 临时凭证刷新后
- 用户手动「刷新模型」时

无需高频轮询；Key 配置变更后用户重新拉取即可。

### 7.2 模型合并（append 模式）

**核心原则：不替换 CLI 自带模型，仅追加灰度额外项。**

```typescript
function mergeModels(
  clientNativeModels: string[],
  platform: GrayscalePlatformModels,
): string[] {
  const seen = new Set(clientNativeModels);
  const result = [...clientNativeModels];

  // optional: 合并网关常规模型（若客户端未内置）
  for (const entry of platform.models) {
    if (!seen.has(entry.id)) {
      seen.add(entry.id);
      result.push(entry.id);
    }
  }

  // 必须：追加灰度专属模型
  for (const entry of platform.additional_models) {
    if (!seen.has(entry.id)) {
      seen.add(entry.id);
      result.push(entry.id);
    }
  }

  return result;
}
```

合并优先级（同一 `id` 只保留首次出现）：

```
客户端原生模型  >  models（builtin）  >  additional_models（grayscale）
```

### 7.3 推理调用

选定模型后，使用对应平台的 `base_path` 拼接 `BASE_URL` 发起请求：

```http
POST {BASE_URL}{base_path}/v1/messages
Authorization: Bearer bf-tmp-...
X-Device-Fingerprint: <DEVICE_FINGERPRINT>
```

具体路径因 SDK 风格而异，参见 [设备对接文档 §④](./aone-device-client-integration.md#④-使用临时凭证调用-aiforward)。

> 灰度模型 API 仅负责**发现**可用模型；实际推理仍走常规 AI 转发链路，网关会自动选择用户有权访问的 Key。

### 7.4 非白名单用户体验

| 场景 | 行为 |
| --- | --- |
| 拉取灰度模型 API | `models` 仍包含常规模型；`additional_models` 为空 |
| 选择灰度专属模型发起推理 | 网关无可用 Key，请求失败 |
| 选择常规模型发起推理 | 正常 |

客户端可选择：仅将 `additional_models` 注入选择器，非白名单用户不会看到灰度专属条目。

### 7.5 UI 展示建议（可选）

对 `additional_models` 中的条目，可在模型名称旁标注灰度标识，例如：

```text
claude-sonnet-4-5  [灰度]
```

`source === "grayscale"` 或条目仅存在于 `additional_models` 均可作为判断依据。

---

## 8. 端到端示例

### 8.1 拉取 Claude 平台灰度模型

```bash
BASE_URL="https://your-gateway"
CRED="bf-tmp-a1b2c3..."   # 来自 /api/aone/devices/token

curl -s "$BASE_URL/api/aone/zwitch/grayscale-models?platform=claude" \
  -H "Authorization: Bearer $CRED" | jq .
```

### 8.2 白名单用户响应示例

假设配置：

- 常规 Key `open-anthropic`：`claude-opus-4`
- 灰度 Key `gray-anthropic`（白名单含当前用户）：`claude-sonnet-4-5`

```json
{
  "injection_mode": "append",
  "platforms": [
    {
      "id": "claude",
      "label": "Claude Code",
      "base_path": "/anthropic",
      "injection_mode": "append",
      "models": [
        { "id": "claude-opus-4", "provider": "anthropic", "source": "builtin" }
      ],
      "additional_models": [
        { "id": "claude-sonnet-4-5", "provider": "anthropic", "source": "grayscale" }
      ]
    }
  ],
  "total": 2,
  "total_builtin": 1,
  "total_additional": 1
}
```

客户端合并后模型选择器应包含：`[...原生模型..., claude-opus-4, claude-sonnet-4-5]`。

### 8.3 非白名单用户响应示例

同一配置下，非白名单用户：

```json
{
  "injection_mode": "append",
  "platforms": [
    {
      "id": "claude",
      "injection_mode": "append",
      "models": [
        { "id": "claude-opus-4", "source": "builtin" }
      ],
      "additional_models": []
    }
  ],
  "total": 1,
  "total_builtin": 1,
  "total_additional": 0
}
```

### 8.4 模型重叠去重示例

常规 Key 与灰度 Key 均配置 `claude-sonnet-4-5` 时：

- 该模型仅出现在 `models`（`source: "builtin"`）
- `additional_models` 为空

---

## 9. 架构示意

```text
┌──────────────────────────────────────────────────────────────┐
│ 管理员（Bifrost UI / config.json）                             │
│   Provider Key: grayscale_enabled + grayscale_users + models │
└────────────────────────────┬─────────────────────────────────┘
                             │
                             ▼
┌──────────────────────────────────────────────────────────────┐
│ Bifrost 网关                                                  │
│  ┌─────────────────┐  ┌──────────────────────────────────┐ │
│  │ Key 选择 / 推理  │  │ GET /api/aone/zwitch/grayscale-  │ │
│  │ 灰度过滤         │  │     models                       │ │
│  └─────────────────┘  └──────────────────────────────────┘ │
└────────────────────────────┬─────────────────────────────────┘
                             │
                             ▼
┌──────────────────────────────────────────────────────────────┐
│ Zwitch 客户端                                                 │
│  原生模型列表 + additional_models（append，不替换）            │
│  → 用户选择模型 → {BASE_URL}{base_path}/... 推理              │
└──────────────────────────────────────────────────────────────┘
```

---

## 10. 接口速查

| 用途 | 方法 | 路径 | 鉴权 |
| --- | --- | --- | --- |
| 获取灰度模型 | GET | `/api/aone/zwitch/grayscale-models` | Bearer session 或 `bf-tmp-...` |
| 获取当前用户 | GET | `/api/aone/users/me` | 同上 |
| 换取设备凭证 | POST | `/api/aone/devices/token` | 设备码 + 指纹 |
| AI 推理 | POST | `{base_path}/...` | `bf-tmp-...` + `X-Device-Fingerprint` |

---

## 11. 实现参考

| 模块 | 路径 |
| --- | --- |
| 灰度模型聚合 | `transports/bifrost-http/handlers/grayscalemodels.go` |
| HTTP Handler | `transports/bifrost-http/handlers/grayscalemodels_handler.go` |
| Key 灰度字段 | `core/schemas/account.go` |
| Key 选择过滤 | `core/bifrost.go` |
| 模型列表过滤 | `transports/bifrost-http/handlers/providers.go` |
| 客户端平台定义 | `cli/internal/harness/harness.go` |
| 单元测试 | `transports/bifrost-http/handlers/grayscalemodels_test.go` |

---

## 12. 变更记录

| 版本 | 说明 |
| --- | --- |
| v1 | 灰度 Key + 白名单用户 + 模型列表/Key 选择过滤 |
| v1.1 | 新增 `GET /api/aone/zwitch/grayscale-models`，`injection_mode=append`，区分 `models` 与 `additional_models` |
