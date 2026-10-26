//! per-app 供应商 → 统一供应商的一次性迁移。
//!
//! 统一供应商已经取代 per-app 供应商成为唯一的新增入口，历史库里还留着按应用分开的
//! 供应商行。本模块把这些行合并成统一供应商，让列表里只剩统一供应商和它们同步出来的
//! 子卡。
//!
//! 归组规则：**baseUrl 和 apiKey 都相同**的行才算同一家网关，合成一个统一供应商，并
//! 开启它出现过的每个应用。任一不同就各算一家——同一个网关地址下的不同 Key 对应不同
//! 额度，合并会丢配置。
//!
//! 不做的事（有意）：
//! - 官方种子（`is_official_seed_id`）原样保留，它们是「一键切回官方」的入口，不是用户
//!   自建的第三方供应商。
//! - 已经由统一供应商同步出来的子卡（`universal-<app>-<id>`）和启动期从 live 导入的
//!   `default` 行同样跳过，否则会被再包一层。
//! - `requires_openai_auth`、`wire_api`、`service_tier` 这类在 Codex live 里由投影现算
//!   的键不迁移：写 live 时按凭据重新决定，存储值本来就不参与。

use std::collections::HashMap;

use serde_json::Value;

use crate::app_config::AppType;
use crate::error::AppError;
use crate::provider::{
    AvailableModel, ClaudeDesktopModelConfig, ClaudeDesktopModelRoute, ClaudeModelConfig,
    CodexModelConfig, GeminiModelConfig, GrokBuildModelConfig, Provider, ProviderMeta,
    UniversalProvider, UniversalProviderApps, UniversalProviderModels,
};
use crate::store::AppState;

/// 迁移完成标记。写进 `settings` 表，避免重复跑。
const MIGRATED_FLAG: &str = "per_app_providers_migrated_to_universal_v1";

/// 迁移前的 `providers` 快照 key，留给人工回退。
const SNAPSHOT_KEY: &str = "per_app_providers_migration_snapshot_v1";

/// 「补齐为全应用」的完成标记。去掉 app 开关后旧记录可能只开了部分应用。
const ALL_APPS_FLAG: &str = "universal_providers_all_apps_v1";

/// 一个待合并的源供应商：来自哪个应用、原本长什么样。
struct SourceRow {
    app: AppType,
    provider: Provider,
}

/// 迁移结果，供日志与测试断言。
#[derive(Debug, Default, PartialEq, Eq)]
pub struct MigrationOutcome {
    /// 新建的统一供应商数量。
    pub universal_created: usize,
    /// 删除的 per-app 行数量。
    pub rows_removed: usize,
    /// 重映射到子卡的用量明细行数。
    pub usage_rows_remapped: usize,
    /// 跳过的行数（官方种子、子卡、认不出地址的行）。
    pub rows_skipped: usize,
}

/// 判断一行是否该跳过迁移。
///
/// - 官方种子：用户的「切回官方」入口，不属于第三方配置。
/// - `universal-` 前缀：已经是统一供应商同步出来的子卡。
/// - `default`：启动期从 live 导入的产物，没有稳定的用户配置。
fn is_protected_row(id: &str) -> bool {
    crate::database::is_official_seed_id(id) || id.starts_with("universal-") || id == "default"
}

/// 从 settings_config 里取出这一行的 (baseUrl, apiKey)。
///
/// 各应用的形状不同：Claude / Gemini 把地址和 Key 放在 `env`，Codex 的地址埋在
/// `config`（TOML 文本）的 `[model_providers.custom].base_url` 里、Key 在
/// `auth.OPENAI_API_KEY`，Grok Build 的两个值都在 TOML 文本里。
fn extract_endpoint(app: &AppType, settings: &Value) -> Option<(String, String)> {
    let env_pair = |url_key: &str, key_keys: &[&str]| -> Option<(String, String)> {
        let env = settings.get("env")?;
        let base = env.get(url_key)?.as_str()?.trim();
        if base.is_empty() {
            return None;
        }
        let key = key_keys
            .iter()
            .find_map(|k| env.get(*k).and_then(Value::as_str))
            .unwrap_or("")
            .trim()
            .to_string();
        Some((base.to_string(), key))
    };

    match app {
        AppType::Claude | AppType::ClaudeDesktop => env_pair(
            "ANTHROPIC_BASE_URL",
            &["ANTHROPIC_AUTH_TOKEN", "ANTHROPIC_API_KEY"],
        ),
        AppType::Gemini => env_pair("GOOGLE_GEMINI_BASE_URL", &["GEMINI_API_KEY"]),
        AppType::Codex => {
            let doc = parse_config_toml(settings)?;
            let base = doc
                .get("model_providers")?
                .get("custom")?
                .get("base_url")?
                .as_str()?
                .trim();
            if base.is_empty() {
                return None;
            }
            let key = settings
                .get("auth")
                .and_then(|auth| auth.get("OPENAI_API_KEY"))
                .and_then(Value::as_str)
                .unwrap_or("")
                .trim()
                .to_string();
            Some((base.to_string(), key))
        }
        AppType::GrokBuild => {
            let doc = parse_config_toml(settings)?;
            let entry = grok_default_entry(&doc)?;
            let base = entry.get("base_url")?.as_str()?.trim();
            if base.is_empty() {
                return None;
            }
            let key = entry
                .get("api_key")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .trim()
                .to_string();
            Some((base.to_string(), key))
        }
    }
}

/// 解析 `settings_config.config`（Codex / Grok Build 的 TOML 文本）。
fn parse_config_toml(settings: &Value) -> Option<toml::Table> {
    settings.get("config")?.as_str()?.parse().ok()
}

/// Grok Build 的 `[model."<default>"]` 表。
fn grok_default_entry(doc: &toml::Table) -> Option<&toml::Value> {
    let profile = doc.get("models")?.get("default")?.as_str()?;
    doc.get("model")?.get(profile)
}

