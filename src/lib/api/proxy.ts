import { invoke } from "@tauri-apps/api/core";
import type {
  ProxyStatus,
  ProxyServerInfo,
  ProxyTakeoverStatus,
  GlobalProxyConfig,
  AppProxyConfig,
  CodexDaemonRestartOutcome,
  CodexStaleClientsView,
  AppModeView,
  StartupAttachFailure,
} from "@/types/proxy";

export const proxyApi = {
  // ========== 代理服务器控制 API ==========

  // 启动代理服务器
  async startProxyServer(): Promise<ProxyServerInfo> {
    return invoke("start_proxy_server");
  },

  // 停止代理服务器（不恢复已接管配置）
  async stopProxyServer(): Promise<void> {
    return invoke("stop_proxy_server");
  },

  // 停止代理服务器并恢复配置
  async stopProxyWithRestore(): Promise<void> {
    return invoke("stop_proxy_with_restore");
  },

  // 获取代理服务器状态
  async getProxyStatus(): Promise<ProxyStatus> {
    return invoke("get_proxy_status");
  },

  // ========== 接管状态 API ==========

  // 获取各应用接管状态
  async getProxyTakeoverStatus(): Promise<ProxyTakeoverStatus> {
    return invoke("get_proxy_takeover_status");
  },

  // 为指定应用进入/退出路由模式。route 是这次的路由目标，不传沿用上次的路由
  async setProxyTakeoverForApp(
    appType: string,
    enabled: boolean,
    route?: string | null,
  ): Promise<void> {
    return invoke("set_proxy_takeover_for_app", {
      appType,
      enabled,
      route: route ?? null,
    });
  },

  // 指定路由目标：直连时只记下来、下次进入路由模式时用它，已经在路由模式时当场生效
  async setProxyRoute(appType: string, providerId: string): Promise<void> {
    return invoke("set_proxy_route", { appType, providerId });
  },

  // 应用页模式行：生效的模式、路由目标（直连时是上次路由的那家）、直连那家
  async getAppMode(appType: string): Promise<AppModeView> {
    return invoke("get_app_mode", { appType });
  },

  // 启动时没能接上代理、已退回直连的应用（取一次就清空）
  async takeStartupAttachFailures(): Promise<StartupAttachFailure[]> {
    return invoke("take_startup_attach_failures");
  },

  // 直连供应商：路由模式下退出路由时写回的那家（和路由到的那家互相独立）
  async getDirectProvider(appType: string): Promise<string | null> {
    return invoke("get_direct_provider", { appType });
  },

  // ========== Codex 客户端提示 API ==========

  // Codex 客户端是否可能还缓存着旧登录或旧模型列表；其他应用返回 null
  async getCodexStaleClients(
    appType: string,
  ): Promise<CodexStaleClientsView | null> {
    return invoke("get_codex_stale_clients", { appType });
  },

  // 重启 Codex 的托管守护进程（codex 命令行连的那个），让它重读模型目录。会中断正在运行的
  // 任务，只在用户确认之后调
  async restartCodexAppServerDaemon(): Promise<CodexDaemonRestartOutcome> {
    return invoke("restart_codex_app_server_daemon");
  },

  // 关掉看不到进程时出的「Codex 可能还在用旧的」提示：后端记下现在的模型目录和登录，同一份
  // 不再提示
  async acknowledgeCodexStaleClients(): Promise<void> {
    return invoke("acknowledge_codex_stale_clients");
  },

  // ========== v3+ 全局/应用级配置 API ==========

  // 获取全局代理配置
  async getGlobalProxyConfig(): Promise<GlobalProxyConfig> {
    return invoke("get_global_proxy_config");
  },

  // 更新全局代理配置
  async updateGlobalProxyConfig(config: GlobalProxyConfig): Promise<void> {
    return invoke("update_global_proxy_config", { config });
  },

  // 获取客户端地址（写给客户端的代理主机，空表示按监听地址推）
  async getProxyClientHost(): Promise<string> {
    return invoke("get_proxy_client_host");
  },

  // 保存客户端地址，并按新地址重写接上路由的客户端
  async setProxyClientHost(host: string): Promise<void> {
    return invoke("set_proxy_client_host", { host });
  },

  // 获取指定应用的代理配置
  async getProxyConfigForApp(appType: string): Promise<AppProxyConfig> {
    return invoke("get_proxy_config_for_app", { appType });
  },

  // 更新指定应用的代理配置
  async updateProxyConfigForApp(config: AppProxyConfig): Promise<void> {
    return invoke("update_proxy_config_for_app", { config });
  },

  // ========== 计费默认配置 API ==========

  // 获取计费模式来源
  async getPricingModelSource(appType: string): Promise<string> {
    return invoke("get_pricing_model_source", { appType });
  },

  // 设置计费模式来源
  async setPricingModelSource(appType: string, value: string): Promise<void> {
    return invoke("set_pricing_model_source", { appType, value });
  },
};
