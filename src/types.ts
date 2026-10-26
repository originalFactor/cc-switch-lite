export type ProviderCategory =
  | "official" // 官方
  | "cn_official" // 开源官方（原"国产官方"）
  | "cloud_provider" // 云服务商（AWS Bedrock 等）
  | "aggregator" // 聚合网站
  | "third_party" // 第三方供应商
  | "custom"; // 自定义

export interface Provider {
  id: string;
  name: string;
  settingsConfig: Record<string, any>; // 应用配置对象：Claude 为 settings.json；Codex 为 { auth, config }
  websiteUrl?: string;
  // 新增：供应商分类（用于差异化提示/能力开关）
  category?: ProviderCategory;
  createdAt?: number; // 添加时间戳（毫秒）
  sortIndex?: number; // 排序索引（用于自定义拖拽排序）
  // 备注信息
  notes?: string;
  // 新增：是否为商业合作伙伴
  isPartner?: boolean;
  // 可选：供应商元数据（仅存于 ~/.cc-switch/config.json，不写入 live 配置）
  meta?: ProviderMeta;
  // 图标配置
  icon?: string; // 图标名称（如 "openai", "anthropic"）
  iconColor?: string; // 图标颜色（Hex 格式，如 "#00A67E"）
  // 是否加入故障转移队列
  inFailoverQueue?: boolean;
}

export interface AppConfig {
  providers: Record<string, Provider>;
  current: string;
}

// 自定义端点配置
export interface CustomEndpoint {
  url: string;
  addedAt: number;
  lastUsed?: number;
}

// 端点候选项（用于端点测速弹窗）
export interface EndpointCandidate {
  id?: string;
  url: string;
  isCustom?: boolean;
}

import type { TemplateType } from "./config/constants";

// 用量查询脚本配置
export interface UsageScript {
  enabled: boolean; // 是否启用用量查询
  language: "javascript"; // 脚本语言
  code: string; // 脚本代码（JSON 格式配置）
  timeout?: number; // 超时时间（秒，默认 10）
  templateType?: TemplateType; // 模板类型（用于后端判断验证规则）
  apiKey?: string; // 用量查询专用的 API Key（通用模板使用）
  baseUrl?: string; // 用量查询专用的 Base URL（通用和 NewAPI 模板使用）
  accessToken?: string; // 访问令牌（NewAPI 模板使用）
  userId?: string; // 用户ID（NewAPI 模板使用）
  accessKeyId?: string; // 火山方舟 AccessKey ID（用量查询签名用，与推理 Key 分离）
  secretAccessKey?: string; // 火山方舟 SecretAccessKey
  teamOrganizationId?: string; // 智谱团队套餐组织 ID（请求头 bigmodel-organization）
  teamProjectId?: string; // 智谱团队套餐项目 ID（请求头 bigmodel-project）
  codingPlanProvider?: string; // Coding Plan 供应商标识（如 "kimi", "zhipu", "minimax"）
  autoQueryInterval?: number; // 自动查询间隔（单位：分钟，0 表示禁用）
  autoIntervalMinutes?: number; // 自动查询间隔（分钟）- 别名字段
  request?: {
    // 请求配置
    url?: string; // 请求 URL
    method?: string; // HTTP 方法
    headers?: Record<string, string>; // 请求头
    body?: any; // 请求体
  };
}

const DEFAULT_USAGE_SCRIPT: UsageScript = {
  enabled: false,
  language: "javascript",
  code: "",
  timeout: 10,
  autoQueryInterval: 5,
};

export function createUsageScript(
  overrides?: Partial<UsageScript>,
): UsageScript {
  return { ...DEFAULT_USAGE_SCRIPT, ...overrides };
}

// 单个套餐用量数据
export interface UsageData {
  planName?: string; // 套餐名称（可选）
  extra?: string; // 扩展字段，可自由补充需要展示的文本（可选）
  isValid?: boolean; // 套餐是否有效（可选）
  invalidMessage?: string; // 失效原因说明（可选，当 isValid 为 false 时显示）
  total?: number; // 总额度（可选）
  used?: number; // 已用额度（可选）
  remaining?: number; // 剩余额度（可选）
  unit?: string; // 单位（可选）
}

// 用量查询结果（支持多套餐）
export interface UsageResult {
  success: boolean;
  data?: UsageData[]; // 改为数组，支持返回多个套餐
  error?: string;
}

