use http::header::{HeaderValue, InvalidHeaderValue};
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;

// SSOT 模式：不再写供应商副本文件

/// 供应商结构体
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Provider {
    pub id: String,
    pub name: String,
    #[serde(rename = "settingsConfig")]
    pub settings_config: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "websiteUrl")]
    pub website_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub category: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "createdAt")]
    pub created_at: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "sortIndex")]
    pub sort_index: Option<usize>,
    /// 备注信息
    #[serde(skip_serializing_if = "Option::is_none")]
    pub notes: Option<String>,
    /// 供应商元数据（不写入 live 配置，仅存于 ~/.cc-switch/config.json）
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<ProviderMeta>,
    /// 图标名称（如 "openai", "anthropic"）
    #[serde(skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    /// 图标颜色（Hex 格式，如 "#00A67E"）
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "iconColor")]
    pub icon_color: Option<String>,
    /// 是否加入故障转移队列
    #[serde(default)]
    #[serde(rename = "inFailoverQueue")]
    pub in_failover_queue: bool,
}

impl Provider {
    /// 从现有ID创建供应商
    pub fn with_id(
        id: String,
        name: String,
        settings_config: Value,
        website_url: Option<String>,
    ) -> Self {
        Self {
            id,
            name,
            settings_config,
            website_url,
            category: None,
            created_at: None,
            sort_index: None,
            notes: None,
            meta: None,
            icon: None,
            icon_color: None,
            in_failover_queue: false,
        }
    }

    /// Whether the provider form's "auth field" was explicitly set to
    /// ANTHROPIC_API_KEY. The form only persists `meta.apiKeyField` for the
    /// non-default choice, so `None` means the default ANTHROPIC_AUTH_TOKEN.
    pub fn claude_uses_api_key_field(&self) -> bool {
        self.meta
            .as_ref()
            .and_then(|m| m.api_key_field.as_deref())
            .map(|field| field.eq_ignore_ascii_case("ANTHROPIC_API_KEY"))
            .unwrap_or(false)
    }

    pub fn codex_fast_mode_enabled(&self) -> bool {
        self.meta
            .as_ref()
            .map(|m| m.codex_fast_mode_enabled())
            .unwrap_or(false)
    }

    pub fn has_usage_script_enabled(&self) -> bool {
        self.meta
            .as_ref()
            .and_then(|m| m.usage_script.as_ref())
            .map(|s| s.enabled)
            .unwrap_or(false)
    }

    /// Resolve `(base_url, api_key)` for usage queries (native balance /
    /// coding-plan and the JS-script `{{apiKey}}`/`{{baseUrl}}` fallback)
    /// from the stored provider config.
    ///
    /// Each app persists credentials in a different shape, so callers must pass
    /// the owning app type. This mirrors the frontend `getProviderCredentials`
    /// in `UsageScriptModal.tsx`.
    pub fn resolve_usage_credentials(
        &self,
        app_type: &crate::app_config::AppType,
    ) -> (String, String) {
        use crate::app_config::AppType;

        let settings = &self.settings_config;
        let str_at =
            |value: Option<&Value>| value.and_then(|v| v.as_str()).unwrap_or("").to_string();

        // First present, non-empty string among `keys`, mirroring the frontend's
        // `a || b || c` — JS `||` skips empty strings, and presets seed fields like
        // `ANTHROPIC_AUTH_TOKEN` as present-but-empty placeholders, so a plain
        // `.get().or_else()` chain (which only skips *absent* keys) would stop short.
        fn first_non_empty(env: Option<&Value>, keys: &[&str]) -> String {
            let Some(env) = env else {
                return String::new();
            };
            for key in keys {
                if let Some(s) = env.get(key).and_then(|v| v.as_str()) {
                    if !s.is_empty() {
                        return s.to_string();
                    }
                }
            }
            String::new()
        }

        let (base_url, api_key) = match app_type {
            // Codex keeps its key in `auth.OPENAI_API_KEY` and its base URL
            // inside a TOML `config` string, not in an `env` map.
            AppType::Codex => {
                let auth = settings.get("auth");
                let config_text = settings.get("config").and_then(|v| v.as_str());
                let api_key = crate::codex_config::extract_codex_api_key(auth, config_text)
                    .unwrap_or_default();
                let base_url = config_text
                    .and_then(crate::codex_config::extract_codex_base_url)
                    .unwrap_or_default();
                (base_url, api_key)
            }
            // Gemini uses Google-specific env keys (with a legacy GOOGLE_API_KEY fallback).
            AppType::Gemini => {
                let env = settings.get("env");
                let base_url = str_at(env.and_then(|e| e.get("GOOGLE_GEMINI_BASE_URL")));
                let api_key = first_non_empty(env, &["GEMINI_API_KEY", "GOOGLE_API_KEY"]);
                (base_url, api_key)
            }
            // GrokBuild 的 base_url 与 api_key 必须各自解析：extract_credentials 在
            // 凭据缺失时整个 Option 变 None，一并 unwrap_or_default 会把明明写在
            // 配置里的 base_url 也清成空串。凭据缺失是常态（env_key 指向的变量在
            // GUI 进程里读不到），端点不该被连坐——否则用量脚本的 {{baseUrl}} 变成
            // 相对路径、余额查询只报「API key is empty」掩盖真因。
            // 与上面 Codex 分支的写法保持一致。
            AppType::GrokBuild => {
                let config_text = settings.get("config").and_then(Value::as_str);
                let base_url = config_text
                    .and_then(crate::grok_config::extract_base_url)
                    .unwrap_or_default();
                let api_key = config_text
                    .and_then(crate::grok_config::extract_credentials)
                    .map(|(_, api_key)| api_key)
                    .unwrap_or_default();
                (base_url, api_key)
            }
            // Claude and Claude Desktop both use the Anthropic-style env map, keeping
            // the OpenRouter/Google key fallbacks the JS-script path relies on.
            // Listed explicitly (not `_`) so a new AppType fails to compile here.
            AppType::Claude | AppType::ClaudeDesktop => {
                let env = settings.get("env");
                let base_url = str_at(env.and_then(|e| e.get("ANTHROPIC_BASE_URL")));
                let api_key = first_non_empty(
                    env,
                    &[
                        "ANTHROPIC_AUTH_TOKEN",
                        "ANTHROPIC_API_KEY",
                        "OPENROUTER_API_KEY",
                        "GOOGLE_API_KEY",
                    ],
                );
                (base_url, api_key)
            }
        };

        // Normalize like the JS-script path (extract_base_url_from_provider) so a
        // future delegation from services/provider/usage.rs is behavior-preserving
        // and `{{baseUrl}}/path` concatenation never produces a double slash.
        (base_url.trim_end_matches('/').to_string(), api_key)
    }
}

/// 供应商管理器
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ProviderManager {
    pub providers: IndexMap<String, Provider>,
    pub current: String,
}

/// 用量查询脚本配置
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UsageScript {
    pub enabled: bool,
    pub language: String,
    pub code: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timeout: Option<u64>,
    /// 用量查询专用的 API Key（通用模板使用）
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "apiKey")]
    pub api_key: Option<String>,
    /// 用量查询专用的 Base URL（通用和 NewAPI 模板使用）
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "baseUrl")]
    pub base_url: Option<String>,
    /// 访问令牌（用于需要登录的接口，NewAPI 模板使用）
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "accessToken")]
    pub access_token: Option<String>,
    /// 用户ID（用于需要用户标识的接口，NewAPI 模板使用）
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "userId")]
    pub user_id: Option<String>,
    /// 模板类型（用于后端判断验证规则）
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "templateType")]
    pub template_type: Option<String>,
    /// 自动查询间隔（单位：分钟，0 表示禁用自动查询）
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "autoQueryInterval")]
    pub auto_query_interval: Option<u64>,
    /// Coding Plan 供应商标识（如 "kimi", "zhipu", "minimax"）
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "codingPlanProvider")]
    pub coding_plan_provider: Option<String>,
    /// 火山方舟控制面 OpenAPI 的 AccessKey ID（用量查询签名用，与推理 Key 是两套凭据）
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "accessKeyId")]
    pub access_key_id: Option<String>,
    /// 火山方舟控制面 OpenAPI 的 SecretAccessKey（同上）
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "secretAccessKey")]
    pub secret_access_key: Option<String>,
    /// 智谱团队套餐（Team Plan）的组织 ID（用量查询请求头 bigmodel-organization）
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "teamOrganizationId")]
    pub team_organization_id: Option<String>,
    /// 智谱团队套餐（Team Plan）的项目 ID（用量查询请求头 bigmodel-project）
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "teamProjectId")]
    pub team_project_id: Option<String>,
}

/// 用量数据
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UsageData {
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "planName")]
    pub plan_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extra: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "isValid")]
    pub is_valid: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "invalidMessage")]
    pub invalid_message: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub used: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub remaining: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unit: Option<String>,
}

/// 用量查询结果（支持多套餐）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UsageResult {
    pub success: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<Vec<UsageData>>, // 支持返回多个套餐
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// Claude Desktop 3P 写入模式。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ClaudeDesktopMode {
    Direct,
    Proxy,
}

/// Claude Desktop 本地路由模式下暴露给 Desktop 的安全模型路由。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ClaudeDesktopModelRoute {
    /// 真实上游模型名，只保存在 CC Switch 内部，不写入 Claude Desktop profile。
    pub model: String,
    /// Claude Desktop 模型菜单显示名；写入 profile 的 `labelOverride`。
    #[serde(rename = "labelOverride", skip_serializing_if = "Option::is_none")]
    pub label_override: Option<String>,
    /// Claude Desktop 3P 识别的 1M 上下文能力标记。
    #[serde(rename = "supports1m", skip_serializing_if = "Option::is_none")]
    pub supports_1m: Option<bool>,
}

/// Codex Responses -> Chat Completions 的 reasoning 能力描述。
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct CodexChatReasoningConfig {
    #[serde(rename = "supportsThinking", skip_serializing_if = "Option::is_none")]
    pub supports_thinking: Option<bool>,
    #[serde(rename = "supportsEffort", skip_serializing_if = "Option::is_none")]
    pub supports_effort: Option<bool>,
    #[serde(rename = "thinkingParam", skip_serializing_if = "Option::is_none")]
    pub thinking_param: Option<String>,
    #[serde(rename = "effortParam", skip_serializing_if = "Option::is_none")]
    pub effort_param: Option<String>,
    #[serde(rename = "effortValueMode", skip_serializing_if = "Option::is_none")]
    pub effort_value_mode: Option<String>,
    /// 声明性字段：标注上游 reasoning 的回传位置（reasoning_content / reasoning /
    /// reasoning_details / think_tags）。当前响应侧 `extract_reasoning_field_text`
    /// 靠穷举字段提取、并不读取本字段；保留作文档说明与未来按格式分发（如 think_tags）的预留。
    #[serde(rename = "outputFormat", skip_serializing_if = "Option::is_none")]
    pub output_format: Option<String>,
    /// 运行时字段（不持久化、不进 meta）：当前请求模型在平台侧声明的合法 effort
    /// 档位，由 resolve 按请求模型从供应商 `settings_config.modelCatalog` 的
    /// `reasoningLevels`（逐模型声明，见 #6228）查表填充。仅 "zen" 值映射消费：
    /// Some → 钳到合法档；None → 不发 effort 字段（模型未收录或为 toggle 型）。
    #[serde(skip)]
    pub effort_levels: Option<Vec<String>>,
}

/// Local proxy request overrides applied after route/protocol transforms.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct LocalProxyRequestOverrides {
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub headers: HashMap<String, String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub body: Option<serde_json::Value>,
}

impl LocalProxyRequestOverrides {
    pub fn is_empty(&self) -> bool {
        self.headers.is_empty() && self.body.is_none()
    }
}

/// 供应商元数据
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ProviderMeta {
    /// 自定义端点列表（按 URL 去重存储）
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub custom_endpoints: HashMap<String, crate::settings::CustomEndpoint>,
    /// 是否在写入 live 时应用通用配置片段
    #[serde(
        rename = "commonConfigEnabled",
        skip_serializing_if = "Option::is_none"
    )]
    pub common_config_enabled: Option<bool>,
    /// Claude Desktop 3P 写入模式：direct（直连）或 proxy（预留）
    #[serde(rename = "claudeDesktopMode", skip_serializing_if = "Option::is_none")]
    pub claude_desktop_mode: Option<ClaudeDesktopMode>,
    /// Claude Desktop proxy 模式的模型路由映射：Claude-safe route -> upstream model。
    #[serde(
        default,
        rename = "claudeDesktopModelRoutes",
        skip_serializing_if = "HashMap::is_empty"
    )]
    pub claude_desktop_model_routes: HashMap<String, ClaudeDesktopModelRoute>,
    /// 用量查询脚本配置
    #[serde(skip_serializing_if = "Option::is_none")]
    pub usage_script: Option<UsageScript>,
    /// 请求地址管理：测速后自动选择最佳端点
    #[serde(rename = "endpointAutoSelect", skip_serializing_if = "Option::is_none")]
    pub endpoint_auto_select: Option<bool>,
    /// 合作伙伴标记（前端使用 isPartner，保持字段名一致）
    #[serde(rename = "isPartner", skip_serializing_if = "Option::is_none")]
    pub is_partner: Option<bool>,
    /// 合作伙伴促销 key，用于识别 PackyCode 等特殊供应商
    #[serde(
        rename = "partnerPromotionKey",
        skip_serializing_if = "Option::is_none"
    )]
    pub partner_promotion_key: Option<String>,
    /// 已停用：供应商级成本倍率。新版不再读取，只为与旧版设备同步时原样往返保留
    #[serde(rename = "costMultiplier", skip_serializing_if = "Option::is_none")]
    pub cost_multiplier: Option<String>,
    /// 已停用：供应商级计费模式覆盖（response/request）。新版只读全局设置，
    /// 该字段只为与旧版设备同步时原样往返保留
    #[serde(rename = "pricingModelSource", skip_serializing_if = "Option::is_none")]
    pub pricing_model_source: Option<String>,
    /// 每日消费限额（USD）
    #[serde(rename = "limitDailyUsd", skip_serializing_if = "Option::is_none")]
    pub limit_daily_usd: Option<String>,
    /// 每月消费限额（USD）
    #[serde(rename = "limitMonthlyUsd", skip_serializing_if = "Option::is_none")]
    pub limit_monthly_usd: Option<String>,
    /// Claude API 格式（仅 Claude 供应商使用）
    /// - "anthropic": 原生 Anthropic Messages API，直接透传
    /// - "openai_chat": OpenAI Chat Completions 格式，需要转换
    /// - "openai_responses": OpenAI Responses API 格式，需要转换
    #[serde(rename = "apiFormat", skip_serializing_if = "Option::is_none")]
    pub api_format: Option<String>,
    /// Claude 认证字段名（"ANTHROPIC_AUTH_TOKEN" 或 "ANTHROPIC_API_KEY"）
    #[serde(rename = "apiKeyField", skip_serializing_if = "Option::is_none")]
    pub api_key_field: Option<String>,
    /// 是否将 base_url 视为完整 API 端点（不拼接 endpoint 路径）
    #[serde(rename = "isFullUrl", skip_serializing_if = "Option::is_none")]
    pub is_full_url: Option<bool>,
    /// Prompt cache key for OpenAI Responses-compatible endpoints.
    /// When set, injected into converted Responses requests to improve cache hit rate.
    /// If not set, Claude -> Responses conversions use a client-provided session/thread
    /// identity when available; generated session IDs are not sent upstream.
    #[serde(rename = "promptCacheKey", skip_serializing_if = "Option::is_none")]
    pub prompt_cache_key: Option<String>,
    /// Session-based prompt-cache routing for Codex Responses -> Chat conversions.
    /// "auto" enables known-compatible upstreams; "enabled" / "disabled" are overrides.
    #[serde(rename = "promptCacheRouting", skip_serializing_if = "Option::is_none")]
    pub prompt_cache_routing: Option<String>,
    /// Codex OAuth FAST mode: inject `service_tier = "priority"` for ChatGPT Codex requests.
    #[serde(rename = "codexFastMode", skip_serializing_if = "Option::is_none")]
    pub codex_fast_mode: Option<bool>,
    /// Codex Responses -> Chat Completions reasoning capability metadata.
    #[serde(rename = "codexChatReasoning", skip_serializing_if = "Option::is_none")]
    pub codex_chat_reasoning: Option<CodexChatReasoningConfig>,
    /// Codex → Anthropic path: whether to emulate the Claude Code client
    /// (User-Agent / anthropic-beta / x-app + injecting the Claude Code system
    /// prompt first line). Disabled by default; only an explicit `true` enables it.
    #[serde(
        rename = "impersonateClaudeCode",
        skip_serializing_if = "Option::is_none"
    )]
    pub impersonate_claude_code: Option<bool>,
    /// Codex → Anthropic path: override the Anthropic `max_tokens` (output ceiling).
    ///
    /// Codex does not forward its `model_max_output_tokens` in the Responses
    /// request body, so without this the path falls back to a conservative
    /// default (8192), which truncates long or thinking-heavy responses
    /// (`stop_reason=max_tokens`). When set (>0), this value is injected as the
    /// request's `max_output_tokens` before conversion, taking precedence over
    /// both any request-supplied value and the default. Kept per-provider on
    /// purpose: a global large default would hard-400 on low-output-ceiling
    /// models/gateways (and that error is non-retryable).
    #[serde(rename = "maxOutputTokens", skip_serializing_if = "Option::is_none")]
    pub max_output_tokens: Option<u64>,
    /// Custom User-Agent for local proxy routing.
    #[serde(rename = "customUserAgent", skip_serializing_if = "Option::is_none")]
    pub custom_user_agent: Option<String>,
    /// Local proxy request overrides applied to the transformed upstream request.
    #[serde(
        rename = "localProxyRequestOverrides",
        skip_serializing_if = "Option::is_none"
    )]
    pub local_proxy_request_overrides: Option<LocalProxyRequestOverrides>,
}

