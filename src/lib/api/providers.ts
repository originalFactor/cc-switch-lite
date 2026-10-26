import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type {
  AvailableModel,
  Provider,
  ProviderMeta,
  UniversalProvider,
  UniversalProvidersMap,
} from "@/types";
import type { AppId } from "./types";

export interface ProviderSortUpdate {
  id: string;
  sortIndex: number;
}

export interface ProviderSwitchEvent {
  appType: AppId;
  providerId: string;
}

export interface SwitchResult {
  warnings: string[];
}

export interface OpenTerminalOptions {
  cwd?: string;
}

export interface ClaudeDesktopStatus {
  supported: boolean;
  configured: boolean;
  appliedId?: string | null;
  profilePath?: string | null;
  configLibraryPath?: string | null;
  mode?: "direct" | "proxy" | null;
  expectedBaseUrl?: string | null;
  actualBaseUrl?: string | null;
  proxyRunning: boolean;
  staleRawModels: boolean;
  missingRouteMappings: boolean;
  gatewayTokenConfigured: boolean;
}

/** 编辑器保存时，live 里的键在编辑期间被别的程序改过怎么办。 */
export type EditorConflictPolicy = "refuse" | "keepMine" | "keepTheirs";

/** 编辑器保存随供应商一起提交：打开时显示的完整配置，用来三方比较。 */
export interface ProviderEditorSave {
  base: Record<string, unknown>;
  /** 新增对话框：投影成 `base` 的那份草稿（自定义模板）。后端按它区分模板带的和从 live 带进来的字段。 */
  draft?: Record<string, unknown>;
  onConflict?: EditorConflictPolicy;
}

/** 行里保存着、但不随切换生效的字段。 */
export interface ProviderEditorInactiveField {
  path: string[];
  value: unknown;
}

/** 编辑器底部 JSON 的显示内容：切到这个供应商之后配置文件会是什么样。 */
export interface ProviderEditorView {
  settings: Record<string, unknown>;
  inactive: ProviderEditorInactiveField[];
}

export interface ClaudeDesktopDefaultRoute {
  routeId: string;
  envKey: string;
  supports1m: boolean;
}

export const providersApi = {
  async getAll(appId: AppId): Promise<Record<string, Provider>> {
    return await invoke("get_providers", { app: appId });
  },

  async getCurrent(appId: AppId): Promise<string> {
    return await invoke("get_current_provider", { app: appId });
  },

  async add(
    provider: Provider,
    appId: AppId,
    addToLive?: boolean,
    editorSave?: ProviderEditorSave,
  ): Promise<boolean> {
    return await invoke("add_provider", {
      provider,
      app: appId,
      addToLive,
      ...(editorSave ? { editorSave } : {}),
    });
  },

  async update(
    provider: Provider,
    appId: AppId,
    originalId?: string,
    editorSave?: ProviderEditorSave,
  ): Promise<boolean> {
    return await invoke("update_provider", {
      provider,
      app: appId,
      originalId,
      ...(editorSave ? { editorSave } : {}),
    });
  },

  /**
   * 切换式应用的编辑器显示内容。编辑已有供应商时传 `providerId`：后端按库里那一行认
   * 官方卡。`meta` 保留 Codex Copilot 等供应商的认证和协议身份，新增时取表单里的身份。
   */
  async getEditorView(
    appId: AppId,
    settingsConfig: Record<string, unknown>,
    category?: string,
    providerId?: string,
    meta?: ProviderMeta,
  ): Promise<ProviderEditorView> {
    return await invoke("get_provider_editor_view", {
      app: appId,
      settingsConfig,
      ...(category ? { category } : {}),
      ...(providerId ? { providerId } : {}),
      ...(meta ? { meta } : {}),
    });
  },

  async delete(id: string, appId: AppId): Promise<boolean> {
    return await invoke("delete_provider", { id, app: appId });
  },

  async switch(id: string, appId: AppId): Promise<SwitchResult> {
    return await invoke("switch_provider", { id, app: appId });
  },

  async importDefault(appId: AppId): Promise<boolean> {
    return await invoke("import_default_config", { app: appId });
  },

  async importClaudeDesktopFromClaude(): Promise<number> {
    return await invoke("import_claude_desktop_providers_from_claude");
  },

  async ensureClaudeDesktopOfficialProvider(): Promise<boolean> {
    return await invoke("ensure_claude_desktop_official_provider");
  },

  async ensureCodexOfficialProvider(): Promise<boolean> {
    return await invoke("ensure_codex_official_provider");
  },

  async ensureGrokBuildOfficialProvider(): Promise<boolean> {
    return await invoke("ensure_grokbuild_official_provider");
  },

  async getClaudeDesktopStatus(): Promise<ClaudeDesktopStatus> {
    return await invoke("get_claude_desktop_status");
  },

  async getClaudeDesktopDefaultRoutes(): Promise<ClaudeDesktopDefaultRoute[]> {
    return await invoke("get_claude_desktop_default_routes");
  },

  async updateTrayMenu(): Promise<boolean> {
    return await invoke("update_tray_menu");
  },

  async updateSortOrder(
    updates: ProviderSortUpdate[],
    appId: AppId,
  ): Promise<boolean> {
    return await invoke("update_providers_sort_order", { updates, app: appId });
  },

  async onSwitched(
    handler: (event: ProviderSwitchEvent) => void,
  ): Promise<UnlistenFn> {
    return await listen("provider-switched", (event) => {
      const payload = event.payload as ProviderSwitchEvent;
      handler(payload);
    });
  },

  /**
   * 打开指定提供商的终端
   * 任何提供商都可以打开终端，不受是否为当前激活提供商的限制
   * 终端会使用该提供商特定的 API 配置，不影响全局设置
   */
  async openTerminal(
    providerId: string,
    appId: AppId,
    options?: OpenTerminalOptions,
  ): Promise<boolean> {
    const { cwd } = options ?? {};
    return await invoke("open_provider_terminal", {
      providerId,
      app: appId,
      cwd,
    });
  },
};

// ============================================================================
// 统一供应商（Universal Provider）API
// ============================================================================

export const universalProvidersApi = {
  /**
   * 获取所有统一供应商
   */
  async getAll(): Promise<UniversalProvidersMap> {
    return await invoke("get_universal_providers");
  },

  /**
   * 获取单个统一供应商
   */
  async get(id: string): Promise<UniversalProvider | null> {
    return await invoke("get_universal_provider", { id });
  },

  /**
   * 添加或更新统一供应商
   */
  async upsert(provider: UniversalProvider): Promise<boolean> {
    return await invoke("upsert_universal_provider", { provider });
  },

  /**
   * 删除统一供应商
   */
  async delete(id: string): Promise<boolean> {
    return await invoke("delete_universal_provider", { id });
  },

  /**
   * 手动同步统一供应商到各应用
   */
  async sync(id: string): Promise<boolean> {
    return await invoke("sync_universal_provider", { id });
  },

  /**
   * 拉取可用模型列表（`/v1/models` + oh-my-pi 模型目录补全）。
   *
   * 传 `id` 时用已存那家的地址与 Key，并把结果写回它；新建时还没有 id，改传
   * `baseUrl`/`apiKey` 现拉，结果只返回给表单。
   */
  async refreshModels(target: {
    id?: string;
    baseUrl?: string;
    apiKey?: string;
  }): Promise<AvailableModel[]> {
    return await invoke("refresh_universal_models", target);
  },
};
