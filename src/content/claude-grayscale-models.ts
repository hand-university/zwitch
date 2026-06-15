export interface GrayscaleModelGuide {
  id: string;
  name: string;
  badge?: string;
  summary: string;
  highlights: string[];
  suitableFor: string[];
  claudeModelId: string;
}

export const CLAUDE_GRAYSCALE_MODELS: GrayscaleModelGuide[] = [
  {
    id: "claude-fable-5",
    name: "claude-fable-5",
    badge: "灰度",
    summary:
      "Fable 系列灰度模型，通过 Claude Code 自定义模型注入。仅对白名单用户可见，模型 ID 与展示名称均为 claude-fable-5。",
    highlights: [
      "ZWitch 开启代理后会自动写入 Claude Code 自定义模型配置",
      "在 Claude Code 模型选择器中可直接选择 claude-fable-5",
      "配置方式与 Claude CLI 自定义模型一致",
    ],
    suitableFor: [
      "日常编码辅助与代码理解",
      "架构设计与多步骤问题拆解",
      "需要体验灰度新模型的开发场景",
    ],
    claudeModelId: "claude-fable-5",
  },
];

export const CLAUDE_GRAYSCALE_USAGE_STEPS = [
  "在 ZWitch 登录账号并开启「启用代理」，确保 Claude Code 开关为开启状态",
  "安装 Claude Code 后重启终端，执行 claude 启动",
  "在 Claude Code 模型选择界面选择 claude-fable-5",
  "若看不到该模型，请确认已登录 ZWitch、代理处于运行中，且账号已在灰度白名单内",
];
