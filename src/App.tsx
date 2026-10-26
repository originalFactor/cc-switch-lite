import { useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "@/lib/toast";
import { invoke } from "@tauri-apps/api/core";
import { MoreHorizontal, Plus, Search, Settings2 } from "lucide-react";
import { useQueryClient } from "@tanstack/react-query";
import { getCurrentWindow } from "@tauri-apps/api/window";
import type { Provider, UniversalProvider, VisibleApps } from "@/types";
import { KNOWN_APP_TYPES, type AppTypeFilter } from "@/types/usage";
import type { EnvConflict } from "@/types/env";
import {
  providersQueryOptions,
  proxyKeys,
  useProvidersQuery,
  useSettingsQuery,
} from "@/lib/query";
import {
  providersApi,
  settingsApi,
  universalProvidersApi,
  type AppId,
  type ProviderSwitchEvent,
} from "@/lib/api";
import { UniversalProviderFormModal } from "@/components/universal/UniversalProviderFormModal";
import { checkAllEnvConflicts, checkEnvConflicts } from "@/lib/api/env";
import { useProviderActions } from "@/hooks/useProviderActions";
import { useProxyStatus } from "@/hooks/useProxyStatus";
import { useUsageCacheBridge } from "@/hooks/useUsageCacheBridge";
import {
  useTrayAppPageSeen,
  useTrayNavigation,
} from "@/hooks/useTrayNavigation";
import { useTauriEvent } from "@/hooks/useTauriEvent";
import { useLastValidValue } from "@/hooks/useLastValidValue";
import { extractErrorMessage } from "@/utils/errorUtils";
import { isTextEditableTarget } from "@/utils/domUtils";
import { isLinux, isMac, isWindows } from "@/lib/platform";
import {
  APP_STORAGE_KEY,
  appPageBelongsTo,
  isAppPage,
  readStoredView,
  sharedFeatureAppOf,
  storeView,
  type GlobalPage,
  type SettingsSection,
  type View,
} from "@/lib/navigation";
import { Sidebar } from "@/components/shell/Sidebar";
import { useUpdate } from "@/contexts/UpdateContext";
import { NewLayoutDialog } from "@/components/shell/NewLayoutDialog";
import {
  AppPageHeader,
  WindowControlsContext,
} from "@/components/shell/AppPageHeader";
import { WindowControls } from "@/components/shell/WindowControls";
import { APP_DISPLAY_NAME, AppGlyph } from "@/components/shell/AppGlyph";
import { ProfileSwitcher } from "@/components/profiles/ProfileSwitcher";
import { ProviderList } from "@/components/providers/ProviderList";
import { ConfirmDialog } from "@/components/ConfirmDialog";
import { discardUnsavedChanges, hasUnsavedChanges } from "@/lib/unsavedChanges";
import { SettingsPage } from "@/components/settings/SettingsPage";
import { AppsPage } from "@/components/apps/AppsPage";
import {
  checkToolUpdatesInBackground,
  useToolUpdatesAvailable,
} from "@/components/apps/useToolManagement";
import { UsagePage } from "@/components/usage/UsagePage";
import { EnvWarningBanner } from "@/components/env/EnvWarningBanner";
import { SwitchModePanel } from "@/components/providers/mode/SwitchModePanel";
import { DesktopAccessBar } from "@/components/providers/mode/DesktopAccessBar";
import { RoutingSwitch } from "@/components/providers/mode/RoutingSwitch";
import { RouteSettingsSheet } from "@/components/providers/mode/RouteSettingsSheet";
import { useAppMode } from "@/lib/query/proxy";
import { useModeActions } from "@/hooks/useModeActions";
import { blockedFromRouting } from "@/components/providers/presentation";
import { proxyApi } from "@/lib/api/proxy";
import type { StartupAttachFailure } from "@/types/proxy";
import UsageScriptModal from "@/components/UsageScriptModal";
import UnifiedMcpPanel from "@/components/mcp/UnifiedMcpPanel";
import PromptPanel from "@/components/prompts/PromptPanel";
import { PROMPT_APP_IDS } from "@/lib/query/prompts";
import UnifiedSkillsPanel from "@/components/skills/UnifiedSkillsPanel";
import { DeepLinkImportDialog } from "@/components/DeepLinkImportDialog";
import { FirstRunNoticeDialog } from "@/components/FirstRunNoticeDialog";
import { WhatsNewNotice } from "@/components/WhatsNewDialog";
import { Button } from "@/components/ui/button";
import { HoverTip } from "@/components/ui/hover-tip";
import { SessionManagerPage } from "@/components/sessions/SessionManagerPage";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import {
  APP_IDS,
  DEFAULT_VISIBLE_APPS,
  isProxyAppId,
} from "@/config/appConfig";

interface SyncStatusUpdatedPayload {
  source?: string;
  status?: string;
  error?: string;
}

const getInitialApp = (): AppId => {
  const saved = localStorage.getItem(APP_STORAGE_KEY) as AppId | null;
  if (saved && APP_IDS.includes(saved)) {
    return saved;
  }
  return "claude";
};

function App() {
  const { t } = useTranslation();
  const queryClient = useQueryClient();
  const { hasUpdate } = useUpdate();

  const [activeApp, setActiveApp] = useState<AppId>(getInitialApp);
  const sharedFeatureApp = sharedFeatureAppOf(activeApp);
  const [currentView, setCurrentView] = useState<View>(readStoredView);
  const [settingsSection, setSettingsSection] =
    useState<SettingsSection>("general");
  const [appConfigScrollTarget, setAppConfigScrollTarget] = useState<AppId>();
  // 进设置前停留的页面：设置目录里的「← 返回」回到这里
  const settingsReturnViewRef = useRef<View>("providers");
  const [promptsApp, setPromptsApp] = useState<AppId>(() =>
    PROMPT_APP_IDS.includes(sharedFeatureApp) ? sharedFeatureApp : "claude",
  );
  const [isAddOpen, setIsAddOpen] = useState(false);
  /**
   * 从任意应用页打开的统一供应商：值是父级 id。卡片都是它同步出来的子卡，编辑要改的是
   * 父级那一份配置。
   */
  const [editingUniversalId, setEditingUniversalId] = useState<string | null>(
    null,
  );
  /** 待确认删除的统一供应商。 */
  const [pendingUniversalDelete, setPendingUniversalDelete] =
    useState<UniversalProvider | null>(null);
  // 供应商搜索面板：页头按钮和 ⌘F 都能打开；换应用或离开供应商页就收起
  const [providerSearchOpen, setProviderSearchOpen] = useState(false);
  useEffect(() => {
    setProviderSearchOpen(false);
  }, [activeApp, currentView]);
  // 托盘里点了直连下需要路由的那家：打开应用页后弹「需要路由」对话框
  const [trayNeedsRoute, setTrayNeedsRoute] = useState<{
    app: AppId;
    providerId: string;
    nonce: number;
  } | null>(null);
  const [mcpManagementBusy, setMcpManagementBusy] = useState(false);
  const [skillsNavigationBusy, setSkillsNavigationBusy] = useState(false);
  const [promptNavigationBusy, setPromptNavigationBusy] = useState(false);

  useEffect(() => {
    storeView(currentView);
  }, [currentView]);

  const { data: settingsData } = useSettingsQuery();
  // Windows 一律去掉系统标题栏，用页头里的应用内窗口按钮；Linux 由设置决定
  const useAppWindowControls =
    isWindows() || (isLinux() && (settingsData?.useAppWindowControls ?? false));
  const visibleApps = useMemo<VisibleApps>(
    () => ({
      ...DEFAULT_VISIBLE_APPS,
      ...settingsData?.visibleApps,
    }),
    [settingsData?.visibleApps],
  );

  const getFirstVisibleApp = (): AppId => {
    return APP_IDS.find((app) => visibleApps[app]) ?? "claude";
  };

  useEffect(() => {
    if (!visibleApps[activeApp]) {
      setActiveApp(getFirstVisibleApp());
    }
  }, [visibleApps, activeApp]);

  // 启动后把其他可见应用的供应商列表预取进缓存：第一次切过去直接有数据，不先画骨架
  const providersPrefetchedRef = useRef(false);
  useEffect(() => {
    if (!settingsData || providersPrefetchedRef.current) return;
    providersPrefetchedRef.current = true;
    for (const app of APP_IDS) {
      if (app !== activeApp && visibleApps[app]) {
        void queryClient.prefetchQuery(providersQueryOptions(app));
      }
    }
  }, [settingsData, visibleApps, activeApp, queryClient]);

  // 「启动时检查应用更新」（默认关）：打开了才在后台查一次，查到新版本侧栏「应用」上出现圆点
  const checkToolUpdatesOnStartup =
    settingsData?.checkToolUpdatesOnStartup ?? false;
  const toolUpdatesAvailable = useToolUpdatesAvailable();
  const startupToolCheckDoneRef = useRef(false);
  useEffect(() => {
    if (!checkToolUpdatesOnStartup || startupToolCheckDoneRef.current) return;
    startupToolCheckDoneRef.current = true;
    void checkToolUpdatesInBackground();
  }, [checkToolUpdatesOnStartup]);

  // 启动时没能接上路由、已退回直连的应用：在对应的应用页提示一次并给「重试」
  const [startupFailures, setStartupFailures] = useState<
    StartupAttachFailure[]
  >([]);
  useEffect(() => {
    proxyApi
      .takeStartupAttachFailures()
      .then((failures) => {
        if (failures?.length) setStartupFailures(failures);
      })
      .catch(() => undefined);
  }, []);

  // 应用专属页（工作区、记忆…）只属于它的应用；换了应用就回到供应商页
  useEffect(() => {
    if (isAppPage(currentView) && !appPageBelongsTo(currentView, activeApp)) {
      setCurrentView("providers");
    }
  }, [activeApp, currentView]);

  const [usageProvider, setUsageProvider] = useState<Provider | null>(null);
  const [confirmAction, setConfirmAction] = useState<Provider | null>(null);
  const [envConflicts, setEnvConflicts] = useState<EnvConflict[]>([]);
  const [showEnvBanner, setShowEnvBanner] = useState(false);

  const effectiveUsageProvider = useLastValidValue(usageProvider);
  const mainScrollRef = useRef<HTMLElement>(null);
  const providerScrollContainerRef = useRef<HTMLDivElement>(null);

  useUsageCacheBridge();

  useLayoutEffect(() => {
    if (currentView !== "providers") return;

    for (const container of [
      mainScrollRef.current,
      providerScrollContainerRef.current,
    ]) {
      if (container) {
        container.scrollTop = 0;
        container.scrollLeft = 0;
      }
    }
  }, [activeApp, currentView]);

  const { isRunning: isProxyRunning, takeoverStatus } = useProxyStatus();
  const proxyAppId = isProxyAppId(activeApp) ? activeApp : null;
  // 应用页页头的路由开关：直连 ↔ 路由只换传输方式，直连指针不动，所以关掉后回到同一家。
  const { data: headerModeView, isPending: headerModePending } = useAppMode(
    proxyAppId ?? "",
    !!proxyAppId,
  );
  const headerModeActions = useModeActions(proxyAppId ?? "claude");
  const openAddProvider = () => {
    setIsAddOpen(true);
  };
  const currentAppUsesProxy =
    proxyAppId !== null || activeApp === "claude-desktop";
  const isCurrentAppTakeoverActive = proxyAppId
    ? takeoverStatus?.[proxyAppId] || false
    : false;

  const { data, isLoading, refetch } = useProvidersQuery(activeApp, {
    isProxyRunning: currentAppUsesProxy && isProxyRunning,
  });
  const providers = useMemo(() => data?.providers ?? {}, [data]);
  const currentProviderId = data?.currentProviderId ?? "";
  const { switchProvider, deleteProvider, saveUsageScript } =
    useProviderActions(activeApp, isProxyRunning && isCurrentAppTakeoverActive);

  useEffect(() => {
    let unsubscribe: (() => void) | undefined;
    let active = true;

    const setupListener = async () => {
      try {
        const off = await providersApi.onSwitched(
          async (event: ProviderSwitchEvent) => {
            if (event.appType === activeApp) {
              await refetch();
            }
          },
        );
        if (!active) {
          off();
          return;
        }
        unsubscribe = off;
      } catch (error) {
        console.error("[App] Failed to subscribe provider switch event", error);
      }
    };

    void setupListener();
    return () => {
      active = false;
      unsubscribe?.();
    };
  }, [activeApp, queryClient, refetch]);

  useTauriEvent("universal-provider-synced", async () => {
    await queryClient.invalidateQueries({ queryKey: ["providers"] });
    try {
      await providersApi.updateTrayMenu();
    } catch (error) {
      console.error("[App] Failed to update tray menu", error);
    }
  });

  // 应用项目后刷新相关缓存（providers 由既有 provider-switched 监听承接；
  // proxy 状态由后端直接改 DB，不走 mutation，必须显式刷新）
  useTauriEvent("profile-applied", async () => {
    await queryClient.invalidateQueries({ queryKey: ["profiles"] });
    await queryClient.invalidateQueries({ queryKey: ["mcp", "all"] });
    await queryClient.invalidateQueries({ queryKey: ["skills"] });
    await queryClient.invalidateQueries({
      queryKey: proxyKeys.takeoverStatus,
    });
    await queryClient.invalidateQueries({ queryKey: proxyKeys.status });
    await queryClient.invalidateQueries({
      queryKey: ["providers", "claude-desktop"],
    });
  });

  useTauriEvent<SyncStatusUpdatedPayload | null | undefined>(
    "webdav-sync-status-updated",
    async (payload) => {
      const statusPayload = payload ?? {};
      await queryClient.invalidateQueries({ queryKey: ["settings"] });
      if (statusPayload.source !== "auto" || statusPayload.status !== "error") {
        return;
      }
      toast.error(
        t("settings.webdavSync.autoSyncFailedToast", {
          error: statusPayload.error || t("common.unknown"),
        }),
      );
    },
  );

  useTauriEvent<SyncStatusUpdatedPayload | null | undefined>(
    "s3-sync-status-updated",
    async (payload) => {
      const statusPayload = payload ?? {};
      await queryClient.invalidateQueries({ queryKey: ["settings"] });
      if (statusPayload.source !== "auto" || statusPayload.status !== "error") {
        return;
      }
      toast.error(
        t("settings.s3Sync.autoSyncFailedToast", {
          error: statusPayload.error || t("common.unknown"),
        }),
      );
    },
  );

  useTauriEvent<{ appType: string; providerName: string }>(
    "proxy-official-warning",
    (payload) => {
      toast.warning(
        t("notifications.proxyOfficialWarning", {
          name: payload.providerName,
          defaultValue: `当前供应商 ${payload.providerName} 是官方供应商，建议切换到第三方供应商后再使用代理接管`,
        }),
        { duration: 8000 },
      );
    },
  );

  useEffect(() => {
    // settingsData 未加载时跳过，避免用 fallback false 覆盖 Rust 侧已设好的装饰状态
    if (!settingsData) return;

    const syncWindowDecorations = async () => {
      try {
        await getCurrentWindow().setDecorations(!useAppWindowControls);
      } catch (error) {
        console.error("[App] Failed to update window decorations", error);
      }
    };

    void syncWindowDecorations();
  }, [useAppWindowControls, settingsData]);

  useEffect(() => {
    const checkEnvOnStartup = async () => {
      try {
        const allConflicts = await checkAllEnvConflicts();
        const flatConflicts = Object.values(allConflicts).flat();

        if (flatConflicts.length > 0) {
          setEnvConflicts(flatConflicts);
          const dismissed = sessionStorage.getItem("env_banner_dismissed");
          if (!dismissed) {
            setShowEnvBanner(true);
          }
        }
      } catch (error) {
        console.error(
          "[App] Failed to check environment conflicts on startup:",
          error,
        );
      }
    };

    checkEnvOnStartup();
  }, []);

  useEffect(() => {
    const checkMigration = async () => {
      try {
        const migrated = await invoke<boolean>("get_migration_result");
        if (migrated) {
          toast.success(
            t("migration.success", { defaultValue: "配置迁移成功" }),
            { closeButton: true },
          );
        }
      } catch (error) {
        console.error("[App] Failed to check migration result:", error);
      }
    };

    checkMigration();
  }, [t]);

  useEffect(() => {
    const checkSkillsMigration = async () => {
      try {
        const result = await invoke<{ count: number; error?: string } | null>(
          "get_skills_migration_result",
        );
        if (result?.error) {
          toast.error(t("migration.skillsFailed"), {
            description: t("migration.skillsFailedDescription"),
            closeButton: true,
          });
          console.error("[App] Skills SSOT migration failed:", result.error);
          return;
        }
        if (result && result.count > 0) {
          toast.success(t("migration.skillsSuccess", { count: result.count }), {
            closeButton: true,
          });
          await queryClient.invalidateQueries({ queryKey: ["skills"] });
        }
      } catch (error) {
        console.error("[App] Failed to check skills migration result:", error);
      }
    };

    checkSkillsMigration();
  }, [t, queryClient]);

  useEffect(() => {
    const checkEnvOnSwitch = async () => {
      try {
        const conflicts = await checkEnvConflicts(activeApp);

        if (conflicts.length > 0) {
          setEnvConflicts((prev) => {
            const existingKeys = new Set(
              prev.map((c) => `${c.varName}:${c.sourcePath}`),
            );
            const newConflicts = conflicts.filter(
              (c) => !existingKeys.has(`${c.varName}:${c.sourcePath}`),
            );
            return [...prev, ...newConflicts];
          });
          const dismissed = sessionStorage.getItem("env_banner_dismissed");
          if (!dismissed) {
            setShowEnvBanner(true);
          }
        }
      } catch (error) {
        console.error(
          "[App] Failed to check environment conflicts on app switch:",
          error,
        );
      }
    };

    checkEnvOnSwitch();
  }, [activeApp]);

  const currentViewRef = useRef(currentView);
  const managementBusy =
    mcpManagementBusy || skillsNavigationBusy || promptNavigationBusy;
  const managementBusyRef = useRef(false);
  managementBusyRef.current = managementBusy;

  useEffect(() => {
    currentViewRef.current = currentView;
  }, [currentView]);

  // ─── 导航 ───────────────────────────────────────────────────────────────
  // MCP / Skills / 提示词在进行中的操作会锁住导航（卸载面板会丢掉进行中的状态）

  // 添加 / 用量脚本面板只盖住内容区，侧栏还能点。离开当前页就把它们关掉：
  // 面板的 appId 跟着 activeApp 走，开着切应用会把 A 应用的供应商写进 B 应用
  const closeProviderPanels = () => {
    setIsAddOpen(false);
    setUsageProvider(null);
  };

  // 整页编辑器里有未保存的修改：先确认，确认放弃后再执行这次导航
  const [pendingLeave, setPendingLeave] = useState<(() => void) | null>(null);
  const confirmLeave = (proceed: () => void) => {
    if (!hasUnsavedChanges()) return false;
    setPendingLeave(() => proceed);
    return true;
  };

  const openSettings = (
    section: SettingsSection = "general",
    appConfigTarget?: AppId,
  ) => {
    if (managementBusyRef.current) return;
    if (confirmLeave(() => openSettings(section, appConfigTarget))) return;
    closeProviderPanels();
    if (currentViewRef.current !== "settings") {
      settingsReturnViewRef.current = currentViewRef.current;
    }
    setSettingsSection(section);
    setAppConfigScrollTarget(appConfigTarget);
    setCurrentView("settings");
  };

  const exitSettings = () => {
    setCurrentView(settingsReturnViewRef.current);
  };

  const selectApp = (app: AppId) => {
    if (managementBusyRef.current) return;
    if (confirmLeave(() => selectApp(app))) return;
    closeProviderPanels();
    setActiveApp(app);
    localStorage.setItem(APP_STORAGE_KEY, app);
    setCurrentView("providers");
  };

  const openPage = (page: GlobalPage | "settings") => {
    if (page === "settings") {
      openSettings("general");
      return;
    }
    if (managementBusyRef.current) return;
    // 再点当前页不会离开：不弹确认，也不能清掉编辑页的未保存登记
    if (currentViewRef.current === page) return;
    if (confirmLeave(() => openPage(page))) return;
    closeProviderPanels();
    if (page === "prompts" && currentViewRef.current !== "prompts") {
      // 提示词页默认选中侧栏里最后选的那个应用
      setPromptsApp(
        PROMPT_APP_IDS.includes(sharedFeatureApp) ? sharedFeatureApp : "claude",
      );
    }
    setCurrentView(page);
  };
  useTrayAppPageSeen(currentView === "providers" ? activeApp : null);

  useTrayNavigation((navigation) => {
    const navigate = () => {
      if (navigation.section) {
        openSettings(navigation.section);
        return;
      }
      if (!navigation.app) return;
      selectApp(navigation.app);
      // 托盘先换应用：新应用那格还没报上来，按它实际生效的模式
      if (navigation.intent === "add") openAddProvider();
      if (navigation.intent === "needsRoute" && navigation.providerId) {
        setTrayNeedsRoute({
          app: navigation.app,
          providerId: navigation.providerId,
          nonce: Date.now(),
        });
      }
    };
    if (managementBusyRef.current || !confirmLeave(navigate)) navigate();
  });

  // 侧栏、⌘K 进用量统计看全部应用；只有应用页 ⋯ 进来时带应用筛选
  const openPageFromNav = (page: GlobalPage | "settings") => {
    // 「设置」上的绿点说的是 CC Switch 有新版本，点进去直接到「关于」里的更新按钮
    if (page === "settings" && hasUpdate) {
      openSettings("about");
      return;
    }
    if (page === "usage") setUsageAppFilter("all");
    openPage(page);
  };

  const openSettingsRef = useRef(openSettings);
  openSettingsRef.current = openSettings;
  const [usageAppFilter, setUsageAppFilter] = useState<AppTypeFilter>("all");

  useEffect(() => {
    const handleKeyDown = (event: KeyboardEvent) => {
      const mod = event.metaKey || event.ctrlKey;
      if (mod && event.key === ",") {
        event.preventDefault();
        openSettingsRef.current("general");
        return;
      }

      if (event.key !== "Escape" || event.defaultPrevented) return;
      if (document.body.style.overflow === "hidden") return;
      if (managementBusyRef.current) return;
      if (isTextEditableTarget(event.target)) return;

      const view = currentViewRef.current;
      if (view === "skillsDiscovery") {
        event.preventDefault();
        setCurrentView("skills");
      } else if (view === "settings") {
        event.preventDefault();
        setCurrentView(settingsReturnViewRef.current);
      }
    };

    window.addEventListener("keydown", handleKeyDown);
    return () => {
      window.removeEventListener("keydown", handleKeyDown);
    };
  }, []);

  const handleOpenWebsite = async (url: string) => {
    try {
      await settingsApi.openExternal(url);
    } catch (error) {
      const detail =
        extractErrorMessage(error) ||
        t("notifications.openLinkFailed", {
          defaultValue: "链接打开失败",
        });
      toast.error(detail);
    }
  };

  const handleConfirmAction = async () => {
    if (!confirmAction) return;

    try {
      await deleteProvider(confirmAction.id);
    } catch {
      // useDeleteProviderMutation 的 onError 已经弹了错误 toast；确认框照样关掉
    }
    setConfirmAction(null);
  };

  // 从卡片打开的统一供应商：按 id 直接取那一份配置。
  // 不用缓存列表：列表在别处（添加页、管理页）改动后未必已失效，取到 null 会让表单
  // 静默不弹。这里每次打开都读一次，状态一定是新的。
  const [editingUniversal, setEditingUniversal] =
    useState<UniversalProvider | null>(null);
  useEffect(() => {
    if (!editingUniversalId) {
      setEditingUniversal(null);
      return;
    }
    let cancelled = false;
    void universalProvidersApi
      .get(editingUniversalId)
      .then((provider) => {
        if (!cancelled) setEditingUniversal(provider ?? null);
      })
      .catch((error) => {
        console.error("[App] Failed to load universal provider", error);
        if (!cancelled) setEditingUniversal(null);
      });
    return () => {
      cancelled = true;
    };
  }, [editingUniversalId]);

  const handleUniversalDelete = async () => {
    const target = pendingUniversalDelete;
    setPendingUniversalDelete(null);
    if (!target) return;
    try {
      await universalProvidersApi.delete(target.id);
      toast.success(
        t("universalProvider.deleted", { defaultValue: "统一供应商已删除" }),
      );
      setEditingUniversalId(null);
      await queryClient.invalidateQueries({ queryKey: ["providers"] });
    } catch (error) {
      console.error("[App] Failed to delete universal provider", error);
      toast.error(
        t("universalProvider.deleteError", {
          defaultValue: "删除统一供应商失败",
        }),
      );
    }
  };

  const handleUniversalSave = async (provider: UniversalProvider) => {
    try {
      await universalProvidersApi.upsert(provider);
      // 卡片是子卡，改完父级必须同步过去；否则应用列表里还是旧配置。
      await universalProvidersApi.sync(provider.id);
      toast.success(
        t("universalProvider.savedAndSynced", {
          defaultValue: "已保存并同步到所有应用",
        }),
      );
      setEditingUniversalId(null);
      await queryClient.invalidateQueries({ queryKey: ["providers"] });
    } catch (error) {
      console.error("[App] Failed to save universal provider", error);
      toast.error(
        t("universalProvider.saveError", {
          defaultValue: "保存统一供应商失败",
        }),
      );
    }
  };

  const confirmActionMessage = useMemo(() => {
    if (!confirmAction) return "";

    return t("confirm.deleteProviderMessage", {
      name: confirmAction.name,
    });
  }, [confirmAction, t]);

  const handleOpenTerminal = async (provider: Provider) => {
    try {
      const selectedDir = await settingsApi.pickDirectory();
      if (!selectedDir) {
        return;
      }

      await providersApi.openTerminal(provider.id, activeApp, {
        cwd: selectedDir,
      });
      toast.success(
        t("provider.terminalOpened", {
          defaultValue: "终端已打开",
        }),
      );
    } catch (error) {
      console.error("[App] Failed to open terminal", error);
      const errorMessage = extractErrorMessage(error);
      toast.error(
        t("provider.terminalOpenFailed", {
          defaultValue: "打开终端失败",
        }) + (errorMessage ? `: ${errorMessage}` : ""),
      );
    }
  };

  const handleImportSuccess = async () => {
    try {
      await queryClient.invalidateQueries({
        queryKey: ["providers"],
        refetchType: "all",
      });
      await queryClient.refetchQueries({
        queryKey: ["providers"],
        type: "all",
      });
    } catch (error) {
      console.error("[App] Failed to refresh providers after import", error);
      await refetch();
    }
    try {
      await providersApi.updateTrayMenu();
    } catch (error) {
      console.error("[App] Failed to refresh tray menu", error);
    }
  };

  const handleHideActiveApp = async () => {
    const visibleCount = Object.values(visibleApps).filter(Boolean).length;
    if (visibleCount <= 1) return;
    try {
      const current = await settingsApi.get();
      await settingsApi.save({
        ...current,
        visibleApps: { ...visibleApps, [activeApp]: false },
      });
      await queryClient.invalidateQueries({ queryKey: ["settings"] });
    } catch (error) {
      toast.error(
        t("settings.saveFailedGeneric", { defaultValue: "保存失败，请重试" }),
        { description: extractErrorMessage(error) || undefined },
      );
    }
  };

  // ─── 应用页 ─────────────────────────────────────────────────────────────

  // 应用页 ⋯ →「查看此应用的用量」：带上这个应用的筛选（Claude Desktop 的流量并在 Claude 里）
  const usageAppOf = (app: AppId): AppTypeFilter | null => {
    const type = app === "claude-desktop" ? "claude" : app;
    return (KNOWN_APP_TYPES as ReadonlyArray<string>).includes(type)
      ? (type as AppTypeFilter)
      : null;
  };
  const activeUsageApp = usageAppOf(activeApp);

  const appMenu = (
    <DropdownMenu>
      <HoverTip content={t("common.more")}>
        <DropdownMenuTrigger asChild>
          <Button
            variant="quiet"
            size="icon-compact"
            className="h-8 w-8"
            aria-label={t("appPage.moreActions", {
              name: APP_DISPLAY_NAME[activeApp],
            })}
          >
            <MoreHorizontal className="h-4 w-4" />
          </Button>
        </DropdownMenuTrigger>
      </HoverTip>
      <DropdownMenuContent align="end" className="min-w-[180px]">
        <DropdownMenuItem onSelect={() => openPage("sessions")}>
          {t("appPage.viewSessions")}
        </DropdownMenuItem>
        {activeUsageApp && (
          <DropdownMenuItem
            onSelect={() => {
              setUsageAppFilter(activeUsageApp);
              openPage("usage");
            }}
          >
            {t("appPage.viewUsage")}
          </DropdownMenuItem>
        )}
        <DropdownMenuItem onSelect={() => openSettings("appConfig", activeApp)}>
          {t("appPage.configDirectory")}
        </DropdownMenuItem>
        <DropdownMenuItem onSelect={() => openPage("apps")}>
          {t("appPage.installAndUpgrade")}
        </DropdownMenuItem>
        <DropdownMenuSeparator />
        <DropdownMenuItem
          disabled={Object.values(visibleApps).filter(Boolean).length <= 1}
          onSelect={() => void handleHideActiveApp()}
        >
          {t("appPage.hideFromSidebar")}
        </DropdownMenuItem>
      </DropdownMenuContent>
    </DropdownMenu>
  );

  const [routeSettingsOpen, setRouteSettingsOpen] = useState(false);

  const headerRouting = (() => {
    if (!proxyAppId) return null;
    const direct = headerModeView?.directProviderId
      ? providers[headerModeView.directProviderId]
      : undefined;
    const route = headerModeView?.routeProviderId
      ? providers[headerModeView.routeProviderId]
      : undefined;
    const active = headerModeView?.mode ?? "direct";
    // 打开开关路由到「现在直连的那家」：直连指针不动，关掉后同一家回来。它不能走路由
    //（官方订阅等，Codex 官方登录除外）时开关是灰的，原因写在悬停说明里。
    const disabledReason =
      active === "route"
        ? undefined
        : direct === undefined
          ? t("mode.noProvider")
          : blockedFromRouting(proxyAppId, direct)
            ? t("providerCard.reason.noRoute")
            : undefined;
    return { active, direct, route, disabledReason };
  })();

  const renderAppPageHeader = () => (
    <AppPageHeader
      variant="app"
      icon={<AppGlyph app={activeApp} size={20} badgeClassName="bg-app" />}
      title={APP_DISPLAY_NAME[activeApp]}
      subtitle={t("appPage.providers")}
      actions={
        <>
          {currentView === "providers" && headerRouting && (
            <div className="flex items-center gap-1">
              <RoutingSwitch
                app={proxyAppId!}
                active={headerRouting.active}
                directProvider={headerRouting.direct}
                routeProvider={headerRouting.route}
                disabledReason={headerRouting.disabledReason}
                pending={headerModeActions.pending || headerModePending}
                onChange={(next) => {
                  if (next) {
                    // 打开时路由到「现在直连的那家」：直连指针不动，关掉后同一家回来。
                    void headerModeActions
                      .enter(
                        headerRouting.direct?.id ?? null,
                        headerRouting.direct?.name,
                      )
                      .catch((error) =>
                        toast.error(
                          t("mode.toast.failed", {
                            detail:
                              extractErrorMessage(error) || t("common.unknown"),
                          }),
                        ),
                      );
                    return;
                  }
                  void headerModeActions
                    .exitToDirect({
                      previous: {
                        mode: headerRouting.active,
                        routeProviderId: headerRouting.route?.id ?? null,
                      },
                      directProviderId: headerRouting.direct?.id ?? null,
                      providerName: headerRouting.direct?.name ?? "",
                    })
                    .catch(() => undefined);
                }}
              />
              <HoverTip content={t("mode.routeSettings")}>
                <Button
                  variant="quiet"
                  size="icon-compact"
                  className="h-8 w-8"
                  aria-label={t("mode.routeSettings")}
                  onClick={() => setRouteSettingsOpen(true)}
                >
                  <Settings2 className="h-4 w-4" strokeWidth={1.75} />
                </Button>
              </HoverTip>
            </div>
          )}
          {currentView === "providers" &&
            (settingsData?.showProfileSwitcher ?? false) && (
              <ProfileSwitcher activeApp={activeApp} />
            )}
          {currentView === "providers" &&
            (settingsData?.showProviderSearch ?? true) && (
              <HoverTip
                content={t("provider.searchButtonTip", {
                  shortcut: isMac() ? "⌘F" : "Ctrl+F",
                })}
              >
                <Button
                  variant="quiet"
                  size="icon-compact"
                  className="h-8 w-8"
                  aria-label={t("provider.searchAriaLabel")}
                  aria-pressed={providerSearchOpen}
                  onClick={() => setProviderSearchOpen((open) => !open)}
                >
                  <Search className="h-4 w-4" />
                </Button>
              </HoverTip>
            )}
          {currentView === "providers" && (
            <Button
              variant="solid"
              size="regular"
              onClick={() => openAddProvider()}
            >
              <Plus className="h-4 w-4" />
              {t("provider.addProvider")}
            </Button>
          )}
          {appMenu}
        </>
      }
    />
  );

  const listCallbacks = {
    onDelete: (provider: Provider) => setConfirmAction(provider),
    onConfigureUsage: setUsageProvider,
    onOpenWebsite: handleOpenWebsite,
    onOpenTerminal: activeApp === "claude" ? handleOpenTerminal : undefined,
    onCreate: () => openAddProvider(),
    searchOpen: providerSearchOpen,
    onSearchOpenChange: setProviderSearchOpen,
    onEditUniversal: (parentId: string) => setEditingUniversalId(parentId),
  };

  const renderProviderList = () => {
    if (proxyAppId) {
      const startupFailure = startupFailures.find(
        (failure) => failure.appType === proxyAppId,
      );
      return (
        <SwitchModePanel
          key={proxyAppId}
          app={proxyAppId}
          providers={providers}
          currentProviderId={currentProviderId}
          isLoading={isLoading}
          scrollRef={providerScrollContainerRef}
          onSwitch={switchProvider}
          needsRouteRequest={
            trayNeedsRoute?.app === proxyAppId ? trayNeedsRoute : undefined
          }
          onNeedsRouteHandled={() => setTrayNeedsRoute(null)}
          startupFailure={startupFailure}
          onDismissStartupFailure={() =>
            setStartupFailures((list) =>
              list.filter((failure) => failure.appType !== proxyAppId),
            )
          }
          {...listCallbacks}
        />
      );
    }

    return (
      <div
        ref={providerScrollContainerRef}
        id="main-content"
        className="min-h-0 flex-1 overflow-y-auto scroll-stable overflow-x-hidden px-6 pb-12 pt-4"
      >
        <div className="space-y-4">
          {activeApp === "claude-desktop" && (
            <DesktopAccessBar
              current={providers[currentProviderId]}
              onOpenRoutingSettings={() => openSettings("routing")}
            />
          )}
          <ProviderList
            {...listCallbacks}
            providers={providers}
            currentProviderId={currentProviderId}
            appId={activeApp}
            isLoading={isLoading}
            onSwitch={switchProvider}
          />
        </div>
      </div>
    );
  };

  const renderAppPage = () => (
    <>
      {renderAppPageHeader()}
      {renderProviderList()}
      {proxyAppId && (
        <RouteSettingsSheet
          app={proxyAppId}
          open={routeSettingsOpen}
          onOpenChange={setRouteSettingsOpen}
          onOpenSettings={() => openSettings("routing")}
        />
      )}
    </>
  );

  // ─── 全局页 ─────────────────────────────────────────────────────────────

  const renderGlobalPage = () => {
    switch (currentView) {
      case "usage":
        return (
          <UsagePage
            initialAppType={usageAppFilter}
            onOpenRoutingSettings={() => openSettings("routing")}
          />
        );
      case "mcp":
        return (
          <UnifiedMcpPanel onInteractionBlockedChange={setMcpManagementBusy} />
        );
      case "skills":
      case "skillsDiscovery":
        // 「已安装 / 发现」合成一页；旧的 skillsDiscovery 视图直接打开「发现」段
        return (
          <UnifiedSkillsPanel
            initialView={
              currentView === "skillsDiscovery" ? "discover" : "installed"
            }
            onNavigationBlockedChange={setSkillsNavigationBusy}
          />
        );
      case "prompts":
        return (
          <div id="main-content" className="flex min-h-0 flex-1 flex-col">
            <PromptPanel
              appId={promptsApp}
              apps={PROMPT_APP_IDS.filter(
                (app) =>
                  visibleApps[app] ||
                  (app === "claude" && visibleApps["claude-desktop"]) ||
                  app === promptsApp,
              )}
              onAppChange={setPromptsApp}
              onNavigationBlockedChange={setPromptNavigationBusy}
            />
          </div>
        );
      case "sessions":
        // 页头（含 ⋯ 菜单）由会话页自己画：菜单里的操作都在页面状态里
        return (
          <SessionManagerPage
            key={sharedFeatureApp}
            appId={sharedFeatureApp}
            fromApp={activeApp}
            onOpenTerminalSettings={() => openSettings("general")}
          />
        );
      case "apps":
        return <AppsPage />;
      default:
        return null;
    }
  };

  const renderContent = () => {
    if (currentView === "settings") {
      return (
        <SettingsPage
          section={settingsSection}
          appConfigScrollTarget={appConfigScrollTarget}
          onImportSuccess={handleImportSuccess}
          onOpenApps={() => setCurrentView("apps")}
          onOpenApp={selectApp}
        />
      );
    }
    return isAppPage(currentView) ? renderAppPage() : renderGlobalPage();
  };

  return (
    <WindowControlsContext.Provider
      value={useAppWindowControls ? <WindowControls /> : null}
    >
      <div className="flex h-screen overflow-hidden bg-app text-fg-1 selection:bg-action/25">
        <Sidebar
          activeApp={activeApp}
          view={currentView}
          visibleApps={visibleApps}
          settingsSection={settingsSection}
          onSelectApp={selectApp}
          onSelectPage={openPageFromNav}
          onSelectSettingsSection={(section) => {
            setAppConfigScrollTarget(undefined);
            setSettingsSection(section);
          }}
          onExitSettings={exitSettings}
          appsUpdateAvailable={
            checkToolUpdatesOnStartup && toolUpdatesAvailable
          }
        />
        <main
          ref={mainScrollRef}
          id="content-area"
          className="relative isolate flex min-w-0 flex-1 flex-col overflow-hidden bg-app"
        >
          {showEnvBanner && envConflicts.length > 0 && (
            <EnvWarningBanner
              conflicts={envConflicts}
              onDismiss={() => {
                setShowEnvBanner(false);
                sessionStorage.setItem("env_banner_dismissed", "true");
              }}
              onDeleted={async () => {
                try {
                  const allConflicts = await checkAllEnvConflicts();
                  const flatConflicts = Object.values(allConflicts).flat();
                  setEnvConflicts(flatConflicts);
                  if (flatConflicts.length === 0) {
                    setShowEnvBanner(false);
                  }
                } catch (error) {
                  console.error(
                    "[App] Failed to re-check conflicts after deletion:",
                    error,
                  );
                }
              }}
            />
          )}
          <div
            key={
              currentView === "settings"
                ? "settings"
                : isAppPage(currentView)
                  ? "app"
                  : currentView
            }
            className="flex min-h-0 flex-1 flex-col"
          >
            {renderContent()}
          </div>
        </main>
      </div>

      {/* 添加 / 编辑统一供应商：应用页与卡片共用一个表单。统一供应商会同步到全部
          五个应用，所以任意应用页打开都在改同一份配置，不再有单独的添加页或管理页。 */}
      {(isAddOpen || editingUniversal) && (
        <UniversalProviderFormModal
          key={editingUniversal?.id ?? "new"}
          isOpen
          editingProvider={editingUniversal}
          subtitle={
            editingUniversal
              ? undefined
              : `${APP_DISPLAY_NAME[activeApp]} · ${t("universalProvider.add", {
                  defaultValue: "添加统一供应商",
                })}`
          }
          onClose={() => {
            setEditingUniversalId(null);
            setIsAddOpen(false);
          }}
          onSave={(provider) => void handleUniversalSave(provider)}
          onSaveAndSync={(provider) => void handleUniversalSave(provider)}
          onDelete={() =>
            editingUniversal && setPendingUniversalDelete(editingUniversal)
          }
        />
      )}

      {/* 删除统一供应商：会连带删掉它在各应用里的子卡，所以要确认一次。 */}
      <ConfirmDialog
        isOpen={Boolean(pendingUniversalDelete)}
        title={t("universalProvider.deleteConfirmTitle", {
          defaultValue: "删除统一供应商",
        })}
        message={t("universalProvider.deleteConfirmDescription", {
          defaultValue: `确定要删除 "${pendingUniversalDelete?.name ?? ""}" 吗？这将同时删除它在各应用中生成的供应商配置。`,
          name: pendingUniversalDelete?.name ?? "",
        })}
        confirmText={t("common.delete", { defaultValue: "删除" })}
        variant="destructive"
        onConfirm={() => void handleUniversalDelete()}
        onCancel={() => setPendingUniversalDelete(null)}
      />

      {effectiveUsageProvider && (
        <UsageScriptModal
          key={effectiveUsageProvider.id}
          provider={effectiveUsageProvider}
          appId={activeApp}
          isOpen={Boolean(usageProvider)}
          onClose={() => setUsageProvider(null)}
          onSave={(script) => {
            if (usageProvider) {
              void saveUsageScript(usageProvider, script);
            }
          }}
        />
      )}

      <ConfirmDialog
        isOpen={Boolean(confirmAction)}
        title={t("confirm.deleteProvider")}
        message={confirmActionMessage}
        variant="destructive"
        onConfirm={() => void handleConfirmAction()}
        onCancel={() => setConfirmAction(null)}
      />

      <ConfirmDialog
        isOpen={pendingLeave !== null}
        title={t("common.unsavedLeaveTitle")}
        message={t("common.unsavedLeaveMessage")}
        confirmText={t("common.unsavedLeaveConfirm")}
        cancelText={t("common.unsavedLeaveCancel")}
        zIndex="top"
        onConfirm={() => {
          const proceed = pendingLeave;
          setPendingLeave(null);
          discardUnsavedChanges();
          proceed?.();
        }}
        onCancel={() => setPendingLeave(null)}
      />

      <DeepLinkImportDialog />
      <FirstRunNoticeDialog />
      <NewLayoutDialog />
      <WhatsNewNotice />
    </WindowControlsContext.Provider>
  );
}

export default App;