export interface ClaudeDesktopModelRoute {
  model: string;
  labelOverride?: string;
  supports1m?: boolean;
}

/** Claude Desktop 3P 写入模式：直连 / 本地路由模型映射 */
export type ClaudeDesktopMode = "direct" | "proxy";

export type CodexChatThinkingParam =
  "none" | "thinking" | "enable_thinking" | "reasoning_split";

export type CodexChatEffortParam =
  | "none"
  | "reasoning_effort"
  // OpenRouter 原生归一化对象 reasoning:{effort}（区别于顶层 OpenAI 别名 reasoning_effort）
  | "reasoning.effort";

export type CodexChatEffortValueMode =
  | "passthrough"
  | "low_high"
  | "deepseek"
  // OpenRouter effort 枚举 xhigh|high|medium|low|minimal（无 max，max 钳到 xhigh）
  | "openrouter"
  // OpenCode Zen 网关：合法档位逐模型，见 modelCatalog 各条目 reasoningLevels
  // （镜像 oh-my-pi 模型目录）；代理转换层按请求模型查表钳制，无表不发 effort 字段
  | "zen";

export type CodexChatReasoningOutputFormat =
  | "auto"
  | "reasoning_content"
  | "reasoning"
  | "reasoning_details"
  | "think_tags";

export interface CodexChatReasoning {
  supportsThinking?: boolean;
  supportsEffort?: boolean;
  thinkingParam?: CodexChatThinkingParam;
  effortParam?: CodexChatEffortParam;
  effortValueMode?: CodexChatEffortValueMode;
  // 声明性字段：标注上游 reasoning 回传位置。当前提取靠穷举字段，未读取此值（think_tags 尚未接线）。
  outputFormat?: CodexChatReasoningOutputFormat;
}

export type PromptCacheRoutingMode = "auto" | "enabled" | "disabled";

export interface LocalProxyRequestOverrides {
  headers?: Record<string, string>;
  body?: Record<string, unknown>;
}

// 供应商元数据（字段名与后端一致，保持 snake_case）
export interface ProviderMeta {
  // 自定义端点：以 URL 为键，值为端点信息
  custom_endpoints?: Record<string, CustomEndpoint>;
  // 是否在切换/同步到 live 时应用通用配置片段
  commonConfigEnabled?: boolean;
  // Claude Desktop 3P 配置写入模式
  claudeDesktopMode?: ClaudeDesktopMode;
  // Claude Desktop 本地路由模式：Claude-safe route -> upstream model
  claudeDesktopModelRoutes?: Record<string, ClaudeDesktopModelRoute>;
  // 用量查询脚本配置
  usage_script?: UsageScript;
  // 请求地址管理：测速后自动选择最佳端点
  endpointAutoSelect?: boolean;
  // 是否为官方合作伙伴
  isPartner?: boolean;
  // 合作伙伴促销 key（用于后端识别 PackyCode 等）
  partnerPromotionKey?: string;
  // API 格式（Claude / Codex 供应商使用）
  // - "anthropic": 原生 Anthropic Messages API 格式，直接透传
  // - "openai_chat": OpenAI Chat Completions 格式，需要格式转换
  // - "openai_responses": OpenAI Responses API 格式，需要格式转换
  // - "gemini_native": Gemini Native generateContent API 格式，需要格式转换
  apiFormat?:
    "anthropic" | "openai_chat" | "openai_responses" | "gemini_native";
  // Managed Codex Copilot uses this instead of apiFormat. Missing/unknown means auto.
  // Keep the raw string so selections from newer versions survive an edit/save.
  codexCopilotApiFormat?: string;
  // Claude 认证字段名
  apiKeyField?: ClaudeApiKeyField;
  // 是否将 base_url 视为完整 API 端点（代理直接使用此 URL，不拼接路径）
  isFullUrl?: boolean;
  // Prompt cache key for OpenAI Responses-compatible endpoints (improves cache hit rate)
  promptCacheKey?: string;
  // Session-based prompt-cache routing for Codex Responses -> Chat conversions.
  // auto enables only for known-compatible upstreams; enabled/disabled are user overrides.
  promptCacheRouting?: PromptCacheRoutingMode;
  // Codex OAuth FAST mode: injects service_tier="priority" on ChatGPT Codex requests
  codexFastMode?: boolean;
  // Codex Responses -> Chat Completions reasoning capability metadata
  codexChatReasoning?: CodexChatReasoning;
  // Codex → Anthropic path: emulate the Claude Code client (disabled by default; only an explicit true enables it)
  impersonateClaudeCode?: boolean;
  // Codex → Anthropic path: override the Anthropic max_tokens (output ceiling).
  // Codex does not forward model_max_output_tokens in the request body; without
  // this the path falls back to a conservative 8192 default, which can truncate
  // long/thinking-heavy responses. When set (>0) it takes precedence over the
  // request value and the default.
  maxOutputTokens?: number;
  // Custom User-Agent for local proxy routing. Only applied by the local proxy.
  customUserAgent?: string;
  // Local proxy request overrides. Only applied by the local proxy after route transforms.
  localProxyRequestOverrides?: LocalProxyRequestOverrides;
  // 供应商类型（用于识别特殊供应商）
  providerType?: string;
  // Stack 模式下这家 Claude Code 供应商发布的模型；没有时按模型映射发布，空列表什么都不发布
  stackModels?: ClaudeStackModel[];
}

