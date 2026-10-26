import { CSS } from "@dnd-kit/utilities";
import { DndContext, closestCenter } from "@dnd-kit/core";
import {
  SortableContext,
  useSortable,
  verticalListSortingStrategy,
} from "@dnd-kit/sortable";
import {
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
  type CSSProperties,
} from "react";
import { AnimatePresence, motion } from "framer-motion";
import { Search, X } from "lucide-react";
import { useTranslation } from "react-i18next";
import { useMutation, useQueryClient } from "@tanstack/react-query";
import { toast } from "@/lib/toast";
import type { Provider } from "@/types";
import type { AppId } from "@/lib/api";
import { providersApi } from "@/lib/api/providers";
import { extractErrorMessage } from "@/utils/errorUtils";
import { useDragSort } from "@/hooks/useDragSort";
import { useStreamCheck } from "@/hooks/useStreamCheck";
import { ProviderCard } from "@/components/providers/ProviderCard";
import { ProviderEmptyState } from "@/components/providers/ProviderEmptyState";
import { Input } from "@/components/ui/input";
import { Button } from "@/components/ui/button";
import { HelpTip } from "@/components/ui/help-tip";
import { cn } from "@/lib/utils";
import { isTextEditableTarget } from "@/utils/domUtils";
import { extractProviderBaseUrl } from "@/utils/providerConfigUtils";
import {
  isUniversalChildId,
  universalParentIdOf,
} from "@/utils/universalProvider";
import {
  buildDesktopSections,
  buildSwitchSections,
  type CardPresentation,
  type ProviderSection,
  type SwitchModeInput,
} from "@/components/providers/presentation";

/** 切换式应用（Claude Code / Codex / Gemini CLI / Grok Build）的模式状态和动作，由供应商页传入。 */
export type SwitchModeProps = Omit<SwitchModeInput, "app" | "t" | "providers">;

interface ProviderListProps {
  providers: Record<string, Provider>;
  currentProviderId: string;
  appId: AppId;
  /** 直连切换 / Claude Desktop 的切换 */
  onSwitch: (provider: Provider) => void;
  onDelete: (provider: Provider) => void;
  onConfigureUsage?: (provider: Provider) => void;
  onOpenWebsite: (url: string) => void;
  onOpenTerminal?: (provider: Provider) => void;
  /** 打开统一供应商表单：只对由它同步出来的子卡给出入口。 */
  onEditUniversal?: (parentId: string) => void;
  onCreate?: () => void;
  /** 切换式应用必传：按当前模式算卡片 */
  switchMode?: SwitchModeProps;
  isLoading?: boolean;
  /** 搜索面板开关由页头按钮控制时传入；不传则列表自己管（只能 ⌘F 打开） */
  searchOpen?: boolean;
  onSearchOpenChange?: (open: boolean) => void;
}