/// 解析 Provider 级自定义 User-Agent 字符串（单一真理来源）。
///
/// 转发（forwarder）、流式检测（stream_check）、获取模型列表（model_fetch）三条路径
/// 共用同一口径，避免出现"某条路径用了 UA、另一条没用 / 报错"的不一致。
///
/// 合法性由 `http::HeaderValue::from_str` 按**字节**判定（`b >= 32 && b != 127 || b == '\t'`），
/// 与前端 `src/lib/userAgent.ts::isValidUserAgentHeader` 严格一致：
/// - `Ok(None)`：未设置或纯空白（trim 后为空）。
/// - `Ok(Some(hv))`：合法。制表符、可见 ASCII（0x20–0x7E）、以及任意非 ASCII 字符
///   （UTF-8 字节均 ≥ 0x80）都合法。
/// - `Err(_)`：仅含控制字符时——除 `\t` 外的 0x00–0x1F（含换行）与 0x7F（DEL）。
///
/// 非法值的处理：三条运行时路径**均静默忽略**（`.ok().flatten()`，绝不让某条路径报错而
/// 另一条放行）；前端在输入框处给出非阻断提示。当前**不在保存时阻断**——deeplink 导入等
/// 非表单路径应宽容，运行时静默忽略即为安全网。
pub fn parse_custom_user_agent(
    raw: Option<&str>,
) -> Result<Option<HeaderValue>, InvalidHeaderValue> {
    match raw.map(str::trim).filter(|s| !s.is_empty()) {
        Some(ua) => HeaderValue::from_str(ua).map(Some),
        None => Ok(None),
    }
}

impl ProviderMeta {
    /// Codex OAuth FAST mode 是否启用。默认关闭，因为 `service_tier="priority"`
    /// 会按更高速率消耗 ChatGPT 订阅配额，用户需显式开启以换取更低延迟。
    pub fn codex_fast_mode_enabled(&self) -> bool {
        self.codex_fast_mode.unwrap_or(false)
    }

    /// 经校验的 Provider 级自定义 User-Agent。见 [`parse_custom_user_agent`]。
    pub fn custom_user_agent_header(&self) -> Result<Option<HeaderValue>, InvalidHeaderValue> {
        parse_custom_user_agent(self.custom_user_agent.as_deref())
    }
}

impl ProviderManager {
    /// 获取所有供应商
    pub fn get_all_providers(&self) -> &IndexMap<String, Provider> {
        &self.providers
    }
}

// ============================================================================
// 统一供应商（Universal Provider）- 跨应用共享配置
// ============================================================================

/// 统一供应商的应用启用状态
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct UniversalProviderApps {
    #[serde(default)]
    pub claude: bool,
    #[serde(default)]
    pub codex: bool,
    #[serde(default)]
    pub gemini: bool,
    #[serde(default, rename = "claudeDesktop")]
    pub claude_desktop: bool,
    #[serde(default, rename = "grokbuild")]
    pub grokbuild: bool,
}

impl UniversalProviderApps {
    /// 全部应用都启用。统一供应商没有 per-app 开关了，一律同步到五个应用；
    /// 这个字段保留只是为了跟旧数据往返和让子卡 id 生成逻辑照旧工作。
    pub fn all() -> Self {
        Self {
            claude: true,
            codex: true,
            gemini: true,
            claude_desktop: true,
            grokbuild: true,
        }
    }

    /// 是否五个应用全开。
    pub fn is_all(&self) -> bool {
        self.claude && self.codex && self.gemini && self.claude_desktop && self.grokbuild
    }
}

/// Claude 模型配置：模型名 + 各档显示名 + 1M 声明 + 上游协议 / 认证字段
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ClaudeModelConfig {
    /// 主模型
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// Haiku 默认模型
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "haikuModel")]
    pub haiku_model: Option<String>,
    /// Sonnet 默认模型
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "sonnetModel")]
    pub sonnet_model: Option<String>,
    /// Opus 默认模型
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "opusModel")]
    pub opus_model: Option<String>,
    /// Fable 默认模型
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "fableModel")]
    pub fable_model: Option<String>,
    /// 主模型声明支持 1M（写 `model[1M]`）
    #[serde(rename = "modelOneM", skip_serializing_if = "Option::is_none")]
    pub model_one_m: Option<bool>,
    #[serde(rename = "haikuModelOneM", skip_serializing_if = "Option::is_none")]
    pub haiku_model_one_m: Option<bool>,
    #[serde(rename = "sonnetModelOneM", skip_serializing_if = "Option::is_none")]
    pub sonnet_model_one_m: Option<bool>,
    #[serde(rename = "opusModelOneM", skip_serializing_if = "Option::is_none")]
    pub opus_model_one_m: Option<bool>,
    #[serde(rename = "fableModelOneM", skip_serializing_if = "Option::is_none")]
    pub fable_model_one_m: Option<bool>,
    /// /model 菜单里的显示名
    #[serde(rename = "modelName", skip_serializing_if = "Option::is_none")]
    pub model_name: Option<String>,
    #[serde(rename = "haikuModelName", skip_serializing_if = "Option::is_none")]
    pub haiku_model_name: Option<String>,
    #[serde(rename = "sonnetModelName", skip_serializing_if = "Option::is_none")]
    pub sonnet_model_name: Option<String>,
    #[serde(rename = "opusModelName", skip_serializing_if = "Option::is_none")]
    pub opus_model_name: Option<String>,
    #[serde(rename = "fableModelName", skip_serializing_if = "Option::is_none")]
    pub fable_model_name: Option<String>,
    /// subagent 模型（`CLAUDE_CODE_SUBAGENT_MODEL`）
    #[serde(rename = "subagentModel", skip_serializing_if = "Option::is_none")]
    pub subagent_model: Option<String>,
    #[serde(rename = "subagentModelOneM", skip_serializing_if = "Option::is_none")]
    pub subagent_model_one_m: Option<bool>,
    /// 上游协议：anthropic / openai_chat / openai_responses / gemini_native
    #[serde(rename = "apiFormat", skip_serializing_if = "Option::is_none")]
    pub api_format: Option<String>,
    /// 写入配置的认证环境变量名
    #[serde(rename = "apiKeyField", skip_serializing_if = "Option::is_none")]
    pub api_key_field: Option<String>,
    /// 地址是完整 API 端点
    #[serde(rename = "isFullUrl", skip_serializing_if = "Option::is_none")]
    pub is_full_url: Option<bool>,
    /// 本地路由时替换的 User-Agent
    #[serde(rename = "customUserAgent", skip_serializing_if = "Option::is_none")]
    pub custom_user_agent: Option<String>,
    /// 本地路由请求覆盖
    #[serde(rename = "requestOverrides", skip_serializing_if = "Option::is_none")]
    pub request_overrides: Option<LocalProxyRequestOverrides>,
    /// 按需加载工具定义（`ENABLE_TOOL_SEARCH`）：上游支持 tool_reference 才能开
    #[serde(rename = "enableToolSearch", skip_serializing_if = "Option::is_none")]
    pub enable_tool_search: Option<bool>,
    /// 不把 Artifact 工具发给模型（`CLAUDE_CODE_DISABLE_ARTIFACT`）
    #[serde(rename = "disableArtifact", skip_serializing_if = "Option::is_none")]
    pub disable_artifact: Option<bool>,
    /// 关掉服务端 auto mode 检查（`CLAUDE_CODE_AUTO_MODE_SERVER=0`）
    #[serde(
        rename = "disableAutoModeServer",
        skip_serializing_if = "Option::is_none"
    )]
    pub disable_auto_mode_server: Option<bool>,
    /// 隐藏 AI 署名（live 顶层 `attribution`）：随这家供应商切换。
    #[serde(rename = "hideAttribution", skip_serializing_if = "Option::is_none")]
    pub hide_attribution: Option<bool>,
    /// 禁用自动升级（live `env.DISABLE_AUTOUPDATER`）：随这家供应商切换。
    #[serde(rename = "disableAutoUpgrade", skip_serializing_if = "Option::is_none")]
    pub disable_auto_upgrade: Option<bool>,
}

/// Codex 模型配置：模型 + 推理档位 + 上游协议 + 上下文窗口
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CodexModelConfig {
    /// 模型名称
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// 推理强度
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "reasoningEffort")]
    pub reasoning_effort: Option<String>,
    /// 上游格式：openai_responses / openai_chat / anthropic
    #[serde(rename = "apiFormat", skip_serializing_if = "Option::is_none")]
    pub api_format: Option<String>,
    /// 走 Anthropic 路径时的认证字段名
    #[serde(rename = "apiKeyField", skip_serializing_if = "Option::is_none")]
    pub api_key_field: Option<String>,
    /// 地址是完整 API 端点
    #[serde(rename = "isFullUrl", skip_serializing_if = "Option::is_none")]
    pub is_full_url: Option<bool>,
    /// 压缩阈值：所选模型上下文窗口的百分比（1-100）。投影时按
    /// `model_context_window × 百分比` 算出顶层 `model_auto_compact_token_limit`；
    /// 模型没有窗口元数据时不写（交给 Codex 默认的 90%）。
    #[serde(rename = "autoCompactPercent", skip_serializing_if = "Option::is_none")]
    pub auto_compact_percent: Option<u32>,
    /// 远程压缩：`[model_providers.<id>].name = "OpenAI"`
    #[serde(rename = "remoteCompaction", skip_serializing_if = "Option::is_none")]
    pub remote_compaction: Option<bool>,
    /// Chat Completions 路径的思考能力声明
    #[serde(rename = "chatReasoning", skip_serializing_if = "Option::is_none")]
    pub chat_reasoning: Option<CodexChatReasoningConfig>,
    /// 会话级提示词缓存路由
    #[serde(rename = "promptCacheRouting", skip_serializing_if = "Option::is_none")]
    pub prompt_cache_routing: Option<String>,
    /// 走 Anthropic 路径时模拟 Claude Code 客户端
    #[serde(
        rename = "impersonateClaudeCode",
        skip_serializing_if = "Option::is_none"
    )]
    pub impersonate_claude_code: Option<bool>,
    /// 模型目录：写进 `settings_config.modelCatalog`
    #[serde(
        rename = "catalogModels",
        skip_serializing_if = "Vec::is_empty",
        default
    )]
    pub catalog_models: Vec<serde_json::Value>,
    /// 子代理模型：写进 `config.toml` 的 `[agents].default_subagent_model`
    #[serde(rename = "subagentModel", skip_serializing_if = "Option::is_none")]
    pub subagent_model: Option<String>,
    /// 子代理的推理档位：`[agents].default_subagent_reasoning_effort`
    #[serde(
        rename = "subagentReasoningEffort",
        skip_serializing_if = "Option::is_none"
    )]
    pub subagent_reasoning_effort: Option<String>,
    /// 本地路由时的 User-Agent 与请求覆盖
    #[serde(rename = "customUserAgent", skip_serializing_if = "Option::is_none")]
    pub custom_user_agent: Option<String>,
    #[serde(rename = "requestOverrides", skip_serializing_if = "Option::is_none")]
    pub request_overrides: Option<LocalProxyRequestOverrides>,
}

/// Gemini 模型配置
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct GeminiModelConfig {
    /// 模型名称
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// 任意 `.env` 键（写进 `settingsConfig.env`）
    #[serde(skip_serializing_if = "Option::is_none")]
    pub env: Option<std::collections::HashMap<String, String>>,
    /// 任意 `settings.json` 键（写进 `settingsConfig.config`）
    #[serde(skip_serializing_if = "Option::is_none")]
    pub config: Option<serde_json::Value>,
}

