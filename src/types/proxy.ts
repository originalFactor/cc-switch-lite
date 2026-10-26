export interface ProxyConfig {
  listen_address: string;
  listen_port: number;
  max_retries: number;
  request_timeout: number;
  enable_logging: boolean;
  live_takeover_active?: boolean;
  // 超时配置
  streaming_first_byte_timeout: number;
  streaming_idle_timeout: number;
  non_streaming_timeout: number;
}

export interface ProxyStatus {
  running: boolean;
  address: string;
  port: number;
  active_connections: number;
  total_requests: number;
  success_requests: number;
  failed_requests: number;
  success_rate: number;
  uptime_seconds: number;
  current_provider: string | null;
  current_provider_id: string | null;
  last_request_at: string | null;
  last_error: string | null;
  failover_count: number;
  active_targets?: ActiveTarget[];
}

export interface ActiveTarget {
  app_type: string;
  provider_name: string;
  provider_id: string;
}

export interface ProxyServerInfo {
  address: string;
  port: number;
  started_at: string;
}

export interface ProxyTakeoverStatus {
  claude: boolean;
  "claude-desktop"?: boolean;
  codex: boolean;
  gemini: boolean;
  grokbuild: boolean;
}

/** 应用当前的连接方式。 */
export type AppMode = "direct" | "route";

/** 应用页模式行用的状态（后端 `get_app_mode`）。 */
export interface AppModeView {
  mode: AppMode;
  /** 客户端文件指着代理（CC Switch 运行时为真） */
  attached: boolean;
  /** 路由目标；直连模式下是上次路由的那家 */
  routeProviderId: string | null;
  /** 直连那家：直连时写进客户端的、回到直连时写回的 */
  directProviderId: string | null;
}

/** 启动时没能接上代理、已退回直连的应用。 */
export interface StartupAttachFailure {
  appType: string;
  error: string;
}

/**
 * 还在用旧模型列表的 Codex 客户端（它们只在启动时读模型目录）：`daemon` 是 `codex` 命令行连的
 * 托管守护进程，可以一键重启；`others` 是桌面版、编辑器插件，要用户自己彻底退出再开。
 */
export interface CodexStaleClients {
  daemon: boolean;
  others: boolean;
  /** File-store account changed after startup; the process may cache the old login. */
  auth?: boolean;
  /**
   * 看不到桌面版、编辑器插件的进程（Windows）：`others` 只表示上次确认之后目录或登录变了，
   * 不知道有没有开着。关掉提示时要告诉后端（`acknowledgeCodexStaleClients`）。
   */
  unverified?: boolean;
}

/** Codex 客户端是否可能还缓存着旧登录或旧模型列表（后端 `get_codex_stale_clients`）。 */
export interface CodexStaleClientsView {
  staleClients: CodexStaleClients;
  /** `staleClients` 对应的目录和登录：关掉提示后这个值变了（目录又改了），重新显示。 */
  staleRevision: string;
}

/** 重启 Codex 守护进程的结果：`notRunning` 表示它没在运行，什么都没做。 */
export type CodexDaemonRestartOutcome = "restarted" | "notRunning";

export interface ProviderHealth {
  provider_id: string;
  app_type: string;
  is_healthy: boolean;
  consecutive_failures: number;
  last_success_at: string | null;
  last_failure_at: string | null;
  last_error: string | null;
  updated_at: string;
}

// 熔断器相关类型
export interface CircuitBreakerConfig {
  failureThreshold: number;
  successThreshold: number;
  timeoutSeconds: number;
  errorRateThreshold: number;
  minRequests: number;
}

export type CircuitState = "closed" | "open" | "half_open";

export interface CircuitBreakerStats {
  state: CircuitState;
  consecutiveFailures: number;
  consecutiveSuccesses: number;
  totalRequests: number;
  failedRequests: number;
}

// 供应商健康状态枚举
export enum ProviderHealthStatus {
  Healthy = "healthy",
  Degraded = "degraded",
  Failed = "failed",
  Unknown = "unknown",
}

// 扩展 ProviderHealth 以包含前端计算的状态
export interface ProviderHealthWithStatus extends ProviderHealth {
  status: ProviderHealthStatus;
  circuitState?: CircuitState;
}

export interface ProxyUsageRecord {
  provider_id: string;
  app_type: string;
  endpoint: string;
  request_tokens: number | null;
  response_tokens: number | null;
  status_code: number;
  latency_ms: number;
  error: string | null;
  timestamp: string;
}

// 故障转移队列条目
export interface FailoverQueueItem {
  providerId: string;
  providerName: string;
  providerNotes?: string;
  sortIndex?: number;
}

// 全局代理配置（统一字段，三行镜像）
export interface GlobalProxyConfig {
  proxyEnabled: boolean;
  listenAddress: string;
  listenPort: number;
  enableLogging: boolean;
}

// 应用级代理配置（每个 app 独立）
export interface AppProxyConfig {
  appType: string;
  enabled: boolean;
  autoFailoverEnabled: boolean;
  maxRetries: number;
  streamingFirstByteTimeout: number;
  streamingIdleTimeout: number;
  nonStreamingTimeout: number;
  circuitFailureThreshold: number;
  circuitSuccessThreshold: number;
  circuitTimeoutSeconds: number;
  circuitErrorRateThreshold: number;
  circuitMinRequests: number;
}
