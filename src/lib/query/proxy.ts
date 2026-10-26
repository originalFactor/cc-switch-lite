import { useQuery, useMutation, useQueryClient } from "@tanstack/react-query";
import { proxyApi } from "@/lib/api/proxy";
import { toast } from "@/lib/toast";
import { useTranslation } from "react-i18next";
import type {
  AppModeView,
  GlobalProxyConfig,
  AppProxyConfig,
  ProxyTakeoverStatus,
} from "@/types/proxy";
import { extractErrorMessage } from "@/utils/errorUtils";

export const proxyKeys = {
  status: ["proxyStatus"] as const,
  takeoverStatus: ["proxyTakeoverStatus"] as const,
  globalConfig: ["globalProxyConfig"] as const,
  clientHost: ["proxyClientHost"] as const,
  appConfig: (appType: string) => ["appProxyConfig", appType] as const,
};

// ========== 代理服务器状态 Hooks ==========

/**
 * 获取代理服务器状态
 */
export function useProxyStatusQuery() {
  return useQuery({
    queryKey: proxyKeys.status,
    queryFn: () => proxyApi.getProxyStatus(),
    // 仅在服务运行时轮询
    refetchInterval: (query) => (query.state.data?.running ? 2000 : false),
    // 保持之前的数据，避免闪烁
    placeholderData: (previousData) => previousData,
  });
}

/**
 * 获取各应用接管状态
 */
export function useProxyTakeoverStatus(poll = true) {
  return useQuery({
    queryKey: proxyKeys.takeoverStatus,
    queryFn: () => proxyApi.getProxyTakeoverStatus(),
    refetchInterval: poll ? 2000 : false,
    ...(poll
      ? {}
      : {
          placeholderData: (previousData: ProxyTakeoverStatus | undefined) =>
            previousData,
        }),
  });
}

/**
 * 应用的模式状态（直连 / 路由、路由目标、直连那家）。放在 ["providers", appId] 前缀下：
 * 进出模式、切换供应商时随供应商列表一起失效。
 */
export function useAppMode(appType: string, enabled = true) {
  return useQuery({
    queryKey: ["providers", appType, "mode"] as const,
    queryFn: () => proxyApi.getAppMode(appType),
    enabled,
    placeholderData: (previous: AppModeView | undefined) => previous,
  });
}

/**
 * 直连供应商（路由模式下退出路由时写回的那家）。
 * 放在 ["providers", appId] 前缀下：切换、编辑供应商时随供应商列表一起失效。
 */
export function useDirectProviderId(appType: string, enabled: boolean) {
  return useQuery({
    queryKey: ["providers", appType, "direct"] as const,
    queryFn: () => proxyApi.getDirectProvider(appType),
    enabled,
  });
}

/**
 * Codex 客户端是否可能还缓存着旧登录或旧模型列表。放在 ["providers", appId] 前缀下：编辑、
 * 切换供应商时随列表一起失效。
 */
export function useCodexStaleClients(appType: string, enabled: boolean) {
  return useQuery({
    queryKey: ["providers", appType, "codexStaleClients"] as const,
    queryFn: () => proxyApi.getCodexStaleClients(appType),
    enabled: enabled && appType === "codex",
  });
}

/**
 * 重启 Codex 的托管守护进程，让它重读模型目录。结束后重新查上面的提示：重启成功时
 * 「还在用旧模型列表」的提示随之消失。
 */
export function useRestartCodexAppServerDaemon() {
  const queryClient = useQueryClient();
  const { t } = useTranslation();

  return useMutation({
    mutationFn: () => proxyApi.restartCodexAppServerDaemon(),
    onSuccess: (outcome) => {
      if (outcome === "notRunning") {
        toast.info(t("proxy.codexStale.notRunning"), {
          closeButton: true,
        });
        return;
      }
      toast.success(t("proxy.codexStale.restarted"), {
        closeButton: true,
      });
    },
    onError: (error: unknown) => {
      toast.error(
        t("proxy.codexStale.failed", {
          error: extractErrorMessage(error),
        }),
      );
    },
    onSettled: () => {
      queryClient.invalidateQueries({
        queryKey: ["providers", "codex", "codexStaleClients"],
      });
      queryClient.invalidateQueries({ queryKey: ["providers", "codex"] });
    },
  });
}