/// Claude Desktop 模型配置：接入方式 + 角色路由表
///
/// 形状与 `meta.claudeDesktopMode` / `meta.claudeDesktopModelRoutes` 一一对应：
/// 直连把角色 ID 原样写进 Desktop profile，模型映射经本地路由把角色 ID 转成上游模型。
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ClaudeDesktopModelConfig {
    /// 接入方式：direct 直连 / proxy 模型映射；缺省按 proxy
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mode: Option<ClaudeDesktopMode>,
    /// 模型映射模式下的上游格式：anthropic / openai_chat / openai_responses / gemini_native
    #[serde(rename = "apiFormat", skip_serializing_if = "Option::is_none")]
    pub api_format: Option<String>,
    /// 角色路由表，key 是 Desktop 认得的角色 ID；留空时用默认四档
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub routes: HashMap<String, ClaudeDesktopModelRoute>,

    // ── 旧版扁平字段（4.0.8 及更早）──
    // 只读不写：老用户存的三个档位和共用的菜单显示名在这里读出来迁移成 routes，
    // 保存一次后就从记录里消失（前端只写 routes）。
    /// 旧版 Sonnet 档上游模型
    #[serde(rename = "sonnetModel", default, skip_serializing)]
    pub legacy_sonnet_model: Option<String>,
    /// 旧版 Opus 档上游模型
    #[serde(rename = "opusModel", default, skip_serializing)]
    pub legacy_opus_model: Option<String>,
    /// 旧版 Haiku 档上游模型
    #[serde(rename = "haikuModel", default, skip_serializing)]
    pub legacy_haiku_model: Option<String>,
    /// 旧版菜单显示名（三档共用）
    #[serde(rename = "labelOverride", default, skip_serializing)]
    pub legacy_label_override: Option<String>,
    /// 旧版各档 1M 声明
    #[serde(rename = "sonnetOneM", default, skip_serializing)]
    pub legacy_sonnet_one_m: Option<bool>,
    #[serde(rename = "opusOneM", default, skip_serializing)]
    pub legacy_opus_one_m: Option<bool>,
    #[serde(rename = "haikuOneM", default, skip_serializing)]
    pub legacy_haiku_one_m: Option<bool>,
}

impl ClaudeDesktopModelConfig {
    /// 路由表：新形状优先；否则把旧版扁平字段迁移成三档角色路由；都没有时返回空。
    fn effective_routes(&self) -> HashMap<String, ClaudeDesktopModelRoute> {
        if !self.routes.is_empty() {
            return self.routes.clone();
        }
        let label = self
            .legacy_label_override
            .clone()
            .filter(|value| !value.trim().is_empty());
        [
            (
                "claude-sonnet-5",
                self.legacy_sonnet_model.clone(),
                self.legacy_sonnet_one_m,
            ),
            (
                "claude-opus-5",
                self.legacy_opus_model.clone(),
                self.legacy_opus_one_m,
            ),
            (
                "claude-haiku-4-5",
                self.legacy_haiku_model.clone(),
                self.legacy_haiku_one_m,
            ),
        ]
        .into_iter()
        .filter_map(|(route_id, model, supports_1m)| {
            let model = model.filter(|value| !value.trim().is_empty())?;
            Some((
                route_id.to_string(),
                ClaudeDesktopModelRoute {
                    model,
                    label_override: label.clone(),
                    supports_1m: supports_1m.filter(|value| *value),
                },
            ))
        })
        .collect()
    }
}

/// Desktop 默认角色路由（与 `claude_desktop_config::DEFAULT_PROXY_ROUTES` 同序同 id）。
const DEFAULT_DESKTOP_ROUTES: &[&str] = &[
    "claude-sonnet-5",
    "claude-opus-5",
    "claude-haiku-4-5",
    "claude-fable-5",
];

/// 统一供应商没填路由表时用的默认四档；直连模式下 model 与 route id 同名，
/// 映射模式下模型取 Claude 主模型兜底（和单应用表单的默认路由一致）。
fn default_desktop_routes(
    fallback_model: &str,
    direct: bool,
) -> HashMap<String, ClaudeDesktopModelRoute> {
    DEFAULT_DESKTOP_ROUTES
        .iter()
        .map(|route_id| {
            (
                (*route_id).to_string(),
                ClaudeDesktopModelRoute {
                    model: if direct {
                        (*route_id).to_string()
                    } else {
                        fallback_model.to_string()
                    },
                    label_override: None,
                    supports_1m: Some(true),
                },
            )
        })
        .collect()
}

/// Grok Build 模型配置
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct GrokBuildModelConfig {
    /// 默认模型：`[models].default`，也是它指向的 `[model."<id>"]` 表名
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// 上游协议：responses / chat_completions / anthropic_messages
    #[serde(rename = "apiBackend", skip_serializing_if = "Option::is_none")]
    pub api_backend: Option<String>,
}

/// 各应用的模型配置
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct UniversalProviderModels {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub claude: Option<ClaudeModelConfig>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub codex: Option<CodexModelConfig>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gemini: Option<GeminiModelConfig>,
    #[serde(rename = "claudeDesktop", skip_serializing_if = "Option::is_none")]
    pub claude_desktop: Option<ClaudeDesktopModelConfig>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub grokbuild: Option<GrokBuildModelConfig>,
}

/// 供应商可用模型的一条：`/v1/models` 的 id 配上 oh-my-pi 模型目录里的名称和上下文窗口。
///
/// 统一供应商所有应用共用这一份列表，表单里的模型字段只从这里选。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AvailableModel {
    /// 模型 id，直接拿去填各应用的模型字段。
    pub id: String,
    /// 显示名：模型目录的 `name`，取不到时回退成 id。
    #[serde(rename = "displayName", skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    /// 上下文窗口：模型目录的 `contextWindow`，取不到就不写。
    #[serde(rename = "contextWindow", skip_serializing_if = "Option::is_none")]
    pub context_window: Option<u64>,
    /// effort 档位：模型目录 `thinking.efforts` 的并集，
    /// 用户可在表单里逐模型改。空表示没声明。
    #[serde(rename = "efforts", default, skip_serializing_if = "Vec::is_empty")]
    pub efforts: Vec<String>,
    /// 默认 effort 档位（`reasoningLevels` 的子集）；空表示目录不写
    /// `default_reasoning_level`，由后端按模板缺省兜底。
    #[serde(rename = "defaultEffort", skip_serializing_if = "Option::is_none")]
    pub default_effort: Option<String>,
    /// 最大输出 tokens：模型目录的 `maxTokens`，取不到就不写。
    #[serde(rename = "maxOutput", skip_serializing_if = "Option::is_none")]
    pub max_output: Option<u64>,
}

/// 统一供应商（跨应用共享配置）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UniversalProvider {
    /// 唯一标识
    pub id: String,
    /// 供应商名称
    pub name: String,
    /// 应用启用状态
    pub apps: UniversalProviderApps,
    /// API 基础地址
    #[serde(rename = "baseUrl")]
    pub base_url: String,
    /// API 密钥
    #[serde(rename = "apiKey")]
    pub api_key: String,
    /// 各应用的模型配置
    #[serde(default)]
    pub models: UniversalProviderModels,
    /// 从 `/v1/models` 拉取、再用 oh-my-pi 模型目录补全的可用模型列表。所有应用共用。
    #[serde(
        rename = "availableModels",
        default,
        skip_serializing_if = "Vec::is_empty"
    )]
    pub available_models: Vec<AvailableModel>,
    /// 网站链接
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "websiteUrl")]
    pub website_url: Option<String>,
    /// 备注信息
    #[serde(skip_serializing_if = "Option::is_none")]
    pub notes: Option<String>,
    /// 图标名称
    #[serde(skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    /// 图标颜色
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "iconColor")]
    pub icon_color: Option<String>,
    /// 元数据
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<ProviderMeta>,
    /// 创建时间戳
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "createdAt")]
    pub created_at: Option<i64>,
    /// 排序索引
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "sortIndex")]
    pub sort_index: Option<usize>,
}

impl UniversalProvider {
    /// 创建新的统一供应商
    pub fn new(id: String, name: String, base_url: String, api_key: String) -> Self {
        Self {
            id,
            name,
            apps: UniversalProviderApps::default(),
            base_url,
            api_key,
            models: UniversalProviderModels::default(),
            website_url: None,
            notes: None,
            icon: None,
            icon_color: None,
            meta: None,
            created_at: Some(chrono::Utc::now().timestamp_millis()),
            sort_index: None,
            available_models: Vec::new(),
        }
    }

    /// 生成 Claude 供应商配置
    pub fn to_claude_provider(&self) -> Option<Provider> {
        if !self.apps.claude {
            return None;
        }

        let models = self.models.claude.as_ref();
        let model = models
            .and_then(|m| m.model.clone())
            .unwrap_or_else(|| "claude-sonnet-4-20250514".to_string());
        let haiku = models
            .and_then(|m| m.haiku_model.clone())
            .unwrap_or_else(|| model.clone());
        let sonnet = models
            .and_then(|m| m.sonnet_model.clone())
            .unwrap_or_else(|| model.clone());
        let opus = models
            .and_then(|m| m.opus_model.clone())
            .unwrap_or_else(|| model.clone());
        let fable = models
            .and_then(|m| m.fable_model.clone())
            .unwrap_or_else(|| model.clone());

        // 1M 声明和单应用表单同一个标记：模型名末尾的 `[1M]`。
        //
        // 标志的来源有两层：显式存了 `*OneM` 就照它（老数据、单应用表单）；
        // 没存（`None`，统一供应商新表单不再手勾）时按可用模型列表的上下文窗口
        // 元数据判定（≥ 1,000,000 视为支持 1M）。列表里查不到的模型按不支持。
        let context_window_of = |model_id: &str| -> Option<u64> {
            let id = model_id.trim();
            if id.is_empty() {
                return None;
            }
            self.available_models
                .iter()
                .find(|model| model.id.trim() == id)
                .and_then(|model| model.context_window)
        };
        let one_m = |value: &str, flag: Option<bool>| -> String {
            let supports = match flag {
                Some(explicit) => explicit,
                None => context_window_of(value).is_some_and(|window| window >= 1_000_000),
            };
            if supports && !value.trim().is_empty() {
                format!("{}[1M]", value.trim_end())
            } else {
                value.to_string()
            }
        };
        let flag = |pick: fn(&ClaudeModelConfig) -> Option<bool>| -> Option<bool> {
            models.and_then(pick)
        };

        let mut env = serde_json::Map::new();
        let auth_field = models
            .and_then(|m| m.api_key_field.clone())
            .filter(|f| f == "ANTHROPIC_API_KEY")
            .unwrap_or_else(|| "ANTHROPIC_AUTH_TOKEN".to_string());
        env.insert("ANTHROPIC_BASE_URL".into(), json!(self.base_url));
        env.insert(auth_field, json!(self.api_key));
        env.insert(
            "ANTHROPIC_MODEL".into(),
            json!(one_m(&model, flag(|m| m.model_one_m))),
        );
        env.insert(
            "ANTHROPIC_DEFAULT_HAIKU_MODEL".into(),
            json!(one_m(&haiku, flag(|m| m.haiku_model_one_m))),
        );
        env.insert(
            "ANTHROPIC_DEFAULT_SONNET_MODEL".into(),
            json!(one_m(&sonnet, flag(|m| m.sonnet_model_one_m))),
        );
        env.insert(
            "ANTHROPIC_DEFAULT_OPUS_MODEL".into(),
            json!(one_m(&opus, flag(|m| m.opus_model_one_m))),
        );
        env.insert(
            "ANTHROPIC_DEFAULT_FABLE_MODEL".into(),
            json!(one_m(&fable, flag(|m| m.fable_model_one_m))),
        );
        // 显示名只影响 /model 菜单，没填就不写。
        for (key, pick) in [
            ("ANTHROPIC_DEFAULT_HAIKU_MODEL_NAME", 0usize),
            ("ANTHROPIC_DEFAULT_SONNET_MODEL_NAME", 1),
            ("ANTHROPIC_DEFAULT_OPUS_MODEL_NAME", 2),
            ("ANTHROPIC_DEFAULT_FABLE_MODEL_NAME", 3),
        ] {
            let name = models.and_then(|m| {
                [
                    m.haiku_model_name.clone(),
                    m.sonnet_model_name.clone(),
                    m.opus_model_name.clone(),
                    m.fable_model_name.clone(),
                ][pick]
                    .clone()
            });
            if let Some(name) = name.filter(|n| !n.trim().is_empty()) {
                env.insert(key.into(), json!(name));
            }
        }
        if let Some(name) = models
            .and_then(|m| m.model_name.clone())
            .filter(|n| !n.trim().is_empty())
        {
            env.insert("ANTHROPIC_MODEL_NAME".into(), json!(name));
        }
        if let Some(subagent) = models
            .and_then(|m| m.subagent_model.clone())
            .filter(|s| !s.trim().is_empty())
        {
            env.insert(
                "CLAUDE_CODE_SUBAGENT_MODEL".into(),
                json!(one_m(&subagent, flag(|m| m.subagent_model_one_m))),
            );
        }
        // 上游兼容开关：都是供应商独有字段，切入时写、切走时删（floor.rs 的
        // CLAUDE_EXCLUSIVE_ENV 已列出这三个键）。
        for (key, value) in [
            (
                "ENABLE_TOOL_SEARCH",
                models.and_then(|m| m.enable_tool_search),
            ),
            (
                "CLAUDE_CODE_DISABLE_ARTIFACT",
                models.and_then(|m| m.disable_artifact),
            ),
            (
                "CLAUDE_CODE_AUTO_MODE_SERVER",
                models.and_then(|m| m.disable_auto_mode_server),
            ),
        ] {
            match value {
                // 取值和单应用表单的开关一致：tool search 写 "true"，auto mode 的
                // 服务端检查是「关掉」语义写 "0"，其余写 "1"。
                Some(true) if key == "ENABLE_TOOL_SEARCH" => {
                    env.insert(key.into(), json!("true"));
                }
                Some(true) if key == "CLAUDE_CODE_AUTO_MODE_SERVER" => {
                    env.insert(key.into(), json!("0"));
                }
                Some(true) => {
                    env.insert(key.into(), json!("1"));
                }
                _ => {}
            }
        }
        // 署名与自动升级也归供应商所有（floor.rs 的关键字段列表里），随切换覆盖。
        let mut settings_config = serde_json::json!({ "env": env });
        if models.and_then(|m| m.hide_attribution).unwrap_or(false) {
            settings_config["attribution"] = json!({ "commit": "", "pr": "", "sessionUrl": false });
        }
        if models.and_then(|m| m.disable_auto_upgrade).unwrap_or(false) {
            settings_config["env"]["DISABLE_AUTOUPDATER"] = json!("1");
        }

        // 上游协议、完整 URL、User-Agent、请求覆盖都是这个应用的专属设置，存在 meta 里。
        // 以 per-app 配置为准（包括清空）：用户把某个选项去掉后，子卡不能留着旧值。
        let mut meta = self.meta.clone().unwrap_or_default();
        meta.api_format = models.and_then(|m| m.api_format.clone());
        meta.api_key_field = models
            .and_then(|m| m.api_key_field.clone())
            .filter(|f| f == "ANTHROPIC_API_KEY");
        meta.is_full_url = models.and_then(|m| m.is_full_url).filter(|v| *v);
        meta.custom_user_agent = models
            .and_then(|m| m.custom_user_agent.clone())
            .filter(|ua| !ua.trim().is_empty());
        meta.local_proxy_request_overrides = models
            .and_then(|m| m.request_overrides.clone())
            .filter(|overrides| !overrides.is_empty());

        Some(Provider {
            id: format!("universal-claude-{}", self.id),
            name: self.name.clone(),
            settings_config,
            website_url: self.website_url.clone(),
            category: Some("aggregator".to_string()),
            created_at: self.created_at,
            sort_index: self.sort_index,
            notes: self.notes.clone(),
            meta: Some(meta),
            icon: self.icon.clone(),
            icon_color: self.icon_color.clone(),
            in_failover_queue: false,
        })
    }

