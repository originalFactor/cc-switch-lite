/**
 * 统一供应商的子卡 id 与它们的父级。
 *
 * 同步时后端把每个应用的一份配置写成 `universal-<app>-<父级 id>`（见
 * `provider.rs` 的 `to_*_provider`）。供应商页里看到的这些卡片都属于某个统一供应商，
 * 在任意应用页编辑它们要打开的是**父级**统一供应商的表单，而不是这一份子卡。
 */

import type { AppId } from "@/lib/api";
import type {
  AvailableModel,
  ClaudeDesktopMode,
  ClaudeDesktopModelRoute,
  CodexApiFormat,
  UniversalProviderModels,
} from "@/types";

/** 应用段全集。解析时按「最长优先」匹配，避免 `claude-` 吃掉 `claude-desktop-`。 */
const APP_IDS: readonly AppId[] = [
  "claude-desktop",
  "claude",
  "codex",
  "gemini",
  "grokbuild",
];

const CHILD_PREFIX = "universal-";

export interface UniversalChildId {
  /** 这个子卡属于哪个应用。 */
  app: AppId;
  /** 父级统一供应商 id。 */
  parentId: string;
}

/**
 * 解析子卡 id。返回 `undefined` 表示这不是统一供应商同步出来的卡片
 * （普通供应商、官方种子等）。
 *
 * 应用段先按最长匹配，所以 `universal-claude-desktop-x` 是 claude-desktop 的子卡，
 * 不会被当成 claude 的；父级 id 本身可能带连字符（迁移时沿用原 id），所以剥掉
 * 「前缀 + 应用段」后剩下的整体都是父级 id。
 */
export function parseUniversalChildId(
  providerId: string,
): UniversalChildId | undefined {
  if (!providerId.startsWith(CHILD_PREFIX)) return undefined;
  const rest = providerId.slice(CHILD_PREFIX.length);
  for (const app of APP_IDS) {
    const prefix = `${app}-`;
    if (rest.startsWith(prefix)) {
      const parentId = rest.slice(prefix.length);
      if (parentId.length > 0) return { app, parentId };
    }
  }
  return undefined;
}

/**
 * 这张卡是统一供应商同步出来的子卡时，返回父级统一供应商 id。
 *
 * 传 `appId` 时会校验子卡确实属于该应用：供应商列表只会列出本应用的卡片，传错应用
 * 时不应把它当成自己的子卡。
 */
export function universalParentIdOf(
  providerId: string,
  appId?: string,
): string | undefined {
  const parsed = parseUniversalChildId(providerId);
  if (!parsed) return undefined;
  if (appId !== undefined && parsed.app !== appId) return undefined;
  return parsed.parentId;
}

/** 这张卡是否由统一供应商同步出来（限定到某个应用时同时校验应用段）。 */
export function isUniversalChildId(
  providerId: string,
  appId?: string,
): boolean {
  return universalParentIdOf(providerId, appId) !== undefined;
}

/**
 * effort 档位全集（预置顺序即展示/写入顺序）。用户在统一供应商里勾选启用哪些；
 * 勾选集与各模型元数据（模型目录 thinking.efforts）求交集后写进目录的
 * `reasoningLevels`，并约束 effort 下拉的选项。
 */
export const EFFORT_PRESETS = [
  "none",
  "minimal",
  "low",
  "medium",
  "high",
  "xhigh",
  "max",
  "ultra",
] as const;

/**
 * 压缩阈值：把「上下文窗口的百分比」换算成 Codex 的绝对 token 数（整数除法，和
 * 后端 `to_codex_provider` 同规则）。窗口或百分比缺一就算不出来，返回 `undefined`
 * ——那种情况整条 `model_auto_compact_token_limit` 不写，交给 Codex 自己的默认。
 */
export function compactLimitFrom(
  contextWindow: number | undefined,
  percent: number | undefined,
): number | undefined {
  if (!contextWindow || !percent || percent <= 0) return undefined;
  return Math.floor((contextWindow * percent) / 100);
}

