//! Live 配置的读取、首次导入和按模式分发的写入。
//!
//! 切换式应用（Claude Code、Codex、Gemini CLI、Grok Build）的客户端文件只经写入引擎写
//! （`*_direct.rs`），这里只负责分发；Claude Desktop 仍在这里写。

use serde_json::{json, Value};
use toml_edit::{DocumentMut, Item, TableLike};

use crate::app_config::AppType;
use crate::config::{get_claude_settings_path, read_json_file};
use crate::error::AppError;
use crate::provider::Provider;
use crate::services::mcp::McpService;
use crate::store::AppState;

use super::normalize_claude_models_in_value;

fn json_is_subset(target: &Value, source: &Value) -> bool {
    match source {
        Value::Object(source_map) => {
            let Some(target_map) = target.as_object() else {
                return false;
            };
            source_map.iter().all(|(key, source_value)| {
                target_map
                    .get(key)
                    .is_some_and(|target_value| json_is_subset(target_value, source_value))
            })
        }
        Value::Array(source_arr) => {
            let Some(target_arr) = target.as_array() else {
                return false;
            };
            json_array_contains_subset(target_arr, source_arr)
        }
        _ => target == source,
    }
}

fn json_array_contains_subset(target_arr: &[Value], source_arr: &[Value]) -> bool {
    let mut matched = vec![false; target_arr.len()];

    source_arr.iter().all(|source_item| {
        if let Some((index, _)) = target_arr.iter().enumerate().find(|(index, target_item)| {
            !matched[*index] && json_is_subset(target_item, source_item)
        }) {
            matched[index] = true;
            true
        } else {
            false
        }
    })
}

fn json_remove_array_items(target_arr: &mut Vec<Value>, source_arr: &[Value]) {
    for source_item in source_arr {
        if let Some(index) = target_arr
            .iter()
            .position(|target_item| json_is_subset(target_item, source_item))
        {
            target_arr.remove(index);
        }
    }
}

fn json_deep_remove(target: &mut Value, source: &Value) {
    let (Some(target_map), Some(source_map)) = (target.as_object_mut(), source.as_object()) else {
        return;
    };

    for (key, source_value) in source_map {
        let mut remove_key = false;

        if let Some(target_value) = target_map.get_mut(key) {
            if source_value.is_object() && target_value.is_object() {
                json_deep_remove(target_value, source_value);
                remove_key = target_value.as_object().is_some_and(|obj| obj.is_empty());
            } else if let (Some(target_arr), Some(source_arr)) =
                (target_value.as_array_mut(), source_value.as_array())
            {
                json_remove_array_items(target_arr, source_arr);
                remove_key = target_arr.is_empty();
            } else if json_is_subset(target_value, source_value) {
                remove_key = true;
            }
        }

        if remove_key {
            target_map.remove(key);
        }
    }
}

fn toml_value_is_subset(target: &toml_edit::Value, source: &toml_edit::Value) -> bool {
    match (target, source) {
        (toml_edit::Value::String(target), toml_edit::Value::String(source)) => {
            target.value() == source.value()
        }
        (toml_edit::Value::Integer(target), toml_edit::Value::Integer(source)) => {
            target.value() == source.value()
        }
        (toml_edit::Value::Float(target), toml_edit::Value::Float(source)) => {
            target.value() == source.value()
        }
        (toml_edit::Value::Boolean(target), toml_edit::Value::Boolean(source)) => {
            target.value() == source.value()
        }
        (toml_edit::Value::Datetime(target), toml_edit::Value::Datetime(source)) => {
            target.value() == source.value()
        }
        (toml_edit::Value::Array(target), toml_edit::Value::Array(source)) => {
            toml_array_contains_subset(target, source)
        }
        (toml_edit::Value::InlineTable(target), toml_edit::Value::InlineTable(source)) => {
            source.iter().all(|(key, source_item)| {
                target
                    .get(key)
                    .is_some_and(|target_item| toml_value_is_subset(target_item, source_item))
            })
        }
        _ => false,
    }
}