// Stack 模式下 Claude Code 供应商发布的一个模型
export interface ClaudeStackModel {
  // 发往上游的模型名（不带 1M 标记）
  model: string;
  // 选择器里的显示名，没有时用模型名
  displayName?: string;
  // 上游是 1M 窗口
  oneM?: boolean;
}

// Skill 同步方式
export type SkillSyncMethod = "auto" | "symlink" | "copy";

// Skill 存储位置
export type SkillStorageLocation = "cc_switch" | "unified";

// Claude API 格式类型
// - "anthropic": 原生 Anthropic Messages API 格式，直接透传
// - "openai_chat": OpenAI Chat Completions 格式，需要格式转换
// - "openai_responses": OpenAI Responses API 格式，需要格式转换
// - "gemini_native": Gemini Native generateContent API 格式，需要格式转换
export type ClaudeApiFormat =
  "anthropic" | "openai_chat" | "openai_responses" | "gemini_native";

// Codex API 格式类型
// - "openai_responses": OpenAI Responses API 格式，直接透传
// - "openai_chat": OpenAI Chat Completions 格式，需要本地路由转换
// - "anthropic": native Anthropic Messages format, needs local routing to convert to Responses
export type CodexApiFormat = "openai_responses" | "openai_chat" | "anthropic";

export type CodexCopilotApiFormat = "auto" | "openai_responses" | "openai_chat";

export interface CodexCatalogModel {
  model: string;
  displayName?: string;
  contextWindow?: string | number;
  // Hidden provider capability metadata for the generated model catalog.
  // supportsParallelToolCalls is native-profile-only; inputModalities wins over
  // automatic text-only model detection for every profile.
  supportsParallelToolCalls?: boolean;
  inputModalities?: string[];
  // Vendor's OFFICIAL base_instructions (model identity / system preamble).
  // Codex requires this field in every catalog entry; when omitted the backend
  // falls back to a neutral default. e.g. MiMo "developed by Xiaomi".
  baseInstructions?: string;
  // Per-model reasoning effort levels exposed in the generated Codex catalog
  // (e.g. ["none", "low", "medium", "high", "xhigh", "max"]). When omitted the
  // backend keeps the template's conservative none/high default.
  reasoningLevels?: string[];
  // Per-model default reasoning effort. Only meaningful together with
  // reasoningLevels; when omitted the backend keeps the template default if it
  // is still in the list, otherwise the highest declared level.
  defaultReasoningLevel?: string;
}

// Claude 认证字段类型
export type ClaudeApiKeyField = "ANTHROPIC_AUTH_TOKEN" | "ANTHROPIC_API_KEY";

// 主页面显示的应用配置
export interface VisibleApps {
  claude: boolean;
  "claude-desktop": boolean;
  codex: boolean;
  gemini: boolean;
  grokbuild: boolean;
}

// WebDAV 同步状态
export interface WebDavSyncStatus {
  lastSyncAt?: number | null;
  lastError?: string | null;
  lastErrorSource?: string | null;
  lastRemoteEtag?: string | null;
  lastLocalManifestHash?: string | null;
  lastRemoteManifestHash?: string | null;
}