/**
 * 关掉看不到进程时出的「Codex 可能还在用旧的」提示：后端记下现在这份，之后再变才提示。
 */
export function useAcknowledgeCodexStaleClients() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: () => proxyApi.acknowledgeCodexStaleClients(),
    onSettled: () => {
      queryClient.invalidateQueries({
        queryKey: ["providers", "codex", "codexStaleClients"],
      });
      queryClient.invalidateQueries({ queryKey: ["providers", "codex"] });
    },
  });
}

// ========== 代理服务器控制 Hooks ==========

/**
 * 设置应用接管状态
 */
export function useSetProxyTakeoverForApp() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: ({
      appType,
      enabled,
      route,
    }: {
      appType: string;
      enabled: boolean;
      route?: string | null;
    }) => proxyApi.setProxyTakeoverForApp(appType, enabled, route),
    onSuccess: (_data, variables) => {
      queryClient.invalidateQueries({ queryKey: proxyKeys.takeoverStatus });
      // 进出路由模式会改「当前」显示的供应商（路由模式下是路由到的那家）。
      queryClient.invalidateQueries({
        queryKey: ["providers", variables.appType],
      });
    },
  });
}

// ========== v3+ 全局/应用级配置 Hooks ==========

/**
 * 获取全局代理配置
 */
export function useGlobalProxyConfig() {
  return useQuery({
    queryKey: proxyKeys.globalConfig,
    queryFn: () => proxyApi.getGlobalProxyConfig(),
  });
}

/**
 * 更新全局代理配置
 */
export function useUpdateGlobalProxyConfig() {
  const queryClient = useQueryClient();
  const { t } = useTranslation();

  return useMutation({
    mutationFn: (config: GlobalProxyConfig) =>
      proxyApi.updateGlobalProxyConfig(config),
    onSuccess: () => {
      toast.success(t("proxy.settings.toast.saved"), { closeButton: true });
      queryClient.invalidateQueries({ queryKey: proxyKeys.globalConfig });
      queryClient.invalidateQueries({ queryKey: proxyKeys.status });
    },
    onError: (error: Error) => {
      toast.error(
        t("proxy.settings.toast.saveFailed", { error: error.message }),
      );
    },
  });
}

/**
 * 获取客户端地址
 */
export function useProxyClientHost() {
  return useQuery({
    queryKey: proxyKeys.clientHost,
    queryFn: () => proxyApi.getProxyClientHost(),
  });
}

/**
 * 保存客户端地址（后端会重写接上路由的客户端）
 */
export function useSetProxyClientHost() {
  const queryClient = useQueryClient();
  const { t } = useTranslation();

  return useMutation({
    mutationFn: (host: string) => proxyApi.setProxyClientHost(host),
    onSuccess: () => {
      toast.success(t("proxy.settings.toast.saved"), { closeButton: true });
    },
    onError: (error: Error) => {
      toast.error(
        t("proxy.settings.toast.saveFailed", { error: error.message }),
      );
    },
    // 重写失败时地址也已经存下了，两种情况都刷新
    onSettled: () => {
      queryClient.invalidateQueries({ queryKey: proxyKeys.clientHost });
    },
  });
}

/**
 * 获取指定应用的代理配置
 */
export function useAppProxyConfig(appType: string) {
  return useQuery({
    queryKey: proxyKeys.appConfig(appType),
    queryFn: () => proxyApi.getProxyConfigForApp(appType),
    enabled: !!appType,
  });
}

/**
 * 更新指定应用的代理配置
 */
export function useUpdateAppProxyConfig() {
  const queryClient = useQueryClient();
  const { t } = useTranslation();

  return useMutation({
    mutationFn: (config: AppProxyConfig) =>
      proxyApi.updateProxyConfigForApp(config),
    onSuccess: (_, variables) => {
      toast.success(t("proxy.settings.toast.saved"), { closeButton: true });
      queryClient.invalidateQueries({
        queryKey: proxyKeys.appConfig(variables.appType),
      });
      queryClient.invalidateQueries({
        queryKey: ["autoFailoverEnabled", variables.appType],
      });
      queryClient.invalidateQueries({ queryKey: ["circuitBreakerConfig"] });
      queryClient.invalidateQueries({ queryKey: proxyKeys.status });
    },
    onError: (error: Error) => {
      toast.error(
        t("proxy.settings.toast.saveFailed", { error: error.message }),
      );
    },
  });
}