/**
 * effort 下拉的选项：模型行声明的档位（模型目录 thinking.efforts 的并集，拉取时补全、
 * 模型列表编辑器里可改）与预置全集求交集；该行没声明档位时退回预置全集。
 *
 * 已存的值不在结果里时（旧记录的手填值，或模型撤下了这个档）补到末尾，免得下拉因为
 * 找不到对应项而显示成空白——那样看起来像配置丢了。
 */
export function effortOptionsFor(
  availableModels: AvailableModel[],
  modelId: string | undefined,
  current?: string,
): string[] {
  const id = modelId?.trim();
  const efforts = id
    ? (availableModels.find((model) => model.id === id)?.efforts ?? [])
    : [];
  const options: string[] =
    efforts.length > 0
      ? EFFORT_PRESETS.filter((preset) => efforts.includes(preset))
      : [...EFFORT_PRESETS];
  const value = current?.trim();
  return value && !options.includes(value) ? [...options, value] : options;
}

/** Claude Desktop 映射模式的四个角色档位（与表单 DESKTOP_ROLE_ROUTES 同序）。 */
export const DESKTOP_QUICK_ROLES = [
  "claude-sonnet-5",
  "claude-opus-5",
  "claude-fable-5",
  "claude-haiku-4-5",
] as const;

/** 表单里的 Claude 四档模型字段（模型字段名 → 归一化槽位键）。 */
const CLAUDE_TIERS = [
  ["sonnetModel", "sonnet"],
  ["opusModel", "opus"],
  ["fableModel", "fable"],
  ["haikuModel", "haiku"],
] as const;

export type QuickModelSlot = "sonnet" | "opus" | "fable" | "haiku" | "default";

/**
 * 快速配置的 API 类型（用户视角的四种）。各应用字段的值集不同，这里归一成一份，
 * 写的时候按下面的映射转成各应用自己的值；应用不支持的组合直接跳过（不报错）。
 */
export type QuickApiFormat =
  "anthropic" | "openai_chat" | "openai_responses" | "gemini_native";

/** 快速配置 API 类型 → 各应用字段值。Claude 系两个字段同名同值。 */
const QUICK_API_FORMAT_TO_CODEX: Record<
  QuickApiFormat,
  CodexApiFormat | undefined
> = {
  anthropic: "anthropic",
  openai_chat: "openai_chat",
  openai_responses: "openai_responses",
  gemini_native: undefined, // Codex 不认 Gemini 上游
};

const QUICK_API_FORMAT_TO_GROK: Record<QuickApiFormat, string | undefined> = {
  anthropic: "anthropic_messages",
  openai_chat: "chat_completions",
  openai_responses: "responses",
  gemini_native: undefined,
};

export interface QuickModelInput {
  /** 表单当前各应用的模型配置（不会改动传入对象）。 */
  models: UniversalProviderModels;
  /** 所有档位一键设为这个模型 id（可用模型列表里的 id）。 */
  modelId: string;
  /**
   * Claude 四档各自的覆盖。没列出的槽位缺省都填所选模型；列出来的槽位：
   * 非空串 = 覆盖成那个模型，`""` / `undefined` = 清空该档（不填）。
   */
  slots?: Partial<Record<QuickModelSlot, string | undefined>>;
  /** Claude Desktop 接入方式：映射模式下填四档路由，直连模式不动（后端自己生成）。 */
  desktopMode?: ClaudeDesktopMode;
  /** Codex 的 reasoning effort（所选模型在列表行声明过档位时才有意义）。 */
  reasoningEffort?: string | undefined;
  /** Codex 子代理的 reasoning effort；缺省不写（子代理跟主模型同档位才需要）。 */
  subagentReasoningEffort?: string | undefined;
  /**
   * API 类型：一键写进所有支持该类型的应用（Claude Code / Desktop / Codex /
   * Grok Build）。不支持的组合（如 Gemini 上游对 Codex）跳过；不传 = 不动任何
   * apiFormat / apiBackend。
   */
  apiFormat?: QuickApiFormat;
}