fn toml_array_contains_subset(target: &toml_edit::Array, source: &toml_edit::Array) -> bool {
    let mut matched = vec![false; target.len()];
    let target_items: Vec<&toml_edit::Value> = target.iter().collect();

    source.iter().all(|source_item| {
        if let Some((index, _)) = target_items
            .iter()
            .enumerate()
            .find(|(index, target_item)| {
                !matched[*index] && toml_value_is_subset(target_item, source_item)
            })
        {
            matched[index] = true;
            true
        } else {
            false
        }
    })
}

fn toml_remove_array_items(target: &mut toml_edit::Array, source: &toml_edit::Array) {
    for source_item in source.iter() {
        let index = {
            let target_items: Vec<&toml_edit::Value> = target.iter().collect();
            target_items
                .iter()
                .enumerate()
                .find(|(_, target_item)| toml_value_is_subset(target_item, source_item))
                .map(|(index, _)| index)
        };

        if let Some(index) = index {
            target.remove(index);
        }
    }
}

fn toml_item_is_subset(target: &Item, source: &Item) -> bool {
    if let Some(source_table) = source.as_table_like() {
        let Some(target_table) = target.as_table_like() else {
            return false;
        };
        return source_table.iter().all(|(key, source_item)| {
            target_table
                .get(key)
                .is_some_and(|target_item| toml_item_is_subset(target_item, source_item))
        });
    }

    match (target.as_value(), source.as_value()) {
        (Some(target_value), Some(source_value)) => {
            toml_value_is_subset(target_value, source_value)
        }
        _ => false,
    }
}

fn remove_toml_item(target: &mut Item, source: &Item) {
    if let Some(source_table) = source.as_table_like() {
        if let Some(target_table) = target.as_table_like_mut() {
            remove_toml_table_like(target_table, source_table);
            if target_table.is_empty() {
                *target = Item::None;
            }
            return;
        }
    }

    if let Some(source_value) = source.as_value() {
        let mut remove_item = false;

        if let Some(target_value) = target.as_value_mut() {
            match (target_value, source_value) {
                (toml_edit::Value::Array(target_arr), toml_edit::Value::Array(source_arr)) => {
                    toml_remove_array_items(target_arr, source_arr);
                    remove_item = target_arr.is_empty();
                }
                (target_value, source_value)
                    if toml_value_is_subset(target_value, source_value) =>
                {
                    remove_item = true;
                }
                _ => {}
            }
        }

        if remove_item {
            *target = Item::None;
        }
    }
}

fn remove_toml_table_like(target: &mut dyn TableLike, source: &dyn TableLike) {
    let keys: Vec<String> = source.iter().map(|(key, _)| key.to_string()).collect();

    for key in keys {
        let mut remove_key = false;
        if let (Some(target_item), Some(source_item)) = (target.get_mut(&key), source.get(&key)) {
            remove_toml_item(target_item, source_item);
            remove_key = target_item.is_none()
                || target_item
                    .as_table_like()
                    .is_some_and(|table_like| table_like.is_empty());
        }

        if remove_key {
            target.remove(&key);
        }
    }
}

fn settings_contain_common_config(app_type: &AppType, settings: &Value, snippet: &str) -> bool {
    let trimmed = snippet.trim();
    if trimmed.is_empty() {
        return false;
    }

    match app_type {
        AppType::Claude => match serde_json::from_str::<Value>(trimmed) {
            Ok(source) if source.is_object() => json_is_subset(settings, &source),
            _ => false,
        },
        AppType::Codex => {
            let config_toml = settings.get("config").and_then(Value::as_str).unwrap_or("");
            if config_toml.trim().is_empty() {
                return false;
            }

            let target_doc = match config_toml.parse::<DocumentMut>() {
                Ok(doc) => doc,
                Err(_) => return false,
            };
            let source_doc = match trimmed.parse::<DocumentMut>() {
                Ok(doc) => doc,
                Err(_) => return false,
            };

            toml_item_is_subset(target_doc.as_item(), source_doc.as_item())
        }
        AppType::Gemini => match serde_json::from_str::<Value>(trimmed) {
            Ok(Value::Object(source_map)) => {
                let Some(target_map) = settings.get("env").and_then(Value::as_object) else {
                    return false;
                };
                source_map.iter().all(|(key, source_value)| {
                    target_map
                        .get(key)
                        .is_some_and(|target_value| json_is_subset(target_value, source_value))
                })
            }
            _ => false,
        },
        AppType::GrokBuild | AppType::ClaudeDesktop => false,
    }
}