/// 取 TOML 里的字符串字段，空串按缺失处理。
fn toml_str(value: Option<&toml::Value>) -> Option<String> {
    value
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

/// 绝对 token 数 → 窗口百分比（四舍五入，至少 1）。统一供应商只存百分比，
/// 编辑时按所选模型的窗口重算，所以这里必须落在 1-100 内。
///
/// 乘 100 用饱和运算：阈值来自用户手写的 config.toml，没有上界，溢出在 debug
/// 构建里会 panic（迁移发生在启动路径上，不能因为一个荒唐的数字挂掉）。
fn compact_percent(limit: i64, window: i64) -> u32 {
    let percent = (limit.saturating_mul(100).saturating_add(window / 2)) / window;
    u32::try_from(percent.clamp(1, 100)).unwrap_or(100)
}

/// 把 `model[1M]` 拆成 (模型名, 是否声明 1M)。
fn split_one_m(raw: Option<String>) -> (Option<String>, Option<bool>) {
    match raw {
        Some(v) if v.ends_with("[1M]") => {
            let name = v.trim_end_matches("[1M]").trim().to_string();
            (Some(name), Some(true))
        }
        other => (other, None),
    }
}

/// 把一行的 settings_config 反解析成该应用的统一供应商模型块。
///
/// 只覆盖统一供应商能表达、且投影会写回去的字段。
fn model_config_for(app: &AppType, settings: &Value) -> Option<UniversalProviderModels> {
    let mut models = UniversalProviderModels::default();
    match app {
        AppType::Claude => {
            let env = settings.get("env")?;
            let at = |key: &str| {
                env.get(key)
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|v| !v.is_empty())
                    .map(str::to_string)
            };
            let (model, model_one_m) = split_one_m(at("ANTHROPIC_MODEL"));
            let (haiku, haiku_model_one_m) = split_one_m(at("ANTHROPIC_DEFAULT_HAIKU_MODEL"));
            let (sonnet, sonnet_model_one_m) = split_one_m(at("ANTHROPIC_DEFAULT_SONNET_MODEL"));
            let (opus, opus_model_one_m) = split_one_m(at("ANTHROPIC_DEFAULT_OPUS_MODEL"));
            let (fable, fable_model_one_m) = split_one_m(at("ANTHROPIC_DEFAULT_FABLE_MODEL"));
            let (subagent_model, subagent_model_one_m) =
                split_one_m(at("CLAUDE_CODE_SUBAGENT_MODEL"));
            models.claude = Some(ClaudeModelConfig {
                model,
                haiku_model: haiku,
                sonnet_model: sonnet,
                opus_model: opus,
                fable_model: fable,
                model_one_m,
                haiku_model_one_m,
                sonnet_model_one_m,
                opus_model_one_m,
                fable_model_one_m,
                subagent_model,
                subagent_model_one_m,
                model_name: at("ANTHROPIC_MODEL_NAME"),
                haiku_model_name: at("ANTHROPIC_DEFAULT_HAIKU_MODEL_NAME"),
                sonnet_model_name: at("ANTHROPIC_DEFAULT_SONNET_MODEL_NAME"),
                opus_model_name: at("ANTHROPIC_DEFAULT_OPUS_MODEL_NAME"),
                fable_model_name: at("ANTHROPIC_DEFAULT_FABLE_MODEL_NAME"),
                ..Default::default()
            });
        }
        AppType::Codex => {
            let doc = parse_config_toml(settings)?;
            let mut codex = CodexModelConfig {
                model: toml_str(doc.get("model")),
                reasoning_effort: toml_str(doc.get("model_reasoning_effort")),
                ..Default::default()
            };
            // 压缩阈值在旧行里是绝对 token 数，统一供应商存的是窗口的百分比：拿同一份
            // 配置里的顶层 `model_context_window` 反算（旧表单只在勾了 1M 时才允许填
            // 阈值，那时窗口一定在）。算不出来就丢掉——倒推百分比只能靠猜，而 Codex
            // 自己的默认就是窗口的 90%，丢掉等于回到默认，写错才是真出事。
            if let Some(limit) = doc
                .get("model_auto_compact_token_limit")
                .and_then(|v| v.as_integer())
            {
                codex.auto_compact_percent = doc
                    .get("model_context_window")
                    .and_then(|v| v.as_integer())
                    .filter(|window| *window > 0)
                    .map(|window| compact_percent(limit, window));
            }
            if let Some(agents) = doc.get("agents") {
                codex.subagent_model = toml_str(agents.get("default_subagent_model"));
                codex.subagent_reasoning_effort =
                    toml_str(agents.get("default_subagent_reasoning_effort"));
            }
            // 模型目录写在 settings_config.modelCatalog.models。
            if let Some(catalog) = settings
                .get("modelCatalog")
                .and_then(|c| c.get("models"))
                .and_then(Value::as_array)
            {
                codex.catalog_models = catalog.clone();
            }
            models.codex = Some(codex);
        }
        AppType::Gemini => {
            let env = settings.get("env")?;
            // 关键字段之外的键进 `.env` 编辑器，跟着这个应用一起切换。
            let mut extra: HashMap<String, String> = HashMap::new();
            for (key, value) in env.as_object()? {
                if matches!(
                    key.as_str(),
                    "GOOGLE_GEMINI_BASE_URL" | "GEMINI_API_KEY" | "GEMINI_MODEL"
                ) {
                    continue;
                }
                if let Some(text) = value.as_str() {
                    extra.insert(key.clone(), text.to_string());
                }
            }
            models.gemini = Some(GeminiModelConfig {
                model: env
                    .get("GEMINI_MODEL")
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|v| !v.is_empty())
                    .map(str::to_string),
                env: (!extra.is_empty()).then_some(extra),
                config: settings
                    .get("config")
                    .and_then(Value::as_object)
                    .filter(|o| !o.is_empty())
                    .map(|o| o.iter().map(|(k, v)| (k.clone(), v.clone())).collect()),
            });
        }
        AppType::GrokBuild => {
            let doc = parse_config_toml(settings)?;
            let profile = doc.get("models")?.get("default")?.as_str()?;
            let entry = grok_default_entry(&doc)?;
            models.grokbuild = Some(GrokBuildModelConfig {
                model: Some(profile.to_string()),
                api_backend: toml_str(entry.get("api_backend")),
            });
        }
        // Claude Desktop 的模型配置全在 meta（接入方式 + 角色路由），单独处理。
        AppType::ClaudeDesktop => {}
    }
    Some(models)
}

/// Claude 的上游兼容开关住在 env 里，需要搬进 `models.claude` 的对应字段。
fn claude_switches(settings: &Value) -> Option<ClaudeModelConfig> {
    let env = settings.get("env")?;
    let mut block = ClaudeModelConfig::default();
    let mut any = false;
    for (key, apply) in [
        ("ENABLE_TOOL_SEARCH", 0usize),
        ("CLAUDE_CODE_DISABLE_ARTIFACT", 1),
        ("CLAUDE_CODE_AUTO_MODE_SERVER", 2),
    ] {
        let Some(raw) = env.get(key).and_then(Value::as_str) else {
            continue;
        };
        let raw = raw.trim();
        any = true;
        match apply {
            // 单应用表单的写法：tool search 开写 "true"，auto mode 的服务端检查是
            // 「关掉」语义写 "0"，其余写 "1"。
            0 => block.enable_tool_search = Some(raw == "true" || raw == "1"),
            1 => block.disable_artifact = Some(raw == "1" || raw == "true"),
            _ => block.disable_auto_mode_server = Some(raw == "0"),
        }
    }
    any.then_some(block)
}

