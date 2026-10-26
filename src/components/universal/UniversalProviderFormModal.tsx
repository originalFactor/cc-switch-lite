import type * as React from "react";
import {
  useState,
  useEffect,
  useLayoutEffect,
  useCallback,
  useMemo,
} from "react";
import { useDarkMode } from "@/hooks/useDarkMode";
import { useTranslation } from "react-i18next";
import {
  ArrowLeft,
  ChevronRight,
  Eye,
  EyeOff,
  RefreshCw,
  Trash2,
  Zap,
} from "lucide-react";
import { Button } from "@/components/ui/button";
import { HoverTip } from "@/components/ui/hover-tip";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Textarea } from "@/components/ui/textarea";
import {
  Dialog,
  DialogContent,
  DialogTrigger,
  DialogClose,
} from "@/components/ui/dialog";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { FullScreenPanel } from "@/components/common/FullScreenPanel";
import { ConfirmDialog } from "@/components/ConfirmDialog";
import {
  Collapsible,
  CollapsibleContent,
  CollapsibleTrigger,
} from "@/components/ui/collapsible";
import { ProviderIcon } from "@/components/ProviderIcon";
import { ProviderIconBox } from "@/components/ProviderIconBox";
import { IconPicker } from "@/components/IconPicker";
import { getIconMetadata } from "@/icons/extracted/metadata";
import EndpointSpeedTest from "@/components/providers/forms/EndpointSpeedTest";
import JsonEditor from "@/components/JsonEditor";
import { ModelSelect } from "./ModelSelect";
import { AvailableModelsEditor } from "./AvailableModelsEditor";
import { universalProvidersApi } from "@/lib/api";
import {
  applyQuickModel,
  compactLimitFrom,
  DESKTOP_QUICK_ROLES,
  effortOptionsFor,
  type QuickApiFormat,
} from "@/utils/universalProvider";
import {
  GROK_BUILD_DEFAULT_API_BACKEND,
  GROK_BUILD_DEFAULT_CONTEXT_WINDOW,
  GROK_BUILD_DEFAULT_MODEL,
} from "@/utils/grokBuildConfig";
import { toast } from "@/lib/toast";
import { extractErrorMessage } from "@/utils/errorUtils";
import type {
  ClaudeDesktopMode,
  ClaudeDesktopModelConfig,
  ClaudeDesktopModelRoute,
  GeminiModelConfig,
  LocalProxyRequestOverrides,
  ProviderMeta,
  AvailableModel,
  UniversalProvider,
  UniversalProviderModels,
} from "@/types";

export interface UniversalProviderFormModalProps {
  isOpen: boolean;
  onClose: () => void;
  onSave: (provider: UniversalProvider) => void;
  onSaveAndSync?: (provider: UniversalProvider) => void;
  editingProvider?: UniversalProvider | null;
  /**
   * 编辑时的删除动作。传了才显示「删除」：新建没有可删的东西。
   *
   * 页脚由本表单自己管（外层全屏面板盖住它那层的页脚，按钮点不到），所以删除放在
   * 这里而不是列表卡片上。
   */
  onDelete?: (provider: UniversalProvider) => void;
  /** 面板副标题：添加页会把当前应用拼进去。 */
  subtitle?: string;
  /** 页头右上角的动作（添加页的「管理」）。 */
  manageAction?: React.ReactNode;
}

const EMPTY_OVERRIDES = { headers: "", body: "" };

/** Radix Select 不接受空 value：子代理推理档位的「不设置」用哨兵值映射成 undefined。 */
const SUBAGENT_EFFORT_UNSET = "__unset__";

/** Claude Desktop 的角色档位，与后端 DEFAULT_PROXY_ROUTES / 单应用表单同 id。 */
const DESKTOP_ROLE_ROUTES: { id: string; label: string }[] = [
  { id: "claude-sonnet-5", label: "Sonnet" },
  { id: "claude-opus-5", label: "Opus" },
  { id: "claude-fable-5", label: "Fable" },
  { id: "claude-haiku-4-5", label: "Haiku" },
];

type DesktopRouteRow = {
  routeId: string;
  model: string;
};

/** 默认四档（模型映射模式）：档位固定，上游模型留空由用户填。 */
const defaultDesktopRouteRows = (): DesktopRouteRow[] =>
  DESKTOP_ROLE_ROUTES.map(({ id }) => ({
    routeId: id,
    model: "",
  }));

/** 预设/已存的路由表 → 可编辑行；没有就用固定四档。 */
const desktopRowsOrDefault = (
  config: ClaudeDesktopModelConfig | undefined,
): DesktopRouteRow[] => {
  const rows = desktopRouteRowsFrom(desktopRoutesOf(config));
  return rows.length > 0 ? rows : defaultDesktopRouteRows();
};

/**
 * 取路由表。新形状优先；旧版（4.0.8 及更早）只存了三个扁平档位，这里迁移成三档
 * 角色路由，编辑老供应商时不会看起来「什么都没配」。旧版共用的菜单显示名不再搬：
 * 显示名已改为按所选模型的元数据自动生成（见 `desktopMenuLabel`）。
 */
const desktopRoutesOf = (
  config: ClaudeDesktopModelConfig | undefined,
): Record<string, ClaudeDesktopModelRoute> | undefined => {
  if (!config) return undefined;
  if (config.routes && Object.keys(config.routes).length > 0) {
    return config.routes;
  }
  const legacy: Record<string, ClaudeDesktopModelRoute> = {};
  const add = (routeId: string, model?: string) => {
    if (!model?.trim()) return;
    legacy[routeId] = { model: model.trim() };
  };
  add("claude-sonnet-5", config.sonnetModel);
  add("claude-opus-5", config.opusModel);
  add("claude-haiku-4-5", config.haikuModel);
  return Object.keys(legacy).length > 0 ? legacy : undefined;
};

/** 把存下来的路由表还原成可编辑的行。 */
const desktopRouteRowsFrom = (
  routes: Record<string, ClaudeDesktopModelRoute> | undefined,
): DesktopRouteRow[] =>
  Object.entries(routes ?? {}).map(([routeId, route]) => ({
    routeId,
    model: route.model ?? "",
  }));

/**
 * 某一档在 Claude Desktop 菜单里的显示名：所选模型在可用模型列表里的 displayName
 * （模型目录的名称）。没有对应模型或该行没有显示名时不写 labelOverride，由
 * Desktop 自己回退成模型名。
 */
const desktopMenuLabel = (
  availableModels: AvailableModel[],
  modelId: string,
): string | undefined =>
  availableModels
    .find((model) => model.id === modelId.trim())
    ?.displayName?.trim() || undefined;

/**
 * 角色档位的显示名。模型映射模式下档位就是固定的四个角色；直连模式的 route 是
 * 用户自己填的模型名，原样显示。
 */
const desktopRoleLabel = (routeId: string) =>
  DESKTOP_ROLE_ROUTES.find(({ id }) => id === routeId)?.label ?? routeId;

/**
 * Claude Desktop 直连只认 `claude-sonnet-* / claude-opus-* / claude-haiku-* /
 * claude-fable-*`（或 `anthropic/claude-…`）这些名字。与后端
 * `claude_desktop_config::is_claude_safe_model_id` 同一条规则。
 */
const isClaudeSafeModelId = (model: string) => {
  const normalized = model.trim().toLowerCase();
  const tail = normalized.startsWith("anthropic/claude-")
    ? normalized.slice("anthropic/claude-".length)
    : (normalized.match(/^claude-(.+)$/)?.[1] ?? null);
  if (tail === null) return false;
  return ["sonnet-", "opus-", "haiku-", "fable-"].some((prefix) => {
    if (!tail.startsWith(prefix)) return false;
    return tail.slice(prefix.length).length > 0;
  });
};

/** 把存下来的请求覆盖还原成两个文本框里的文本 */
const overridesToText = (
  models: { requestOverrides?: LocalProxyRequestOverrides } | undefined,
): { headers: string; body: string } => ({
  headers: models?.requestOverrides?.headers
    ? JSON.stringify(models.requestOverrides.headers, null, 2)
    : "",
  body: models?.requestOverrides?.body
    ? JSON.stringify(models.requestOverrides.body, null, 2)
    : "",
});

/** 应用区块的展开状态：默认全收起，只看应用名和摘要；点标题展开。 */
type AppSectionKey =
  "claude" | "codex" | "gemini" | "claudeDesktop" | "grokbuild";

/**
 * 应用区块的标题行：图标 + 应用名 + 已配置摘要，点击整行展开/收起。
 *
 * 折叠是这里的主要布局手段——每个应用的字段都很多，全展开时一屏只看得下一张卡。
 * 摘要让收起时也能一眼看出「这个应用配了什么」。
 */
function AppSection({
  icon,
  name,
  summary,
  open,
  onOpenChange,
  children,
}: {
  icon: string;
  name: string;
  /** 收起时标题右侧的一行摘要；空数组就不显示。 */
  summary?: string[];
  open: boolean;
  onOpenChange: (open: boolean) => void;
  children: React.ReactNode;
}) {
  return (
    <Collapsible open={open} onOpenChange={onOpenChange}>
      <div className="rounded-lg border">
        <CollapsibleTrigger asChild>
          <button
            type="button"
            aria-expanded={open}
            className="flex w-full items-center gap-2 px-3 py-2.5 text-left"
          >
            <ChevronRight
              className={`h-4 w-4 shrink-0 text-fg-3 transition-transform ${
                open ? "rotate-90" : ""
              }`}
            />
            <ProviderIcon icon={icon} name={name} size={20} />
            <span className="font-medium">{name}</span>
            {summary && summary.length > 0 && (
              <span className="min-w-0 flex-1 truncate text-right text-xs text-fg-3">
                {summary.join(" · ")}
              </span>
            )}
          </button>
        </CollapsibleTrigger>
        <CollapsibleContent>
          <div className="border-t px-3 pb-3 pt-3">{children}</div>
        </CollapsibleContent>
      </div>
    </Collapsible>
  );
}

/** 一行内联的开关组（勾选框 + 文案 + 可选帮助问号）。 */
function CheckPill({
  checked,
  onChange,
  label,
  help,
}: {
  checked: boolean;
  onChange: (checked: boolean) => void;
  label: string;
  help?: string;
}) {
  return (
    <span className="inline-flex items-center gap-1">
      <label className="inline-flex cursor-pointer items-center gap-1.5 text-xs text-fg-2">
        <input
          type="checkbox"
          className="ui-checkbox"
          checked={checked}
          onChange={(e) => onChange(e.target.checked)}
        />
        {label}
      </label>
      {help && (
        <HoverTip content={help}>
          <span className="text-fg-3">?</span>
        </HoverTip>
      )}
    </span>
  );
}

