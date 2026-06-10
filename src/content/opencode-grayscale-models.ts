export interface GrayscaleModelGuide {
  id: string;
  name: string;
  badge?: string;
  summary: string;
  highlights: string[];
  suitableFor: string[];
  opencodeModel: string;
}

export const OPENCODE_GRAYSCALE_PROVIDER = "灰度 Mythos";

export const OPENCODE_GRAYSCALE_MODELS: GrayscaleModelGuide[] = [
  {
    id: "claude-mythos-preview-fast",
    name: "Claude Mythos Preview Fast",
    badge: "默认推荐",
    summary:
      "Mythos 系列的快速版本，在保持较强代码理解与生成能力的同时优先响应速度，适合日常开发中的高频交互场景。",
    highlights: [
      "ZWitch 开启代理后会自动设为 OpenCode 默认模型",
      "响应更快，适合连续对话、快速迭代与小范围改动",
      "在复杂任务上仍具备 Mythos 系列的核心推理能力",
    ],
    suitableFor: [
      "日常编码辅助与 Bug 定位",
      "快速阅读、解释与重构局部代码",
      "需要频繁切换上下文的开发流程",
    ],
    opencodeModel: `${OPENCODE_GRAYSCALE_PROVIDER}/claude-mythos-preview-fast`,
  },
  {
    id: "claude-mythos-preview",
    name: "Claude Mythos Preview",
    summary:
      "Mythos 系列的标准预览版，侧重更深层的分析与推理，适合需要仔细思考后再给出方案的任务。",
    highlights: [
      "推理深度更高，适合架构设计与多步骤问题拆解",
      "对大型代码库的理解与跨文件关联更稳健",
      "在复杂需求下通常能给出更完整的实现思路",
    ],
    suitableFor: [
      "模块设计、接口规划与重构方案",
      "跨多文件的联动修改与影响分析",
      "需要更高质量输出的关键任务",
    ],
    opencodeModel: `${OPENCODE_GRAYSCALE_PROVIDER}/claude-mythos-preview`,
  },
];

export const OPENCODE_GRAYSCALE_USAGE_STEPS = [
  "在 ZWitch 登录账号并开启「启用代理」，确保 OpenCode 开关为开启状态",
  "安装 OpenCode 后重启终端，执行 opencode 启动",
  "在模型选择界面切换到带「灰度」标识的 Provider（灰度 Mythos）",
  "选择上方任一 Mythos 模型开始对话；默认已选中 Fast 版本",
  "粘贴图片时，ZWitch 会自动通过图片代理插件辅助不支持视觉的模型",
];