pub(crate) fn provider_uses_common_config(
    app_type: &AppType,
    provider: &Provider,
    snippet: Option<&str>,
) -> bool {
    match provider
        .meta
        .as_ref()
        .and_then(|meta| meta.common_config_enabled)
    {
        Some(explicit) => explicit && snippet.is_some_and(|value| !value.trim().is_empty()),
        None => snippet.is_some_and(|value| {
            settings_contain_common_config(app_type, &provider.settings_config, value)
        }),
    }
}

pub(crate) fn remove_common_config_from_settings(
    app_type: &AppType,
    settings: &Value,
    snippet: &str,
) -> Result<Value, AppError> {
    let trimmed = snippet.trim();
    if trimmed.is_empty() {
        return Ok(settings.clone());
    }

    match app_type {
        AppType::Claude => {
            let source = serde_json::from_str::<Value>(trimmed)
                .map_err(|e| AppError::Message(format!("Invalid Claude common config: {e}")))?;
            let mut result = settings.clone();
            json_deep_remove(&mut result, &source);
            Ok(result)
        }
        AppType::Codex => {
            let mut result = settings.clone();
            let config_toml = settings.get("config").and_then(Value::as_str).unwrap_or("");
            let mut target_doc = if config_toml.trim().is_empty() {
                DocumentMut::new()
            } else {
                config_toml.parse::<DocumentMut>().map_err(|e| {
                    AppError::Message(format!(
                        "Invalid Codex config.toml while removing common config: {e}"
                    ))
                })?
            };
            let source_doc = trimmed.parse::<DocumentMut>().map_err(|e| {
                AppError::Message(format!("Invalid Codex common config snippet: {e}"))
            })?;

            remove_toml_table_like(target_doc.as_table_mut(), source_doc.as_table());
            if let Some(obj) = result.as_object_mut() {
                obj.insert("config".to_string(), Value::String(target_doc.to_string()));
            }
            Ok(result)
        }
        AppType::Gemini => {
            let source = serde_json::from_str::<Value>(trimmed)
                .map_err(|e| AppError::Message(format!("Invalid Gemini common config: {e}")))?;
            let mut result = settings.clone();
            if let Some(env) = result.get_mut("env") {
                json_deep_remove(env, &source);
            }
            Ok(result)
        }
        AppType::GrokBuild | AppType::ClaudeDesktop => Ok(settings.clone()),
    }
}

/// 把 `provider` 写进 live（live 当前对应的就是它：同步、退出代理写回）。切换式应用只
/// 替换关键字段；通用配置片段冻结在库里只给旧版读，这里不再合并。
pub(crate) fn write_live_for_state(
    state: &AppState,
    app_type: &AppType,
    provider: &Provider,
) -> Result<(), AppError> {
    let db = state.db.as_ref();
    if matches!(app_type, AppType::Claude) {
        // Claude 不再整份写，也不合并片段：只替换关键字段和独有字段。live 当前对应的
        // 就是这个供应商（同步、退出代理写回），它带进来的独有字段按同一行比对。
        super::claude_direct::reapply(db, Some(provider), provider)?;
        return Ok(());
    }
    if matches!(app_type, AppType::Codex) {
        // Codex 同理：只替换关键字段和独有字段，不合并片段、不补回 MCP。
        super::codex_direct::write_direct(
            db,
            crate::mode::state::op::APPLY,
            super::codex_direct::Owner::Provider(provider),
            Some(provider),
            crate::mode::state::PendingTarget::default(),
        )?;
        return Ok(());
    }
    if matches!(app_type, AppType::Gemini) {
        // Gemini、Grok Build 同理：只替换关键字段，不合并片段、不补回 MCP。
        super::gemini_direct::reapply(db, provider)?;
        return Ok(());
    }
    if matches!(app_type, AppType::GrokBuild) {
        super::grok_direct::reapply(db, Some(provider), provider)?;
        return Ok(());
    }

    if matches!(app_type, AppType::ClaudeDesktop) {
        crate::claude_desktop_config::apply_provider(db, provider)?;
        log::info!(
            "Claude Desktop 3P profile '{}' written for provider '{}'",
            crate::claude_desktop_config::PROFILE_ID,
            provider.id
        );
        return Ok(());
    }

    write_live_snapshot(app_type, provider)
}