export function UniversalProviderFormModal({
  isOpen,
  onClose,
  onSave,
  onSaveAndSync,
  editingProvider,
  onDelete,
  subtitle,
  manageAction,
}: UniversalProviderFormModalProps) {
  const isDarkMode = useDarkMode();
  const { t } = useTranslation();
  const isEditMode = !!editingProvider;

  // 表单状态
  const [name, setName] = useState("");
  const [baseUrl, setBaseUrl] = useState("");
  const [apiKey, setApiKey] = useState("");
  const [showApiKey, setShowApiKey] = useState(false);
  const [websiteUrl, setWebsiteUrl] = useState("");
  const [notes, setNotes] = useState("");

  const [icon, setIcon] = useState("");
  const [iconColor, setIconColor] = useState("");
  const [iconDialogOpen, setIconDialogOpen] = useState(false);
  const [isEndpointModalOpen, setIsEndpointModalOpen] = useState(false);
  const [endpointAutoSelect, setEndpointAutoSelect] = useState(true);
  const [draftCustomEndpoints, setDraftCustomEndpoints] = useState<string[]>(
    [],
  );
  // Gemini 的 .env / settings.json 块：输入过程中大多不是合法 JSON，所以按文本保存，
  // 只有保存时才解析（和单应用表单把编辑器内容镜像成本地状态是同一个道理）。
  const [geminiEnvText, setGeminiEnvText] = useState("{}");
  const [geminiConfigText, setGeminiConfigText] = useState("{}");
  // Claude / Codex 的本地代理请求覆盖同样是「按文本输入、保存时解析」。
  const [claudeOverrides, setClaudeOverrides] = useState({
    headers: "",
    body: "",
  });
  const [codexOverrides, setCodexOverrides] = useState({
    headers: "",
    body: "",
  });

  // 模型配置
  const [models, setModels] = useState<UniversalProviderModels>({});
  /** 从 `/v1/models` 拉到的可用模型列表（oh-my-pi 模型目录补全过）；所有应用共用。 */
  const [availableModels, setAvailableModels] = useState<AvailableModel[]>([]);
  /** 上次拉取的结果，用于「恢复」：用户手改过之后还能还原。 */
  const [fetchedModels, setFetchedModels] = useState<AvailableModel[]>([]);
  const [modelsLoading, setModelsLoading] = useState(false);

  // Claude Desktop：接入方式和路由表单独放，切换接入方式时清空重填
  const [desktopMode, setDesktopMode] = useState<ClaudeDesktopMode>("proxy");
  const [desktopRoutes, setDesktopRoutes] = useState<DesktopRouteRow[]>([]);

  // 保存并同步确认弹窗
  const [syncConfirmOpen, setSyncConfirmOpen] = useState(false);
  const [pendingProvider, setPendingProvider] =
    useState<UniversalProvider | null>(null);

  // 应用区块的展开状态：默认全收起（编辑时展开有配置内容的应用，见下面的 effect）。
  const [openSections, setOpenSections] = useState<
    Record<AppSectionKey, boolean>
  >({
    claude: false,
    codex: false,
    gemini: false,
    claudeDesktop: false,
    grokbuild: false,
  });
  const toggleSection = useCallback((key: AppSectionKey, open: boolean) => {
    setOpenSections((prev) => ({ ...prev, [key]: open }));
  }, []);

  // Populate before paint so opening the panel never shows empty/stale fields.
  useLayoutEffect(() => {
    if (editingProvider) {
      // 编辑模式：加载现有数据
      setName(editingProvider.name);
      setBaseUrl(editingProvider.baseUrl);
      setApiKey(editingProvider.apiKey);
      setWebsiteUrl(editingProvider.websiteUrl || "");
      setNotes(editingProvider.notes || "");
      setIcon(editingProvider.icon || "");
      setIconColor(editingProvider.iconColor || "");
      setDesktopMode(editingProvider.models?.claudeDesktop?.mode ?? "proxy");
      setDesktopRoutes(
        desktopRowsOrDefault(editingProvider.models?.claudeDesktop),
      );
      setModels(editingProvider.models || {});
      setGeminiEnvText(
        JSON.stringify(editingProvider.models?.gemini?.env ?? {}, null, 2),
      );
      setGeminiConfigText(
        JSON.stringify(editingProvider.models?.gemini?.config ?? {}, null, 2),
      );
      setClaudeOverrides(overridesToText(editingProvider.models?.claude));
      setCodexOverrides(overridesToText(editingProvider.models?.codex));
      setDraftCustomEndpoints(
        Object.keys(editingProvider.meta?.custom_endpoints ?? {}),
      );
      setEndpointAutoSelect(editingProvider.meta?.endpointAutoSelect ?? true);
      setAvailableModels(editingProvider.availableModels ?? []);
      setFetchedModels(editingProvider.availableModels ?? []);
    } else {
      // 新建模式：留空表单，由用户自己填。不再有预设，也就没有预填的模型和类型；
      // 五个应用默认全开，模型留空时后端用各应用的默认值。
      setName("");
      setNotes("");
      setIcon("");
      setIconColor("");
      setApiKey("");
      setWebsiteUrl("");
      setModels({});
      setDesktopMode("proxy");
      setDesktopRoutes(desktopRowsOrDefault(undefined));
      setGeminiEnvText("{}");
      setGeminiConfigText("{}");
      setClaudeOverrides(EMPTY_OVERRIDES);
      setCodexOverrides(EMPTY_OVERRIDES);
      setDraftCustomEndpoints([]);
      setEndpointAutoSelect(true);
      setAvailableModels([]);
      setFetchedModels([]);
    }
  }, [editingProvider, isOpen]);

  // 编辑时把「有实际配置」的应用区块自动展开，新建时全收起。
  useEffect(() => {
    if (!editingProvider) {
      setOpenSections({
        claude: false,
        codex: false,
        gemini: false,
        claudeDesktop: false,
        grokbuild: false,
      });
      return;
    }
    const m = editingProvider.models ?? {};
    setOpenSections({
      claude: Object.keys(m.claude ?? {}).length > 0,
      codex: Object.keys(m.codex ?? {}).length > 0,
      gemini: Object.keys(m.gemini ?? {}).length > 0,
      claudeDesktop: Object.keys(m.claudeDesktop ?? {}).length > 0,
      grokbuild: Object.keys(m.grokbuild ?? {}).length > 0,
    });
  }, [editingProvider]);

  // Codex effort 下拉的选项：跟 Codex 卡所选模型那一行的档位（拉取时由
  // 模型目录补全，模型列表编辑器里可改）；该行没声明时退回预置全集。
  const codexEffortOptions = useMemo(
    () =>
      effortOptionsFor(
        availableModels,
        models.codex?.model,
        models.codex?.reasoningEffort,
      ),
    [models.codex?.model, models.codex?.reasoningEffort, availableModels],
  );

  // 子代理推理档位跟子代理模型那一行：没选子代理模型时 Codex 用主模型跑子代理，
  // 于是退回主模型的档位（和上面那个下拉同一份元数据规则）。
  const subagentEffortOptions = useMemo(
    () =>
      effortOptionsFor(
        availableModels,
        models.codex?.subagentModel || models.codex?.model,
        models.codex?.subagentReasoningEffort,
      ),
    [
      models.codex?.subagentModel,
      models.codex?.model,
      models.codex?.subagentReasoningEffort,
      availableModels,
    ],
  );

  // Codex 压缩阈值的实际写入值：所选模型行的上下文窗口 × 输入的百分比。没窗口元数据
  // 或没填百分比时是 undefined（整条不写）。预览与输入框下的提示都读它。
  const compactLimit = useMemo(
    () =>
      compactLimitFrom(
        availableModels.find((row) => row.id === models.codex?.model?.trim())
          ?.contextWindow,
        models.codex?.autoCompactPercent,
      ),
    [availableModels, models.codex?.model, models.codex?.autoCompactPercent],
  );

  // Codex 的上下文窗口 / 最大输出自动值：跟 Codex 卡所选模型行的元数据。
  // 字段留空 = 用自动值（保存时后端同规则兜底）；手填值优先。
  const sectionSummaries = useMemo<Record<AppSectionKey, string[]>>(() => {
    const pick = (...parts: (string | undefined | false)[]) =>
      parts.filter((part): part is string => Boolean(part)).slice(0, 3);
    const t_or = (key: string, fallback: string) =>
      t(key, { defaultValue: fallback });
    return {
      claude: pick(
        models.claude?.model,
        models.claude?.apiFormat && models.claude?.apiFormat !== "anthropic"
          ? t_or("universalProvider.summaryNonDefaultFormat", "自定义上游格式")
          : undefined,
        models.claude?.customUserAgent,
      ),
      codex: pick(models.codex?.model, models.codex?.reasoningEffort),
      gemini: pick(models.gemini?.model),
      claudeDesktop: pick(
        desktopMode === "direct"
          ? t_or("claudeDesktop.modelModeDirect", "直连")
          : t_or("claudeDesktop.modelModeProxy", "模型映射"),
        desktopMode === "proxy"
          ? (Object.values(desktopRoutes).find((row) => row.model.trim())
              ?.model ?? undefined)
          : undefined,
      ),
      grokbuild: pick(models.grokbuild?.model),
    };
  }, [models, desktopMode, desktopRoutes, t]);

  // 选择图标：同时带上图标自带的默认颜色（和单应用供应商表单一致）
  const handleIconSelect = useCallback((nextIcon: string) => {
    const meta = getIconMetadata(nextIcon);
    setIcon(nextIcon);
    setIconColor(meta?.defaultColor ?? "");
    // 单选：点了就是选好了，不用再找「完成」
    setIconDialogOpen(false);
  }, []);

  // 更新某个应用的配置。值可以是字符串（文本框）、布尔（勾选）或数字（阈值），
  // 空字符串和未勾选都表示「没配」，清掉这个键而不是留个空值。
  const updateModel = useCallback(
    (
      app: "claude" | "codex" | "gemini" | "claudeDesktop" | "grokbuild",
      field: string,
      value: string | boolean | number,
    ) => {
      setModels((prev) => {
        const next = { ...(prev[app] || {}) } as Record<string, unknown>;
        const cleared =
          value === "" ||
          value === false ||
          (typeof value === "number" && Number.isNaN(value));
        if (cleared) {
          delete next[field];
        } else {
          next[field] =
            typeof value === "string" && /^\d+$/.test(value)
              ? Number(value)
              : value;
        }
        return { ...prev, [app]: next };
      });
    },
    [],
  );

  // Claude Desktop 的路由行：改哪一行就只动那一行。
  const updateDesktopRoute = useCallback(
    (index: number, patch: Partial<DesktopRouteRow>) => {
      setDesktopRoutes((prev) =>
        prev.map((row, i) => (i === index ? { ...row, ...patch } : row)),
      );
    },
    [],
  );

  // 思考能力：只有勾上的项才写，两个都关掉时整块清掉。
  const updateChatReasoning = useCallback(
    (field: "supportsThinking" | "supportsEffort", checked: boolean) => {
      setModels((prev) => {
        const current = { ...(prev.codex?.chatReasoning ?? {}) };
        if (checked) {
          current[field] = true;
          // 和单应用表单一样的默认参数名
          if (field === "supportsThinking" && !current.thinkingParam) {
            current.thinkingParam = "thinking";
          }
          if (field === "supportsEffort") {
            current.effortParam ??= "reasoning_effort";
            current.effortValueMode ??= "passthrough";
            current.outputFormat ??= "auto";
          }
        } else {
          delete current[field];
        }
        const next = { ...(prev.codex ?? {}) };
        if (Object.keys(current).length > 0) {
          next.chatReasoning = current;
        } else {
          delete next.chatReasoning;
        }
        return { ...prev, codex: next };
      });
    },
    [],
  );

  // 快速配置：一键把选中的模型写进所有应用的所有模型槽位（四档 + 主模型 +
  // 子代理 + Desktop 路由 + Codex/Gemini/Grok Build）。纯函数在 utils 里，
  // 这里只负责接上表单状态。
  const handleQuickApply = useCallback(
    (modelId: string, reasoningEffort?: string, apiFormat?: string) => {
      // 主模型与子代理同档位：快速配置里 effort 是整家供应商的默认档，两个都写。
      const next = applyQuickModel({
        models: models,
        modelId,
        reasoningEffort,
        subagentReasoningEffort: reasoningEffort,
        apiFormat: apiFormat as QuickApiFormat | undefined,
        desktopMode,
      });
      setModels(next);
      // Desktop 的路由行是表单自己的可编辑状态，buildProvider 保存时从它重建
      // 路由表：快速配置写进去的 routes 必须同步成行，否则保存时被空行顶掉。
      if (desktopMode === "proxy") {
        setDesktopRoutes(
          DESKTOP_QUICK_ROLES.map((routeId) => ({ routeId, model: modelId })),
        );
      }
      toast.success(
        t("universalProvider.quickConfigApplied", {
          defaultValue: `已将 ${modelId} 应用到所有应用`,
          model: modelId,
        }),
      );
    },
    [desktopMode, models, t],
  );

  // 计算 Claude 配置 JSON 预览
  const claudeConfigJson = useMemo(() => {
    const m = models.claude ?? {};
    const withOneM = (value: string | undefined, flag: boolean | undefined) =>
      value && flag ? `${value.trimEnd()}[1M]` : value;
    const model = m.model || "claude-sonnet-4-20250514";
    const haiku = m.haikuModel || "claude-haiku-4-20250514";
    const sonnet = m.sonnetModel || "claude-sonnet-4-20250514";
    const opus = m.opusModel || "claude-opus-4-20250514";
    const fable = m.fableModel || "claude-fable-4-20250514";
    const env: Record<string, string> = {
      ANTHROPIC_BASE_URL: baseUrl,
      [m.apiKeyField === "ANTHROPIC_API_KEY"
        ? "ANTHROPIC_API_KEY"
        : "ANTHROPIC_AUTH_TOKEN"]: apiKey,
      ANTHROPIC_MODEL: withOneM(model, m.modelOneM)!,
      ANTHROPIC_DEFAULT_HAIKU_MODEL: withOneM(haiku, m.haikuModelOneM)!,
      ANTHROPIC_DEFAULT_SONNET_MODEL: withOneM(sonnet, m.sonnetModelOneM)!,
      ANTHROPIC_DEFAULT_OPUS_MODEL: withOneM(opus, m.opusModelOneM)!,
      ANTHROPIC_DEFAULT_FABLE_MODEL: withOneM(fable, m.fableModelOneM)!,
    };
    if (m.haikuModelName)
      env.ANTHROPIC_DEFAULT_HAIKU_MODEL_NAME = m.haikuModelName;
    if (m.sonnetModelName)
      env.ANTHROPIC_DEFAULT_SONNET_MODEL_NAME = m.sonnetModelName;
    if (m.opusModelName)
      env.ANTHROPIC_DEFAULT_OPUS_MODEL_NAME = m.opusModelName;
    if (m.fableModelName)
      env.ANTHROPIC_DEFAULT_FABLE_MODEL_NAME = m.fableModelName;
    if (m.subagentModel) {
      env.CLAUDE_CODE_SUBAGENT_MODEL = withOneM(
        m.subagentModel,
        m.subagentModelOneM,
      )!;
    }
    return { env };
  }, [baseUrl, apiKey, models.claude]);

  // 计算 Codex 配置预览：与后端 `to_codex_provider` 生成的子卡行同形（auth.json +
  // config.toml + 模型目录），不要在预览里发明后端不会写的东西。
  const codexConfigJson = useMemo(() => {
    const m = models.codex ?? {};
    const model = m.model || "gpt-4o";
    const reasoningEffort = m.reasoningEffort || "high";
    // 与后端同一条 base_url 规则：以 /v1 结尾照抄；纯 origin（没有路径）补 /v1；
    // 其余（带自定义路径）照抄，不强加版本前缀。
    const baseTrimmed = baseUrl.trim().replace(/\/+$/, "");
    const originOnly = !baseTrimmed.split("://")[1]?.includes("/");
    const codexBaseUrl = baseTrimmed.endsWith("/v1")
      ? baseTrimmed
      : originOnly
        ? `${baseTrimmed}/v1`
        : baseTrimmed;
    let configToml = `model_provider = "custom"
model = "${model}"
model_reasoning_effort = "${reasoningEffort}"
disable_response_storage = true`;
    // 顶层字段写在表头之前：TOML 里表头之后的键属于那张表。
    // 上下文窗口跟所选模型行的元数据（模型目录的 contextWindow）；压缩阈值存的是
    // 百分比，按同一个窗口换算成绝对 token 数（和后端 to_codex_provider 同规则）。
    const contextWindow = availableModels.find(
      (row) => row.id === model,
    )?.contextWindow;
    if (contextWindow) {
      configToml += `\nmodel_context_window = ${contextWindow}`;
    }
    const compactLimit = compactLimitFrom(contextWindow, m.autoCompactPercent);
    if (compactLimit !== undefined) {
      configToml += `\nmodel_auto_compact_token_limit = ${compactLimit}`;
    }
    configToml += `

[model_providers.custom]
name = "${m.remoteCompaction ? "OpenAI" : "NewAPI"}"
base_url = "${codexBaseUrl}"
wire_api = "responses"
requires_openai_auth = true`;
    // 子代理模型放最后：`[agents]` 表头之后的键都属于它
    if (m.subagentModel?.trim()) {
      configToml += `\n\n[agents]\ndefault_subagent_model = "${m.subagentModel.trim()}"`;
      if (m.subagentReasoningEffort?.trim()) {
        configToml += `\ndefault_subagent_reasoning_effort = "${m.subagentReasoningEffort.trim()}"`;
      }
    }
    const preview: {
      auth: { OPENAI_API_KEY: string };
      config: string;
      modelCatalog?: { models: typeof catalog };
    } = {
      auth: { OPENAI_API_KEY: apiKey },
      config: configToml,
    };
    // 目录：用户填了就用它，否则和写 live 时一样从可用模型列表自动生成；
    // effort 档位与默认档都是逐模型（拉取时补全，表单里可改）。
    const catalog =
      m.catalogModels && m.catalogModels.length > 0
        ? m.catalogModels
        : availableModels.map((model) => ({
            model: model.id,
            ...(model.displayName ? { displayName: model.displayName } : {}),
            ...(model.contextWindow
              ? { contextWindow: model.contextWindow }
              : {}),
            ...(model.efforts && model.efforts.length > 0
              ? { reasoningLevels: model.efforts }
              : {}),
            ...(model.defaultEffort
              ? { defaultReasoningLevel: model.defaultEffort }
              : {}),
          }));
    if (catalog.length > 0) {
      preview.modelCatalog = { models: catalog };
    }
    return preview;
  }, [baseUrl, apiKey, models.codex, availableModels]);

  // 预览里 config 字段的类型就是 TOML 字符串；目录条数决定要不要显示提示行。
  const codexTomlPreview = codexConfigJson?.config;
  const catalogPreviewCount = codexConfigJson?.modelCatalog?.models.length ?? 0;

  // 计算 Gemini 配置 JSON 预览
  const geminiConfigJson = useMemo(() => {
    const m = models.gemini ?? {};
    const model = m.model || "gemini-2.5-pro";
    const preview: Record<string, unknown> = {
      env: {
        GOOGLE_GEMINI_BASE_URL: baseUrl,
        GEMINI_API_KEY: apiKey,
        GEMINI_MODEL: model,
        ...(m.env ?? {}),
      },
    };
    if (m.config && Object.keys(m.config).length > 0) {
      preview.config = m.config;
    }
    return preview;
  }, [baseUrl, apiKey, models.gemini]);

  // 计算 Claude Desktop 配置 JSON 预览（和后端投影同形）
  const claudeDesktopConfigJson = useMemo(() => {
    const routeMap: Record<string, Record<string, unknown>> = {};
    if (desktopMode === "direct") {
      // 直连：模型 = 供应商级可用模型列表里的 claude-* 角色（后端同款规则），
      // 1M 按上下文窗口元数据判定。
      for (const model of availableModels) {
        const routeId = model.id.trim();
        if (!routeId) continue;
        if (!isClaudeSafeModelId(routeId)) continue;
        const entry: Record<string, unknown> = { model: routeId };
        if ((model.contextWindow ?? 0) >= 1_000_000) {
          entry.supports1m = true;
        }
        routeMap[routeId] = entry;
      }
    } else {
      for (const row of desktopRoutes) {
        const routeId = row.routeId.trim();
        if (!routeId) continue;
        // 映射模式下没填上游模型的档位后端会丢掉，预览保持一致
        if (!row.model.trim()) continue;
        const entry: Record<string, unknown> = { model: row.model.trim() };
        // 菜单显示名跟所选模型行的 displayName（后端投影同规则）
        const label = desktopMenuLabel(availableModels, row.model);
        if (label) {
          entry.labelOverride = label;
        }
        // 1M 声明跟随所选模型的上下文窗口元数据
        if (
          (availableModels.find((m) => m.id === row.model.trim())
            ?.contextWindow ?? 0) >= 1_000_000
        ) {
          entry.supports1m = true;
        }
        routeMap[routeId] = entry;
      }
    }
    const meta: Record<string, unknown> = {
      claudeDesktopMode: desktopMode,
      claudeDesktopModelRoutes: routeMap,
    };
    if (desktopMode === "proxy" && models.claudeDesktop?.apiFormat) {
      meta.apiFormat = models.claudeDesktop.apiFormat;
    }
    return {
      env: {
        ANTHROPIC_BASE_URL: baseUrl,
        ANTHROPIC_AUTH_TOKEN: apiKey,
      },
      meta,
    };
  }, [
    baseUrl,
    apiKey,
    desktopMode,
    desktopRoutes,
    models.claudeDesktop,
    availableModels,
  ]);

  // 计算 Grok Build 配置 TOML 预览
  //
  // 预览必须和后端投影 `to_grokbuild_provider` 生成的 config.toml 逐字节一致：
  // 它把供应商的整份可用模型列表写成一个 [model."<id>"] 表（每个模型一行），
  // 卡片上的「默认模型」只决定 [models].default 指向哪个表。
  const grokbuildConfigJson = useMemo(() => {
    const grok = models.grokbuild ?? {};
    const requestedModel = grok.model?.trim();
    const apiBackend =
      grok.apiBackend?.trim() || GROK_BUILD_DEFAULT_API_BACKEND;
    const baseUrlTrimmed = baseUrl.trim().replace(/\/+$/, "");

    // ids：可用模型列表里所有非空 id，按列表顺序去重；默认模型不在其中时追加。
    const ids = Array.from(
      new Set(availableModels.map((model) => model.id.trim()).filter(Boolean)),
    );
    if (requestedModel && !ids.includes(requestedModel))
      ids.push(requestedModel);
    if (ids.length === 0) ids.push(GROK_BUILD_DEFAULT_MODEL);

    // default：默认模型在 id 列表里就用它，否则用第一个。
    const defaultId =
      requestedModel && ids.includes(requestedModel) ? requestedModel : ids[0];

    const tables = ids.map((id) => {
      const contextWindow =
        availableModels.find((model) => model.id.trim() === id)
          ?.contextWindow ?? GROK_BUILD_DEFAULT_CONTEXT_WINDOW;
      return `[model."${id}"]
model = "${id}"
base_url = "${baseUrlTrimmed}"
name = "${name}"
api_key = "${apiKey}"
api_backend = "${apiBackend}"
context_window = ${contextWindow}`;
    });

    const config = `[models]
default = "${defaultId}"

${tables.join("\n\n")}
`;
    return { config };
  }, [baseUrl, apiKey, name, models.grokbuild, availableModels]);

  // 构建 provider 对象的辅助函数：新建和编辑共用，字段与 per-app 设置一次写全
  const buildProvider = useCallback((): UniversalProvider | null => {
    if (!name.trim() || !baseUrl.trim() || !apiKey.trim()) {
      return null;
    }

    const base: UniversalProvider = editingProvider
      ? { ...editingProvider }
      : {
          id: crypto.randomUUID(),
          name: "",
          apps: {
            claude: false,
            codex: false,
            gemini: false,
            claudeDesktop: false,
            grokbuild: false,
          },
          baseUrl: "",
          apiKey: "",
          models: {},
          createdAt: Date.now(),
        };

    // 端点测速面板维护的自定义端点与「自动选择最佳端点」是统一供应商自己的设置，
    // 存进它的 meta：首次同步创建子供应商时作为初值，之后端点设置归各子供应商自己。
    // 重新保存时沿用原有的 addedAt / lastUsed，别把端点当成刚加的。
    const knownEndpoints = editingProvider?.meta?.custom_endpoints ?? {};
    const customEndpoints = Object.fromEntries(
      draftCustomEndpoints.map((url) => [
        url,
        knownEndpoints[url] ?? {
          url,
          addedAt: Date.now(),
          lastUsed: undefined,
        },
      ]),
    );
    const meta: ProviderMeta = {
      ...(editingProvider?.meta ?? {}),
      endpointAutoSelect,
      ...(draftCustomEndpoints.length > 0
        ? { custom_endpoints: customEndpoints }
        : {}),
    };
    if (draftCustomEndpoints.length === 0) {
      delete meta.custom_endpoints;
    }

    // Gemini 的两个 JSON 块在保存时才解析：填了非法 JSON 就当没配，不让它挡住保存。
    const parseBlock = (text: string): Record<string, unknown> | undefined => {
      try {
        const parsed = JSON.parse(text || "{}");
        if (
          parsed &&
          typeof parsed === "object" &&
          !Array.isArray(parsed) &&
          Object.keys(parsed).length > 0
        ) {
          return parsed as Record<string, unknown>;
        }
      } catch {
        // 非法 JSON：忽略这一块
      }
      return undefined;
    };
    const geminiEnv = parseBlock(geminiEnvText);
    const geminiConfig = parseBlock(geminiConfigText);

    // 请求覆盖：Header / Body 两个文本框，保存时才解析；两个都空就不写。
    const parseOverrides = (text: {
      headers: string;
      body: string;
    }): LocalProxyRequestOverrides | undefined => {
      const headers = parseBlock(text.headers);
      const body = parseBlock(text.body);
      if (!headers && !body) return undefined;
      return {
        ...(headers ? { headers: headers as Record<string, string> } : {}),
        ...(body ? { body } : {}),
      };
    };
    const claudeOverridesParsed = parseOverrides(claudeOverrides);
    const codexOverridesParsed = parseOverrides(codexOverrides);

    // Gemini 的 .env / settings.json 文本块要能独立成块：新建时不再预填模型，
    // models.gemini 可能还不存在，但用户在卡片里填的块仍要存下来。
    const geminiNext: GeminiModelConfig = {
      ...(models.gemini ?? {}),
      ...(geminiEnv ? { env: geminiEnv as Record<string, string> } : {}),
      ...(geminiConfig ? { config: geminiConfig } : {}),
    };
    const nextModels: UniversalProviderModels = {
      ...models,
      ...(models.gemini || geminiEnv || geminiConfig
        ? { gemini: geminiNext }
        : {}),
    };
    if (nextModels.gemini) {
      if (!geminiEnv) delete nextModels.gemini.env;
      if (!geminiConfig) delete nextModels.gemini.config;
    }
    if (nextModels.claude) {
      // 显示名与 1M 声明都不再手填：某档选了哪个模型，就从可用模型列表里取它的
      // displayName（→ *ModelName）和 contextWindow（≥ 1,000,000 → *OneM 加 [1M]
      // 标记）。没选模型或没有对应元数据就清掉，后端「没填就不写」。
      const tierMetaOf = (
        modelId: string | undefined,
      ): { displayName?: string; supports1m: boolean } | undefined => {
        const id = modelId?.trim();
        if (!id) return undefined;
        const hit = availableModels.find((model) => model.id === id);
        if (!hit) return undefined;
        return {
          ...(hit.displayName ? { displayName: hit.displayName } : {}),
          supports1m: (hit.contextWindow ?? 0) >= 1_000_000,
        };
      };
      nextModels.claude = {
        ...nextModels.claude,
        ...(claudeOverridesParsed
          ? { requestOverrides: claudeOverridesParsed }
          : {}),
        // 署名与自动升级归供应商所有：和普通模型字段一样始终随保存写入，
        // 后端写进子供应商行并投影到 live。
      };
      // 显示名跟随所选模型，总是重算（不保留手填的旧值——表单里已经没有这个输入）。
      for (const [field, modelField] of [
        ["sonnetModelName", "sonnetModel"],
        ["opusModelName", "opusModel"],
        ["fableModelName", "fableModel"],
        ["haikuModelName", "haikuModel"],
      ] as const) {
        const name = tierMetaOf(models.claude?.[modelField])?.displayName;
        if (name) {
          nextModels.claude[field] = name;
        } else {
          delete nextModels.claude[field];
        }
      }
      // 1M 声明跟随所选模型，也总是重算（表单里已经没有这些勾选框）。
      for (const [field, modelField] of [
        ["modelOneM", "model"],
        ["sonnetModelOneM", "sonnetModel"],
        ["opusModelOneM", "opusModel"],
        ["fableModelOneM", "fableModel"],
        ["haikuModelOneM", "haikuModel"],
        ["subagentModelOneM", "subagentModel"],
      ] as const) {
        if (tierMetaOf(models.claude?.[modelField])?.supports1m) {
          nextModels.claude[field] = true;
        } else {
          delete nextModels.claude[field];
        }
      }
      if (!claudeOverridesParsed) delete nextModels.claude.requestOverrides;
    }
    if (nextModels.claudeDesktop) {
      // 旧版扁平字段只读：迁移成 routes 后必须丢掉，否则老值会一直跟着记录。
      const {
        sonnetModel: _sonnetModel,
        opusModel: _opusModel,
        haikuModel: _haikuModel,
        labelOverride: _labelOverride,
        sonnetOneM: _sonnetOneM,
        opusOneM: _opusOneM,
        haikuOneM: _haikuOneM,
        ...desktopConfig
      } = nextModels.claudeDesktop;
      nextModels.claudeDesktop = desktopConfig;

      // 路由表只在映射模式下保存：直连的模型由后端从供应商级可用模型列表自动
      // 生成（claude-* 角色 + 上下文窗口判定 1M），表单里没有直连的模型列表。
      // 映射模式下没填上游模型的行也丢掉——留着空模型会把后端的默认四档顶掉，
      // 整张表会写成空模型名。1M 声明不再手勾：按所选模型的上下文窗口自动判定。
      let routes: Record<string, ClaudeDesktopModelRoute> = {};
      if (desktopMode === "proxy") {
        for (const row of desktopRoutes) {
          const routeId = row.routeId.trim();
          if (!routeId) continue;
          if (!row.model.trim()) continue;
          const entry: ClaudeDesktopModelRoute = {
            model: row.model.trim(),
          };
          // 显示名跟随所选模型，总是重算（表单里已经没有这个输入）
          const label = desktopMenuLabel(availableModels, row.model);
          if (label) {
            entry.labelOverride = label;
          }
          if (
            (availableModels.find((model) => model.id === row.model.trim())
              ?.contextWindow ?? 0) >= 1_000_000
          ) {
            entry.supports1m = true;
          }
          routes[routeId] = entry;
        }
      }
      nextModels.claudeDesktop = {
        ...nextModels.claudeDesktop,
        mode: desktopMode,
        ...(Object.keys(routes).length > 0 ? { routes } : {}),
      };
      if (Object.keys(routes).length === 0) {
        delete nextModels.claudeDesktop.routes;
      }
      // 直连不能带上游格式
      if (desktopMode === "direct") {
        delete nextModels.claudeDesktop.apiFormat;
      }
    }
    if (nextModels.codex) {
      // 目录不再由表单维护（后端从可用模型列表自动生成），只带请求覆盖。
      nextModels.codex = {
        ...nextModels.codex,
        ...(codexOverridesParsed
          ? { requestOverrides: codexOverridesParsed }
          : {}),
      };
      if (!codexOverridesParsed) delete nextModels.codex.requestOverrides;
      delete nextModels.codex.catalogModels;
      // 认证字段是 Claude Code 的配置，Codex 没有：清掉旧数据里可能存的值。
      delete nextModels.codex.apiKeyField;
    }

    return {
      ...base,
      name: name.trim(),
      baseUrl: baseUrl.trim(),
      apiKey: apiKey.trim(),
      websiteUrl: websiteUrl.trim() || undefined,
      notes: notes.trim() || undefined,
      icon: icon.trim() || undefined,
      iconColor: iconColor.trim() || undefined,
      apps: {
        claude: true,
        codex: true,
        gemini: true,
        claudeDesktop: true,
        grokbuild: true,
      },
      models: nextModels,
      // 可用模型列表要一起存：它是所有应用共用的那份，模型下拉也从这里来。
      // 手加的空行（id 还没填）只是草稿，不落库。全局 effort 档位同属供应商级。
      ...(() => {
        const named = availableModels.filter((model) => model.id.trim());
        return named.length > 0 ? { availableModels: named } : {};
      })(),
      meta,
    };
  }, [
    editingProvider,
    geminiEnvText,
    geminiConfigText,
    claudeOverrides,
    codexOverrides,
    desktopMode,
    desktopRoutes,
    name,
    baseUrl,
    apiKey,
    websiteUrl,
    notes,
    icon,
    iconColor,
    models,
    availableModels,
    draftCustomEndpoints,
    endpointAutoSelect,
  ]);

  // 端点测速面板的初始候选：当前 API 地址 + 已保存的自定义端点
  const endpointCandidates = useMemo(() => {
    const map = new Map<string, { url: string; isCustom: boolean }>();
    const add = (raw: string, isCustom: boolean) => {
      const url = raw.trim().replace(/\/+$/, "");
      if (url) map.set(url, { url, isCustom });
    };
    add(baseUrl, false);
    draftCustomEndpoints.forEach((url) => add(url, true));
    return Array.from(map.values());
  }, [baseUrl, draftCustomEndpoints]);

  // 提交表单
  const handleSubmit = useCallback(() => {
    const provider = buildProvider();
    if (!provider) return;

    onSave(provider);
    onClose();
  }, [buildProvider, onSave, onClose]);

  // 打开保存并同步确认弹窗
  const handleSaveAndSyncClick = useCallback(() => {
    const provider = buildProvider();
    if (!provider || !onSaveAndSync) return;

    setPendingProvider(provider);
    setSyncConfirmOpen(true);
  }, [buildProvider, onSaveAndSync]);

  // 确认保存并同步
  const confirmSaveAndSync = useCallback(() => {
    if (!pendingProvider || !onSaveAndSync) return;

    onSaveAndSync(pendingProvider);
    setSyncConfirmOpen(false);
    setPendingProvider(null);
    onClose();
  }, [pendingProvider, onSaveAndSync, onClose]);

  /**
   * 拉取可用模型：后端问供应商的 `/v1/models`，再用 oh-my-pi 模型目录补名称和上下文窗口，
   * 结果写回该供应商。新建时还没有 id，先不拉。
   */
  const handleFetchModels = useCallback(async () => {
    // 编辑已存的那家用它的 id（后端写回）；新建时用表单当前填的地址和 Key 现拉。
    if (!editingProvider && (!baseUrl.trim() || !apiKey.trim())) {
      toast.error(
        t("universalProvider.fetchNeedsEndpoint", {
          defaultValue: "先填写 API 地址和 API Key，再获取模型列表",
        }),
      );
      return;
    }
    const target = editingProvider
      ? { id: editingProvider.id }
      : { baseUrl: baseUrl.trim(), apiKey: apiKey.trim() };
    setModelsLoading(true);
    try {
      const fetched = await universalProvidersApi.refreshModels(target);
      setAvailableModels(fetched);
      setFetchedModels(fetched);
      toast.success(
        t("universalProvider.modelsFetched", {
          defaultValue: `已获取 ${fetched.length} 个模型`,
        }),
      );
    } catch (error) {
      console.error(
        "[UniversalProviderFormModal] Failed to fetch models",
        error,
      );
      toast.error(
        extractErrorMessage(error) ||
          t("universalProvider.modelsFetchFailed", {
            defaultValue: "获取模型列表失败",
          }),
      );
    } finally {
      setModelsLoading(false);
    }
  }, [editingProvider, baseUrl, apiKey, t]);

  const footer = (
    <>
      {isEditMode && onDelete && editingProvider && (
        <Button
          variant="outline"
          className="mr-auto text-danger-text hover:text-danger-text"
          onClick={() => onDelete(editingProvider)}
        >
          <Trash2 className="mr-1.5 h-4 w-4" />
          {t("common.delete", { defaultValue: "删除" })}
        </Button>
      )}
      <Button variant="outline" onClick={onClose}>
        {t("common.cancel", { defaultValue: "取消" })}
      </Button>
      {isEditMode && onSaveAndSync ? (
        <Button
          onClick={handleSaveAndSyncClick}
          disabled={!name.trim() || !baseUrl.trim() || !apiKey.trim()}
        >
          <RefreshCw className="mr-1.5 h-4 w-4" />
          {t("universalProvider.saveAndSync", { defaultValue: "保存并同步" })}
        </Button>
      ) : (
        <Button
          onClick={handleSubmit}
          disabled={!name.trim() || !baseUrl.trim() || !apiKey.trim()}
        >
          {t("common.add", { defaultValue: "添加" })}
        </Button>
      )}
    </>
  );

  return (
    <FullScreenPanel
      isOpen={isOpen}
      trackUnsavedChanges
      title={
        isEditMode
          ? t("universalProvider.edit", { defaultValue: "编辑统一供应商" })
          : t("universalProvider.add", { defaultValue: "添加统一供应商" })
      }
      onClose={onClose}
      subtitle={subtitle}
      actions={manageAction}
      footer={footer}
    >
      {/* 用真正的 form 包住字段，提交走本表单页脚的按钮 */}
      <form
        id="universal-provider-form"
        className="space-y-6"
        onSubmit={(event) => {
          event.preventDefault();
          if (isEditMode && onSaveAndSync) {
            handleSaveAndSyncClick();
          } else {
            handleSubmit();
          }
        }}
      >
        {/* 基本信息：两列网格压缩高度；名称带图标占一整行，其余两两并排 */}
        <div className="grid gap-4 sm:grid-cols-2">
          <div className="space-y-2 sm:col-span-2">
            <Label htmlFor="name">
              {t("universalProvider.name", { defaultValue: "名称" })}
            </Label>
            <div className="flex items-center gap-2">
              <Dialog open={iconDialogOpen} onOpenChange={setIconDialogOpen}>
                <HoverTip
                  content={
                    icon
                      ? t("providerIcon.clickToChange", {
                          defaultValue: "点击更换图标",
                        })
                      : t("providerIcon.clickToSelect", {
                          defaultValue: "点击选择图标",
                        })
                  }
                >
                  <DialogTrigger asChild>
                    <button
                      type="button"
                      aria-label={t("providerIcon.change", {
                        defaultValue: "更换图标",
                      })}
                      className="group shrink-0 rounded-[8px]"
                    >
                      <ProviderIconBox
                        icon={icon}
                        name={name || "Provider"}
                        color={
                          iconColor ||
                          (icon
                            ? getIconMetadata(icon)?.defaultColor
                            : undefined)
                        }
                        className="bg-surface transition-colors group-hover:bg-subtle"
                      />
                    </button>
                  </DialogTrigger>
                </HoverTip>
                <DialogContent
                  variant="fullscreen"
                  zIndex="top"
                  overlayClassName="bg-[hsl(var(--background))] backdrop-blur-0"
                  className="p-0 sm:rounded-none"
                >
                  <div className="flex h-full flex-col">
                    <div className="flex-shrink-0 border-b border-border bg-subtle py-4">
                      <div className="flex items-center gap-4 px-6">
                        <DialogClose asChild>
                          <Button
                            type="button"
                            variant="outline"
                            size="icon"
                            aria-label={t("common.back")}
                          >
                            <ArrowLeft className="h-4 w-4" />
                          </Button>
                        </DialogClose>
                        <p className="text-lg font-semibold leading-tight">
                          {t("providerIcon.selectIcon", {
                            defaultValue: "选择图标",
                          })}
                        </p>
                      </div>
                    </div>
                    <div className="flex-1 overflow-y-auto">
                      <div className="w-full space-y-2 px-6 py-6">
                        <IconPicker
                          value={icon}
                          onValueChange={handleIconSelect}
                          color={iconColor}
                          defaultName={name}
                        />
                      </div>
                    </div>
                  </div>
                </DialogContent>
              </Dialog>
              <Input
                id="name"
                value={name}
                onChange={(e) => setName(e.target.value)}
                placeholder={t("universalProvider.namePlaceholder", {
                  defaultValue: "例如：我的 NewAPI",
                })}
              />
            </div>
          </div>

          <div className="space-y-2">
            <div className="flex flex-wrap items-center justify-between gap-2">
              <Label htmlFor="baseUrl">
                {t("universalProvider.baseUrl", { defaultValue: "API 地址" })}
              </Label>
              <button
                type="button"
                onClick={() => setIsEndpointModalOpen(true)}
                className="flex items-center gap-1 text-caption text-fg-2 transition-colors hover:text-fg-1"
              >
                <Zap className="h-3.5 w-3.5" />
                {t("providerForm.manageAndTest", {
                  defaultValue: "管理和测速",
                })}
              </button>
            </div>
            <Input
              id="baseUrl"
              value={baseUrl}
              onChange={(e) => setBaseUrl(e.target.value)}
              placeholder="https://api.example.com"
            />
          </div>

          <div className="space-y-2">
            <Label htmlFor="apiKey">
              {t("universalProvider.apiKey", { defaultValue: "API Key" })}
            </Label>
            <div className="relative">
              <Input
                id="apiKey"
                type={showApiKey ? "text" : "password"}
                value={apiKey}
                onChange={(e) => setApiKey(e.target.value)}
                placeholder="sk-..."
                className="pr-10"
              />
              <Button
                type="button"
                variant="ghost"
                size="icon"
                className="absolute right-0 top-0 h-full px-3"
                onClick={() => setShowApiKey(!showApiKey)}
              >
                {showApiKey ? (
                  <EyeOff className="h-4 w-4" />
                ) : (
                  <Eye className="h-4 w-4" />
                )}
              </Button>
            </div>
          </div>

          <div className="space-y-2">
            <Label htmlFor="websiteUrl">
              {t("universalProvider.websiteUrl", { defaultValue: "官网地址" })}
            </Label>
            <Input
              id="websiteUrl"
              value={websiteUrl}
              onChange={(e) => setWebsiteUrl(e.target.value)}
              placeholder={t("universalProvider.websiteUrlPlaceholder", {
                defaultValue: "https://example.com（可选，用于在列表中显示）",
              })}
            />
          </div>

          <div className="space-y-2">
            <Label htmlFor="notes">
              {t("universalProvider.notes", { defaultValue: "备注" })}
            </Label>
            <Input
              id="notes"
              value={notes}
              onChange={(e) => setNotes(e.target.value)}
              placeholder={t("universalProvider.notesPlaceholder", {
                defaultValue: "可选：添加备注信息",
              })}
            />
          </div>
        </div>

        {/* 模型列表：所有应用共用，表单里的模型下拉和各应用的目录都由它而来 */}
        <AvailableModelsEditor
          models={availableModels}
          onChange={setAvailableModels}
          onRefresh={() => void handleFetchModels()}
          refreshing={modelsLoading}
          fetched={fetchedModels}
          onQuickApply={handleQuickApply}
          desktopMode={desktopMode}
          quickModel={models.codex?.model}
          quickEffort={models.codex?.reasoningEffort}
        />

        {/* 各应用的专属字段。统一供应商始终同步到全部五个应用，没有开关。
            每个应用默认收起，展开才看到全部字段；收起时标题右侧有已配置摘要。 */}
        <div className="space-y-3">
          <Label>
            {t("universalProvider.appFields", {
              defaultValue: "各应用的配置",
            })}
          </Label>
          <div className="flex flex-col gap-3">
            <AppSection
              icon="claude"
              name="Claude Code"
              summary={sectionSummaries.claude}
              open={openSections.claude}
              onOpenChange={(open) => toggleSection("claude", open)}
            >
              <div className="grid gap-3 sm:grid-cols-2">
                <div className="space-y-1">
                  <Label className="text-xs">
                    {t("universalProvider.mainModel", {
                      defaultValue: "主模型",
                    })}
                  </Label>
                  <ModelSelect
                    models={availableModels}
                    value={models.claude?.model || ""}
                    onChange={(value) => updateModel("claude", "model", value)}
                    label={t("modelLabel", {
                      defaultValue: "主模型",
                    })}
                  />
                </div>
                <div className="space-y-1">
                  <Label className="text-xs">Sonnet</Label>
                  <ModelSelect
                    models={availableModels}
                    value={models.claude?.sonnetModel || ""}
                    onChange={(value) =>
                      updateModel("claude", "sonnetModel", value)
                    }
                    label={t("sonnetModelLabel", {
                      defaultValue: "Sonnet",
                    })}
                  />
                </div>
                <div className="space-y-1">
                  <Label className="text-xs">Opus</Label>
                  <ModelSelect
                    models={availableModels}
                    value={models.claude?.opusModel || ""}
                    onChange={(value) =>
                      updateModel("claude", "opusModel", value)
                    }
                    label={t("opusModelLabel", {
                      defaultValue: "Opus",
                    })}
                  />
                </div>
                <div className="space-y-1">
                  <Label className="text-xs">Fable</Label>
                  <ModelSelect
                    models={availableModels}
                    value={models.claude?.fableModel || ""}
                    onChange={(value) =>
                      updateModel("claude", "fableModel", value)
                    }
                    label={t("fableModelLabel", {
                      defaultValue: "Fable",
                    })}
                  />
                </div>
                <div className="space-y-1">
                  <Label className="text-xs">Haiku</Label>
                  <ModelSelect
                    models={availableModels}
                    value={models.claude?.haikuModel || ""}
                    onChange={(value) =>
                      updateModel("claude", "haikuModel", value)
                    }
                    label={t("haikuModelLabel", {
                      defaultValue: "Haiku",
                    })}
                  />
                </div>
                <div className="space-y-1">
                  <Label className="text-xs">
                    {t("universalProvider.subagentModel", {
                      defaultValue: "Subagent 模型",
                    })}
                  </Label>
                  <ModelSelect
                    models={availableModels}
                    value={models.claude?.subagentModel || ""}
                    onChange={(value) =>
                      updateModel("claude", "subagentModel", value)
                    }
                    label={t("subagentModelLabel", {
                      defaultValue: "子代理模型",
                    })}
                  />
                </div>
              </div>

              {/* 1M 声明不再手勾：保存时按所选模型在可用列表里的上下文窗口自动判定
                  （≥ 1,000,000 才给模型名加 [1M] 标记）。 */}

              <div className="grid gap-3 pt-3 sm:grid-cols-2">
                <div className="space-y-1">
                  <Label className="text-xs">
                    {t("universalProvider.apiFormat", {
                      defaultValue: "上游格式",
                    })}
                  </Label>
                  <Select
                    value={models.claude?.apiFormat || "anthropic"}
                    onValueChange={(value) =>
                      updateModel("claude", "apiFormat", value)
                    }
                  >
                    <SelectTrigger aria-label="上游格式">
                      <SelectValue />
                    </SelectTrigger>
                    <SelectContent>
                      <SelectItem value="anthropic">
                        Anthropic Messages
                      </SelectItem>
                      <SelectItem value="openai_chat">
                        Chat Completions
                      </SelectItem>
                      <SelectItem value="openai_responses">
                        Responses
                      </SelectItem>
                      <SelectItem value="gemini_native">
                        Gemini Native
                      </SelectItem>
                    </SelectContent>
                  </Select>
                </div>
                <div className="space-y-1">
                  <Label className="text-xs">
                    {t("universalProvider.apiKeyField", {
                      defaultValue: "认证字段",
                    })}
                  </Label>
                  <Select
                    value={models.claude?.apiKeyField || "ANTHROPIC_AUTH_TOKEN"}
                    onValueChange={(value) =>
                      updateModel("claude", "apiKeyField", value)
                    }
                  >
                    <SelectTrigger>
                      <SelectValue />
                    </SelectTrigger>
                    <SelectContent>
                      <SelectItem value="ANTHROPIC_AUTH_TOKEN">
                        ANTHROPIC_AUTH_TOKEN
                      </SelectItem>
                      <SelectItem value="ANTHROPIC_API_KEY">
                        ANTHROPIC_API_KEY
                      </SelectItem>
                    </SelectContent>
                  </Select>
                </div>
                <div className="space-y-1 sm:col-span-2">
                  <Label className="text-xs">
                    {t("providerForm.customUserAgent", {
                      defaultValue: "自定义 User-Agent",
                    })}
                  </Label>
                  <Input
                    value={models.claude?.customUserAgent || ""}
                    onChange={(e) =>
                      updateModel("claude", "customUserAgent", e.target.value)
                    }
                    placeholder="claude-cli/2.0.0"
                  />
                </div>
              </div>

              {/* 开关行：上游兼容 + 署名 / 自动升级，全部归这家供应商所有 */}
              <div className="flex flex-wrap items-center gap-x-4 gap-y-1 pt-3 text-xs text-fg-2">
                {(
                  [
                    [
                      "enableToolSearch",
                      "claudeConfig.enableToolSearch",
                      "启用 Tool Search",
                    ],
                    [
                      "disableArtifact",
                      "claudeConfig.disableArtifact",
                      "禁用 Artifact 工具",
                    ],
                    [
                      "disableAutoModeServer",
                      "claudeConfig.disableAutoModeServer",
                      "关闭 auto mode 服务端检查",
                    ],
                    [
                      "hideAttribution",
                      "claudeConfig.hideAttribution",
                      "隐藏 AI 署名",
                    ],
                    [
                      "disableAutoUpgrade",
                      "claudeConfig.disableAutoUpgrade",
                      "禁用自动升级",
                    ],
                  ] as const
                ).map(([field, key, fallback]) => (
                  <CheckPill
                    key={field}
                    checked={models.claude?.[field] === true}
                    onChange={(checked) =>
                      updateModel("claude", field, checked)
                    }
                    label={t(key, { defaultValue: fallback })}
                    help={t(`${key}Help`, { defaultValue: fallback })}
                  />
                ))}
              </div>

              {/* 本地路由请求覆盖 */}
              <div className="grid gap-3 pt-3 sm:grid-cols-2">
                {(
                  [
                    [
                      "headers",
                      "providerForm.localProxyHeaderOverrides",
                      "Header 覆盖",
                    ],
                    [
                      "body",
                      "providerForm.localProxyBodyOverrides",
                      "Body 覆盖",
                    ],
                  ] as const
                ).map(([field, key, fallback]) => (
                  <div key={field} className="space-y-1">
                    <Label className="text-xs">
                      {t(key, { defaultValue: fallback })}
                    </Label>
                    <Textarea
                      value={claudeOverrides[field]}
                      onChange={(e) =>
                        setClaudeOverrides((prev) => ({
                          ...prev,
                          [field]: e.target.value,
                        }))
                      }
                      className="min-h-[72px] resize-y font-mono text-xs"
                      placeholder={'{\n  "X-Provider": "cc-switch"\n}'}
                    />
                  </div>
                ))}
              </div>
              {/* Claude 配置预览 */}
              {claudeConfigJson && (
                <div className="pt-3">
                  <JsonEditor
                    value={JSON.stringify(claudeConfigJson, null, 2)}
                    onChange={() => {}}
                    height={180}
                    darkMode={isDarkMode}
                  />
                </div>
              )}
            </AppSection>
            <AppSection
              icon="openai"
              name="OpenAI Codex"
              summary={sectionSummaries.codex}
              open={openSections.codex}
              onOpenChange={(open) => toggleSection("codex", open)}
            >
              <div className="grid gap-3 pt-3 sm:grid-cols-2">
                <div className="space-y-1">
                  <Label className="text-xs">
                    {t("universalProvider.model", { defaultValue: "模型" })}
                  </Label>
                  <ModelSelect
                    models={availableModels}
                    value={models.codex?.model || ""}
                    onChange={(value) => updateModel("codex", "model", value)}
                    label={t("modelLabel", { defaultValue: "模型" })}
                  />
                </div>
                <div className="space-y-1">
                  <Label className="text-xs">
                    {t("codexConfig.reasoningEffortLabel", {
                      defaultValue: "Reasoning Effort",
                    })}
                  </Label>
                  <Select
                    value={models.codex?.reasoningEffort || "high"}
                    onValueChange={(value) =>
                      updateModel("codex", "reasoningEffort", value)
                    }
                  >
                    <SelectTrigger aria-label="Reasoning Effort">
                      <SelectValue />
                    </SelectTrigger>
                    <SelectContent>
                      {(codexEffortOptions ?? []).map((level: string) => (
                        <SelectItem key={level} value={level}>
                          {level}
                        </SelectItem>
                      ))}
                    </SelectContent>
                  </Select>
                </div>
              </div>

              {/* Effort 档位编辑在上方「可用模型列表」（全局），所有应用共用 */}

              <div className="grid gap-3 pt-3 sm:grid-cols-2">
                <div className="space-y-1">
                  <Label className="text-xs">
                    {t("universalProvider.apiFormat", {
                      defaultValue: "上游格式",
                    })}
                  </Label>
                  <Select
                    value={models.codex?.apiFormat || "openai_responses"}
                    onValueChange={(value) =>
                      updateModel("codex", "apiFormat", value)
                    }
                  >
                    <SelectTrigger aria-label="上游格式">
                      <SelectValue />
                    </SelectTrigger>
                    <SelectContent>
                      <SelectItem value="openai_responses">
                        Responses
                      </SelectItem>
                      <SelectItem value="openai_chat">
                        Chat Completions
                      </SelectItem>
                      <SelectItem value="anthropic">
                        Anthropic Messages
                      </SelectItem>
                    </SelectContent>
                  </Select>
                </div>
                <div className="space-y-1">
                  <Label className="text-xs">
                    {t("codexConfig.autoCompactLimit", {
                      defaultValue: "压缩阈值 (%)",
                    })}
                  </Label>
                  <Input
                    inputMode="numeric"
                    value={models.codex?.autoCompactPercent?.toString() ?? ""}
                    onChange={(e) => {
                      const digits = e.target.value.replace(/\D/g, "");
                      const percent = Math.min(100, Number(digits));
                      updateModel(
                        "codex",
                        "autoCompactPercent",
                        // 0 等于没填（换算不出阈值）：清掉这个键，别存个 0 进去
                        digits === "" || percent === 0 ? "" : String(percent),
                      );
                    }}
                    placeholder="90"
                    title={t("codexConfig.autoCompactLimitHint", {
                      defaultValue:
                        "按所选模型上下文窗口的百分比换算成 model_auto_compact_token_limit；模型没有窗口元数据时不写",
                    })}
                  />
                  {/* 百分比只是输入形态：写进 config.toml 的是算出来的绝对 token 数，
                      这里把结果显出来，和下面的预览对得上。填了百分比却没有窗口元数据
                      时说明白为什么不写，而不是留一句换算不出来的提示。 */}
                  {compactLimit !== undefined ? (
                    <p className="text-xs text-fg-3">
                      {t("codexConfig.autoCompactLimitResolved", {
                        value: compactLimit,
                        defaultValue: "= {{value}} tokens",
                      })}
                    </p>
                  ) : models.codex?.autoCompactPercent !== undefined ? (
                    <p className="text-xs text-fg-3">
                      {t("codexConfig.autoCompactLimitNoWindow", {
                        defaultValue:
                          "所选模型没有上下文窗口元数据，不写该字段",
                      })}
                    </p>
                  ) : null}
                </div>
                <div className="space-y-1">
                  <Label className="text-xs">
                    {t("providerForm.customUserAgent", {
                      defaultValue: "自定义 User-Agent",
                    })}
                  </Label>
                  <Input
                    value={models.codex?.customUserAgent || ""}
                    onChange={(e) =>
                      updateModel("codex", "customUserAgent", e.target.value)
                    }
                    placeholder="codex/1.0"
                  />
                </div>
              </div>

              {/* 配置文件的开关：远程压缩、模拟客户端、完整 URL */}
              <div className="flex flex-wrap items-center gap-x-4 gap-y-1 pt-3 text-xs text-fg-2">
                <label
                  className="inline-flex cursor-pointer items-center gap-1.5"
                  title={t("codexConfig.remoteCompactionHint", {
                    defaultValue:
                      "开启后会将当前 model_providers 条目的 name 写为 OpenAI，使 Codex 尝试使用远程压缩。",
                  })}
                >
                  <input
                    type="checkbox"
                    className="ui-checkbox"
                    checked={models.codex?.remoteCompaction === true}
                    onChange={(e) =>
                      updateModel("codex", "remoteCompaction", e.target.checked)
                    }
                  />
                  {t("codexConfig.enableRemoteCompaction", {
                    defaultValue: "启用远程压缩",
                  })}
                </label>
                <label className="inline-flex cursor-pointer items-center gap-1.5">
                  <input
                    type="checkbox"
                    className="ui-checkbox"
                    checked={models.codex?.impersonateClaudeCode === true}
                    onChange={(e) =>
                      updateModel(
                        "codex",
                        "impersonateClaudeCode",
                        e.target.checked,
                      )
                    }
                  />
                  {t("codexConfig.impersonateClaudeCodeLabel", {
                    defaultValue: "模拟 Claude Code 客户端",
                  })}
                </label>
                <label className="inline-flex cursor-pointer items-center gap-1.5">
                  <input
                    type="checkbox"
                    className="ui-checkbox"
                    checked={models.codex?.isFullUrl === true}
                    onChange={(e) =>
                      updateModel("codex", "isFullUrl", e.target.checked)
                    }
                  />
                  {t("providerForm.fullUrlLabel", {
                    defaultValue: "完整 URL",
                  })}
                </label>
              </div>

              <div className="grid gap-3 pt-3 sm:grid-cols-2">
                <div className="space-y-1">
                  <Label className="text-xs">
                    {t("codexConfig.subagentModelLabel", {
                      defaultValue: "子代理模型",
                    })}
                  </Label>
                  <ModelSelect
                    models={availableModels}
                    value={models.codex?.subagentModel || ""}
                    onChange={(value) =>
                      updateModel("codex", "subagentModel", value)
                    }
                    label={t("subagentModelLabel", {
                      defaultValue: "子代理模型",
                    })}
                  />
                </div>
                <div className="space-y-1">
                  <Label className="text-xs">
                    {t("codexConfig.subagentReasoningEffortLabel", {
                      defaultValue: "子代理推理档位",
                    })}
                  </Label>
                  <Select
                    value={
                      models.codex?.subagentReasoningEffort?.trim() ||
                      SUBAGENT_EFFORT_UNSET
                    }
                    onValueChange={(value) =>
                      updateModel(
                        "codex",
                        "subagentReasoningEffort",
                        value === SUBAGENT_EFFORT_UNSET ? "" : value,
                      )
                    }
                  >
                    <SelectTrigger aria-label="子代理推理档位">
                      <SelectValue />
                    </SelectTrigger>
                    <SelectContent>
                      {/* 空 = 不写 `default_subagent_reasoning_effort`，由 Codex 自己决定 */}
                      <SelectItem value={SUBAGENT_EFFORT_UNSET}>
                        {t("codexConfig.subagentReasoningEffortDefault", {
                          defaultValue: "不设置",
                        })}
                      </SelectItem>
                      {subagentEffortOptions.map((level: string) => (
                        <SelectItem key={level} value={level}>
                          {level}
                        </SelectItem>
                      ))}
                    </SelectContent>
                  </Select>
                </div>
                <div className="space-y-1">
                  <Label className="text-xs">
                    {t("codexConfig.promptCacheRoutingLabel", {
                      defaultValue: "提示词缓存路由",
                    })}
                  </Label>
                  <Select
                    value={models.codex?.promptCacheRouting || "auto"}
                    onValueChange={(value) =>
                      updateModel("codex", "promptCacheRouting", value)
                    }
                  >
                    <SelectTrigger>
                      <SelectValue />
                    </SelectTrigger>
                    <SelectContent>
                      <SelectItem value="auto">
                        {t("codexConfig.promptCacheRoutingAuto", {
                          defaultValue: "自动",
                        })}
                      </SelectItem>
                      <SelectItem value="enabled">
                        {t("codexConfig.promptCacheRoutingEnabled", {
                          defaultValue: "开启",
                        })}
                      </SelectItem>
                      <SelectItem value="disabled">
                        {t("codexConfig.promptCacheRoutingDisabled", {
                          defaultValue: "关闭",
                        })}
                      </SelectItem>
                    </SelectContent>
                  </Select>
                </div>
              </div>

              {/* 思考能力（走 Chat Completions 时声明） */}
              <div className="flex flex-wrap items-center gap-x-4 gap-y-1 pt-3 text-xs text-fg-2">
                <span>
                  {t("codexConfig.reasoningGroupTitle", {
                    defaultValue: "思考能力",
                  })}
                </span>
                <label className="inline-flex cursor-pointer items-center gap-1.5">
                  <input
                    type="checkbox"
                    className="ui-checkbox"
                    checked={
                      models.codex?.chatReasoning?.supportsThinking === true
                    }
                    onChange={(e) =>
                      updateChatReasoning("supportsThinking", e.target.checked)
                    }
                  />
                  {t("codexConfig.reasoningModeToggle", {
                    defaultValue: "支持思考模式",
                  })}
                </label>
                <label className="inline-flex cursor-pointer items-center gap-1.5">
                  <input
                    type="checkbox"
                    className="ui-checkbox"
                    checked={
                      models.codex?.chatReasoning?.supportsEffort === true
                    }
                    onChange={(e) =>
                      updateChatReasoning("supportsEffort", e.target.checked)
                    }
                  />
                  {t("codexConfig.reasoningEffortToggle", {
                    defaultValue: "支持思考等级",
                  })}
                </label>
              </div>

              {/* 模型目录不再手填：后端用统一供应商的可用模型列表自动生成
                  （名称与上下文窗口来自拉取时的 oh-my-pi 模型目录补全）。 */}
              <p className="pt-3 text-xs text-fg-2">
                {t("codexConfig.catalogAutoHint", {
                  defaultValue:
                    "Codex 的 /model 菜单由可用模型列表自动生成；想调整就先在上面维护模型列表。",
                })}
              </p>

              {/* 本地路由请求覆盖 */}
              <div className="grid gap-3 pt-3 sm:grid-cols-2">
                {(
                  [
                    [
                      "headers",
                      "providerForm.localProxyHeaderOverrides",
                      "Header 覆盖",
                    ],
                    [
                      "body",
                      "providerForm.localProxyBodyOverrides",
                      "Body 覆盖",
                    ],
                  ] as const
                ).map(([field, key, fallback]) => (
                  <div key={field} className="space-y-1">
                    <Label className="text-xs">
                      {t(key, { defaultValue: fallback })}
                    </Label>
                    <Textarea
                      value={codexOverrides[field]}
                      onChange={(e) =>
                        setCodexOverrides((prev) => ({
                          ...prev,
                          [field]: e.target.value,
                        }))
                      }
                      className="min-h-[72px] resize-y font-mono text-xs"
                      placeholder={'{\n  "X-Provider": "cc-switch"\n}'}
                    />
                  </div>
                ))}
              </div>
              {/* Codex 配置预览：与单应用表单一样分 auth.json / config.toml 两块 */}
              {codexConfigJson && (
                <div className="space-y-3 pt-3">
                  <div className="space-y-1">
                    <Label className="text-xs">
                      {t("codexConfig.authJson", {
                        defaultValue: "auth.json (JSON) *",
                      })}
                    </Label>
                    <JsonEditor
                      value={JSON.stringify(codexConfigJson.auth, null, 2)}
                      onChange={() => {}}
                      height={100}
                      darkMode={isDarkMode}
                    />
                  </div>
                  <div className="space-y-1">
                    <Label className="text-xs">
                      {t("codexConfig.configToml", {
                        defaultValue: "config.toml (TOML)",
                      })}
                    </Label>
                    <JsonEditor
                      value={codexTomlPreview}
                      onChange={() => {}}
                      height={240}
                      darkMode={isDarkMode}
                    />
                  </div>
                  {catalogPreviewCount > 0 && (
                    <p className="text-xs text-fg-2">
                      {t("codexConfig.catalogAutoHint", {
                        defaultValue:
                          "Codex 的 /model 菜单由可用模型列表自动生成；想调整就先在上面维护模型列表。",
                      })}
                    </p>
                  )}
                </div>
              )}
            </AppSection>
            <AppSection
              icon="gemini"
              name="Gemini CLI"
              summary={sectionSummaries.gemini}
              open={openSections.gemini}
              onOpenChange={(open) => toggleSection("gemini", open)}
            >
              <div className="space-y-1 pt-3">
                <Label className="text-xs">
                  {t("universalProvider.model", { defaultValue: "模型" })}
                </Label>
                <ModelSelect
                  models={availableModels}
                  value={models.gemini?.model || ""}
                  onChange={(value) => updateModel("gemini", "model", value)}
                  label={t("universalProvider.modelField", {
                    defaultValue: "模型",
                  })}
                />
              </div>
              {/* .env / settings.json 里的其他键：跟这个应用一起切换 */}
              <div className="space-y-1 pt-3">
                <Label className="text-xs">
                  {t("geminiConfig.envFile", {
                    defaultValue: "环境变量 (.env)",
                  })}
                </Label>
                <JsonEditor
                  value={geminiEnvText}
                  onChange={setGeminiEnvText}
                  height={140}
                  darkMode={isDarkMode}
                />
              </div>
              <div className="space-y-1 pt-3">
                <Label className="text-xs">
                  {t("geminiConfig.configJson", {
                    defaultValue: "settings.json",
                  })}
                </Label>
                <JsonEditor
                  value={geminiConfigText}
                  onChange={setGeminiConfigText}
                  height={140}
                  darkMode={isDarkMode}
                />
              </div>
              {/* Gemini JSON */}
              {geminiConfigJson && (
                <div className="space-y-2">
                  <div className="flex items-center gap-2 text-sm font-medium">
                    <ProviderIcon icon="gemini" name="Gemini" size={16} />
                    Gemini
                  </div>
                  <JsonEditor
                    value={JSON.stringify(geminiConfigJson, null, 2)}
                    onChange={() => {}}
                    height={140}
                    darkMode={isDarkMode}
                  />
                </div>
              )}
            </AppSection>
            <AppSection
              icon="claude"
              name="Claude Desktop"
              summary={sectionSummaries.claudeDesktop}
              open={openSections.claudeDesktop}
              onOpenChange={(open) => toggleSection("claudeDesktop", open)}
            >
              {/* 接入方式：直连的供应商自己认 Claude 角色 ID；模型映射经本地路由 */}
              <div className="flex items-start justify-between gap-4 pt-3">
                <p className="min-w-0 flex-1 text-xs leading-relaxed text-fg-2">
                  {desktopMode === "direct"
                    ? t("claudeDesktop.modelMappingOffHint", {
                        defaultValue:
                          "仅当供应商直接接受 Claude Desktop 可识别的三档角色 ID（claude-sonnet-* / claude-opus-* / claude-haiku-*）时才适用直连；其他模型名（含 claude-3-5-sonnet-… 等旧式 ID）请选择模型映射。",
                      })
                    : t("claudeDesktop.modelMappingOnHint", {
                        defaultValue:
                          "Claude Desktop 只接受 claude-sonnet-* / claude-opus-* / claude-haiku-* 三档角色 ID。选择模型映射后，CC Switch 会把这三档映射到供应商的实际模型，并在使用期间保持本地路由开启。",
                      })}
                </p>
                <div className="flex shrink-0 items-center gap-2 border-l border-border pl-4">
                  <Label
                    htmlFor="desktop-model-mode"
                    className="text-sm font-normal text-fg-2"
                  >
                    {t("claudeDesktop.modelModeLabel", {
                      defaultValue: "接入方式",
                    })}
                  </Label>
                  <Select
                    value={desktopMode}
                    onValueChange={(value) => {
                      const next = value as ClaudeDesktopMode;
                      // 直连不用路由表（后端按可用模型列表生成），但切回来时必须还在：
                      // 只在「当前是直连、之前有行」时清空——即首次进入直连前的空表。
                      // 有行的表跨模式切换保留，模型映射切回来照原样编辑。
                      if (next === "direct" && desktopRoutes.length === 0) {
                        setDesktopRoutes([]);
                      }
                      setDesktopMode(next);
                    }}
                  >
                    <SelectTrigger
                      id="desktop-model-mode"
                      className="w-[156px]"
                      aria-label={t("claudeDesktop.modelModeLabel", {
                        defaultValue: "接入方式",
                      })}
                    >
                      <SelectValue />
                    </SelectTrigger>
                    <SelectContent>
                      <SelectItem value="direct">
                        {t("claudeDesktop.modelModeDirect", {
                          defaultValue: "直连",
                        })}
                      </SelectItem>
                      <SelectItem value="proxy">
                        {t("claudeDesktop.modelModeProxy", {
                          defaultValue: "模型映射",
                        })}
                      </SelectItem>
                    </SelectContent>
                  </Select>
                </div>
              </div>

              {desktopMode === "proxy" && (
                <div className="space-y-1 pt-3">
                  <Label className="text-xs">
                    {t("providerForm.apiFormat", {
                      defaultValue: "上游格式",
                    })}
                  </Label>
                  <Select
                    value={models.claudeDesktop?.apiFormat || "anthropic"}
                    onValueChange={(value) =>
                      updateModel("claudeDesktop", "apiFormat", value)
                    }
                  >
                    <SelectTrigger className="w-full" aria-label="上游格式">
                      <SelectValue />
                    </SelectTrigger>
                    <SelectContent>
                      <SelectItem value="anthropic">
                        {t("providerForm.apiFormatAnthropic", {
                          defaultValue: "Anthropic Messages (原生)",
                        })}
                      </SelectItem>
                      <SelectItem value="openai_chat">
                        {t("providerForm.apiFormatOpenAIChat", {
                          defaultValue: "OpenAI Chat Completions (需开启路由)",
                        })}
                      </SelectItem>
                      <SelectItem value="openai_responses">
                        {t("providerForm.apiFormatOpenAIResponses", {
                          defaultValue: "OpenAI Responses API (需开启路由)",
                        })}
                      </SelectItem>
                      <SelectItem value="gemini_native">
                        {t("providerForm.apiFormatGeminiNative", {
                          defaultValue:
                            "Gemini Native generateContent (需开启路由)",
                        })}
                      </SelectItem>
                    </SelectContent>
                  </Select>
                </div>
              )}

              {/* 路由表只在映射模式下出现：直连的模型就是供应商级可用模型列表里
                  的 claude-* 角色（后端强制角色 ID 同名），不需要在这里维护。
                  1M 声明跟随所选模型的上下文窗口元数据，不再手勾。 */}
              {desktopMode === "proxy" && (
                <div className="space-y-2 pt-3">
                  <Label className="text-xs">
                    {t("claudeDesktop.routeMapTitle", {
                      defaultValue: "模型映射",
                    })}
                  </Label>
                  <p className="text-xs leading-relaxed text-fg-2">
                    {t("claudeDesktop.routeMapHint", {
                      defaultValue:
                        "为 Sonnet、Opus、Haiku 三档分别选择实际请求模型；菜单显示名按所选模型在可用模型列表里的名称自动生成。留空的档会自动沿用 Sonnet（或第一个已填档）的模型，确保子 agent 调用的 Haiku 始终可用。",
                    })}
                  </p>

                  <div className="hidden grid-cols-[140px_1fr_1fr] gap-2 px-1 text-xs font-medium text-fg-2 md:grid">
                    <span>
                      {t("claudeDesktop.routeModelLabel", {
                        defaultValue: "模型角色",
                      })}
                    </span>
                    <span>
                      {t("claudeDesktop.labelOverrideLabel", {
                        defaultValue: "菜单显示名",
                      })}
                    </span>
                    <span>
                      {t("claudeDesktop.upstreamModelLabel", {
                        defaultValue: "实际请求模型",
                      })}
                    </span>
                  </div>

                  <div className="space-y-2">
                    {desktopRoutes.map((row, index) => (
                      <div
                        key={`${row.routeId}-${index}`}
                        className="grid grid-cols-1 gap-2 md:grid-cols-[140px_1fr_1fr]"
                      >
                        <div className="flex h-9 items-center rounded-md border border-input bg-subtle px-3 text-sm font-medium text-fg-2">
                          {desktopRoleLabel(row.routeId)}
                        </div>
                        {/* 显示名按所选模型的元数据自动生成，不再手填 */}
                        <div
                          aria-label={t("claudeDesktop.labelOverrideLabel", {
                            defaultValue: "菜单显示名",
                          })}
                          className="flex h-8 items-center truncate rounded-control border border-border-strong bg-surface px-2.5 text-body text-fg-3"
                          title={desktopMenuLabel(availableModels, row.model)}
                        >
                          {desktopMenuLabel(availableModels, row.model) ?? "—"}
                        </div>
                        <ModelSelect
                          models={availableModels}
                          value={row.model}
                          onChange={(value) =>
                            updateDesktopRoute(index, { model: value })
                          }
                          label={t("claudeDesktop.upstreamModelLabel", {
                            defaultValue: "实际请求模型",
                          })}
                        />
                      </div>
                    ))}
                  </div>
                </div>
              )}
              {/* Claude Desktop 配置预览（和后端投影同形） */}
              {claudeDesktopConfigJson && (
                <div className="pt-3">
                  <JsonEditor
                    value={JSON.stringify(claudeDesktopConfigJson, null, 2)}
                    onChange={() => {}}
                    height={280}
                    darkMode={isDarkMode}
                  />
                </div>
              )}
            </AppSection>
            <AppSection
              icon="grok"
              name="Grok Build"
              summary={sectionSummaries.grokbuild}
              open={openSections.grokbuild}
              onOpenChange={(open) => toggleSection("grokbuild", open)}
            >
              <div className="grid gap-3 pt-3 sm:grid-cols-2">
                <div className="space-y-1">
                  <Label className="text-xs">
                    {t("universalProvider.grokDefaultModel", {
                      defaultValue: "默认模型",
                    })}
                  </Label>
                  <ModelSelect
                    models={availableModels}
                    value={models.grokbuild?.model || ""}
                    onChange={(value) =>
                      updateModel("grokbuild", "model", value)
                    }
                    label={t("modelLabel", { defaultValue: "默认模型" })}
                  />
                </div>
                <div className="space-y-1">
                  <Label className="text-xs">
                    {t("universalProvider.apiBackend", {
                      defaultValue: "上游协议",
                    })}
                  </Label>
                  <Select
                    value={models.grokbuild?.apiBackend || "responses"}
                    onValueChange={(value) =>
                      updateModel("grokbuild", "apiBackend", value)
                    }
                  >
                    <SelectTrigger>
                      <SelectValue />
                    </SelectTrigger>
                    <SelectContent>
                      <SelectItem value="responses">Responses</SelectItem>
                      <SelectItem value="chat_completions">
                        Chat Completions
                      </SelectItem>
                      <SelectItem value="anthropic_messages">
                        Anthropic Messages
                      </SelectItem>
                    </SelectContent>
                  </Select>
                </div>
              </div>
              {/* Grok Build TOML */}
              {grokbuildConfigJson && (
                <div className="space-y-2">
                  <div className="flex items-center gap-2 text-sm font-medium">
                    <ProviderIcon icon="grok" name="Grok Build" size={16} />
                    Grok Build
                  </div>
                  <JsonEditor
                    value={grokbuildConfigJson.config}
                    onChange={() => {}}
                    height={280}
                    darkMode={isDarkMode}
                  />
                </div>
              )}
            </AppSection>
          </div>
          <p className="text-xs text-fg-2">
            {t("universalProvider.allAppsHint", {
              defaultValue:
                "统一供应商会同步到全部五个应用。Claude Desktop 通过本地路由把三档角色模型映射到上游；Grok Build 按这里填写的模型与协议直连。",
            })}
          </p>
        </div>
      </form>

      {/* 端点测速：统一供应商只有一份地址，测速选中的地址写回 API 地址；
          自定义端点列表和「自动选择」存进 meta。appId 只用来选测速超时档位，
          取最宽松的 Codex 档（12s）——一份地址供所有应用用。 */}
      {isEndpointModalOpen && (
        <EndpointSpeedTest
          appId="codex"
          value={baseUrl}
          onChange={setBaseUrl}
          initialEndpoints={endpointCandidates}
          visible={isEndpointModalOpen}
          onClose={() => setIsEndpointModalOpen(false)}
          autoSelect={endpointAutoSelect}
          onAutoSelectChange={setEndpointAutoSelect}
          onCustomEndpointsChange={setDraftCustomEndpoints}
        />
      )}

      {/* 保存并同步确认弹窗 */}
      <ConfirmDialog
        isOpen={syncConfirmOpen}
        title={t("universalProvider.syncConfirmTitle", {
          defaultValue: "同步统一供应商",
        })}
        message={t("universalProvider.syncConfirmDescription", {
          defaultValue: `同步 "${name}" 将会覆盖 Claude Code、Claude Desktop、Codex、Gemini CLI 和 Grok Build 中关联的供应商配置。确定要继续吗？`,
          name: name,
        })}
        confirmText={t("universalProvider.saveAndSync", {
          defaultValue: "保存并同步",
        })}
        onConfirm={confirmSaveAndSync}
        onCancel={() => {
          setSyncConfirmOpen(false);
          setPendingProvider(null);
        }}
      />
    </FullScreenPanel>
  );
}