/**
 * 一键把选中的模型填进所有应用的所有模型槽位：Claude Code 四档 + 主模型 + 子代理、
 * Claude Desktop 四档路由、Codex 主模型 + 子代理、Gemini、Grok Build。
 *
 * 规则：
 * - 主模型（`default`）缺省跟随所选模型；`slots.default = ""` 时不写主模型。
 * - Claude 四档缺省都填同一个模型，逐档可覆盖；填过的开关（思考能力等）不动。
 * - Desktop 只在映射模式下填路由（直连模式的路由由后端从可用模型列表生成）。
 * - Codex 的 effort 只在明确传了非空值时写；没声明档位的模型不写 effort。
 * - 只改模型相关字段，不改 apiFormat / 请求覆盖 / 思考开关等其余设置。
 */
export function applyQuickModel(
  input: QuickModelInput,
): UniversalProviderModels {
  const { models, modelId } = input;
  const id = modelId.trim();
  if (id === "") return models;

  const slotValue = (slot: QuickModelSlot): string | undefined => {
    if (!input.slots) return id;
    if (!(slot in input.slots)) return id;
    const override = input.slots[slot];
    // 在 slots 里点名的档：非空串 = 覆盖成该模型；"" 或 undefined = 清空该档。
    // 没点名的档（键不存在）跟着缺省走：四档全填所选模型。
    return override?.trim() || undefined;
  };

  const next: UniversalProviderModels = { ...models };

  // Claude Code：四档 + 主模型 + 子代理。缺省四档都跟随所选模型，逐档可覆盖。
  const claude = { ...(next.claude ?? {}) };
  for (const [field, slot] of CLAUDE_TIERS) {
    const value = slotValue(slot);
    if (value) claude[field] = value;
    else delete claude[field];
  }
  const mainValue = slotValue("default");
  if (mainValue) claude.model = mainValue;
  else delete claude.model;
  // 子代理跟主模型走：一家供应商通常一个主力模型跑子代理；主模型都清掉时子代理也清。
  if (mainValue) claude.subagentModel = mainValue;
  else delete claude.subagentModel;
  next.claude = claude;

  // Claude Desktop：只有映射模式有可填的路由表；直连模式不碰（后端按可用模型列表生成）。
  if (input.desktopMode === "proxy") {
    const routes: Record<string, ClaudeDesktopModelRoute> = {};
    for (const role of DESKTOP_QUICK_ROLES) {
      routes[role] = { model: id };
    }
    next.claudeDesktop = {
      ...(next.claudeDesktop ?? {}),
      mode: "proxy",
      routes,
    };
  }

  // Codex：主模型 + 子代理模型；effort 只在传了非空值时写。
  const codex = { ...(next.codex ?? {}) };
  codex.model = id;
  codex.subagentModel = id;
  const effort = input.reasoningEffort?.trim();
  if (effort) codex.reasoningEffort = effort;
  else delete codex.reasoningEffort;
  const subEffort = input.subagentReasoningEffort?.trim();
  if (subEffort) codex.subagentReasoningEffort = subEffort;
  else delete codex.subagentReasoningEffort;
  next.codex = codex;

  // Gemini CLI / Grok Build：各一个模型槽。
  next.gemini = { ...(next.gemini ?? {}), model: id };
  next.grokbuild = { ...(next.grokbuild ?? {}), model: id };

  // API 类型：一键写进所有支持的应用。不支持的组合跳过（Gemini 上游对 Codex /
  // Grok 无对应值，Gemini CLI 本身只走原生协议、没有这个字段）。
  if (input.apiFormat) {
    const fmt = input.apiFormat;
    next.claude = { ...next.claude, apiFormat: fmt };
    if (input.desktopMode === "proxy") {
      next.claudeDesktop = {
        ...(next.claudeDesktop ?? {}),
        apiFormat: fmt,
      };
    }
    const codexFmt = QUICK_API_FORMAT_TO_CODEX[fmt];
    if (codexFmt) {
      next.codex = { ...next.codex, apiFormat: codexFmt };
    }
    const grokFmt = QUICK_API_FORMAT_TO_GROK[fmt];
    if (grokFmt) {
      next.grokbuild = { ...next.grokbuild, apiBackend: grokFmt };
    }
  }

  return next;
}