    /// 生成 Codex 供应商配置
    pub fn to_codex_provider(&self) -> Option<Provider> {
        if !self.apps.codex {
            return None;
        }

        let models = self.models.codex.as_ref();
        let model = models
            .and_then(|m| m.model.clone())
            .unwrap_or_else(|| "gpt-4o".to_string());
        let reasoning_effort = models
            .and_then(|m| m.reasoning_effort.clone())
            .unwrap_or_else(|| "high".to_string());

        // Codex/OpenAI 的 base_url 既可能是纯 origin（需要补 /v1），也可能包含自定义前缀（不应强行补版本）
        let base_trimmed = self.base_url.trim_end_matches('/');
        let origin_only = match base_trimmed.split_once("://") {
            Some((_scheme, rest)) => !rest.contains('/'),
            None => !base_trimmed.contains('/'),
        };
        let codex_base_url = if base_trimmed.ends_with("/v1") {
            base_trimmed.to_string()
        } else if origin_only {
            format!("{base_trimmed}/v1")
        } else {
            base_trimmed.to_string()
        };

        // 远程压缩靠把供应商表的名字写成 OpenAI 触发（和单应用表单同一个开关）。
        let provider_table_name = if models.and_then(|m| m.remote_compaction) == Some(true) {
            "OpenAI"
        } else {
            "NewAPI"
        };

        // 生成 Codex 的 config.toml 内容
        //
        // 顶层字段必须先写、再写 `[model_providers.custom]`：TOML 里表头之后的键属于
        // 那张表，写在后面会被解析成 `model_providers.custom.model_context_window`。
        let mut config_toml = format!(
            r#"model_provider = "custom"
model = "{model}"
model_reasoning_effort = "{reasoning_effort}"
disable_response_storage = true"#
        );
        // 上下文窗口跟随所选模型行的元数据（模型目录的 contextWindow），有才写
        // （交给 Codex 默认）。压缩阈值存的是百分比，按同一个窗口换算成绝对
        // token 数；没有窗口元数据时算不出来，整条不写。
        let context_window = models.and_then(|m| m.model.clone()).and_then(|model| {
            self.available_models
                .iter()
                .find(|row| row.id.trim() == model.trim())
                .and_then(|row| row.context_window)
        });
        if let Some(window) = context_window {
            config_toml.push_str(&format!("\nmodel_context_window = {window}"));
        }
        if let Some(percent) = models.and_then(|m| m.auto_compact_percent) {
            if let Some(window) = context_window {
                // 百分比来自存下来的记录，可能是手改的 JSON：饱和乘法避免溢出
                let limit = window.saturating_mul(u64::from(percent)) / 100;
                config_toml.push_str(&format!("\nmodel_auto_compact_token_limit = {limit}"));
            }
        }
        config_toml.push_str(&format!(
            r#"

[model_providers.custom]
name = "{provider_table_name}"
base_url = "{codex_base_url}"
wire_api = "responses"
requires_openai_auth = true"#
        ));
        // 子代理模型是嵌套关键字段（`floor.rs` 的 CODEX_FLOOR_NESTED）：整张表放在最后，
        // 免得后面的表头把它的键收进别的表。
        if let Some(subagent) = models
            .and_then(|m| m.subagent_model.as_ref())
            .filter(|s| !s.trim().is_empty())
        {
            config_toml.push_str(&format!(
                "\n\n[agents]\ndefault_subagent_model = \"{}\"",
                subagent.trim()
            ));
            if let Some(effort) = models
                .and_then(|m| m.subagent_reasoning_effort.as_ref())
                .filter(|s| !s.trim().is_empty())
            {
                config_toml.push_str(&format!(
                    "\ndefault_subagent_reasoning_effort = \"{}\"",
                    effort.trim()
                ));
            }
        }

        let mut settings_config = serde_json::json!({
            "auth": {
                "OPENAI_API_KEY": self.api_key
            },
            "config": config_toml
        });
        // 模型目录：用户填了行就用它；没填时用统一供应商的可用模型列表自动生成，
        // 名称、上下文窗口、effort 档位与默认档都是逐模型（拉取时由 oh-my-pi
        // 模型目录补全，表单里可改）。两者都没有就不写目录（和没配区分开）。
        let catalog = models
            .map(|m| m.catalog_models.clone())
            .filter(|c| !c.is_empty())
            .unwrap_or_else(|| {
                self.available_models
                    .iter()
                    // 手加的空行（id 还没填）不进目录。
                    .filter(|model| !model.id.trim().is_empty())
                    .map(|model| {
                        let mut row = serde_json::json!({ "model": model.id });
                        if let Some(name) = model.display_name.as_ref() {
                            row["displayName"] = serde_json::json!(name);
                        }
                        if let Some(window) = model.context_window {
                            row["contextWindow"] = serde_json::json!(window);
                        }
                        if !model.efforts.is_empty() {
                            row["reasoningLevels"] = serde_json::json!(model.efforts);
                        }
                        if let Some(default_effort) = model.default_effort.as_ref() {
                            row["defaultReasoningLevel"] = serde_json::json!(default_effort);
                        }
                        row
                    })
                    .collect()
            });
        if !catalog.is_empty() {
            settings_config["modelCatalog"] = serde_json::json!({ "models": catalog });
        }

        let mut meta = self.meta.clone().unwrap_or_default();
        meta.api_format = models.and_then(|m| m.api_format.clone());
        meta.api_key_field = models
            .and_then(|m| m.api_key_field.clone())
            .filter(|f| f == "ANTHROPIC_API_KEY");
        meta.is_full_url = models.and_then(|m| m.is_full_url).filter(|v| *v);
        meta.custom_user_agent = models
            .and_then(|m| m.custom_user_agent.clone())
            .filter(|ua| !ua.trim().is_empty());
        meta.local_proxy_request_overrides = models
            .and_then(|m| m.request_overrides.clone())
            .filter(|overrides| !overrides.is_empty());
        meta.codex_chat_reasoning = models.and_then(|m| m.chat_reasoning.clone());
        meta.prompt_cache_routing = models
            .and_then(|m| m.prompt_cache_routing.clone())
            .filter(|r| r != "auto");
        meta.impersonate_claude_code = models
            .and_then(|m| m.impersonate_claude_code)
            .filter(|v| *v);
        // max_output_tokens：用户手填优先；没填时从所选模型行的元数据
        // （模型目录的 maxTokens）自动补——与表单的自动预填同源。
        let selected_row = models.and_then(|m| m.model.clone()).and_then(|model| {
            self.available_models
                .iter()
                .find(|row| row.id.trim() == model.trim())
        });
        // 最大输出：跟所选模型行的元数据（模型目录的 maxTokens）。
        meta.max_output_tokens = selected_row
            .and_then(|row| row.max_output)
            .filter(|t| *t > 0);

        Some(Provider {
            id: format!("universal-codex-{}", self.id),
            name: self.name.clone(),
            settings_config,
            website_url: self.website_url.clone(),
            category: Some("aggregator".to_string()),
            created_at: self.created_at,
            sort_index: self.sort_index,
            notes: self.notes.clone(),
            meta: Some(meta),
            icon: self.icon.clone(),
            icon_color: self.icon_color.clone(),
            in_failover_queue: false,
        })
    }

    /// 生成 Gemini 供应商配置
    pub fn to_gemini_provider(&self) -> Option<Provider> {
        if !self.apps.gemini {
            return None;
        }

        let models = self.models.gemini.as_ref();
        let model = models
            .and_then(|m| m.model.clone())
            .unwrap_or_else(|| "gemini-2.5-pro".to_string());

        let mut env = serde_json::Map::new();
        env.insert("GOOGLE_GEMINI_BASE_URL".into(), json!(self.base_url));
        env.insert("GEMINI_API_KEY".into(), json!(self.api_key));
        env.insert("GEMINI_MODEL".into(), json!(model));
        // 用户在 .env 编辑器里加的键跟着这个应用一起切换。
        if let Some(extra) = models.and_then(|m| m.env.clone()) {
            for (key, value) in extra {
                env.insert(key, json!(value));
            }
        }

        let mut settings_config = serde_json::json!({ "env": env });
        // settings.json 块同理；`model.name` 是关键字段，但用户显式填的优先。
        if let Some(config) = models.and_then(|m| m.config.clone()) {
            if config.as_object().is_some_and(|o| !o.is_empty()) {
                settings_config["config"] = config;
            }
        }

        Some(Provider {
            id: format!("universal-gemini-{}", self.id),
            name: self.name.clone(),
            settings_config,
            website_url: self.website_url.clone(),
            category: Some("aggregator".to_string()),
            created_at: self.created_at,
            sort_index: self.sort_index,
            notes: self.notes.clone(),
            meta: self.meta.clone(),
            icon: self.icon.clone(),
            icon_color: self.icon_color.clone(),
            in_failover_queue: false,
        })
    }

    /// 生成 Claude Desktop 供应商配置
    ///
    /// 接入方式由 per-app 配置决定：直连把角色 ID 原样交给 Desktop（供应商自己认
    /// claude-* 角色），模型映射经本地路由把角色 ID 转成上游模型。路由表留空时用
    /// 默认四档（Sonnet / Opus / Haiku / Fable），与单应用表单一致。
    pub fn to_claude_desktop_provider(&self) -> Option<Provider> {
        if !self.apps.claude_desktop {
            return None;
        }

        let models = self.models.claude_desktop.as_ref();
        let mode = models
            .and_then(|m| m.mode.clone())
            .unwrap_or(ClaudeDesktopMode::Proxy);
        let direct = matches!(mode, ClaudeDesktopMode::Direct);
        let fallback = self
            .models
            .claude
            .as_ref()
            .and_then(|m| m.model.clone())
            .unwrap_or_else(|| "claude-sonnet-4-20250514".to_string());

        // 路由表：新形状优先，旧版扁平字段自动迁移；都没有时用默认四档。
        // 直连模式不再有「模型列表」编辑：直接从供应商级可用模型列表里挑
        // claude-* / anthropic/claude-* 的 id 生成路由（Claude Desktop 直连只认
        // 这些名字），1M 声明按上下文窗口元数据（≥ 1,000,000）自动判定。
        let mut routes = if direct {
            let derived: HashMap<String, ClaudeDesktopModelRoute> = self
                .available_models
                .iter()
                .filter(|model| crate::claude_desktop_config::is_claude_safe_model_id(&model.id))
                .map(|model| {
                    (
                        model.id.clone(),
                        ClaudeDesktopModelRoute {
                            model: model.id.clone(),
                            label_override: None,
                            supports_1m: model
                                .context_window
                                .is_some_and(|window| window >= 1_000_000)
                                .then_some(true),
                        },
                    )
                })
                .collect();
            if derived.is_empty() {
                default_desktop_routes(&fallback, direct)
            } else {
                derived
            }
        } else {
            models
                .map(ClaudeDesktopModelConfig::effective_routes)
                .filter(|routes| !routes.is_empty())
                .unwrap_or_else(|| default_desktop_routes(&fallback, direct))
        };
        // 映射模式下没存 1M 声明的档（统一供应商新表单不再手勾），按所选上游模型
        // 的上下文窗口元数据判定；显式存了 true / false 的照旧（老数据兼容）。
        // 菜单显示名同理：没存 labelOverride 时取所选模型行的 displayName
        // （模型目录的名称），取不到就不写，由 Desktop 回退成模型名。
        if !direct {
            for route in routes.values_mut() {
                if route.supports_1m.is_none() {
                    route.supports_1m = self
                        .available_models
                        .iter()
                        .find(|model| model.id.trim() == route.model.trim())
                        .and_then(|model| model.context_window)
                        .is_some_and(|window| window >= 1_000_000)
                        .then_some(true);
                }
                if route.label_override.is_none() {
                    route.label_override = self
                        .available_models
                        .iter()
                        .find(|model| model.id.trim() == route.model.trim())
                        .and_then(|model| model.display_name.clone())
                        .map(|name| name.trim().to_string())
                        .filter(|name| !name.is_empty());
                }
            }
        }
        if direct {
            for (route_id, route) in routes.iter_mut() {
                route.model = route_id.clone();
            }
        }

        let mut meta = self.meta.clone().unwrap_or_default();
        meta.claude_desktop_mode = Some(mode);
        meta.claude_desktop_model_routes = routes;
        meta.api_format = if direct {
            None
        } else {
            models.and_then(|m| m.api_format.clone())
        };
        // 统一供应商的子卡不走端点自动选择，地址由统一供应商驱动。
        meta.endpoint_auto_select = None;
        meta.is_full_url = None;

        let settings_config = serde_json::json!({
            "env": {
                "ANTHROPIC_BASE_URL": self.base_url.trim_end_matches('/'),
                "ANTHROPIC_AUTH_TOKEN": self.api_key,
            }
        });

        Some(Provider {
            id: format!("universal-claude-desktop-{}", self.id),
            name: self.name.clone(),
            settings_config,
            website_url: self.website_url.clone(),
            category: Some("aggregator".to_string()),
            created_at: self.created_at,
            sort_index: self.sort_index,
            notes: self.notes.clone(),
            meta: Some(meta),
            icon: self.icon.clone(),
            icon_color: self.icon_color.clone(),
            in_failover_queue: false,
        })
    }