// WebDAV 同步配置
export interface WebDavSyncSettings {
  enabled?: boolean;
  autoSync?: boolean;
  baseUrl?: string;
  username?: string;
  password?: string;
  remoteRoot?: string;
  profile?: string;
  status?: WebDavSyncStatus;
}

// S3 同步配置
export interface S3SyncSettings {
  enabled?: boolean;
  autoSync?: boolean;
  region?: string;
  bucket?: string;
  accessKeyId?: string;
  secretAccessKey?: string;
  endpoint?: string;
  remoteRoot?: string;
  profile?: string;
  status?: WebDavSyncStatus;
}

export type RemoteSnapshotLayout = "current" | "legacy";

// 远端快照信息（下载前预览）
export interface RemoteSnapshotInfo {
  deviceName: string;
  createdAt: string;
  snapshotId: string;
  version: number;
  protocolVersion: number;
  dbCompatVersion?: number | null;
  compatible: boolean;
  artifacts: string[];
  layout: RemoteSnapshotLayout;
  remotePath: string;
}

// 应用设置类型（用于设置对话框与 Tauri API）
// 存储在本地 ~/.cc-switch/settings.json，不随数据库同步
export interface Settings {
  // ===== 设备级 UI 设置 =====
  // 是否在系统托盘（macOS 菜单栏）显示图标
  showInTray: boolean;
  // 点击关闭按钮时是否最小化到托盘而不是关闭应用
  minimizeToTrayOnClose: boolean;
  // 是否启用应用级窗口控制按钮（最小化/最大化/关闭）
  useAppWindowControls?: boolean;
  // 启用 Claude 插件联动（写入 ~/.claude/config.json 的 primaryApiKey）
  enableClaudePluginIntegration?: boolean;
  // 跳过 Claude Code 初次安装确认（写入 ~/.claude.json 的 hasCompletedOnboarding）
  skipClaudeOnboarding?: boolean;
  // 是否开机自启
  launchOnStartup?: boolean;
  // 静默启动（程序启动时不显示主窗口）
  silentStartup?: boolean;
  // 是否启用主页面本地代理功能（默认关闭）
  enableLocalProxy?: boolean;
  // 是否在主页面显示 Stack 模式开关（默认关闭）。和 enableLocalProxy 二选一，只影响 Claude Code、Codex
  enableStackMode?: boolean;
  // User has confirmed the local proxy first-run notice
  proxyConfirmed?: boolean;
  // User has confirmed the usage query first-run notice
  usageConfirmed?: boolean;
  usageDashboardRefreshIntervalMs?: number;
  // 会话用量自动扫描开关（默认开启=自动模式；关闭后仅手动同步时扫描会话日志，代理记账不受影响）
  sessionAutoSyncEnabled?: boolean;
  // Whether to show the failover toggle independently on the main page
  enableFailoverToggle?: boolean;
  // Whether to show the project profile switcher on the main page header
  showProfileSwitcher?: boolean;
  // 供应商页页头显示搜索按钮（默认开；关掉后 ⌘F 仍可用）
  showProviderSearch?: boolean;
  // 启动时检查已安装的命令行应用有没有新版本（默认关）
  checkToolUpdatesOnStartup?: boolean;
  // Preserve Codex ChatGPT login in auth.json when switching third-party providers
  preserveCodexOfficialAuthOnSwitch?: boolean;
  // Run official Codex under the shared "custom" provider id so future
  // sessions share one resume-history bucket with third-party providers
  unifyCodexSessionHistory?: boolean;
  // User opted in (enable dialog checkbox) to migrate existing official sessions
  unifyCodexMigrateExisting?: boolean;
  // Codex aggregation: every catalog row uses the classic (v1) sub-agent tools,
  // so a sub-agent on another provider can read its task (off by default)
  codexStackClassicSubagents?: boolean;
  // User has confirmed the failover toggle first-run notice
  failoverConfirmed?: boolean;
  // User has confirmed the first-run welcome notice
  firstRunNoticeConfirmed?: boolean;
  // User has confirmed the one-time "new layout" dialog shown to upgrading users
  newLayoutNoticeConfirmed?: boolean;
  // Highest app version whose "what's new" summary the user has seen on this device
  whatsNewSeenVersion?: string;
  // User has confirmed the auto-sync traffic warning
  autoSyncConfirmed?: boolean;
  // User has confirmed the common config first-run notice
  commonConfigConfirmed?: boolean;
  // 首选语言（可选，默认中文）
  language?: "en" | "zh" | "zh-TW" | "ja";
  // 按档的额度百分比写剩余还是已用（默认剩余；余额、Credits 不受影响）
  quotaDisplay?: "left" | "used";