/// Write live configuration snapshot for a provider
///
/// Every supported app now has a dedicated key-field write engine, so this
/// snapshot path is only reachable if a new app type is added without one.
pub(crate) fn write_live_snapshot(
    app_type: &AppType,
    _provider: &Provider,
) -> Result<(), AppError> {
    match app_type {
        AppType::Claude => Err(AppError::localized(
            "claude.live.requires_engine",
            "Claude Code 配置只能经关键字段写入流程写入",
            "Claude Code configuration must be written through the key-field write flow",
        )),
        AppType::ClaudeDesktop => Err(AppError::localized(
            "claude_desktop.live.requires_db_context",
            "Claude Desktop 配置写入需要通过供应商切换流程执行",
            "Claude Desktop configuration must be written through the provider switch flow",
        )),
        AppType::Codex => Err(AppError::localized(
            "codex.live.requires_engine",
            "Codex 配置只能经关键字段写入流程写入",
            "Codex configuration must be written through the key-field write flow",
        )),
        AppType::Gemini => Err(AppError::localized(
            "gemini.live.requires_engine",
            "Gemini CLI 配置只能经关键字段写入流程写入",
            "Gemini CLI configuration must be written through the key-field write flow",
        )),
        AppType::GrokBuild => Err(AppError::localized(
            "grokbuild.live.requires_engine",
            "Grok Build 配置只能经关键字段写入流程写入",
            "Grok Build configuration must be written through the key-field write flow",
        )),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LiveSyncOutcome {
    /// 按直连投影写了 live。
    WroteLive,
    /// 应用在代理模式：live 是代理契约，没有按直连写。
    ProxyMode,
}

/// 把 `provider` 同步到 live，按应用的模式处理：
/// - 直连模式：按直连投影写 live；
/// - 代理模式：live 是代理契约。`provider` 是代理路由的那家时按新契约重写（契约没变就
///   不动）；其余供应商（包括直连指针那家）只在退出代理时写回，这里不碰 live。
///
/// `prev` 是 live 现在对应的那一版供应商行（编辑前的行），Claude 按它删上一版带进来的
/// 独有字段；`None` 表示 live 对应的就是 `provider` 自己。调用方持有这个应用的代理切换锁
/// （`controller::lock_settled_blocking`），并且在拿锁之后才读谁是当前供应商：不拿锁的
/// 话，读完模式到写完 live 之间进入代理，直连的关键字段会盖掉刚写的代理契约。
pub(crate) fn sync_live_for_provider_respecting_mode(
    state: &AppState,
    app_type: &AppType,
    provider: &Provider,
    prev: Option<&Provider>,
) -> Result<LiveSyncOutcome, AppError> {
    if crate::mode::current::is_proxy(app_type) {
        futures::executor::block_on(crate::mode::controller::resync_saved_row_locked(
            state, app_type, provider,
        ))
        .map_err(AppError::Message)?;
        return Ok(LiveSyncOutcome::ProxyMode);
    }
    if matches!(app_type, AppType::Claude) {
        super::claude_direct::reapply(state.db.as_ref(), prev.or(Some(provider)), provider)?;
    } else if matches!(app_type, AppType::GrokBuild) {
        // 编辑前的行用来推断旧版写的表（还没有写入记录时），改了表名也能删掉旧表。
        super::grok_direct::reapply(state.db.as_ref(), prev.or(Some(provider)), provider)?;
    } else {
        write_live_for_state(state, app_type, provider)?;
    }
    Ok(LiveSyncOutcome::WroteLive)
}

/// 把正在用的那家（代理模式下是代理路由）同步到 live；没有正在用的那家时返回 `None`。
/// 返回时已经放开切换锁。
pub(crate) fn sync_current_provider_for_app_respecting_mode(
    state: &AppState,
    app_type: &AppType,
) -> Result<Option<LiveSyncOutcome>, AppError> {
    let _switch_guard = crate::mode::controller::lock_settled_blocking(state, app_type)?;
    let current_id = match crate::mode::current::provider_for(
        &state.db,
        app_type,
        crate::mode::current::Purpose::InUse,
    )? {
        Some(id) => id,
        None => return Ok(None),
    };

    let providers = state.db.get_all_providers(app_type.as_str())?;
    let Some(provider) = providers.get(&current_id) else {
        return Ok(None);
    };

    sync_live_for_provider_respecting_mode(state, app_type, provider, None).map(Some)
}

/// Sync current provider to live configuration
///
/// 使用有效的当前供应商 ID（验证过存在性）。
/// 优先从本地 settings 读取，验证后 fallback 到数据库的 is_current 字段。
/// 这确保了配置导入后无效 ID 会自动 fallback 到数据库。
pub fn sync_current_to_live(state: &AppState) -> Result<(), AppError> {
    let mut failures = Vec::new();

    // Sync providers based on mode
    for app_type in AppType::all() {
        // Switch mode: sync only current provider. During proxy takeover,
        // update the restore backup instead of rewriting the taken-over
        // live file.
        let result = sync_current_provider_for_app_respecting_mode(state, &app_type).map(|_| ());

        if let Err(error) = result {
            log::warn!("同步 Provider 到 {app_type:?} 失败: {error}");
            failures.push(format!("provider/{}: {error}", app_type.as_str()));
        }
    }

    // MCP sync is already best-effort per application. Preserve its aggregate
    // error while continuing with Skills.
    if let Err(error) = McpService::sync_all_enabled(state) {
        failures.push(format!("mcp: {error}"));
    }

    // Skill sync
    for app_type in AppType::all() {
        if let Err(e) = crate::services::skill::SkillService::sync_to_app(&state.db, &app_type) {
            log::warn!("同步 Skill 到 {app_type:?} 失败: {e}");
            failures.push(format!("skill/{}: {e}", app_type.as_str()));
        }
    }

    if failures.is_empty() {
        Ok(())
    } else {
        Err(AppError::Message(format!(
            "部分 live 配置同步失败: {}",
            failures.join("; ")
        )))
    }
}

/// Read current live settings for an app type
pub fn read_live_settings(app_type: AppType) -> Result<Value, AppError> {
    match app_type {
        AppType::Codex => {
            let mut result = crate::codex_config::read_codex_live_settings()?;
            // `modelCatalog` is a cc-switch private field that lives only in
            // the DB SSOT plus the `cc-switch-model-catalog.json` projection
            // file — it is never inlined into `auth.json` or `config.toml`.
            // Reverse-parse the projection so the edit form for the active
            // Codex provider doesn't see an empty mapping table.
            if let Ok(Some(model_catalog)) =
                crate::codex_config::read_codex_model_catalog_simplified_from_live()
            {
                if let Some(obj) = result.as_object_mut() {
                    obj.insert("modelCatalog".to_string(), model_catalog);
                }
            }
            Ok(result)
        }
        AppType::Claude => {
            let path = get_claude_settings_path();
            if !path.exists() {
                return Err(AppError::localized(
                    "claude.live.missing",
                    "Claude Code 配置文件不存在",
                    "Claude settings file is missing",
                ));
            }
            read_json_file(&path)
        }
        AppType::ClaudeDesktop => Err(AppError::localized(
            "claude_desktop.live.read_unsupported",
            "Claude Desktop 3P 配置不支持作为通用 live 配置导入，请使用“从 Claude 导入兼容供应商”。",
            "Claude Desktop 3P configuration cannot be imported as a generic live config. Use 'Import compatible providers from Claude' instead.",
        )),
        AppType::Gemini => {
            use crate::gemini_config::{
                env_to_json, get_gemini_env_path, get_gemini_settings_path, read_gemini_env,
            };

            // Read .env file (environment variables)
            let env_path = get_gemini_env_path();
            if !env_path.exists() {
                return Err(AppError::localized(
                    "gemini.env.missing",
                    "Gemini .env 文件不存在",
                    "Gemini .env file not found",
                ));
            }

            let env_map = read_gemini_env()?;
            let env_json = env_to_json(&env_map);
            let env_obj = env_json.get("env").cloned().unwrap_or_else(|| json!({}));

            // Read settings.json file (MCP config etc.)
            let settings_path = get_gemini_settings_path();
            let config_obj = if settings_path.exists() {
                read_json_file(&settings_path)?
            } else {
                json!({})
            };

            // Return complete structure: { "env": {...}, "config": {...} }
            Ok(json!({
                "env": env_obj,
                "config": config_obj
            }))
        }
        AppType::GrokBuild => crate::grok_config::read_grok_live_settings(),
    }
}

/// Import default configuration from live files
///
/// Returns `Ok(true)` if a provider was actually imported,
/// `Ok(false)` if skipped (providers already exist for this app).
pub fn import_default_config(state: &AppState, app_type: AppType) -> Result<bool, AppError> {
    // 允许 "只有官方 seed 预设" 的情况下继续导入 live：
    // - 启动编排顺序是先 import 后 seed，新用户启动时 providers 为空，导入照常
    // - 老用户已有非 seed provider，跳过导入（正确）
    // - 用户手动点 ProviderEmptyState 的导入按钮时，与官方 seed 共存而不被阻塞
    if state.db.has_non_official_seed_provider(app_type.as_str())? {
        return Ok(false);
    }

    // 拒绝把"代理模式下的 Live"导入为供应商：代理模式下 Live 里只有
    // PROXY_MANAGED 占位符和本地代理地址，不是用户的真实配置。一旦导入，
    // 它会成为直连指针（SSOT），退出代理时会把占位符当真实配置写回 Live。
    // 典型触发场景：代理模式下切换 app_config_dir 并重启，新数据库首启导入。
    if state.proxy_service.live_has_proxy_placeholder(&app_type) {
        return Err(AppError::localized(
            "provider.import.live_taken_over",
            "Live 配置当前处于代理接管状态（包含占位符），不能导入为供应商。请先关闭代理接管或恢复 Live 配置后重试。",
            "The live config is currently taken over by the proxy (contains placeholders) and cannot be imported as a provider. Disable proxy takeover or restore the live config first.",
        ));
    }

    let settings_config = match app_type {
        AppType::Codex => crate::codex_config::read_codex_live_settings()?,
        AppType::GrokBuild => {
            let mut settings = crate::grok_config::read_grok_live_settings()?;
            let config = settings
                .get("config")
                .and_then(Value::as_str)
                .unwrap_or_default();
            // 官方登录态（无自定义模型表）在这里必须报错：本函数也被启动
            // 自动导入调用，而全项目惯例是"启动自动导入只产出 default，
            // 从不产出官方条目"——否则删掉的官方条目每次重启都会复活。
            // 官方态的成功导入（补官方条目并激活）只挂在手动导入的命令层
            // （`import_default_config_internal`）。
            crate::grok_config::validate_config_toml(config)?;
            crate::grok_config::strip_grok_mcp_servers_from_settings(&mut settings)?;
            settings
        }
        AppType::Claude => {
            let settings_path = get_claude_settings_path();
            if !settings_path.exists() {
                return Err(AppError::localized(
                    "claude.live.missing",
                    "Claude Code 配置文件不存在",
                    "Claude settings file is missing",
                ));
            }
            let mut v = read_json_file::<Value>(&settings_path)?;
            let _ = normalize_claude_models_in_value(&mut v);
            v
        }
        AppType::ClaudeDesktop => {
            return Err(AppError::localized(
                "claude_desktop.import_unsupported",
                "Claude Desktop 3P 配置不能通过通用导入读取，请使用“从 Claude 导入兼容供应商”。",
                "Claude Desktop 3P config cannot be imported through the generic import flow. Use 'Import compatible providers from Claude' instead.",
            ));
        }
        AppType::Gemini => {
            use crate::gemini_config::{
                env_to_json, get_gemini_env_path, get_gemini_settings_path, read_gemini_env,
            };

            // Read .env file (environment variables)
            let env_path = get_gemini_env_path();
            if !env_path.exists() {
                return Err(AppError::localized(
                    "gemini.live.missing",
                    "Gemini 配置文件不存在",
                    "Gemini configuration file is missing",
                ));
            }

            let env_map = read_gemini_env()?;
            let env_json = env_to_json(&env_map);
            let env_obj = env_json.get("env").cloned().unwrap_or_else(|| json!({}));

            // Read settings.json file (MCP config etc.)
            let settings_path = get_gemini_settings_path();
            let config_obj = if settings_path.exists() {
                read_json_file(&settings_path)?
            } else {
                json!({})
            };

            // Return complete structure: { "env": {...}, "config": {...} }
            json!({
                "env": env_obj,
                "config": config_obj
            })
        }
    };

    let mut provider = Provider::with_id(
        "default".to_string(),
        "default".to_string(),
        settings_config,
        None,
    );
    provider.category = Some(
        if matches!(app_type, AppType::Codex) {
            let config_text = provider
                .settings_config
                .get("config")
                .and_then(Value::as_str);
            let has_provider_key = crate::codex_config::extract_codex_api_key(
                provider.settings_config.get("auth"),
                config_text,
            )
            .is_some();
            let has_login_material = provider
                .settings_config
                .get("auth")
                .is_some_and(crate::codex_config::codex_auth_has_login_material);

            if has_login_material && !has_provider_key {
                "official"
            } else {
                "custom"
            }
        } else {
            "custom"
        }
        .to_string(),
    );

    state.db.save_provider(app_type.as_str(), &provider)?;
    state
        .db
        .set_current_provider(app_type.as_str(), &provider.id)?;
    crate::settings::set_current_provider(&app_type, Some(provider.id.as_str()))?;

    // 初次导入已有配置时随手补出官方入口，对齐其它应用"首启动 = 导入 default
    // + 播种官方条目"的观感。grokbuild 种子晚于 `official_providers_seeded`
    // flag 引入，存量库的主播种不会再跑，只能挂在导入动作上补。
    // 只在导入成功时执行；live 完全不可导入（文件缺失/语法错误/残缺配置）
    // 不会到达这里。失败只 warn。
    if matches!(app_type, AppType::GrokBuild) {
        if let Err(e) = state.db.ensure_official_seed_by_id(
            crate::database::GROKBUILD_OFFICIAL_PROVIDER_ID,
            AppType::GrokBuild,
        ) {
            log::warn!("Failed to ensure grokbuild-official seed after import: {e}");
        }
    }

    Ok(true) // 真正导入了
}

/// Decide whether startup should auto-import the current live config as `default`.
///
/// This is intentionally stricter than the manual import path:
/// if the app already has any provider row at all (including official seeds),
/// startup must skip auto-import to avoid recreating `default` on each launch.
pub fn should_import_default_config_on_startup(
    state: &AppState,
    app_type: &AppType,
) -> Result<bool, AppError> {
    Ok(!state.db.has_any_provider_for_app(app_type.as_str())?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn claude_common_config_remove_strips_what_old_versions_merged() {
        let settings = json!({
            "env": {
                "ANTHROPIC_API_KEY": "sk-test"
            }
        });
        let snippet = r#"{
  "includeCoAuthoredBy": false,
  "env": {
    "CLAUDE_CODE_USE_BEDROCK": "1"
  }
}"#;

        // 旧版切换时把片段深合并进行里的样子。
        let applied = json!({
            "env": {
                "ANTHROPIC_API_KEY": "sk-test",
                "CLAUDE_CODE_USE_BEDROCK": "1"
            },
            "includeCoAuthoredBy": false
        });

        let stripped =
            remove_common_config_from_settings(&AppType::Claude, &applied, snippet).unwrap();
        assert_eq!(stripped, settings);
    }

    #[test]
    fn codex_common_config_remove_strips_what_old_versions_merged() {
        let settings = json!({
            "auth": {
                "OPENAI_API_KEY": "sk-test"
            },
            "config": "model_provider = \"openai\"\n[general]\nmodel = \"gpt-5\"\n"
        });
        let snippet = "[shared]\nreasoning = \"medium\"\n";

        // 旧版切换时把片段合并进行里的样子。
        let applied = json!({
            "auth": {
                "OPENAI_API_KEY": "sk-test"
            },
            "config": "model_provider = \"openai\"\n[general]\nmodel = \"gpt-5\"\n\n[shared]\nreasoning = \"medium\"\n"
        });

        let stripped =
            remove_common_config_from_settings(&AppType::Codex, &applied, snippet).unwrap();
        assert_eq!(stripped, settings);
    }

    #[test]
    fn explicit_common_config_flag_overrides_legacy_subset_detection() {
        let mut provider = Provider::with_id(
            "claude-test".to_string(),
            "Claude Test".to_string(),
            json!({
                "includeCoAuthoredBy": false
            }),
            None,
        );
        provider.meta = Some(crate::provider::ProviderMeta {
            common_config_enabled: Some(false),
            ..Default::default()
        });

        assert!(
            !provider_uses_common_config(
                &AppType::Claude,
                &provider,
                Some(r#"{ "includeCoAuthoredBy": false }"#),
            ),
            "explicit false should win over legacy subset detection"
        );
    }

    #[test]
    fn claude_common_config_array_subset_detection_and_strip_preserve_extra_items() {
        let settings = json!({
            "allowedTools": ["tool1", "tool2"]
        });
        let snippet = r#"{
  "allowedTools": ["tool1"]
}"#;

        assert!(
            settings_contain_common_config(&AppType::Claude, &settings, snippet),
            "array subset should be detected for legacy providers"
        );

        let stripped =
            remove_common_config_from_settings(&AppType::Claude, &settings, snippet).unwrap();
        assert_eq!(
            stripped,
            json!({
                "allowedTools": ["tool2"]
            })
        );
    }

    #[test]
    fn codex_common_config_array_subset_detection_and_strip_preserve_extra_items() {
        let settings = json!({
            "auth": {},
            "config": "allowed_tools = [\"tool1\", \"tool2\"]\n"
        });
        let snippet = "allowed_tools = [\"tool1\"]\n";

        assert!(
            settings_contain_common_config(&AppType::Codex, &settings, snippet),
            "TOML array subset should be detected for legacy providers"
        );

        let stripped =
            remove_common_config_from_settings(&AppType::Codex, &settings, snippet).unwrap();
        assert_eq!(stripped["auth"], json!({}));
        let stripped_config = stripped["config"].as_str().unwrap_or_default();
        let parsed = stripped_config
            .parse::<DocumentMut>()
            .expect("stripped codex config should remain valid TOML");
        let allowed_tools = parsed["allowed_tools"]
            .as_array()
            .expect("allowed_tools should remain an array");
        let values: Vec<&str> = allowed_tools
            .iter()
            .map(|value| value.as_str().expect("tool id should be string"))
            .collect();
        assert_eq!(values, vec!["tool2"]);
    }
}