/// Grok Build 的 `meta.apiFormat`（链接词表）换算成模型表的 `api_backend`。
///
/// 深链接按「一套格式词」声明上游协议，per-app 行把它记在 meta 上；统一供应商只有
/// `apiBackend` 一个字段，不换算的话导入或迁移后协议会退回默认的 responses。
fn grok_api_backend(api_format: &str) -> Option<&'static str> {
    match api_format.trim() {
        "openai_chat" | "chat_completions" => Some("chat_completions"),
        "openai_responses" => Some("responses"),
        "anthropic" | "anthropic_messages" => Some("anthropic_messages"),
        _ => None,
    }
}

/// 把行 meta 里「统一供应商也认识」的应用专属字段并进 per-app 模型块。
fn merge_meta_into_models(
    app: &AppType,
    meta: Option<&ProviderMeta>,
    models: &mut UniversalProviderModels,
) {
    let Some(m) = meta else {
        return;
    };
    match app {
        AppType::Claude => {
            let claude = models.claude.get_or_insert_with(ClaudeModelConfig::default);
            claude.api_format = m.api_format.clone();
            claude.api_key_field = m.api_key_field.clone();
            claude.is_full_url = m.is_full_url;
            claude.custom_user_agent = m.custom_user_agent.clone();
            claude.request_overrides = m.local_proxy_request_overrides.clone();
        }
        AppType::ClaudeDesktop => {
            let routes: HashMap<String, ClaudeDesktopModelRoute> =
                m.claude_desktop_model_routes.clone();
            models.claude_desktop = Some(ClaudeDesktopModelConfig {
                mode: m.claude_desktop_mode.clone(),
                api_format: m.api_format.clone(),
                routes,
                ..Default::default()
            });
        }
        // Codex 的专属设置全在行 meta（apiFormat / 完整 URL / UA / 请求覆盖 / 兼容
        // 开关 / 思考声明 / 缓存路由 / 最大输出）。不搬的话，迁移后这些开关在表单里
        // 全部回到默认值，`to_codex_provider` 也不会再写进生成的子卡。
        AppType::Codex => {
            let codex = models.codex.get_or_insert_with(CodexModelConfig::default);
            codex.api_format = m.api_format.clone();
            codex.is_full_url = m.is_full_url;
            codex.custom_user_agent = m.custom_user_agent.clone();
            codex.request_overrides = m.local_proxy_request_overrides.clone();
            codex.impersonate_claude_code = m.impersonate_claude_code;
            codex.prompt_cache_routing = m.prompt_cache_routing.clone();
            codex.chat_reasoning = m.codex_chat_reasoning.clone();
        }
        AppType::GrokBuild => {
            // TOML 里已经写了 api_backend 的照旧；meta 里另有一套链接词表时以它为准
            // （深链接导入的行走这条）。
            if let Some(backend) = m.api_format.as_deref().and_then(grok_api_backend) {
                let grok = models
                    .grokbuild
                    .get_or_insert_with(GrokBuildModelConfig::default);
                grok.api_backend = Some(backend.to_string());
            }
        }
        _ => {}
    }
}

/// 组内第一个非空值优先：用量脚本、端点自动选择、通用配置开关只可能是同一家。
fn merge_meta(target: &mut ProviderMeta, source: &ProviderMeta) {
    if target.usage_script.is_none() {
        target.usage_script = source.usage_script.clone();
    }
    if target.endpoint_auto_select.is_none() {
        target.endpoint_auto_select = source.endpoint_auto_select;
    }
    if target.common_config_enabled.is_none() {
        target.common_config_enabled = source.common_config_enabled;
    }
    if target.custom_endpoints.is_empty() {
        target.custom_endpoints = source.custom_endpoints.clone();
    }
}

/// 链接只声明了几个模型名，但统一供应商的表单和子卡都从 `available_models` 取模型与
/// 窗口元数据：不种进去的话导入后模型下拉是空的，Grok Build / Codex 的上下文窗口也会
/// 退回默认值。这里把来源应用配置里出现的模型 id 收进列表；窗口能从 config.toml 读到的
/// 一起带上（Claude 的 `[1M]` 声明存在模型块自己的字段里，投影时照写）。
fn seed_available_models(
    app: &AppType,
    provider: &Provider,
    models: &UniversalProviderModels,
) -> Vec<AvailableModel> {
    let mut ids: Vec<String> = Vec::new();
    let mut push = |value: Option<&String>| {
        let Some(id) = value.map(|v| v.trim()).filter(|v| !v.is_empty()) else {
            return;
        };
        if !ids.iter().any(|existing| existing == id) {
            ids.push(id.to_string());
        }
    };
    if let Some(claude) = models.claude.as_ref() {
        for value in [
            &claude.model,
            &claude.haiku_model,
            &claude.sonnet_model,
            &claude.opus_model,
            &claude.fable_model,
            &claude.subagent_model,
        ] {
            push(value.as_ref());
        }
    }
    push(models.codex.as_ref().and_then(|m| m.model.as_ref()));
    push(models.gemini.as_ref().and_then(|m| m.model.as_ref()));
    push(models.grokbuild.as_ref().and_then(|m| m.model.as_ref()));

    // 只有一张模型表时窗口属于它；Codex 的顶层窗口同理。
    let window = match app {
        AppType::GrokBuild => parse_config_toml(&provider.settings_config)
            .as_ref()
            .and_then(grok_default_entry)
            .and_then(|entry| entry.get("context_window"))
            .and_then(|value| value.as_integer())
            .and_then(|value| u64::try_from(value).ok()),
        AppType::Codex => parse_config_toml(&provider.settings_config)
            .as_ref()
            .and_then(|doc| doc.get("model_context_window"))
            .and_then(|value| value.as_integer())
            .and_then(|value| u64::try_from(value).ok()),
        _ => None,
    };

    ids.into_iter()
        .map(|id| AvailableModel {
            id,
            display_name: None,
            context_window: window,
            efforts: Vec::new(),
            default_effort: None,
            max_output: None,
        })
        .collect()
}