  // 主页面显示的应用（默认全部显示）
  visibleApps?: VisibleApps;

  // ===== 设备级目录覆盖 =====
  // 覆盖 Claude Code 配置目录（可选）
  claudeConfigDir?: string;
  // 覆盖 Codex 配置目录（可选）
  codexConfigDir?: string;
  // 覆盖 Gemini 配置目录（可选）
  geminiConfigDir?: string;
  // 覆盖 Grok Build 配置目录（可选）
  grokConfigDir?: string;

  // ===== 当前供应商 ID（设备级）=====
  // 当前 Claude 供应商 ID（优先于数据库 is_current）
  currentProviderClaude?: string;
  // 当前 Claude Desktop 供应商 ID（优先于数据库 is_current）
  currentProviderClaudeDesktop?: string;
  // 当前 Codex 供应商 ID（优先于数据库 is_current）
  currentProviderCodex?: string;
  // 当前 Gemini 供应商 ID（优先于数据库 is_current）
  currentProviderGemini?: string;

  // ===== Skill 同步设置 =====
  // Skill 同步方式：auto（默认，优先 symlink）、symlink、copy
  skillSyncMethod?: SkillSyncMethod;
  // Skill 存储位置：cc_switch（默认）或 unified（~/.agents/skills/）
  skillStorageLocation?: SkillStorageLocation;

  // ===== WebDAV v2 同步设置 =====
  webdavSync?: WebDavSyncSettings;

  // ===== S3 同步设置 =====
  s3Sync?: S3SyncSettings;

  // ===== 备份策略设置 =====
  // Auto-backup interval in hours (0=disabled, default 24)
  backupIntervalHours?: number;
  // Maximum backup files to retain (default 10)
  backupRetainCount?: number;

  // ===== 终端设置 =====
  // 首选终端应用（可选，默认使用系统默认终端）
  // macOS: "terminal" | "iterm2" | "warp" | "alacritty" | "kitty" | "ghostty" | "otty" | "wezterm" | "kaku"
  // Windows: "cmd" | "powershell" | "wt"
  // Linux: "gnome-terminal" | "konsole" | "xfce4-terminal" | "alacritty" | "kitty" | "ghostty"
  preferredTerminal?: string;

  // ===== 本机自动迁移状态 =====
  localMigrations?: {
    codexThirdPartyHistoryProviderBucketV1?: {
      completedAt: string;
      targetProviderId: string;
      sourceProviderIds?: string[];
      migratedJsonlFiles?: number;
      migratedStateRows?: number;
    };
  };
}

export interface SessionMeta {
  providerId: string;
  sessionId: string;
  title?: string;
  summary?: string;
  projectDir?: string | null;
  createdAt?: number;
  lastActiveAt?: number;
  sourcePath?: string;
  resumeCommand?: string;
}

// ─── 会话阅读页数据模型（对应 Rust session_manager/model.rs） ───

export type ToolKind =
  | "shell"
  | "read"
  | "search"
  | "edit"
  | "write"
  | "web"
  | "mcp"
  | "agent"
  | "ask"
  | "todo"
  | "other";

export type ToolStatus =
  "success" | "error" | "interrupted" | "pending" | "unknown";

export type EventKind =
  | "aborted"
  | "compaction"
  | "model_change"
  | "thinking_level"
  | "hook"
  | "pr_link"
  | "slash_command"
  | "info"
  | "error"
  | "sub_agent"
  | "other";

/** 大内容的「按需取」引用：前端不解析，原样回传给后端 */
export type ContentRef =
  | { kind: "jsonl"; offset: number; len: number; pointer: string }
  | { kind: "file"; relPath: string; pointer: string }
  | { kind: "sidecar"; relPath: string };

export type ImageSource =
  | { kind: "inline"; content: ContentRef }
  | { kind: "local_file"; path: string };

export interface ImageRef {
  source: ImageSource;
  mediaType: string;
  /** 解码后的字节数估算 */
  size: number;
  alt?: string;
}

export interface DiffFile {
  path: string;
  op: "add" | "update" | "delete" | "rename";
  added: number;
  removed: number;
}