export function ProviderList({
  providers,
  currentProviderId,
  appId,
  onSwitch,
  onDelete,
  onConfigureUsage,
  onOpenWebsite,
  onOpenTerminal,
  onEditUniversal,
  onCreate,
  switchMode,
  isLoading = false,
  searchOpen,
  onSearchOpenChange,
}: ProviderListProps) {
  const { t } = useTranslation();
  const { checkProvider, isChecking } = useStreamCheck(appId);
  const { sortedProviders, sensors, handleDragEnd } = useDragSort(
    providers,
    appId,
  );

  const handleTest = useCallback(
    (provider: Provider) => {
      checkProvider(provider.id, provider.name);
    },
    [checkProvider],
  );

  const queryClient = useQueryClient();
  const importMutation = useMutation({
    mutationFn: async (): Promise<boolean> => {
      if (appId === "claude-desktop") {
        return (await providersApi.importClaudeDesktopFromClaude()) > 0;
      }
      return providersApi.importDefault(appId);
    },
    onSuccess: (imported) => {
      if (imported) {
        queryClient.invalidateQueries({ queryKey: ["providers", appId] });
        if (appId === "claude-desktop") {
          queryClient.invalidateQueries({ queryKey: ["claudeDesktopStatus"] });
        }
        toast.success(t("provider.importCurrentDescription"));
      } else {
        toast.info(t("provider.noProviders"));
      }
    },
    onError: (error: unknown) => {
      // Tauri invoke 的 reject 值是后端序列化出的纯字符串而非 Error 对象，
      // 取 .message 只会得到 undefined（空 toast）。
      toast.error(extractErrorMessage(error) || t("settings.importFailed"));
      // 导入失败前也可能已产生需要上屏的副作用：GrokBuild 官方登录态下点
      // 导入，命令层会先补种官方条目、随后才因 live 不可导入而报错。
      queryClient.invalidateQueries({ queryKey: ["providers", appId] });
    },
  });

  const [searchTerm, setSearchTerm] = useState("");
  const [innerSearchOpen, setInnerSearchOpen] = useState(false);
  const isSearchOpen = searchOpen ?? innerSearchOpen;
  const setIsSearchOpen = onSearchOpenChange ?? setInnerSearchOpen;
  const searchInputRef = useRef<HTMLInputElement>(null);

  useEffect(() => {
    const handleKeyDown = (event: KeyboardEvent) => {
      if (event.defaultPrevented) return;

      const key = event.key.toLowerCase();
      if ((event.metaKey || event.ctrlKey) && key === "f") {
        // 正在输入框/可编辑区域中时不抢占 Ctrl+F（例如添加供应商表单里的
        // 输入框），避免与其同名快捷键冲突。
        if (isTextEditableTarget(document.activeElement)) return;
        event.preventDefault();
        setIsSearchOpen(true);
        return;
      }

      if (key === "escape") {
        setIsSearchOpen(false);
      }
    };

    globalThis.addEventListener("keydown", handleKeyDown);
    return () => globalThis.removeEventListener("keydown", handleKeyDown);
  }, [setIsSearchOpen]);

  useEffect(() => {
    if (isSearchOpen) {
      const frame = requestAnimationFrame(() => {
        searchInputRef.current?.focus();
        searchInputRef.current?.select();
      });
      return () => cancelAnimationFrame(frame);
    }
  }, [isSearchOpen]);

  // 每家可被搜到的文字：名称、备注、官网、请求地址（请求地址要解析配置，只随列表变化重算）
  const searchIndex = useMemo(
    () =>
      sortedProviders.map((provider) => ({
        provider,
        text: [
          provider.name,
          provider.notes,
          provider.websiteUrl,
          extractProviderBaseUrl(provider.settingsConfig),
        ]
          .filter(Boolean)
          .join("\n")
          .toLowerCase(),
      })),
    [sortedProviders],
  );

  // 关掉面板就不再过滤：搜索词留着，下次打开还在，但列表不会悄悄少几家
  const filteredProviders = useMemo(() => {
    const keyword = isSearchOpen ? searchTerm.trim().toLowerCase() : "";
    if (!keyword) return sortedProviders;
    return searchIndex
      .filter((entry) => entry.text.includes(keyword))
      .map((entry) => entry.provider);
  }, [isSearchOpen, searchTerm, searchIndex, sortedProviders]);

  const sections = useMemo<ProviderSection[]>(() => {
    if (switchMode) {
      // 队列按没过滤的全部供应商剔掉已不存在的 id：序号和上下移要按完整队列算，
      // 搜索只决定画哪些卡
      const known = new Set(sortedProviders.map((p) => p.id));
      return buildSwitchSections({
        ...switchMode,
        app: appId,
        t,
        providers: filteredProviders,
        queue: switchMode.queue.filter((id) => known.has(id)),
      });
    }
    return buildDesktopSections({
      t,
      providers: filteredProviders,
      currentId: currentProviderId,
      onSwitch,
    });
  }, [
    switchMode,
    appId,
    t,
    sortedProviders,
    filteredProviders,
    currentProviderId,
    onSwitch,
  ]);

  if (isLoading) {
    return (
      <div className="space-y-2">
        {[0, 1, 2].map((index) => (
          <div
            key={index}
            className="h-[60px] w-full rounded-panel border border-dashed border-border bg-subtle"
          />
        ))}
      </div>
    );
  }

  if (sortedProviders.length === 0) {
    return (
      <div className="space-y-4">
        <ProviderEmptyState
          appId={appId}
          onCreate={onCreate}
          onImport={() => importMutation.mutate()}
        />
      </div>
    );
  }

  const renderCard = (provider: Provider, presentation: CardPresentation) => (
    <SortableProviderCard
      key={provider.id}
      provider={provider}
      appId={appId}
      presentation={presentation}
      isCurrent={provider.id === currentProviderId}
      onDelete={onDelete}
      onConfigureUsage={
        onConfigureUsage ? (item) => onConfigureUsage(item) : () => undefined
      }
      onOpenWebsite={onOpenWebsite}
      onOpenTerminal={onOpenTerminal}
      onEditUniversal={
        onEditUniversal && isUniversalChildId(provider.id, appId)
          ? (item) => {
              const parentId = universalParentIdOf(item.id, appId);
              if (parentId) onEditUniversal(parentId);
            }
          : undefined
      }
      onTest={handleTest}
      isTesting={isChecking(provider.id)}
    />
  );

  // 搜索框放在列表上方第一行的最右端，不盖住卡片、打开时卡片也不下移
  const searchField = (
    <AnimatePresence>
      {isSearchOpen && (
        <motion.div
          key="provider-search"
          initial={{ opacity: 0, x: 8 }}
          animate={{ opacity: 1, x: 0 }}
          exit={{ opacity: 0, x: 8 }}
          transition={{ duration: 0.15, ease: "easeOut" }}
          className="relative flex w-64 min-w-[10rem] shrink items-center"
        >
          <Search className="pointer-events-none absolute start-2.5 top-1/2 h-4 w-4 -translate-y-1/2 text-fg-3" />
          <Input
            ref={searchInputRef}
            value={searchTerm}
            onChange={(event) => setSearchTerm(event.target.value)}
            placeholder={t("provider.searchPlaceholder", {
              defaultValue: "Search name, notes, or URL...",
            })}
            aria-label={t("provider.searchAriaLabel", {
              defaultValue: "Search providers",
            })}
            title={`${t("provider.searchScopeHint", {
              defaultValue:
                "Matches provider name, notes, website, and API address.",
            })} ${t("provider.searchCloseHint", {
              defaultValue: "Press Esc to close",
            })}`}
            className={searchTerm ? "pe-[5.5rem] ps-8" : "pe-9 ps-8"}
          />
          {searchTerm && (
            <Button
              variant="quiet"
              size="compact"
              className="absolute end-8 top-1/2 h-6 -translate-y-1/2 px-1.5"
              onClick={() => setSearchTerm("")}
            >
              {t("common.clear", { defaultValue: "Clear" })}
            </Button>
          )}
          <Button
            variant="quiet"
            size="icon-compact"
            className="absolute end-1 top-1/2 h-6 w-6 -translate-y-1/2"
            onClick={() => setIsSearchOpen(false)}
            aria-label={t("provider.searchCloseAriaLabel", {
              defaultValue: "Close provider search",
            })}
          >
            <X className="h-3.5 w-3.5" />
          </Button>
        </motion.div>
      )}
    </AnimatePresence>
  );

  const renderSectionTitle = (section: ProviderSection) => (
    <div className="flex items-center gap-1 text-caption font-semibold text-fg-2">
      <h2 className="m-0 text-caption font-semibold">{section.title}</h2>
      {section.help && (
        <HelpTip title={section.help.title}>{section.help.body}</HelpTip>
      )}
    </div>
  );

  // 第一个分组标题和搜索框同一行。这一行一直在（搜不到结果时只剩搜索框），输入框在树里
  // 的位置不变，打字中途不会因为结果清空被重新挂载而丢焦点
  const leadSection = sections[0];
  const liftLeadTitle =
    Boolean(leadSection?.title) && filteredProviders.length > 0;
  const showListHeader = Boolean(leadSection?.title) || isSearchOpen;

  // 卡片列表铺满主区域，和页头同宽
  return (
    <div className="space-y-4">
      {showListHeader && (
        <div
          className={cn(
            "flex min-h-8 items-center gap-3",
            liftLeadTitle && "-mb-2",
          )}
        >
          <div className="min-w-0 flex-1">
            {liftLeadTitle && renderSectionTitle(leadSection)}
          </div>
          {searchField}
        </div>
      )}

      {filteredProviders.length === 0 ? (
        <div className="rounded-panel border border-dashed border-border px-6 py-8 text-center text-body text-fg-2">
          {t("provider.noSearchResults", {
            defaultValue: "No providers match your search.",
          })}
        </div>
      ) : (
        <DndContext
          sensors={sensors}
          collisionDetection={closestCenter}
          onDragEnd={handleDragEnd}
        >
          {sections.map((section, index) => (
            <section key={section.key} className="space-y-2">
              {section.title && !(index === 0 && liftLeadTitle) && (
                <div className="pt-1">{renderSectionTitle(section)}</div>
              )}
              {section.items.length === 0 && section.emptyText ? (
                <div className="rounded-panel border border-dashed border-border px-4 py-3 text-caption text-fg-3">
                  {section.emptyText}
                </div>
              ) : (
                <SortableContext
                  items={section.items.map((item) => item.provider.id)}
                  strategy={verticalListSortingStrategy}
                >
                  <div className="space-y-2">
                    {section.items.map((item) =>
                      renderCard(item.provider, item.presentation),
                    )}
                  </div>
                </SortableContext>
              )}
            </section>
          ))}
        </DndContext>
      )}
    </div>
  );
}

interface SortableProviderCardProps {
  provider: Provider;
  appId: AppId;
  presentation: CardPresentation;
  isCurrent: boolean;
  onDelete: (provider: Provider) => void;
  onConfigureUsage: (provider: Provider) => void;
  onOpenWebsite: (url: string) => void;
  onOpenTerminal?: (provider: Provider) => void;
  onEditUniversal?: (provider: Provider) => void;
  onTest?: (provider: Provider) => void;
  isTesting: boolean;
}

function SortableProviderCard(props: SortableProviderCardProps) {
  const {
    setNodeRef,
    attributes,
    listeners,
    transform,
    transition,
    isDragging,
  } = useSortable({ id: props.provider.id });

  const style: CSSProperties = {
    transform: CSS.Transform.toString(transform),
    transition,
  };

  return (
    <div ref={setNodeRef} style={style}>
      <ProviderCard
        {...props}
        dragHandleProps={{ attributes, listeners, isDragging }}
      />
    </div>
  );
}