/// 把一份 per-app 配置转成统一供应商（深链接导入用）。
///
/// 链接只描述一个应用的模型与连接；统一供应商把这份配置放进该应用的模型块，其余
/// 应用按各自的默认值同步。字段映射与 per-app 迁移共用同一套规则，避免两条「转成
/// 统一供应商」的路各写一遍、慢慢长歪。
///
/// `provider` 应当是一份已经过新增校验的行（`ProviderService::prepare_new_provider`），
/// 凭据覆盖、Claude 模型补全等规范化都已做过；这里只做字段搬运。
pub fn universal_from_provider(
    app: &AppType,
    provider: &Provider,
) -> Result<UniversalProvider, AppError> {
    let (base_url, api_key) =
        extract_endpoint(app, &provider.settings_config).ok_or_else(|| {
            AppError::InvalidInput(format!(
                "Cannot derive the API endpoint and key for '{}' from the provider configuration",
                app.as_str()
            ))
        })?;

    let mut models = model_config_for(app, &provider.settings_config).unwrap_or_default();
    // Claude 的上游兼容开关住在 env 里，和迁移一样叠到模型块上（投影会再写回 env）。
    if matches!(app, AppType::Claude) {
        if let (Some(claude), Some(switches)) = (
            models.claude.as_mut(),
            claude_switches(&provider.settings_config),
        ) {
            claude.enable_tool_search = switches.enable_tool_search;
            claude.disable_artifact = switches.disable_artifact;
            claude.disable_auto_mode_server = switches.disable_auto_mode_server;
        }
    }
    merge_meta_into_models(app, provider.meta.as_ref(), &mut models);

    let mut meta = ProviderMeta::default();
    if let Some(source) = provider.meta.as_ref() {
        merge_meta(&mut meta, source);
    }
    // 旧版本仍按这个标记合并通用配置片段：深链接新建行和表单新增保持一致。
    if matches!(app, AppType::Claude | AppType::Codex | AppType::Gemini)
        && meta.common_config_enabled.is_none()
    {
        meta.common_config_enabled = Some(true);
    }

    let available_models = seed_available_models(app, provider, &models);
    Ok(UniversalProvider {
        id: provider.id.clone(),
        name: provider.name.clone(),
        apps: UniversalProviderApps::all(),
        base_url,
        api_key,
        models,
        available_models,
        website_url: provider.website_url.clone(),
        notes: provider.notes.clone(),
        icon: provider.icon.clone(),
        icon_color: provider.icon_color.clone(),
        meta: Some(meta),
        created_at: provider
            .created_at
            .or_else(|| Some(chrono::Utc::now().timestamp_millis())),
        sort_index: None,
    })
}

/// 执行迁移：把 per-app 供应商合并成统一供应商。
///
/// 幂等：`MIGRATED_FLAG` 已置位时直接返回。
pub fn migrate_per_app_to_universal(state: &AppState) -> Result<MigrationOutcome, AppError> {
    if state.db.get_bool_flag(MIGRATED_FLAG).unwrap_or(false) {
        return Ok(MigrationOutcome::default());
    }

    let mut outcome = MigrationOutcome::default();

    // 1. 收集各应用里可迁移的行，按 (baseUrl, apiKey) 归组。用 Vec + 索引表保持稳定顺序。
    let mut groups: Vec<(String, String, Vec<SourceRow>)> = Vec::new();
    let mut index: HashMap<(String, String), usize> = HashMap::new();
    for app in AppType::all() {
        for (_, provider) in state.db.get_all_providers(app.as_str())? {
            if is_protected_row(&provider.id) {
                outcome.rows_skipped += 1;
                continue;
            }
            let Some((base_url, api_key)) = extract_endpoint(&app, &provider.settings_config)
            else {
                // 认不出地址/Key 的行不迁移：合并进任何一家都可能丢配置。
                log::warn!(
                    "per-app 迁移跳过无法解析网关地址的行: {} ({})",
                    provider.id,
                    app.as_str()
                );
                outcome.rows_skipped += 1;
                continue;
            };
            let slot = *index
                .entry((base_url.clone(), api_key.clone()))
                .or_insert_with(|| {
                    groups.push((base_url, api_key, Vec::new()));
                    groups.len() - 1
                });
            groups[slot].2.push(SourceRow {
                app: app.clone(),
                provider,
            });
        }
    }

    // 2. 每组生成一个统一供应商。
    for (base_url, api_key, rows) in groups {
        // 去掉 per-app 开关后统一供应商一律同步到全部应用，所以这里不再按出现的应用累加。
        let apps = UniversalProviderApps::all();
        let mut models = UniversalProviderModels::default();
        let mut meta = ProviderMeta::default();
        let mut name: Option<String> = None;
        let mut icon: Option<String> = None;
        let mut icon_color: Option<String> = None;
        let mut website: Option<String> = None;
        let mut created_at: Option<i64> = None;
        let universal_id = rows[0].provider.id.clone();

        for row in &rows {
            name = name.or_else(|| Some(row.provider.name.clone()));
            icon = icon.or_else(|| row.provider.icon.clone());
            icon_color = icon_color.or_else(|| row.provider.icon_color.clone());
            website = website.or_else(|| row.provider.website_url.clone());
            // 组内取最早创建时间，尽量保持列表里的相对位置。
            created_at = match (created_at, row.provider.created_at) {
                (Some(a), Some(b)) => Some(a.min(b)),
                (a, b) => a.or(b),
            };

            if let Some(block) = model_config_for(&row.app, &row.provider.settings_config) {
                if let Some(claude) = block.claude {
                    // Claude 的兼容开关在 env 里，叠到同一块上。
                    let mut claude = claude;
                    if let Some(switches) = claude_switches(&row.provider.settings_config) {
                        claude.enable_tool_search = switches.enable_tool_search;
                        claude.disable_artifact = switches.disable_artifact;
                        claude.disable_auto_mode_server = switches.disable_auto_mode_server;
                    }
                    models.claude = Some(claude);
                }
                models.codex = block.codex.or(models.codex);
                models.gemini = block.gemini.or(models.gemini);
                models.grokbuild = block.grokbuild.or(models.grokbuild);
                models.claude_desktop = block.claude_desktop.or(models.claude_desktop);
            }
            merge_meta_into_models(&row.app, row.provider.meta.as_ref(), &mut models);
            if let Some(m) = row.provider.meta.as_ref() {
                merge_meta(&mut meta, m);
            }
        }

        let universal = UniversalProvider {
            id: universal_id.clone(),
            name: name.unwrap_or_else(|| "Untitled".to_string()),
            apps,
            base_url,
            api_key,
            models,
            website_url: website,
            notes: None,
            icon,
            icon_color,
            meta: Some(meta),
            created_at,
            sort_index: None,
            available_models: Vec::new(),
        };
        state.db.save_universal_provider(&universal)?;
        outcome.universal_created += 1;

        // 3. 同步出子卡；同步不会改 is_current，所以先记下哪些行是当前供应商。
        crate::services::provider::ProviderService::sync_universal_to_apps(state, &universal_id)?;

        for row in &rows {
            let app = row.app.as_str();
            // 直连指针走 mode::current 读取（仓库有守卫测试禁止直接读 DB 指针）。
            let was_current = crate::mode::current::provider_for(
                &state.db,
                &row.app,
                crate::mode::current::Purpose::Direct,
            )?
            .as_deref()
                == Some(row.provider.id.as_str());
            let child_id = format!("universal-{app}-{universal_id}");

            outcome.usage_rows_remapped +=
                remap_usage_references(state, app, &row.provider.id, &child_id)?;
            state.db.delete_provider(app, &row.provider.id)?;
            outcome.rows_removed += 1;

            // 原本在用的那家，删除后把指针挪到它的子卡上；live 里写的还是旧行的配置，
            // 所以紧接着按新指针重投影一次（同步发生在挪指针之前，那时子卡还不是当前）。
            if was_current {
                state.db.set_current_provider(app, &child_id)?;
                if let Err(e) =
                    crate::services::provider::ProviderService::sync_current_provider_for_app(
                        state,
                        row.app.clone(),
                    )
                {
                    log::warn!("迁移后重投影 {app} 的 live 配置失败: {e}");
                }
            }
            // 代理路由指针住在 device 状态里（不是 DB），同样可能指向刚删掉的这一行。
            remap_proxy_route(app, &row.provider.id, &child_id);
        }
    }

    state.db.set_setting(MIGRATED_FLAG, "true")?;
    log::info!(
        "✓ per-app → 统一供应商迁移完成: 新建 {} 个统一供应商，删除 {} 行，重映射 {} 条用量，跳过 {} 行",
        outcome.universal_created,
        outcome.rows_removed,
        outcome.usage_rows_remapped,
        outcome.rows_skipped
    );
    Ok(outcome)
}