export interface DiffSummary {
  files: DiffFile[];
  added: number;
  removed: number;
  full?: ContentRef;
}

export interface TextBlock {
  type: "text";
  /** 正文；带 full 时只是预览（超长注入文本） */
  text: string;
  full?: ContentRef;
}

export interface ThinkingBlock {
  type: "thinking";
  /** 可见正文的预览；可能为空（加密/只有签名） */
  text: string;
  summary?: string;
  redacted?: boolean;
  durationMs?: number;
  full?: ContentRef;
}

export interface ToolCallBlock {
  type: "tool_call";
  /** 与 ToolResultBlock.callId 配对 */
  id: string;
  rawName: string;
  kind: ToolKind;
  title: string;
  detail?: string;
  server?: string;
  inputPreview: string;
  inputTotalLen: number;
  inputFull?: ContentRef;
  diff?: DiffSummary;
  byUser?: boolean;
}

export interface ToolResultBlock {
  type: "tool_result";
  /** 为空串表示源数据没有配对信息，显示为通用「工具输出」 */
  callId: string;
  status: ToolStatus;
  preview: string;
  totalLen: number;
  lineCount: number;
  truncated: boolean;
  full?: ContentRef;
  exitCode?: number;
  durationMs?: number;
  images?: ImageRef[];
  savedPath?: string;
}

export interface ImageBlock {
  type: "image";
  image: ImageRef;
}

export interface EventBlock {
  type: "event";
  kind: EventKind;
  /** 说明；带 full 时只是预览（压缩摘要等长文本） */
  text?: string;
  url?: string;
  full?: ContentRef;
}

export type SessionBlock =
  | TextBlock
  | ThinkingBlock
  | ToolCallBlock
  | ToolResultBlock
  | ImageBlock
  | EventBlock;

export interface MessageMeta {
  model?: string;
  inputTokens?: number;
  outputTokens?: number;
  cacheReadTokens?: number;
  cacheWriteTokens?: number;
  reasoningTokens?: number;
  costUsd?: number;
  durationMs?: number;
  stopReason?: string;
}

export interface SessionMessage {
  role: string;
  /**
   * 旧格式的纯文本内容：只有 blocks 缺失时（旧后端）才下发，
   * 有 blocks 时前端一律从 blocks 推导（见 reader/turns.ts `messageText`）
   */
  content?: string;
  ts?: number;
  id?: string;
  turnId?: string;
  injected?: boolean;
  /** 旧后端 / 尚未迁移的解析器会省略 → 前端按 content 兜底 */
  blocks?: SessionBlock[];
  meta?: MessageMeta;
}

export interface TurnIndex {
  turnId: string;
  firstMessageIndex: number;
  lastMessageIndex: number;
  questionPreview: string;
  ts?: number;
  stepCount: number;
  errorCount: number;
  hasFinalReply: boolean;
  aborted: boolean;
}

export type TranscriptChunk =
  | {
      type: "header";
      total: number;
      turns: TurnIndex[];
      cached: boolean;
      parseMs: number;
    }
  | { type: "messages"; start: number; messages: SessionMessage[] }
  | { type: "done"; payloadBytes: number }
  | { type: "error"; message: string };

// MCP 服务器连接参数（宽松：允许扩展字段）
export interface McpServerSpec {
  // 可选：社区常见 .mcp.json 中 stdio 配置可不写 type
  type?: "stdio" | "http" | "sse";
  // stdio 字段
  command?: string;
  args?: string[];
  env?: Record<string, string>;
  cwd?: string;
  // http 和 sse 字段
  url?: string;
  headers?: Record<string, string>;
  // 通用字段
  [key: string]: any;
}

// v3.7.0: MCP 服务器应用启用状态
export interface McpApps {
  claude: boolean;
  "claude-desktop"?: boolean;
  codex: boolean;
  gemini: boolean;
  grokbuild?: boolean;
}

// MCP 服务器条目（v3.7.0 统一结构）
export interface McpServer {
  id: string;
  name: string;
  server: McpServerSpec;
  apps: McpApps; // v3.7.0: 标记应用到哪些客户端
  description?: string;
  tags?: string[];
  homepage?: string;
  docs?: string;
  // 兼容旧字段（v3.6.x 及以前）
  enabled?: boolean; // 已废弃，v3.7.0 使用 apps 字段
  source?: string;
  [key: string]: any;
}