    /// 生成 Grok Build 供应商配置
    pub fn to_grokbuild_provider(&self) -> Option<Provider> {
        if !self.apps.grokbuild {
            return None;
        }

        let models = self.models.grokbuild.as_ref();
        let api_backend = models
            .and_then(|m| m.api_backend.clone())
            .filter(|m| !m.trim().is_empty())
            .unwrap_or_else(|| crate::grok_config::DEFAULT_API_BACKEND.to_string());

        // 模型表：可用模型列表里的每个 id 一张 `[model."<id>"]`（去重保序），默认模型不在
        // 列表里时补上（用户手填过、或列表还没拉取）；列表为空时用内置默认模型兜底。
        let mut ids: Vec<String> = Vec::with_capacity(self.available_models.len() + 1);
        for model in &self.available_models {
            let id = model.id.trim();
            if id.is_empty() || ids.iter().any(|existing| existing == id) {
                continue;
            }
            ids.push(id.to_string());
        }
        let configured = models
            .and_then(|m| m.model.clone())
            .map(|model| model.trim().to_string())
            .filter(|model| !model.is_empty());
        if let Some(configured) = configured.as_ref() {
            if !ids.iter().any(|id| id == configured) {
                ids.push(configured.clone());
            }
        }
        if ids.is_empty() {
            ids.push(crate::grok_config::DEFAULT_MODEL.to_string());
        }
        // 默认模型：配置里那个在列表里就用它，否则退回第一张表。
        let default = configured
            .filter(|configured| ids.iter().any(|id| id == configured))
            .unwrap_or_else(|| ids[0].clone());

        let base_url = self.base_url.trim_end_matches('/');
        let window_of = |id: &str| {
            self.available_models
                .iter()
                .find(|row| row.id.trim() == id)
                .and_then(|row| row.context_window)
                .unwrap_or(crate::grok_config::DEFAULT_CONTEXT_WINDOW as u64)
        };
        let mut config = format!("[models]\ndefault = \"{default}\"\n");
        for id in &ids {
            config.push_str(&format!(
                r#"
[model."{id}"]
model = "{id}"
base_url = "{base_url}"
name = "{name}"
api_key = "{api_key}"
api_backend = "{api_backend}"
context_window = {context_window}
"#,
                id = id,
                base_url = base_url,
                name = self.name,
                api_key = self.api_key,
                api_backend = api_backend,
                context_window = window_of(id),
            ));
        }

        let settings_config = serde_json::json!({
            "config": config
        });

        Some(Provider {
            id: format!("universal-grokbuild-{}", self.id),
            name: self.name.clone(),
            settings_config,
            website_url: self.website_url.clone(),
            category: Some("aggregator".to_string()),
            created_at: self.created_at,
            sort_index: self.sort_index,
            notes: self.notes.clone(),
            meta: self.meta.clone(),
            icon: self.icon.clone(),
            icon_color: self.icon_color.clone(),
            in_failover_queue: false,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::{
        AvailableModel, ClaudeDesktopMode, ClaudeDesktopModelConfig, ClaudeDesktopModelRoute,
        ClaudeModelConfig, CodexModelConfig, GeminiModelConfig, GrokBuildModelConfig,
        LocalProxyRequestOverrides, Provider, ProviderManager, ProviderMeta, UniversalProvider,
        UniversalProviderApps, UniversalProviderModels,
    };
    use serde_json::json;
    use std::collections::HashMap;

    #[test]
    fn provider_meta_serializes_pricing_model_source() {
        let meta = ProviderMeta {
            pricing_model_source: Some("response".to_string()),
            ..ProviderMeta::default()
        };

        let value = serde_json::to_value(&meta).expect("serialize ProviderMeta");

        assert_eq!(
            value
                .get("pricingModelSource")
                .and_then(|item| item.as_str()),
            Some("response")
        );
        assert!(value.get("pricing_model_source").is_none());
    }

    #[test]
    fn provider_meta_omits_pricing_model_source_when_none() {
        let meta = ProviderMeta::default();
        let value = serde_json::to_value(&meta).expect("serialize ProviderMeta");

        assert!(value.get("pricingModelSource").is_none());
    }

    #[test]
    fn provider_meta_roundtrips_max_output_tokens() {
        let meta = ProviderMeta {
            max_output_tokens: Some(64000),
            ..ProviderMeta::default()
        };

        let value = serde_json::to_value(&meta).expect("serialize ProviderMeta");
        assert_eq!(
            value.get("maxOutputTokens").and_then(|v| v.as_u64()),
            Some(64000)
        );
        assert!(value.get("max_output_tokens").is_none());

        let parsed: ProviderMeta = serde_json::from_value(value).expect("deserialize ProviderMeta");
        assert_eq!(parsed.max_output_tokens, Some(64000));
    }

    #[test]
    fn provider_meta_omits_max_output_tokens_when_none() {
        let value = serde_json::to_value(ProviderMeta::default()).expect("serialize ProviderMeta");
        assert!(value.get("maxOutputTokens").is_none());
    }

    #[test]
    fn provider_meta_roundtrips_local_proxy_request_overrides() {
        let meta = ProviderMeta {
            local_proxy_request_overrides: Some(LocalProxyRequestOverrides {
                headers: HashMap::from([("X-Test".to_string(), "yes".to_string())]),
                body: Some(json!({ "temperature": 0.2 })),
            }),
            ..ProviderMeta::default()
        };

        let value = serde_json::to_value(&meta).expect("serialize ProviderMeta");
        assert_eq!(
            value["localProxyRequestOverrides"]["headers"]["X-Test"],
            "yes"
        );
        assert_eq!(
            value["localProxyRequestOverrides"]["body"]["temperature"],
            0.2
        );

        let decoded: ProviderMeta =
            serde_json::from_value(value).expect("deserialize ProviderMeta");
        let overrides = decoded.local_proxy_request_overrides.unwrap();
        assert_eq!(overrides.headers.get("X-Test"), Some(&"yes".to_string()));
        assert_eq!(overrides.body.unwrap()["temperature"], 0.2);
    }

    #[test]
    fn provider_with_id_populates_defaults() {
        let settings_config = json!({
            "env": { "API_KEY": "test" }
        });
        let provider = Provider::with_id(
            "provider-1".to_string(),
            "Provider".to_string(),
            settings_config.clone(),
            Some("https://example.com".to_string()),
        );

        assert_eq!(provider.id, "provider-1");
        assert_eq!(provider.name, "Provider");
        assert_eq!(provider.settings_config, settings_config);
        assert_eq!(provider.website_url.as_deref(), Some("https://example.com"));
        assert!(provider.category.is_none());
        assert!(provider.created_at.is_none());
        assert!(provider.sort_index.is_none());
        assert!(provider.notes.is_none());
        assert!(provider.meta.is_none());
        assert!(provider.icon.is_none());
        assert!(provider.icon_color.is_none());
        assert!(!provider.in_failover_queue);
    }

    #[test]
    fn provider_manager_get_all_providers_returns_map() {
        let mut manager = ProviderManager::default();
        let provider = Provider::with_id(
            "provider-1".to_string(),
            "Provider".to_string(),
            json!({ "env": {} }),
            None,
        );
        manager.providers.insert("provider-1".to_string(), provider);

        assert_eq!(manager.get_all_providers().len(), 1);
        assert!(manager.get_all_providers().contains_key("provider-1"));
    }

    #[test]
    fn universal_provider_to_claude_provider_uses_models() {
        let mut universal = UniversalProvider::new(
            "u1".to_string(),
            "Universal".to_string(),
            "https://api.example.com".to_string(),
            "api-key".to_string(),
        );
        universal.apps.claude = true;
        universal.models.claude = Some(ClaudeModelConfig {
            model: Some("claude-main".to_string()),
            haiku_model: Some("claude-haiku".to_string()),
            sonnet_model: Some("claude-sonnet".to_string()),
            opus_model: Some("claude-opus".to_string()),
            fable_model: Some("claude-fable".to_string()),
            fable_model_one_m: Some(true),
            fable_model_name: Some("Fable Display".to_string()),
            ..Default::default()
        });
        let provider = universal.to_claude_provider().expect("claude provider");

        assert_eq!(provider.id, "universal-claude-u1");
        assert_eq!(provider.name, "Universal");
        assert_eq!(provider.category.as_deref(), Some("aggregator"));
        assert_eq!(
            provider
                .settings_config
                .pointer("/env/ANTHROPIC_MODEL")
                .and_then(|item| item.as_str()),
            Some("claude-main")
        );
        assert_eq!(
            provider
                .settings_config
                .pointer("/env/ANTHROPIC_DEFAULT_HAIKU_MODEL")
                .and_then(|item| item.as_str()),
            Some("claude-haiku")
        );
        assert_eq!(
            provider
                .settings_config
                .pointer("/env/ANTHROPIC_DEFAULT_SONNET_MODEL")
                .and_then(|item| item.as_str()),
            Some("claude-sonnet")
        );
        assert_eq!(
            provider
                .settings_config
                .pointer("/env/ANTHROPIC_DEFAULT_OPUS_MODEL")
                .and_then(|item| item.as_str()),
            Some("claude-opus")
        );
        // Fable 档和 Sonnet/Opus/Haiku 一样是第一类角色，1M 标记与显示名都要写出来
        assert_eq!(
            provider
                .settings_config
                .pointer("/env/ANTHROPIC_DEFAULT_FABLE_MODEL")
                .and_then(|item| item.as_str()),
            Some("claude-fable[1M]")
        );
        assert_eq!(
            provider
                .settings_config
                .pointer("/env/ANTHROPIC_DEFAULT_FABLE_MODEL_NAME")
                .and_then(|item| item.as_str()),
            Some("Fable Display")
        );
    }

    #[test]
    fn universal_provider_to_claude_provider_derives_one_m_from_context_window() {
        // 1M 声明不再由表单勾选：所在档选的模型在可用列表里上下文窗口 ≥ 1,000,000
        // 才加 [1M] 标记；没选模型或窗口不足就不加。Sonnet 顺带拿到元数据里的
        // 显示名。
        let mut universal = UniversalProvider::new(
            "u1".to_string(),
            "Universal".to_string(),
            "https://api.example.com".to_string(),
            "api-key".to_string(),
        );
        universal.apps.claude = true;
        universal.available_models = vec![
            AvailableModel {
                id: "claude-sonnet-5".to_string(),
                display_name: Some("Claude Sonnet 5".to_string()),
                context_window: Some(1_000_000),
                efforts: Vec::new(),
                default_effort: None,
                max_output: None,
            },
            AvailableModel {
                id: "claude-haiku-5".to_string(),
                display_name: Some("Claude Haiku 5".to_string()),
                context_window: Some(200_000),
                efforts: Vec::new(),
                default_effort: None,
                max_output: None,
            },
        ];
        universal.models.claude = Some(ClaudeModelConfig {
            model: Some("claude-sonnet-5".to_string()),
            sonnet_model: Some("claude-sonnet-5".to_string()),
            // 显示名由表单按元数据写进 models（保存时推导），后端照存照写。
            sonnet_model_name: Some("Claude Sonnet 5".to_string()),
            haiku_model: Some("claude-haiku-5".to_string()),
            opus_model: Some("not-in-list".to_string()),
            ..Default::default()
        });

        let env = &universal
            .to_claude_provider()
            .expect("claude provider")
            .settings_config["env"];
        assert_eq!(env["ANTHROPIC_MODEL"], json!("claude-sonnet-5[1M]"));
        assert_eq!(
            env["ANTHROPIC_DEFAULT_SONNET_MODEL"],
            json!("claude-sonnet-5[1M]")
        );
        assert_eq!(
            env["ANTHROPIC_DEFAULT_SONNET_MODEL_NAME"],
            json!("Claude Sonnet 5")
        );
        // 窗口 200,000：不加标记
        assert_eq!(
            env["ANTHROPIC_DEFAULT_HAIKU_MODEL"],
            json!("claude-haiku-5")
        );
        // 不在可用列表里：无元数据，不加标记也不写显示名
        assert_eq!(env["ANTHROPIC_DEFAULT_OPUS_MODEL"], json!("not-in-list"));
        assert!(env.get("ANTHROPIC_DEFAULT_OPUS_MODEL_NAME").is_none());
    }

    #[test]
    fn universal_provider_to_claude_provider_disabled_returns_none() {
        let universal = UniversalProvider::new(
            "u1".to_string(),
            "Universal".to_string(),
            "https://api.example.com".to_string(),
            "api-key".to_string(),
        );

        assert!(universal.to_claude_provider().is_none());
    }

    #[test]
    fn universal_provider_to_codex_provider_appends_v1() {
        let mut universal = UniversalProvider::new(
            "u1".to_string(),
            "Universal".to_string(),
            "https://api.example.com".to_string(),
            "api-key".to_string(),
        );
        universal.apps.codex = true;
        universal.models.codex = Some(CodexModelConfig {
            model: Some("gpt-4o-mini".to_string()),
            reasoning_effort: Some("low".to_string()),
            ..Default::default()
        });

        let provider = universal.to_codex_provider().expect("codex provider");
        let config = provider
            .settings_config
            .get("config")
            .and_then(|item| item.as_str())
            .expect("config toml");

        assert!(config.contains("base_url = \"https://api.example.com/v1\""));
        assert_eq!(
            provider
                .settings_config
                .pointer("/auth/OPENAI_API_KEY")
                .and_then(|item| item.as_str()),
            Some("api-key")
        );
    }

    #[test]
    fn universal_provider_to_codex_provider_keeps_v1_suffix() {
        let mut universal = UniversalProvider::new(
            "u1".to_string(),
            "Universal".to_string(),
            "https://api.example.com/v1".to_string(),
            "api-key".to_string(),
        );
        universal.apps.codex = true;

        let provider = universal.to_codex_provider().expect("codex provider");
        let config = provider
            .settings_config
            .get("config")
            .and_then(|item| item.as_str())
            .expect("config toml");

        assert!(config.contains("base_url = \"https://api.example.com/v1\""));
    }

    #[test]
    fn universal_provider_to_codex_provider_disabled_returns_none() {
        let universal = UniversalProvider::new(
            "u1".to_string(),
            "Universal".to_string(),
            "https://api.example.com".to_string(),
            "api-key".to_string(),
        );

        assert!(universal.to_codex_provider().is_none());
    }

    #[test]
    fn universal_provider_to_gemini_provider_defaults_model() {
        let mut universal = UniversalProvider::new(
            "u1".to_string(),
            "Universal".to_string(),
            "https://api.example.com".to_string(),
            "api-key".to_string(),
        );
        universal.apps.gemini = true;

        let provider = universal.to_gemini_provider().expect("gemini provider");

        assert_eq!(
            provider
                .settings_config
                .pointer("/env/GEMINI_MODEL")
                .and_then(|item| item.as_str()),
            Some("gemini-2.5-pro")
        );
    }

    #[test]
    fn universal_provider_to_gemini_provider_uses_model() {
        let mut universal = UniversalProvider::new(
            "u1".to_string(),
            "Universal".to_string(),
            "https://api.example.com".to_string(),
            "api-key".to_string(),
        );
        universal.apps.gemini = true;
        universal.models.gemini = Some(GeminiModelConfig {
            model: Some("gemini-custom".to_string()),
            ..Default::default()
        });

        let provider = universal.to_gemini_provider().expect("gemini provider");

        assert_eq!(
            provider
                .settings_config
                .pointer("/env/GEMINI_MODEL")
                .and_then(|item| item.as_str()),
            Some("gemini-custom")
        );
    }

    #[test]
    fn universal_codex_provider_origin_base_url_adds_v1() {
        let mut p = UniversalProvider::new(
            "id".to_string(),
            "Test".to_string(),
            "https://api.openai.com".to_string(),
            "sk-test".to_string(),
        );
        p.apps.codex = true;

        let provider = p.to_codex_provider().expect("should build codex provider");
        let toml = provider
            .settings_config
            .get("config")
            .and_then(|v| v.as_str())
            .expect("config should be a toml string");

        assert!(toml.contains("base_url = \"https://api.openai.com/v1\""));
    }

    #[test]
    fn universal_codex_provider_custom_prefix_does_not_force_v1() {
        let mut p = UniversalProvider::new(
            "id".to_string(),
            "Test".to_string(),
            "https://example.com/openai".to_string(),
            "sk-test".to_string(),
        );
        p.apps.codex = true;

        let provider = p.to_codex_provider().expect("should build codex provider");
        let toml = provider
            .settings_config
            .get("config")
            .and_then(|v| v.as_str())
            .expect("config should be a toml string");

        assert!(toml.contains("base_url = \"https://example.com/openai\""));
        assert!(!toml.contains("https://example.com/openai/v1"));
    }

    // ── resolve_usage_credentials (per-app credential extraction) ──

    use crate::app_config::AppType;

    fn provider_with(settings_config: serde_json::Value) -> Provider {
        Provider::with_id("p".to_string(), "P".to_string(), settings_config, None)
    }

    #[test]
    fn resolve_credentials_claude_env() {
        let p = provider_with(json!({
            "env": {
                "ANTHROPIC_BASE_URL": "https://api.deepseek.com/anthropic",
                "ANTHROPIC_AUTH_TOKEN": "sk-claude",
            }
        }));
        assert_eq!(
            p.resolve_usage_credentials(&AppType::Claude),
            (
                "https://api.deepseek.com/anthropic".to_string(),
                "sk-claude".to_string()
            )
        );
    }

    #[test]
    fn resolve_credentials_claude_openrouter_fallback() {
        // OpenRouter-on-Claude keeps its key in OPENROUTER_API_KEY; the superset
        // fallback must still find it (regression guard for the per-app refactor).
        let p = provider_with(json!({
            "env": {
                "ANTHROPIC_BASE_URL": "https://openrouter.ai/api/v1",
                "OPENROUTER_API_KEY": "sk-or",
            }
        }));
        let (base_url, api_key) = p.resolve_usage_credentials(&AppType::Claude);
        assert_eq!(base_url, "https://openrouter.ai/api/v1");
        assert_eq!(api_key, "sk-or");
    }

    #[test]
    fn resolve_credentials_codex_auth_and_toml() {
        let p = provider_with(json!({
            "auth": { "OPENAI_API_KEY": "sk-codex" },
            "config": "model_provider = \"deepseek\"\n\
                       [model_providers.deepseek]\n\
                       base_url = \"https://api.deepseek.com\"\n",
        }));
        assert_eq!(
            p.resolve_usage_credentials(&AppType::Codex),
            (
                "https://api.deepseek.com".to_string(),
                "sk-codex".to_string()
            )
        );
    }

    #[test]
    fn resolve_credentials_gemini_env_with_google_fallback() {
        let p = provider_with(json!({
            "env": {
                "GOOGLE_GEMINI_BASE_URL": "https://generativelanguage.googleapis.com",
                "GOOGLE_API_KEY": "g-legacy",
            }
        }));
        let (base_url, api_key) = p.resolve_usage_credentials(&AppType::Gemini);
        assert_eq!(base_url, "https://generativelanguage.googleapis.com");
        assert_eq!(api_key, "g-legacy");
    }

    #[test]
    fn resolve_credentials_claude_skips_empty_primary_key() {
        // Presets seed ANTHROPIC_AUTH_TOKEN as a present-but-empty placeholder.
        // The fallback chain must skip empty values (matching the frontend's
        // `a || b` semantics), not just absent keys.
        let p = provider_with(json!({
            "env": {
                "ANTHROPIC_BASE_URL": "https://openrouter.ai/api/v1",
                "ANTHROPIC_AUTH_TOKEN": "",
                "ANTHROPIC_API_KEY": "",
                "OPENROUTER_API_KEY": "sk-or",
            }
        }));
        let (_, api_key) = p.resolve_usage_credentials(&AppType::Claude);
        assert_eq!(api_key, "sk-or");
    }

    #[test]
    fn resolve_credentials_gemini_skips_empty_primary_key() {
        let p = provider_with(json!({
            "env": {
                "GOOGLE_GEMINI_BASE_URL": "https://generativelanguage.googleapis.com",
                "GEMINI_API_KEY": "",
                "GOOGLE_API_KEY": "g-real",
            }
        }));
        let (_, api_key) = p.resolve_usage_credentials(&AppType::Gemini);
        assert_eq!(api_key, "g-real");
    }

    #[test]
    fn resolve_credentials_claude_desktop_uses_env() {
        // ClaudeDesktop persists the Anthropic env shape (ClaudeDesktopProviderForm
        // reads env.ANTHROPIC_BASE_URL / ANTHROPIC_AUTH_TOKEN), so it resolves via
        // the default env branch — it is NOT unsupported.
        let p = provider_with(json!({
            "env": {
                "ANTHROPIC_BASE_URL": "https://api.deepseek.com/anthropic",
                "ANTHROPIC_AUTH_TOKEN": "sk-desktop",
            }
        }));
        assert_eq!(
            p.resolve_usage_credentials(&AppType::ClaudeDesktop),
            (
                "https://api.deepseek.com/anthropic".to_string(),
                "sk-desktop".to_string()
            )
        );
    }

    #[test]
    fn resolve_credentials_trims_trailing_slash_on_base_url() {
        let p = provider_with(json!({
            "env": {
                "ANTHROPIC_BASE_URL": "https://api.deepseek.com/anthropic/",
                "ANTHROPIC_AUTH_TOKEN": "sk-claude",
            }
        }));
        let (base_url, _) = p.resolve_usage_credentials(&AppType::Claude);
        assert_eq!(base_url, "https://api.deepseek.com/anthropic");
    }

    #[test]
    fn resolve_credentials_missing_fields_yield_empty() {
        let p = provider_with(json!({}));
        assert_eq!(
            p.resolve_usage_credentials(&AppType::Claude),
            (String::new(), String::new())
        );
    }

    fn universal_with_apps(
        id: &str,
        apps: impl FnOnce(&mut UniversalProviderApps),
        models: impl FnOnce(&mut UniversalProviderModels),
    ) -> UniversalProvider {
        let mut universal = UniversalProvider::new(
            id.to_string(),
            "Universal".to_string(),
            "https://api.example.com/".to_string(),
            "api-key".to_string(),
        );
        apps(&mut universal.apps);
        models(&mut universal.models);
        universal
    }

    #[test]
    fn universal_provider_to_claude_desktop_provider_maps_roles() {
        let universal = universal_with_apps(
            "u1",
            |apps| apps.claude_desktop = true,
            |models| {
                models.claude_desktop = Some(ClaudeDesktopModelConfig {
                    mode: Some(ClaudeDesktopMode::Proxy),
                    api_format: Some("openai_chat".to_string()),
                    routes: HashMap::from([
                        (
                            "claude-sonnet-5".to_string(),
                            ClaudeDesktopModelRoute {
                                model: "up-sonnet".to_string(),
                                label_override: Some("Gateway".to_string()),
                                supports_1m: Some(true),
                            },
                        ),
                        (
                            "claude-opus-5".to_string(),
                            ClaudeDesktopModelRoute {
                                model: "up-opus".to_string(),
                                label_override: None,
                                supports_1m: None,
                            },
                        ),
                    ]),
                    ..Default::default()
                })
            },
        );

        let provider = universal
            .to_claude_desktop_provider()
            .expect("claude desktop provider");

        assert_eq!(provider.id, "universal-claude-desktop-u1");
        assert_eq!(
            provider
                .settings_config
                .pointer("/env/ANTHROPIC_BASE_URL")
                .and_then(|item| item.as_str()),
            // 尾斜杠要清掉，否则 Claude Desktop 拼出双斜杠
            Some("https://api.example.com")
        );
        let meta = provider.meta.expect("meta");
        assert_eq!(meta.claude_desktop_mode, Some(ClaudeDesktopMode::Proxy));
        // 映射模式把上游格式透给代理
        assert_eq!(meta.api_format.as_deref(), Some("openai_chat"));
        let routes = &meta.claude_desktop_model_routes;
        // 用户填了几档就只写几档，不再被补齐成固定三档
        assert_eq!(routes.len(), 2, "{routes:?}");
        let sonnet = routes.get("claude-sonnet-5").expect("sonnet");
        assert_eq!(sonnet.model, "up-sonnet");
        assert_eq!(sonnet.label_override.as_deref(), Some("Gateway"));
        assert_eq!(sonnet.supports_1m, Some(true));
        assert_eq!(routes["claude-opus-5"].model, "up-opus");
    }

    #[test]
    fn universal_provider_to_claude_desktop_provider_direct_derives_from_available_models() {
        let mut universal = universal_with_apps(
            "u1",
            |apps| apps.claude_desktop = true,
            |models| {
                models.claude_desktop = Some(ClaudeDesktopModelConfig {
                    mode: Some(ClaudeDesktopMode::Direct),
                    // 直连不允许映射，填了也要被供应商级列表生成的路由顶掉
                    api_format: Some("openai_chat".to_string()),
                    routes: HashMap::from([(
                        "claude-sonnet-5".to_string(),
                        ClaudeDesktopModelRoute {
                            model: "up-sonnet".to_string(),
                            label_override: None,
                            supports_1m: None,
                        },
                    )]),
                    ..Default::default()
                });
            },
        );
        // 供应商级可用模型列表：claude-* 角色进直连路由，1M 按上下文窗口判定；
        // 非 claude-* 的 id 不进。
        universal.available_models = vec![
            AvailableModel {
                id: "claude-sonnet-5".to_string(),
                display_name: Some("Claude Sonnet 5".to_string()),
                context_window: Some(1_000_000),
                efforts: Vec::new(),
                default_effort: None,
                max_output: None,
            },
            AvailableModel {
                id: "claude-opus-5".to_string(),
                display_name: Some("Claude Opus 5".to_string()),
                context_window: Some(200_000),
                efforts: Vec::new(),
                default_effort: None,
                max_output: None,
            },
            AvailableModel {
                id: "gpt-5.6-sol".to_string(),
                display_name: None,
                context_window: Some(1_000_000),
                efforts: Vec::new(),
                default_effort: None,
                max_output: None,
            },
        ];

        let meta = universal
            .to_claude_desktop_provider()
            .expect("provider")
            .meta
            .expect("meta");
        assert_eq!(meta.claude_desktop_mode, Some(ClaudeDesktopMode::Direct));
        // 直连不能带 apiFormat：Desktop 只接受原生 Anthropic
        assert_eq!(meta.api_format, None);
        let routes = &meta.claude_desktop_model_routes;
        // 只有 claude-* 的两条；模型与角色同名
        assert_eq!(routes.len(), 2, "{routes:?}");
        let sonnet = routes.get("claude-sonnet-5").expect("sonnet");
        assert_eq!(sonnet.model, "claude-sonnet-5");
        // 1M 按元数据自动判定：sonnet 1,000,000 → 声明；opus 200,000 → 不声明
        assert_eq!(sonnet.supports_1m, Some(true));
        assert_eq!(routes["claude-opus-5"].supports_1m, None);
        // 存量手填路由不参与：direct 一律以列表生成结果为准
        assert!(routes.get("claude-haiku-4-5").is_none());
    }

    #[test]
    fn universal_provider_to_claude_desktop_provider_direct_without_models_uses_defaults() {
        let universal = universal_with_apps(
            "u1",
            |apps| apps.claude_desktop = true,
            |models| {
                models.claude = Some(ClaudeModelConfig {
                    model: Some("claude-main".to_string()),
                    ..ClaudeModelConfig::default()
                });
                models.claude_desktop = Some(ClaudeDesktopModelConfig {
                    mode: Some(ClaudeDesktopMode::Direct),
                    ..Default::default()
                });
            },
        );

        let meta = universal
            .to_claude_desktop_provider()
            .expect("provider")
            .meta
            .expect("meta");
        // 列表为空时回退默认四档（模型 = 角色 ID）
        assert_eq!(meta.claude_desktop_mode, Some(ClaudeDesktopMode::Direct));
        let routes = &meta.claude_desktop_model_routes;
        assert_eq!(routes.len(), 4, "{routes:?}");
        assert_eq!(
            routes["claude-sonnet-5"].model, "claude-sonnet-5",
            "{routes:?}"
        );
    }

    #[test]
    fn universal_provider_to_claude_desktop_provider_defaults_to_four_proxy_routes() {
        let universal = universal_with_apps(
            "u1",
            |apps| apps.claude_desktop = true,
            |models| {
                models.claude = Some(ClaudeModelConfig {
                    model: Some("claude-main".to_string()),
                    ..ClaudeModelConfig::default()
                })
            },
        );

        let meta = universal
            .to_claude_desktop_provider()
            .expect("provider")
            .meta
            .expect("meta");
        // 没填路由表时用默认四档（含 Fable），全部映射到 Claude 主模型
        assert_eq!(meta.claude_desktop_mode, Some(ClaudeDesktopMode::Proxy));
        let routes = &meta.claude_desktop_model_routes;
        assert_eq!(routes.len(), 4, "{routes:?}");
        for route in [
            "claude-sonnet-5",
            "claude-opus-5",
            "claude-haiku-4-5",
            "claude-fable-5",
        ] {
            assert_eq!(
                routes.get(route).map(|entry| entry.model.as_str()),
                Some("claude-main"),
                "{route}"
            );
        }
        // 默认档不带菜单显示名
        assert!(routes.values().all(|entry| entry.label_override.is_none()));
    }

    #[test]
    fn universal_desktop_provider_migrates_legacy_flat_models() {
        // 4.0.8 及更早存的是扁平的三档 + 共用菜单显示名，投影时要迁移成角色路由
        let universal = universal_with_apps(
            "legacy",
            |apps| apps.claude_desktop = true,
            |models| {
                models.claude_desktop = Some(ClaudeDesktopModelConfig {
                    legacy_sonnet_model: Some("old-sonnet".into()),
                    legacy_opus_model: Some("old-opus".into()),
                    legacy_haiku_model: Some("old-haiku".into()),
                    legacy_label_override: Some("OldGateway".into()),
                    legacy_sonnet_one_m: Some(true),
                    ..Default::default()
                });
            },
        );

        let routes = universal
            .to_claude_desktop_provider()
            .unwrap()
            .meta
            .unwrap()
            .claude_desktop_model_routes;
        assert_eq!(routes["claude-sonnet-5"].model, "old-sonnet");
        assert_eq!(routes["claude-opus-5"].model, "old-opus");
        assert_eq!(routes["claude-haiku-4-5"].model, "old-haiku");
        // 共用的显示名和 1M 声明一起迁移
        assert_eq!(
            routes["claude-sonnet-5"].label_override.as_deref(),
            Some("OldGateway")
        );
        assert_eq!(routes["claude-sonnet-5"].supports_1m, Some(true));
        // 没填的档位不凭空生成
        assert!(!routes.contains_key("claude-fable-5"));
    }

    #[test]
    fn universal_desktop_provider_new_routes_win_over_legacy() {
        let universal = universal_with_apps(
            "both",
            |apps| apps.claude_desktop = true,
            |models| {
                models.claude_desktop = Some(ClaudeDesktopModelConfig {
                    routes: HashMap::from([(
                        "claude-sonnet-5".to_string(),
                        ClaudeDesktopModelRoute {
                            model: "new-sonnet".to_string(),
                            label_override: None,
                            supports_1m: None,
                        },
                    )]),
                    legacy_sonnet_model: Some("old-sonnet".into()),
                    ..Default::default()
                });
            },
        );

        let routes = universal
            .to_claude_desktop_provider()
            .unwrap()
            .meta
            .unwrap()
            .claude_desktop_model_routes;
        assert_eq!(routes["claude-sonnet-5"].model, "new-sonnet");
    }

    #[test]
    fn universal_desktop_provider_legacy_fields_are_not_written_back() {
        // 旧字段只读：序列化出去的记录里不能带 legacy 键
        let config = ClaudeDesktopModelConfig {
            legacy_sonnet_model: Some("old-sonnet".into()),
            legacy_label_override: Some("Old".into()),
            ..Default::default()
        };
        let json = serde_json::to_value(&config).unwrap();
        assert!(json.get("sonnetModel").is_none(), "{json}");
        assert!(json.get("labelOverride").is_none(), "{json}");
    }

    #[test]
    fn universal_provider_to_claude_desktop_provider_disabled_returns_none() {
        let universal = universal_with_apps("u1", |_| {}, |_| {});
        assert!(universal.to_claude_desktop_provider().is_none());
    }

    #[test]
    fn universal_provider_to_grokbuild_provider_builds_config_toml() {
        // 模型表按可用模型列表一家一张；默认模型指向配置里选的那张。
        let mut universal = universal_with_apps(
            "u1",
            |apps| apps.grokbuild = true,
            |models| {
                models.grokbuild = Some(GrokBuildModelConfig {
                    model: Some("grok-4.6".to_string()),
                    api_backend: Some("chat_completions".to_string()),
                })
            },
        );
        universal.available_models = vec![
            AvailableModel {
                id: "grok-4.5".into(),
                display_name: None,
                context_window: Some(500_000),
                efforts: Vec::new(),
                default_effort: None,
                max_output: None,
            },
            AvailableModel {
                id: "grok-4.6".into(),
                display_name: None,
                context_window: Some(2_000_000),
                efforts: Vec::new(),
                default_effort: None,
                max_output: None,
            },
        ];

        let provider = universal
            .to_grokbuild_provider()
            .expect("grokbuild provider");

        assert_eq!(provider.id, "universal-grokbuild-u1");
        let config = provider
            .settings_config
            .get("config")
            .and_then(|item| item.as_str())
            .expect("config toml");
        assert!(
            config.contains("[models]\ndefault = \"grok-4.6\""),
            "{config}"
        );
        // 列表里的每个模型一张表，窗口各取各的元数据
        assert!(config.contains("[model.\"grok-4.5\"]"), "{config}");
        assert!(config.contains("[model.\"grok-4.6\"]"), "{config}");
        assert!(config.contains("context_window = 500000"), "{config}");
        assert!(config.contains("context_window = 2000000"), "{config}");
        assert!(
            config.contains("base_url = \"https://api.example.com\""),
            "{config}"
        );
        assert!(
            config.contains("api_backend = \"chat_completions\""),
            "{config}"
        );
        assert!(config.contains("api_key = \"api-key\""), "{config}");
        // 逐字节锁住形状：前端预览（`grokbuildConfigJson`）按同一条规则生成，两边必须一致。
        assert_eq!(
            config,
            r#"[models]
default = "grok-4.6"

[model."grok-4.5"]
model = "grok-4.5"
base_url = "https://api.example.com"
name = "Universal"
api_key = "api-key"
api_backend = "chat_completions"
context_window = 500000

[model."grok-4.6"]
model = "grok-4.6"
base_url = "https://api.example.com"
name = "Universal"
api_key = "api-key"
api_backend = "chat_completions"
context_window = 2000000
"#
        );
        // 后端自己的校验器要能读回去（和前端的 parseGrokBuildConfig 同一形状）
        crate::grok_config::validate_config_toml(config).expect("valid grok config");
        let model_config =
            crate::grok_config::extract_model_config(config).expect("extract model config");
        assert_eq!(model_config.model, "grok-4.6");
        assert_eq!(model_config.base_url, "https://api.example.com");
        assert_eq!(model_config.api_backend, "chat_completions");
        assert_eq!(model_config.context_window, 2_000_000);
        assert_eq!(
            crate::grok_config::extract_credentials(config),
            // (base_url, api_key)
            Some(("https://api.example.com".to_string(), "api-key".to_string()))
        );
    }

    #[test]
    fn universal_provider_to_grokbuild_provider_disabled_returns_none() {
        let universal = universal_with_apps("u1", |_| {}, |_| {});
        assert!(universal.to_grokbuild_provider().is_none());
    }

    #[test]
    fn universal_provider_apps_default_to_false_for_new_apps() {
        // 旧数据里没有 claudeDesktop / grokbuild 两个键，反序列化后必须是 false
        let value = json!({
            "claude": true,
            "codex": true,
            "gemini": true,
        });
        let apps: UniversalProviderApps = serde_json::from_value(value).expect("apps");
        assert!(apps.claude && apps.codex && apps.gemini);
        assert!(!apps.claude_desktop);
        assert!(!apps.grokbuild);
    }

    #[test]
    fn universal_claude_provider_emits_advanced_fields() {
        let universal = universal_with_apps(
            "adv",
            |apps| apps.claude = true,
            |models| {
                models.claude = Some(ClaudeModelConfig {
                    model: Some("claude-opus-5".into()),
                    sonnet_model: Some("claude-sonnet-5".into()),
                    sonnet_model_one_m: Some(true),
                    sonnet_model_name: Some("Sonnet 5".into()),
                    subagent_model: Some("claude-haiku-4-5".into()),
                    api_format: Some("openai_responses".into()),
                    api_key_field: Some("ANTHROPIC_API_KEY".into()),
                    is_full_url: Some(true),
                    custom_user_agent: Some("relay/1.0".into()),
                    request_overrides: Some(LocalProxyRequestOverrides {
                        headers: HashMap::from([("X-Provider".to_string(), "cc".to_string())]),
                        body: None,
                    }),
                    ..Default::default()
                });
            },
        );

        let provider = universal.to_claude_provider().expect("claude provider");
        let env = &provider.settings_config["env"];
        // 认证字段名跟着选项走，不再是写死的 ANTHROPIC_AUTH_TOKEN
        assert_eq!(env["ANTHROPIC_API_KEY"], json!("api-key"));
        assert!(env.get("ANTHROPIC_AUTH_TOKEN").is_none());
        // 1M 用模型名末尾的标记表达
        assert_eq!(
            env["ANTHROPIC_DEFAULT_SONNET_MODEL"],
            json!("claude-sonnet-5[1M]")
        );
        assert_eq!(env["ANTHROPIC_MODEL"], json!("claude-opus-5"));
        assert_eq!(
            env["ANTHROPIC_DEFAULT_SONNET_MODEL_NAME"],
            json!("Sonnet 5")
        );
        assert_eq!(env["CLAUDE_CODE_SUBAGENT_MODEL"], json!("claude-haiku-4-5"));

        let meta = provider.meta.expect("meta");
        assert_eq!(meta.api_format.as_deref(), Some("openai_responses"));
        assert_eq!(meta.api_key_field.as_deref(), Some("ANTHROPIC_API_KEY"));
        assert_eq!(meta.is_full_url, Some(true));
        assert_eq!(meta.custom_user_agent.as_deref(), Some("relay/1.0"));
        assert_eq!(
            meta.local_proxy_request_overrides
                .expect("overrides")
                .headers["X-Provider"],
            "cc"
        );
    }

    #[test]
    fn universal_claude_provider_emits_upstream_compat_switches() {
        // 三个随供应商切换的兼容开关，取值和单应用表单一致
        let universal = universal_with_apps(
            "compat",
            |apps| apps.claude = true,
            |models| {
                models.claude = Some(ClaudeModelConfig {
                    model: Some("claude-opus-5".into()),
                    enable_tool_search: Some(true),
                    disable_artifact: Some(true),
                    disable_auto_mode_server: Some(true),
                    ..Default::default()
                });
            },
        );

        let env = &universal.to_claude_provider().unwrap().settings_config["env"];
        assert_eq!(env["ENABLE_TOOL_SEARCH"], json!("true"));
        assert_eq!(env["CLAUDE_CODE_DISABLE_ARTIFACT"], json!("1"));
        // auto mode 的服务端检查是「关掉」语义
        assert_eq!(env["CLAUDE_CODE_AUTO_MODE_SERVER"], json!("0"));
    }

    #[test]
    fn universal_claude_provider_omits_upstream_compat_switches_when_off() {
        let universal = universal_with_apps(
            "compat-off",
            |apps| apps.claude = true,
            |models| {
                models.claude = Some(ClaudeModelConfig {
                    model: Some("claude-opus-5".into()),
                    ..Default::default()
                });
            },
        );

        let env = &universal.to_claude_provider().unwrap().settings_config["env"];
        for key in [
            "ENABLE_TOOL_SEARCH",
            "CLAUDE_CODE_DISABLE_ARTIFACT",
            "CLAUDE_CODE_AUTO_MODE_SERVER",
        ] {
            assert!(env.get(key).is_none(), "{key} must not be written when off");
        }
    }

    #[test]
    fn universal_claude_provider_clears_advanced_meta_when_options_removed() {
        // 用户把选项去掉后，子卡不能留着上一版的 meta。
        let mut universal = universal_with_apps(
            "adv-clear",
            |apps| apps.claude = true,
            |models| {
                models.claude = Some(ClaudeModelConfig {
                    model: Some("claude-opus-5".into()),
                    ..Default::default()
                });
            },
        );
        universal.meta = Some(ProviderMeta {
            api_format: Some("openai_chat".into()),
            is_full_url: Some(true),
            ..Default::default()
        });

        let meta = universal.to_claude_provider().unwrap().meta.unwrap();
        assert_eq!(meta.api_format, None);
        assert_eq!(meta.is_full_url, None);
        assert_eq!(meta.api_key_field, None);
    }

    #[test]
    fn universal_codex_provider_generates_catalog_from_available_models() {
        // 表单不再手填目录：没给 catalog_models 时，用可用模型列表自动生成。
        let mut universal = universal_with_apps(
            "codex-auto",
            |apps| apps.codex = true,
            |models| {
                models.codex = Some(CodexModelConfig {
                    model: Some("gpt-5.6-sol".into()),
                    ..Default::default()
                });
            },
        );
        universal.available_models = vec![
            AvailableModel {
                id: "gpt-5.6-sol".into(),
                display_name: Some("GPT-5.6 Sol".into()),
                context_window: Some(1_050_000),
                efforts: Vec::new(),
                default_effort: None,
                max_output: None,
            },
            AvailableModel {
                id: "plain-id".into(),
                display_name: None,
                context_window: None,
                efforts: Vec::new(),
                default_effort: None,
                max_output: None,
            },
        ];

        let catalog = universal.to_codex_provider().unwrap().settings_config["modelCatalog"]
            ["models"]
            .clone();
        let rows = catalog.as_array().expect("catalog rows");

        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0]["model"], json!("gpt-5.6-sol"));
        assert_eq!(rows[0]["displayName"], json!("GPT-5.6 Sol"));
        assert_eq!(rows[0]["contextWindow"], json!(1_050_000));
        // 模型目录没补到的字段就不写，交给后端默认值
        assert_eq!(rows[1]["model"], json!("plain-id"));
        assert!(rows[1].get("displayName").is_none());
        assert!(rows[1].get("contextWindow").is_none());
    }

    #[test]
    fn universal_codex_catalog_efforts_intersect_user_levels() {
        // 目录的 reasoningLevels = 模型元数据 ∩ 用户勾选；用户没勾（空）= 全按
        // 元数据。用户勾了模型没声明的档位不会凭空出现。
        let mut universal = universal_with_apps(
            "codex-effort",
            |apps| apps.codex = true,
            |models| {
                models.codex = Some(CodexModelConfig {
                    model: Some("gpt-5.6-sol".into()),
                    ..Default::default()
                });
            },
        );
        universal.available_models = vec![
            AvailableModel {
                id: "gpt-5.6-sol".into(),
                display_name: None,
                context_window: None,
                efforts: vec!["low".into(), "high".into(), "max".into()],
                default_effort: None,
                max_output: None,
            },
            AvailableModel {
                id: "undeclared".into(),
                display_name: None,
                context_window: None,
                efforts: Vec::new(),
                default_effort: None,
                max_output: None,
            },
        ];
        // 默认档位也逐模型（可选，目录写 defaultReasoningLevel）
        universal.available_models[0].default_effort = Some("high".into());

        let rows = universal.to_codex_provider().unwrap().settings_config["modelCatalog"]["models"]
            .as_array()
            .expect("catalog rows")
            .clone();
        assert_eq!(
            rows[0]["reasoningLevels"],
            json!(["low", "high", "max"]),
            "逐模型档位直接来自该模型的元数据"
        );
        assert_eq!(rows[0]["defaultReasoningLevel"], json!("high"));
        assert!(
            rows[1].get("reasoningLevels").is_none(),
            "没声明的模型不写档位"
        );

        // 默认档改成 low：目录的 defaultReasoningLevel 跟着走（档位列表不变）。
        universal.available_models[0].default_effort = Some("low".into());
        let rows = universal.to_codex_provider().unwrap().settings_config["modelCatalog"]["models"]
            .as_array()
            .expect("catalog rows")
            .clone();
        assert_eq!(rows[0]["defaultReasoningLevel"], json!("low"));
    }

    #[test]
    fn universal_codex_provider_skips_blank_model_ids_in_the_generated_catalog() {
        // 表单里手加的草稿行 id 还是空的：不能进 /model 目录。
        let mut universal = universal_with_apps(
            "codex-blank",
            |apps| apps.codex = true,
            |models| {
                models.codex = Some(CodexModelConfig {
                    model: Some("gpt-5.6-sol".into()),
                    ..Default::default()
                });
            },
        );
        universal.available_models = vec![
            AvailableModel {
                id: "  ".into(),
                display_name: None,
                context_window: None,
                efforts: Vec::new(),
                default_effort: None,
                max_output: None,
            },
            AvailableModel {
                id: "real-model".into(),
                display_name: None,
                context_window: None,
                efforts: Vec::new(),
                default_effort: None,
                max_output: None,
            },
        ];

        let rows = universal.to_codex_provider().unwrap().settings_config["modelCatalog"]["models"]
            .clone();
        let rows = rows.as_array().expect("catalog rows");
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0]["model"], json!("real-model"));
    }

    #[test]
    fn universal_codex_provider_prefers_explicit_catalog_over_the_model_list() {
        // 显式给的目录优先，不被可用模型列表盖掉。
        let mut universal = universal_with_apps(
            "codex-explicit",
            |apps| apps.codex = true,
            |models| {
                models.codex = Some(CodexModelConfig {
                    catalog_models: vec![json!({"model": "explicit-only"})],
                    ..Default::default()
                });
            },
        );
        universal.available_models = vec![AvailableModel {
            id: "from-list".into(),
            display_name: None,
            context_window: None,
            efforts: Vec::new(),
            default_effort: None,
            max_output: None,
        }];

        let rows = universal.to_codex_provider().unwrap().settings_config["modelCatalog"]["models"]
            .clone();
        assert_eq!(rows.as_array().unwrap().len(), 1);
        assert_eq!(rows[0]["model"], json!("explicit-only"));
    }

    #[test]
    fn universal_codex_provider_omits_catalog_when_nothing_is_configured() {
        // 既没手填也没拉取过：不写目录（和「配了空目录」区分开）。
        let universal = universal_with_apps("codex-nocatalog", |apps| apps.codex = true, |_| {});
        let settings = universal.to_codex_provider().unwrap().settings_config;
        assert!(settings.get("modelCatalog").is_none());
    }

    #[test]
    fn universal_codex_provider_emits_config_switches_and_catalog() {
        let mut universal = universal_with_apps(
            "codex-adv",
            |apps| apps.codex = true,
            |models| {
                models.codex = Some(CodexModelConfig {
                    model: Some("gpt-5.6-sol".into()),
                    // 压缩阈值存百分比，投影时按窗口换算成绝对 token 数
                    auto_compact_percent: Some(90),
                    remote_compaction: Some(true),
                    impersonate_claude_code: Some(true),
                    prompt_cache_routing: Some("enabled".into()),
                    api_format: Some("anthropic".into()),
                    subagent_model: Some("gpt-5.6-mini".into()),
                    subagent_reasoning_effort: Some("low".into()),
                    catalog_models: vec![json!({"model": "gpt-5.6-sol", "displayName": "Sol"})],
                    ..Default::default()
                });
            },
        );

        // 上下文窗口跟随所选模型行的元数据
        universal.available_models = vec![AvailableModel {
            id: "gpt-5.6-sol".into(),
            display_name: None,
            context_window: Some(1_000_000),
            efforts: Vec::new(),
            default_effort: None,
            max_output: None,
        }];

        let provider = universal.to_codex_provider().expect("codex provider");
        let toml = provider.settings_config["config"]
            .as_str()
            .expect("toml string");
        // 远程压缩把供应商表名写成 OpenAI；窗口写顶层（跟模型元数据），压缩阈值由
        // 窗口 × 百分比算出（1,000,000 × 90% = 900000）
        assert!(toml.contains(r#"name = "OpenAI""#), "{toml}");
        assert!(toml.contains("model_context_window = 1000000"), "{toml}");
        assert!(
            toml.contains("model_auto_compact_token_limit = 900000"),
            "{toml}"
        );
        // 子代理模型写在 `[agents]` 表里
        assert!(
            toml.contains("[agents]\ndefault_subagent_model = \"gpt-5.6-mini\""),
            "{toml}"
        );
        assert!(
            toml.contains("default_subagent_reasoning_effort = \"low\""),
            "{toml}"
        );
        // TOML 的表头会吃掉后面的键：窗口字段必须是顶层，不能在 `[model_providers.custom]` 里
        let doc: toml::Value = toml.parse().expect("valid toml");
        assert_eq!(
            doc.get("model_context_window")
                .and_then(toml::Value::as_integer),
            Some(1000000),
            "model_context_window must be top-level, got: {toml}"
        );
        assert!(
            doc.get("model_providers")
                .and_then(|tables| tables.get("custom"))
                .and_then(|custom| custom.get("model_context_window"))
                .is_none(),
            "model_context_window must not land inside the provider table: {toml}"
        );
        assert_eq!(
            doc.get("agents")
                .and_then(|agents| agents.get("default_subagent_model"))
                .and_then(toml::Value::as_str),
            Some("gpt-5.6-mini")
        );
        assert_eq!(
            provider.settings_config["modelCatalog"]["models"][0]["displayName"],
            json!("Sol")
        );

        let meta = provider.meta.expect("meta");
        assert_eq!(meta.api_format.as_deref(), Some("anthropic"));
        assert_eq!(meta.impersonate_claude_code, Some(true));
        // max_output_tokens 跟所选模型行的元数据（此测试的行没有 max_output）
        assert_eq!(meta.max_output_tokens, None);
        assert_eq!(meta.prompt_cache_routing.as_deref(), Some("enabled"));
    }

    #[test]
    fn universal_codex_provider_skips_compact_limit_without_window_metadata() {
        // 压缩阈值是窗口的百分比：所选模型行没有 contextWindow 时换算不出来，
        // 整条 `model_auto_compact_token_limit` 不写（写了只能是瞎猜的绝对值）。
        let universal = universal_with_apps(
            "codex-percent-nowindow",
            |apps| apps.codex = true,
            |models| {
                models.codex = Some(CodexModelConfig {
                    model: Some("gpt-5.6-sol".into()),
                    auto_compact_percent: Some(90),
                    ..Default::default()
                });
            },
        );

        let provider = universal.to_codex_provider().unwrap();
        let toml = provider.settings_config["config"].as_str().unwrap();
        assert!(!toml.contains("model_context_window"), "{toml}");
        assert!(!toml.contains("model_auto_compact_token_limit"), "{toml}");
    }

    #[test]
    fn universal_codex_provider_omits_config_switches_when_off() {
        let universal = universal_with_apps(
            "codex-off",
            |apps| apps.codex = true,
            |models| {
                models.codex = Some(CodexModelConfig {
                    model: Some("gpt-5.6-sol".into()),
                    ..Default::default()
                });
            },
        );

        let provider = universal.to_codex_provider().unwrap();
        let toml = provider.settings_config["config"].as_str().unwrap();
        assert!(toml.contains(r#"name = "NewAPI""#), "{toml}");
        assert!(!toml.contains("model_context_window"), "{toml}");
        assert!(provider.settings_config.get("modelCatalog").is_none());
        // promptCacheRouting 的 auto 是默认值，不写进 meta
        assert_eq!(provider.meta.unwrap().prompt_cache_routing, None);
    }

    #[test]
    fn universal_desktop_provider_keeps_per_route_1m() {
        let universal = universal_with_apps(
            "desktop-1m",
            |apps| apps.claude_desktop = true,
            |models| {
                models.claude_desktop = Some(ClaudeDesktopModelConfig {
                    routes: HashMap::from([
                        (
                            "claude-sonnet-5".to_string(),
                            ClaudeDesktopModelRoute {
                                model: "up-sonnet".to_string(),
                                label_override: None,
                                supports_1m: None,
                            },
                        ),
                        (
                            "claude-opus-5".to_string(),
                            ClaudeDesktopModelRoute {
                                model: "up-opus".to_string(),
                                label_override: None,
                                supports_1m: Some(true),
                            },
                        ),
                    ]),
                    ..Default::default()
                });
            },
        );

        let routes = universal
            .to_claude_desktop_provider()
            .unwrap()
            .meta
            .unwrap()
            .claude_desktop_model_routes;
        assert_eq!(routes["claude-opus-5"].supports_1m, Some(true));
        assert_eq!(routes["claude-sonnet-5"].supports_1m, None);
    }

    #[test]
    fn universal_desktop_provider_derives_menu_label_from_model_metadata() {
        // 菜单显示名没存时取所选模型行的 displayName（模型目录的名称）；
        // 该行没有名字时留空，由 Desktop 回退成模型名。显式存的值优先。
        let mut universal = universal_with_apps(
            "desktop-label",
            |apps| apps.claude_desktop = true,
            |models| {
                models.claude_desktop = Some(ClaudeDesktopModelConfig {
                    routes: HashMap::from([
                        (
                            "claude-sonnet-5".to_string(),
                            ClaudeDesktopModelRoute {
                                model: "deepseek-v4-pro".to_string(),
                                label_override: None,
                                supports_1m: None,
                            },
                        ),
                        (
                            "claude-opus-5".to_string(),
                            ClaudeDesktopModelRoute {
                                model: "no-name-model".to_string(),
                                label_override: None,
                                supports_1m: None,
                            },
                        ),
                        (
                            "claude-haiku-4-5".to_string(),
                            ClaudeDesktopModelRoute {
                                model: "glm-5.3-flash".to_string(),
                                label_override: Some("Custom Label".to_string()),
                                supports_1m: None,
                            },
                        ),
                    ]),
                    ..Default::default()
                });
            },
        );
        universal.available_models = vec![
            AvailableModel {
                id: "deepseek-v4-pro".to_string(),
                display_name: Some("DeepSeek V4 Pro".to_string()),
                context_window: Some(128_000),
                efforts: Vec::new(),
                default_effort: None,
                max_output: None,
            },
            AvailableModel {
                id: "no-name-model".to_string(),
                display_name: None,
                context_window: Some(128_000),
                efforts: Vec::new(),
                default_effort: None,
                max_output: None,
            },
            AvailableModel {
                id: "glm-5.3-flash".to_string(),
                display_name: Some("GLM-5.3 Flash".to_string()),
                context_window: Some(200_000),
                efforts: Vec::new(),
                default_effort: None,
                max_output: None,
            },
        ];

        let routes = universal
            .to_claude_desktop_provider()
            .unwrap()
            .meta
            .unwrap()
            .claude_desktop_model_routes;
        assert_eq!(
            routes["claude-sonnet-5"].label_override.as_deref(),
            Some("DeepSeek V4 Pro")
        );
        assert_eq!(routes["claude-opus-5"].label_override, None);
        // 手填的值不覆盖
        assert_eq!(
            routes["claude-haiku-4-5"].label_override.as_deref(),
            Some("Custom Label")
        );
    }

    #[test]
    fn universal_gemini_provider_carries_extra_env_and_settings() {
        let universal = universal_with_apps(
            "gemini-extra",
            |apps| apps.gemini = true,
            |models| {
                models.gemini = Some(GeminiModelConfig {
                    model: Some("gemini-3.6-flash".into()),
                    env: Some(HashMap::from([(
                        "GEMINI_CLI_CUSTOM_HEADERS".to_string(),
                        "X-Test: 1".to_string(),
                    )])),
                    config: Some(json!({"telemetry": {"enabled": false}})),
                });
            },
        );

        let provider = universal.to_gemini_provider().unwrap();
        assert_eq!(
            provider.settings_config["env"]["GEMINI_CLI_CUSTOM_HEADERS"],
            json!("X-Test: 1")
        );
        assert_eq!(
            provider.settings_config["config"]["telemetry"]["enabled"],
            json!(false)
        );
    }

    #[test]
    fn universal_grokbuild_provider_uses_model_row_context_window() {
        // 手填入口已移除：窗口永远跟 Grok 卡所选模型行（配置档位）的元数据。
        let mut universal = universal_with_apps(
            "grok-cw",
            |apps| apps.grokbuild = true,
            |models| {
                models.grokbuild = Some(GrokBuildModelConfig {
                    model: Some("grok-4.5".into()),
                    ..Default::default()
                });
            },
        );
        universal.available_models = vec![AvailableModel {
            id: "grok-4.5".into(),
            display_name: None,
            context_window: Some(1_000_000),
            efforts: Vec::new(),
            default_effort: None,
            max_output: None,
        }];

        let config = universal.to_grokbuild_provider().unwrap().settings_config["config"]
            .as_str()
            .unwrap()
            .to_string();
        assert!(config.contains("context_window = 1000000"), "{config}");
    }

    #[test]
    fn universal_codex_max_output_falls_back_to_selected_model_row() {
        // max_output_tokens 手填优先；没填时从所选模型行的 limit.output 自动补。
        let mut universal = universal_with_apps(
            "codex-mo",
            |apps| apps.codex = true,
            |models| {
                models.codex = Some(CodexModelConfig {
                    model: Some("gpt-5.6-sol".into()),
                    ..Default::default()
                });
            },
        );
        universal.available_models = vec![
            AvailableModel {
                id: "gpt-5.6-sol".into(),
                display_name: None,
                context_window: Some(1_050_000),
                efforts: Vec::new(),
                default_effort: None,
                max_output: Some(128_000),
            },
            AvailableModel {
                id: "other".into(),
                display_name: None,
                context_window: None,
                efforts: Vec::new(),
                default_effort: None,
                max_output: Some(999),
            },
        ];

        let meta = universal.to_codex_provider().unwrap().meta.expect("meta");
        assert_eq!(meta.max_output_tokens, Some(128_000), "自动取所选行的值");
    }

    #[test]
    fn universal_grokbuild_context_window_falls_back_to_model_row() {
        // 每张表的窗口取自己那行的元数据；取不到才用保守默认 500_000。
        let mut universal = universal_with_apps(
            "grok-auto",
            |apps| apps.grokbuild = true,
            |models| {
                models.grokbuild = Some(GrokBuildModelConfig {
                    model: Some("grok-4.5".into()),
                    ..Default::default()
                });
            },
        );
        universal.available_models = vec![
            AvailableModel {
                id: "grok-4.5".into(),
                display_name: None,
                context_window: Some(2_000_000),
                efforts: Vec::new(),
                default_effort: None,
                max_output: None,
            },
            AvailableModel {
                id: "grok-4.6".into(),
                display_name: None,
                context_window: None,
                efforts: Vec::new(),
                default_effort: None,
                max_output: None,
            },
        ];

        let config = universal.to_grokbuild_provider().unwrap().settings_config["config"]
            .as_str()
            .unwrap()
            .to_string();
        assert!(config.contains("[model.\"grok-4.5\"]"), "{config}");
        assert!(config.contains("context_window = 2000000"), "{config}");
        // 没有元数据的行用默认窗口
        assert!(config.contains("[model.\"grok-4.6\"]"), "{config}");
        assert!(config.contains("context_window = 500000"), "{config}");

        // 没有可用模型列表：只写默认模型那一张表，窗口用默认值
        let mut empty = universal_with_apps(
            "grok-none",
            |apps| apps.grokbuild = true,
            |models| {
                models.grokbuild = Some(GrokBuildModelConfig {
                    model: Some("unknown-model".into()),
                    ..Default::default()
                });
            },
        );
        empty.available_models = Vec::new();
        let config = empty.to_grokbuild_provider().unwrap().settings_config["config"]
            .as_str()
            .unwrap()
            .to_string();
        assert!(config.contains("default = \"unknown-model\""), "{config}");
        assert!(config.contains("[model.\"unknown-model\"]"), "{config}");
        assert!(config.contains("context_window = 500000"), "{config}");
        // 列表为空且没配默认模型：退回内置默认模型
        let bare = universal_with_apps("grok-bare", |apps| apps.grokbuild = true, |_| {});
        let config = bare.to_grokbuild_provider().unwrap().settings_config["config"]
            .as_str()
            .unwrap()
            .to_string();
        assert!(config.contains("default = \"grok-4.5\""), "{config}");
        assert!(config.contains("[model.\"grok-4.5\"]"), "{config}");
    }
}