/// 代理路由指针存在 device 状态文件（`live-state.json`）里，不在数据库。它可能正好
/// 指向被删掉的 per-app 行；不跟着挪，代理模式下次请求就会找不到那家。
///
/// 只改指针值，不动模式本身：模式由 `mode::controller::startup` 在迁移之后自行判定。
fn remap_proxy_route(app: &str, old_id: &str, new_id: &str) {
    use crate::live::engine::DeviceStore;
    let store = DeviceStore::for_device();
    let result = crate::mode::state::update(&store, |state| {
        let entry = state.apps.entry(app.to_string()).or_default();
        if entry.proxy_route.as_deref() == Some(old_id) {
            entry.proxy_route = Some(new_id.to_string());
            true
        } else {
            false
        }
    });
    if let Err(e) = result {
        log::warn!("重映射 {app} 的代理路由指针失败: {e}");
    }
}

/// 把用量相关表里指向旧 per-app id 的引用改指到新的子卡 id，统计不断档。
fn remap_usage_references(
    state: &AppState,
    app_type: &str,
    old_id: &str,
    new_id: &str,
) -> Result<usize, AppError> {
    let conn = crate::database::lock_conn!(state.db.conn);
    let mut total = 0_usize;
    for table in [
        "proxy_request_logs",
        "usage_daily_rollups",
        "stream_check_logs",
        "provider_health",
    ] {
        let updated = conn
            .execute(
                &format!(
                    "UPDATE {table} SET provider_id = ?1 WHERE provider_id = ?2 AND app_type = ?3"
                ),
                rusqlite::params![new_id, old_id, app_type],
            )
            .map_err(|e| AppError::Database(format!("重映射 {table} 的 provider_id 失败: {e}")))?;
        total += updated;
    }
    Ok(total)
}

/// 导出迁移前的 `providers` 表，写进 settings 留作人工回退。
pub fn save_providers_snapshot(state: &AppState) -> Result<(), AppError> {
    let mut dump = Vec::new();
    for app in AppType::all() {
        for (_, provider) in state.db.get_all_providers(app.as_str())? {
            dump.push(serde_json::json!({
                "app": app.as_str(),
                "id": provider.id,
                "name": provider.name,
                "settingsConfig": provider.settings_config,
            }));
        }
    }
    let json = serde_json::to_string(&dump).map_err(|e| AppError::Database(e.to_string()))?;
    state.db.set_setting(SNAPSHOT_KEY, &json)
}