// MCP 服务器映射（id -> McpServer）
export type McpServersMap = Record<string, McpServer>;

// MCP 配置状态
export interface McpStatus {
  userConfigPath: string;
  userConfigExists: boolean;
  serverCount: number;
}

// 新：来自 config.json 的 MCP 列表响应
export interface McpConfigResponse {
  configPath: string;
  servers: Record<string, McpServer>;
}

// ============================================================================
// 统一供应商（Universal Provider）- 跨应用共享配置
// ============================================================================

// 统一供应商的应用启用状态
export interface UniversalProviderApps {
  claude: boolean;
  codex: boolean;
  gemini: boolean;
  claudeDesktop: boolean;
  grokbuild: boolean;
}

// Claude 模型配置：模型名 + 各档显示名 + 1M 声明
export interface ClaudeModelConfig {
  model?: string;
  haikuModel?: string;
  sonnetModel?: string;
  opusModel?: string;
  fableModel?: string;
  /** 上游为 1M 窗口时，主模型写 `model[1M]`（和单应用表单同一个标记） */
  modelOneM?: boolean;
  haikuModelOneM?: boolean;
  sonnetModelOneM?: boolean;
  opusModelOneM?: boolean;
  fableModelOneM?: boolean;
  /** /model 菜单里的显示名（可选） */
  modelName?: string;
  haikuModelName?: string;
  sonnetModelName?: string;
  opusModelName?: string;
  fableModelName?: string;
  /** Claude Code 的 subagent 模型（`CLAUDE_CODE_SUBAGENT_MODEL`） */
  subagentModel?: string;
  subagentModelOneM?: boolean;
  /** 上游协议：anthropic（原生）/ openai_chat / openai_responses / gemini_native */
  apiFormat?: ClaudeApiFormat;
  /** 写入配置的认证环境变量名 */
  apiKeyField?: ClaudeApiKeyField;
  /** 地址是完整 API 端点（代理直接用，不拼路径） */
  isFullUrl?: boolean;
  /** 本地路由时替换发往上游的 User-Agent */
  customUserAgent?: string;
  /** 本地路由时对上游请求的 Header / Body 覆盖 */
  requestOverrides?: LocalProxyRequestOverrides;
  /** 上游支持 tool_reference 时打开按需加载工具定义（`ENABLE_TOOL_SEARCH`） */
  enableToolSearch?: boolean;
  /** 上游严格校验工具 schema 时不让 Artifact 进 tools（`CLAUDE_CODE_DISABLE_ARTIFACT`） */
  disableArtifact?: boolean;
  /** 上游不支持服务端 auto mode 检查时关掉（`CLAUDE_CODE_AUTO_MODE_SERVER=0`） */
  disableAutoModeServer?: boolean;
  /** 隐藏 AI 署名（live 顶层 `attribution`）：随这家供应商切换 */
  hideAttribution?: boolean;
  /** 禁用自动升级（`DISABLE_AUTOUPDATER`）：随这家供应商切换 */
  disableAutoUpgrade?: boolean;
}

// Claude Desktop 模型配置：接入方式 + 角色路由表
//
// 形状和后端 `meta.claudeDesktopMode` / `meta.claudeDesktopModelRoutes` 一一对应：
// 直连写进 Desktop profile 的 inferenceModels，模型映射经本地路由把三/四档角色
// 转成上游模型。`routes` 的 key 是 Desktop 认得的角色 ID（claude-sonnet-5 等），
// 直连时 model 与 key 同名，映射时 model 是实际请求的上游模型。
export interface ClaudeDesktopModelConfig {
  /** 接入方式：direct 直连（供应商自己就认 Claude 角色 ID）/ proxy 模型映射 */
  mode?: ClaudeDesktopMode;
  /** 模型映射模式下的上游格式 */
  apiFormat?: ClaudeApiFormat;
  /** 角色路由表；留空时用后端默认四档 */
  routes?: Record<string, ClaudeDesktopModelRoute>;
  /**
   * 旧版扁平字段（4.0.8 及更早的版本写过）。只读：编辑老供应商时迁移成 `routes`，
   * 保存一次后就不再有这些键。
   */
  sonnetModel?: string;
  opusModel?: string;
  haikuModel?: string;
  /** 旧版菜单显示名（三档共用） */
  labelOverride?: string;
  sonnetOneM?: boolean;
  opusOneM?: boolean;
  haikuOneM?: boolean;
}

// Codex 模型配置：模型 + 推理档位 + 上游协议 + 上下文窗口
export interface CodexModelConfig {
  model?: string;
  reasoningEffort?: string;
  /** 上游格式：openai_responses / openai_chat / anthropic */
  apiFormat?: CodexApiFormat;
  /** 走 Anthropic 路径时的认证字段名 */
  apiKeyField?: ClaudeApiKeyField;
  /** 地址是完整 API 端点 */
  isFullUrl?: boolean;
  /**
   * 压缩阈值：所选模型上下文窗口的百分比（1-100）。写 live 时按
   * `model_context_window × 百分比` 算出 `model_auto_compact_token_limit`；
   * 模型没有窗口元数据时整条不写（交给 Codex 默认）。
   */
  autoCompactPercent?: number;
  /** 远程压缩：把 `[model_providers.<id>].name` 写成 OpenAI */
  remoteCompaction?: boolean;
  /** 走 Chat Completions 时的思考能力声明 */
  chatReasoning?: CodexChatReasoning;
  /** 会话级提示词缓存路由（auto / enabled / disabled） */
  promptCacheRouting?: PromptCacheRoutingMode;
  /** 走 Anthropic 路径时模拟 Claude Code 客户端 */
  impersonateClaudeCode?: boolean;
  /** 走 Anthropic 路径时的 max_tokens 覆盖（>0） */
  maxOutputTokens?: number;
  /** 模型目录（/model 菜单里发布的模型） */
  catalogModels?: CodexCatalogModel[];
  /** 子代理模型：`config.toml` 的 `[agents].default_subagent_model` */
  subagentModel?: string;
  /** 子代理推理档位：`[agents].default_subagent_reasoning_effort` */
  subagentReasoningEffort?: string;
  /** 本地路由时的 User-Agent 与请求覆盖 */
  customUserAgent?: string;
  requestOverrides?: LocalProxyRequestOverrides;
}

// Gemini 模型配置
export interface GeminiModelConfig {
  model?: string;
  /** 任意 `settingsConfig.env` 键（.env 文件） */
  env?: Record<string, string>;
  /** 任意 `settingsConfig.config` 键（settings.json） */
  config?: Record<string, unknown>;
}

// Grok Build 模型配置
export interface GrokBuildModelConfig {
  /** 默认模型：[models].default，也是它指向的 [model."<id>"] 表名 */
  model?: string;
  /** 上游协议：responses / chat_completions / anthropic_messages */
  apiBackend?: string;
}

// 各应用的模型配置
export interface UniversalProviderModels {
  claude?: ClaudeModelConfig;
  codex?: CodexModelConfig;
  gemini?: GeminiModelConfig;
  claudeDesktop?: ClaudeDesktopModelConfig;
  grokbuild?: GrokBuildModelConfig;
}

/** 统一供应商的一条可用模型：`/v1/models` 的 id + oh-my-pi 模型目录的名称与上下文窗口。 */
export interface AvailableModel {
  id: string;
  displayName?: string;
  contextWindow?: number;
  /** effort 档位（模型目录 thinking.efforts 的并集），逐模型可改；空表示没声明。 */
  efforts?: string[];
  /** 默认 effort 档位（efforts 的子集）；不设则目录不写 defaultReasoningLevel。 */
  defaultEffort?: string;
  /** 最大输出 tokens（模型目录的 maxTokens），逐模型可改；空表示没声明。 */
  maxOutput?: number;
}

// 统一供应商（跨应用共享配置）
export interface UniversalProvider {
  id: string;
  name: string;
  apps: UniversalProviderApps;
  baseUrl: string;
  apiKey: string;
  models: UniversalProviderModels;
  /** 从 `/v1/models` 拉取、oh-my-pi 模型目录补全的可用模型列表；所有应用共用。 */
  availableModels?: AvailableModel[];
  websiteUrl?: string;
  notes?: string;
  icon?: string;
  iconColor?: string;
  meta?: ProviderMeta;
  createdAt?: number;
  sortIndex?: number;
}

// 统一供应商映射（id -> UniversalProvider）
export type UniversalProvidersMap = Record<string, UniversalProvider>;