/// 把所有已存在的统一供应商补齐为「五个应用全开」，并同步一次。
///
/// 去掉 per-app 开关之前的记录可能只在部分应用里有配置，而界面已经没有开关能让用户
/// 打开剩下的应用。幂等：`ALL_APPS_FLAG` 已置位时直接返回。
pub fn enable_all_apps_for_existing(state: &AppState) -> Result<usize, AppError> {
    if state.db.get_bool_flag(ALL_APPS_FLAG).unwrap_or(false) {
        return Ok(0);
    }

    let all = crate::provider::UniversalProviderApps::all();
    let mut updated = 0_usize;
    for (id, provider) in state.db.get_all_universal_providers()? {
        if provider.apps.is_all() {
            continue;
        }
        let mut provider = provider;
        provider.apps = all.clone();
        state.db.save_universal_provider(&provider)?;
        // 补齐后同步一次，让新增的那几个应用也拿到子卡。
        crate::services::provider::ProviderService::sync_universal_to_apps(state, &id)?;
        updated += 1;
    }

    state.db.set_setting(ALL_APPS_FLAG, "true")?;
    if updated > 0 {
        log::info!("✓ 统一供应商补齐为全应用: {updated} 个");
    }
    Ok(updated)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::provider::CodexChatReasoningConfig;
    use serde_json::json;
    use std::sync::Arc;

    use crate::database::Database;
    use serial_test::serial;
    /// 测试用：插一条用量明细。
    #[cfg(test)]
    impl Database {
        pub(crate) fn insert_usage_row_for_test(
            &self,
            request_id: &str,
            provider_id: &str,
            app_type: &str,
        ) -> Result<(), AppError> {
            let conn = crate::database::lock_conn!(self.conn);
            conn.execute(
                "INSERT INTO proxy_request_logs (request_id, provider_id, app_type, model, latency_ms, status_code, created_at)
                 VALUES (?1, ?2, ?3, 'gpt-5.6-sol', 10, 200, 1)",
                rusqlite::params![request_id, provider_id, app_type],
            )
            .map_err(|e| AppError::Database(e.to_string()))?;
            Ok(())
        }

        pub(crate) fn usage_provider_id_for_test(
            &self,
            request_id: &str,
        ) -> Result<Option<String>, AppError> {
            let conn = crate::database::lock_conn!(self.conn);
            conn.query_row(
                "SELECT provider_id FROM proxy_request_logs WHERE request_id = ?1",
                rusqlite::params![request_id],
                |row| row.get(0),
            )
            .map_err(|e| AppError::Database(e.to_string()))
        }
    }

    /// 全库只在内存里建：迁移只碰数据库与 device 状态。
    fn with_state<T>(test: impl FnOnce(&AppState) -> T) -> T {
        let temp = tempfile::tempdir().expect("tempdir");
        // 旧值要还原：整套测试共享进程环境，留下临时目录会让别的用例读到空 home。
        let old_test_home = std::env::var_os("CC_SWITCH_TEST_HOME");
        std::env::set_var("CC_SWITCH_TEST_HOME", temp.path());
        let db = Arc::new(Database::memory().expect("in-memory database"));
        let state = AppState::new(db);
        let result = test(&state);
        match old_test_home {
            Some(value) => std::env::set_var("CC_SWITCH_TEST_HOME", value),
            None => std::env::remove_var("CC_SWITCH_TEST_HOME"),
        }
        result
    }

    fn claude_settings(base_url: &str, key: &str, model: &str) -> Value {
        json!({
            "env": {
                "ANTHROPIC_BASE_URL": base_url,
                "ANTHROPIC_AUTH_TOKEN": key,
                "ANTHROPIC_MODEL": model,
            }
        })
    }

    fn codex_settings(base_url: &str, key: &str, model: &str) -> Value {
        json!({
            "auth": { "OPENAI_API_KEY": key },
            "config": format!(
                "model_provider = \"custom\"\nmodel = \"{model}\"\nmodel_reasoning_effort = \"high\"\n\
                 \n[model_providers.custom]\nname = \"GW\"\nbase_url = \"{base_url}\"\n\
                 wire_api = \"responses\"\nrequires_openai_auth = false\n"
            ),
        })
    }

    fn insert(state: &AppState, app: AppType, id: &str, name: &str, settings: Value) {
        let mut provider = Provider::with_id(id.to_string(), name.to_string(), settings, None);
        provider.created_at = Some(1_000);
        state
            .db
            .save_provider(app.as_str(), &provider)
            .expect("save");
    }

    #[test]
    fn universal_from_provider_keeps_the_source_apps_model_and_opens_all_apps() {
        // 深链接导入走这条：链接描述一个应用，转换后这份配置落在它的模型块里，
        // 其余应用没配置（投影时各用自己的默认值）。
        let mut provider = Provider::with_id(
            "relay-a".to_string(),
            "Relay A".to_string(),
            claude_settings("https://gw.example/v1", "sk-shared", "claude-sonnet-5"),
            Some("https://gw.example".to_string()),
        );
        provider.notes = Some("from link".to_string());
        provider.icon = Some("anthropic".to_string());

        let universal = universal_from_provider(&AppType::Claude, &provider).expect("convert");

        assert_eq!(universal.id, "relay-a");
        assert_eq!(universal.name, "Relay A");
        assert_eq!(universal.base_url, "https://gw.example/v1");
        assert_eq!(universal.api_key, "sk-shared");
        assert_eq!(universal.website_url.as_deref(), Some("https://gw.example"));
        assert_eq!(universal.notes.as_deref(), Some("from link"));
        assert_eq!(universal.icon.as_deref(), Some("anthropic"));
        assert!(universal.apps.is_all(), "统一供应商同步到全部应用");
        assert_eq!(
            universal
                .models
                .claude
                .as_ref()
                .and_then(|m| m.model.as_deref()),
            Some("claude-sonnet-5")
        );
        assert!(universal.models.codex.is_none());
        assert!(universal.models.gemini.is_none());
        // 链接里的模型种进可用模型列表：导入后模型下拉就有值，不用先拉一次 /v1/models。
        assert_eq!(
            universal
                .available_models
                .iter()
                .map(|model| model.id.as_str())
                .collect::<Vec<_>>(),
            vec!["claude-sonnet-5"]
        );
        // 新建行与表单新增同一口径：旧版本仍按这个标记合并通用配置片段。
        assert_eq!(
            universal
                .meta
                .as_ref()
                .and_then(|m| m.common_config_enabled),
            Some(true)
        );
    }

    #[test]
    fn universal_from_provider_maps_link_api_format_to_grok_api_backend() {
        let mut provider = Provider::with_id(
            "grok-relay".to_string(),
            "Grok Relay".to_string(),
            json!({
                "config": "[models]\ndefault = \"grok-4.6\"\n\n[model.\"grok-4.6\"]\n\
                           model = \"grok-4.6\"\nbase_url = \"https://gw.example/v1\"\n\
                           name = \"Grok Relay\"\napi_key = \"xai-key\"\n\
                           api_backend = \"responses\"\ncontext_window = 256000\n"
            }),
            None,
        );
        provider.meta = Some(ProviderMeta {
            api_format: Some("openai_chat".to_string()),
            ..Default::default()
        });

        let universal = universal_from_provider(&AppType::GrokBuild, &provider).expect("convert");
        assert_eq!(
            universal
                .models
                .grokbuild
                .as_ref()
                .and_then(|m| m.api_backend.as_deref()),
            Some("chat_completions"),
            "链接的 apiFormat 应换算成 Grok Build 的 api_backend"
        );
    }

    #[test]
    fn universal_from_provider_rejects_a_config_without_an_endpoint() {
        let provider = Provider::with_id(
            "broken".to_string(),
            "Broken".to_string(),
            json!({ "env": { "ANTHROPIC_AUTH_TOKEN": "sk" } }),
            None,
        );
        let error = universal_from_provider(&AppType::Claude, &provider)
            .expect_err("missing endpoint must be rejected");
        assert!(error.to_string().contains("Cannot derive"), "{error}");
    }

    #[test]
    #[serial]
    fn merges_rows_that_share_base_url_and_key_and_removes_them() {
        with_state(|state| {
            // 同一个网关、同一个 Key：Claude 和 Codex 两行应合成一个统一供应商。
            // 同一个网关：base_url 与 apiKey 必须逐字相同才会合并（用户规则）。
            insert(
                state,
                AppType::Claude,
                "claude-a",
                "GW",
                claude_settings("https://gw.example/v1", "sk-shared", "claude-sonnet-5"),
            );
            insert(
                state,
                AppType::Codex,
                "codex-a",
                "GW",
                codex_settings("https://gw.example/v1", "sk-shared", "gpt-5.6-sol"),
            );

            let outcome = migrate_per_app_to_universal(state).expect("migrate");

            assert_eq!(outcome.universal_created, 1, "两行同网关同 Key 应合成一个");
            assert_eq!(outcome.rows_removed, 2);

            let universal = state.db.get_all_universal_providers().expect("load");
            assert_eq!(universal.len(), 1);
            let (id, provider) = universal.iter().next().expect("one");
            // 去掉 app 开关后一律同步到全部应用。
            assert!(provider.apps.is_all(), "统一供应商应同步到全部应用");
            // 模型配置按应用各归各位。
            assert_eq!(
                provider
                    .models
                    .claude
                    .as_ref()
                    .and_then(|m| m.model.as_deref()),
                Some("claude-sonnet-5")
            );
            assert_eq!(
                provider
                    .models
                    .codex
                    .as_ref()
                    .and_then(|m| m.model.as_deref()),
                Some("gpt-5.6-sol")
            );
            assert_eq!(
                provider
                    .models
                    .codex
                    .as_ref()
                    .and_then(|m| m.reasoning_effort.as_deref()),
                Some("high")
            );

            // 原行被删、子卡建出来。
            assert!(state
                .db
                .get_provider_by_id("claude-a", "claude")
                .expect("q")
                .is_none());
            assert!(state
                .db
                .get_provider_by_id("codex-a", "codex")
                .expect("q")
                .is_none());
            let child = state
                .db
                .get_provider_by_id(&format!("universal-claude-{id}"), "claude")
                .expect("q")
                .expect("子卡应存在");
            // 子卡的地址和 Key 来自统一供应商。
            assert_eq!(
                child.settings_config["env"]["ANTHROPIC_BASE_URL"],
                json!("https://gw.example/v1")
            );
            assert_eq!(
                child.settings_config["env"]["ANTHROPIC_AUTH_TOKEN"],
                json!("sk-shared")
            );
        });
    }

    #[test]
    #[serial]
    fn different_keys_stay_separate() {
        with_state(|state| {
            // 同一地址、不同 Key：各算一家（不同额度），不能合并。
            insert(
                state,
                AppType::Codex,
                "codex-a",
                "A",
                codex_settings("https://gw.example/v1", "sk-aaa", "gpt-5.6-sol"),
            );
            insert(
                state,
                AppType::Codex,
                "codex-b",
                "B",
                codex_settings("https://gw.example/v1", "sk-bbb", "gpt-5.6-sol"),
            );

            let outcome = migrate_per_app_to_universal(state).expect("migrate");
            assert_eq!(outcome.universal_created, 2);
        });
    }

    #[test]
    #[serial]
    fn different_base_urls_stay_separate() {
        with_state(|state| {
            insert(
                state,
                AppType::Codex,
                "codex-a",
                "A",
                codex_settings("https://a.example/v1", "sk-shared", "gpt-5.6-sol"),
            );
            insert(
                state,
                AppType::Codex,
                "codex-b",
                "B",
                codex_settings("https://b.example/v1", "sk-shared", "gpt-5.6-sol"),
            );

            let outcome = migrate_per_app_to_universal(state).expect("migrate");
            assert_eq!(outcome.universal_created, 2);
        });
    }

    #[test]
    #[serial]
    fn backfills_existing_partial_apps_to_all() {
        with_state(|state| {
            // 旧记录：只开了 codex。
            let mut provider = UniversalProvider::new(
                "legacy".into(),
                "Legacy".into(),
                "https://gw.example/v1".into(),
                "sk-1".into(),
            );
            provider.apps.codex = true;
            state.db.save_universal_provider(&provider).expect("save");

            let updated = enable_all_apps_for_existing(state).expect("backfill");
            assert_eq!(updated, 1);

            let stored = state
                .db
                .get_universal_provider("legacy")
                .expect("q")
                .expect("exists");
            assert!(stored.apps.is_all(), "应补齐为全部应用");
            // 五个应用都该有子卡了。
            for app in ["claude", "claude-desktop", "codex", "gemini", "grokbuild"] {
                assert!(
                    state
                        .db
                        .get_provider_by_id(&format!("universal-{app}-legacy"), app)
                        .expect("q")
                        .is_some(),
                    "{app} 应有子卡"
                );
            }

            // 幂等：再跑一次什么都不做。
            assert_eq!(enable_all_apps_for_existing(state).expect("again"), 0);
        });
    }

    #[test]
    #[serial]
    fn keeps_official_seeds_and_is_idempotent() {
        with_state(|state| {
            state
                .db
                .init_default_official_providers()
                .expect("seed official");
            insert(
                state,
                AppType::Codex,
                "codex-custom",
                "GW",
                codex_settings("https://gw.example/v1", "sk-1", "gpt-5.6-sol"),
            );

            let first = migrate_per_app_to_universal(state).expect("first");
            assert_eq!(first.universal_created, 1);
            assert_eq!(first.rows_removed, 1);
            // 五个官方种子都留着，且没被包成统一供应商。
            for id in ["claude-official", "codex-official", "gemini-official"] {
                let app = if id.starts_with("claude") {
                    "claude"
                } else if id.starts_with("codex") {
                    "codex"
                } else {
                    "gemini"
                };
                assert!(
                    state.db.get_provider_by_id(id, app).expect("q").is_some(),
                    "官方种子 {id} 应保留"
                );
            }

            // 再跑一次：标记已置位，什么都不做。
            let second = migrate_per_app_to_universal(state).expect("second");
            assert_eq!(second, MigrationOutcome::default());
        });
    }

    #[test]
    #[serial]
    fn remaps_usage_rows_to_the_child() {
        with_state(|state| {
            insert(
                state,
                AppType::Codex,
                "codex-a",
                "GW",
                codex_settings("https://gw.example/v1", "sk-1", "gpt-5.6-sol"),
            );
            state
                .db
                .insert_usage_row_for_test("r1", "codex-a", "codex")
                .expect("insert usage");

            migrate_per_app_to_universal(state).expect("migrate");

            let mapped = state
                .db
                .usage_provider_id_for_test("r1")
                .expect("read back");
            assert!(
                mapped
                    .as_deref()
                    .is_some_and(|id| id.starts_with("universal-codex-")),
                "用量应指到新子卡，实际 {mapped:?}"
            );
        });
    }

    #[test]
    #[serial]
    fn carries_claude_upstream_switches_and_meta() {
        with_state(|state| {
            let mut provider = Provider::with_id(
                "claude-a".to_string(),
                "GW".to_string(),
                json!({
                    "env": {
                        "ANTHROPIC_BASE_URL": "https://gw.example",
                        "ANTHROPIC_AUTH_TOKEN": "sk-1",
                        "ANTHROPIC_MODEL": "claude-sonnet-5[1M]",
                        "ENABLE_TOOL_SEARCH": "true",
                        "CLAUDE_CODE_AUTO_MODE_SERVER": "0",
                    }
                }),
                None,
            );
            provider.meta = Some(ProviderMeta {
                api_format: Some("openai_responses".into()),
                is_full_url: Some(true),
                ..Default::default()
            });
            state.db.save_provider("claude", &provider).expect("save");

            migrate_per_app_to_universal(state).expect("migrate");

            let universal = state.db.get_all_universal_providers().expect("load");
            let provider = universal.values().next().expect("one");
            let claude = provider.models.claude.as_ref().expect("claude block");
            // `[1M]` 拆成模型名 + 标记。
            assert_eq!(claude.model.as_deref(), Some("claude-sonnet-5"));
            assert_eq!(claude.model_one_m, Some(true));
            assert_eq!(claude.api_format.as_deref(), Some("openai_responses"));
            assert_eq!(claude.is_full_url, Some(true));
            // env 里的兼容开关搬进模型块。
            assert_eq!(claude.enable_tool_search, Some(true));
            assert_eq!(claude.disable_auto_mode_server, Some(true));
        });
    }

    #[test]
    #[serial]
    fn moves_the_current_pointer_to_the_child() {
        with_state(|state| {
            insert(
                state,
                AppType::Codex,
                "codex-a",
                "GW",
                codex_settings("https://gw.example/v1", "sk-1", "gpt-5.6-sol"),
            );
            state
                .db
                .set_current_provider("codex", "codex-a")
                .expect("set current");

            migrate_per_app_to_universal(state).expect("migrate");

            let current = state.db.get_current_provider("codex").expect("q");
            assert!(
                current
                    .as_deref()
                    .is_some_and(|id| id.starts_with("universal-codex-")),
                "当前指针应挪到子卡，实际 {current:?}"
            );
        });
    }

    #[test]
    #[serial]
    fn converts_codex_absolute_compact_limit_to_window_percent() {
        with_state(|state| {
            // 旧 per-app 行的压缩阈值是绝对 token 数；统一供应商存窗口百分比，
            // 窗口取同一份配置里的顶层 `model_context_window`。
            let mut settings = codex_settings("https://gw.example/v1", "sk-1", "gpt-5.6-sol");
            settings["config"] = json!(
                "model_provider = \"custom\"\nmodel = \"gpt-5.6-sol\"\n\
                 model_reasoning_effort = \"high\"\nmodel_context_window = 1000000\n\
                 model_auto_compact_token_limit = 900000\n\
                 \n[model_providers.custom]\nname = \"GW\"\n\
                 base_url = \"https://gw.example/v1\"\nwire_api = \"responses\"\n\
                 requires_openai_auth = false\n"
            );
            insert(state, AppType::Codex, "codex-pct", "GW", settings);

            migrate_per_app_to_universal(state).expect("migrate");

            let universal = state.db.get_all_universal_providers().expect("load");
            let codex = universal
                .values()
                .next()
                .expect("one")
                .models
                .codex
                .as_ref()
                .expect("codex block");
            assert_eq!(codex.auto_compact_percent, Some(90));
        });
    }

    #[test]
    #[serial]
    fn drops_codex_compact_limit_without_any_window() {
        with_state(|state| {
            // 配置和模型目录里都没有窗口：百分比倒推不出来，丢掉而不是猜。
            let mut settings = codex_settings("https://gw.example/v1", "sk-1", "gpt-5.6-sol");
            settings["config"] = json!(
                "model_provider = \"custom\"\nmodel = \"gpt-5.6-sol\"\n\
                 model_auto_compact_token_limit = 900000\n\
                 \n[model_providers.custom]\nname = \"GW\"\n\
                 base_url = \"https://gw.example/v1\"\nwire_api = \"responses\"\n\
                 requires_openai_auth = false\n"
            );
            insert(state, AppType::Codex, "codex-nowindow", "GW", settings);

            migrate_per_app_to_universal(state).expect("migrate");

            let universal = state.db.get_all_universal_providers().expect("load");
            let codex = universal
                .values()
                .next()
                .expect("one")
                .models
                .codex
                .as_ref()
                .expect("codex block");
            assert_eq!(codex.auto_compact_percent, None);
        });
    }

    #[test]
    #[serial]
    fn carries_codex_meta_switches_into_the_model_block() {
        with_state(|state| {
            let mut provider = Provider::with_id(
                "codex-a".to_string(),
                "GW".to_string(),
                codex_settings("https://gw.example/v1", "sk-1", "gpt-5.6-sol"),
                None,
            );
            // 旧 per-app 行的 Codex 专属设置全在 meta 里。
            provider.meta = Some(ProviderMeta {
                api_format: Some("anthropic".into()),
                is_full_url: Some(true),
                custom_user_agent: Some("relay/1.0".into()),
                impersonate_claude_code: Some(true),
                prompt_cache_routing: Some("enabled".into()),
                codex_chat_reasoning: Some(CodexChatReasoningConfig {
                    supports_thinking: Some(true),
                    ..Default::default()
                }),
                ..Default::default()
            });
            state.db.save_provider("codex", &provider).expect("save");

            migrate_per_app_to_universal(state).expect("migrate");

            let universal = state.db.get_all_universal_providers().expect("load");
            let universal = universal.values().next().expect("one");
            let codex = universal.models.codex.as_ref().expect("codex block");
            assert_eq!(codex.api_format.as_deref(), Some("anthropic"));
            assert_eq!(codex.is_full_url, Some(true));
            assert_eq!(codex.custom_user_agent.as_deref(), Some("relay/1.0"));
            assert_eq!(codex.impersonate_claude_code, Some(true));
            assert_eq!(codex.prompt_cache_routing.as_deref(), Some("enabled"));
            assert_eq!(
                codex
                    .chat_reasoning
                    .as_ref()
                    .and_then(|r| r.supports_thinking),
                Some(true)
            );
        });
    }
}
