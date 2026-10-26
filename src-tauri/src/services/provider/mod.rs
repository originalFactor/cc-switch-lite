//! Provider service module
//!
//! Handles provider CRUD operations, switching, and configuration management.

pub(crate) mod claude_direct;
mod claude_editor;
pub(crate) mod codex_client_catalog;
pub(crate) mod codex_direct;
mod codex_editor;
mod codex_login;
pub(crate) mod codex_official_models;
mod editor_toml;
mod endpoints;
mod gemini_auth;
pub(crate) mod gemini_direct;
mod gemini_editor;
pub(crate) mod grok_direct;
mod grok_editor;
pub(crate) mod icon;
mod live;
pub mod universal_migration;
mod usage;

use indexmap::IndexMap;
use regex::Regex;
use serde::Deserialize;
use serde_json::Value;

use crate::app_config::AppType;
use crate::error::AppError;
use crate::provider::{Provider, ProviderMeta, UsageResult};
use crate::services::mcp::McpService;
use crate::settings::CustomEndpoint;
use crate::store::AppState;

// Re-export sub-module functions for external access
pub use live::{
    import_default_config, read_live_settings, should_import_default_config_on_startup,
    sync_current_to_live,
};

pub use claude_editor::{EditorSave, EditorView};

// Internal re-exports (pub(crate))
pub(crate) use live::{write_live_for_state, LiveSyncOutcome};

use usage::validate_usage_script;

/// Codex official providers are safe to select during takeover: Codex keeps
/// ownership of the active ChatGPT login and the proxy only forwards the
/// authenticated request. Other apps' official providers retain the block.
pub fn official_provider_supports_proxy_takeover(app_type: &AppType, provider: &Provider) -> bool {
    matches!(app_type, AppType::Codex)
        && crate::proxy::providers::is_codex_official_provider(provider)
}

/// 统一会话开关变更后，立即按新开关状态重写当前官方 Codex 供应商的选路（关键字段），
/// 使开关即时生效，无需等下一次切换。当前供应商非官方（或不存在）时为 no-op：开关只
/// 影响官方直连的选路。代理模式下 live 是代理契约，不受这个开关影响。
pub fn reapply_current_codex_official_live(state: &AppState) -> Result<bool, AppError> {
    let _switch_guard = crate::mode::controller::lock_settled_blocking(state, &AppType::Codex)?;
    let current_id = ProviderService::current(state, AppType::Codex)?;
    if current_id.is_empty() {
        return Ok(false);
    }
    let providers = state.db.get_all_providers(AppType::Codex.as_str())?;
    let Some(provider) = providers.get(&current_id) else {
        return Ok(false);
    };
    if !codex_direct::is_official(provider) {
        return Ok(false);
    }
    live::sync_live_for_provider_respecting_mode(state, &AppType::Codex, provider, None)?;
    Ok(true)
}

/// 新版不再读通用配置片段，但旧设备经云同步拿到新建的行时仍按这个标记合并片段；不写的
/// 话，旧版切到它会把 hooks、MCP 等共享设置整份抹掉。
fn keep_common_config_for_old_versions(provider: &mut Provider) {
    provider
        .meta
        .get_or_insert_with(Default::default)
        .common_config_enabled = Some(true);
}

/// 编辑器保存的是新增的供应商，还是已有的。
#[derive(Clone, Copy, PartialEq, Eq)]
enum EditorSaveKind {
    Add,
    Update,
}

impl EditorSaveKind {
    /// 这次保存要不要把关键字段也换进 live：直连模式下编辑的是当前供应商，或者新增的是
    /// 第一个供应商。代理模式下 live 的关键字段是代理契约，只写全局改动。
    fn writes_key_fields(
        self,
        state: &AppState,
        app_type: &AppType,
        mode: &crate::mode::state::ModeState,
        id: &str,
    ) -> Result<bool, AppError> {
        if mode.is_proxy() {
            return Ok(false);
        }
        let direct = crate::mode::current::provider_for(
            &state.db,
            app_type,
            crate::mode::current::Purpose::Direct,
        )?;
        Ok(match self {
            Self::Add => direct.is_none(),
            Self::Update => direct.as_deref() == Some(id),
        })
    }
}

/// Provider business logic service
pub struct ProviderService;

/// Result of a provider switch operation, including any non-fatal warnings
#[derive(Debug, serde::Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct SwitchResult {
    pub warnings: Vec<String>,
}

impl ProviderService {
    fn normalize_provider_if_claude(app_type: &AppType, provider: &mut Provider) {
        if matches!(app_type, AppType::Claude) {
            let mut v = provider.settings_config.clone();
            if normalize_claude_models_in_value(&mut v) {
                provider.settings_config = v;
            }
        }
    }

    fn normalize_usage_script_credential_overrides(app_type: &AppType, provider: &mut Provider) {
        let current_credentials = provider.resolve_usage_credentials(app_type);

        let Some(usage_script) = provider
            .meta
            .as_mut()
            .and_then(|meta| meta.usage_script.as_mut())
        else {
            return;
        };

        if usage_script.template_type.as_deref() == Some("token_plan") {
            return;
        }

        if usage_script.api_key.as_deref().is_some_and(|api_key| {
            Self::should_clear_usage_api_key_override(api_key, &current_credentials)
        }) {
            usage_script.api_key = None;
        }

        if usage_script.base_url.as_deref().is_some_and(|base_url| {
            Self::should_clear_usage_base_url_override(base_url, &current_credentials)
        }) {
            usage_script.base_url = None;
        }
    }

    fn should_clear_usage_api_key_override(
        script_api_key: &str,
        current_credentials: &(String, String),
    ) -> bool {
        let candidate = script_api_key.trim();
        if candidate.is_empty() {
            return true;
        }

        let matches_provider_key = |api_key: &str| {
            let api_key = api_key.trim();
            !api_key.is_empty() && api_key == candidate
        };

        matches_provider_key(&current_credentials.1)
    }

    fn should_clear_usage_base_url_override(
        script_base_url: &str,
        current_credentials: &(String, String),
    ) -> bool {
        let candidate = Self::normalize_usage_base_url_for_compare(script_base_url);
        if candidate.is_empty() {
            return true;
        }

        let matches_provider_base_url = |base_url: &str| {
            let base_url = Self::normalize_usage_base_url_for_compare(base_url);
            !base_url.is_empty() && base_url == candidate
        };

        matches_provider_base_url(&current_credentials.0)
    }

    fn normalize_usage_base_url_for_compare(base_url: &str) -> String {
        base_url.trim().trim_end_matches('/').to_string()
    }

    /// List all providers for an app type
    pub fn list(
        state: &AppState,
        app_type: AppType,
    ) -> Result<IndexMap<String, Provider>, AppError> {
        state.db.get_all_providers(app_type.as_str())
    }

    /// Get current provider ID
    ///
    /// 使用有效的当前供应商 ID（验证过存在性）。
    /// 优先从本地 settings 读取，验证后 fallback 到数据库的 is_current 字段。
    /// 这确保了云同步场景下多设备可以独立选择供应商，且返回的 ID 一定有效。
    pub fn current(state: &AppState, app_type: AppType) -> Result<String, AppError> {
        // 代理模式下界面上的「当前」是代理路由到的那家。
        crate::mode::current::provider_for(
            &state.db,
            &app_type,
            crate::mode::current::Purpose::InUse,
        )
        .map(|opt| opt.unwrap_or_default())
    }

    /// 新增供应商之前的规范化和校验，`add`、`add_without_switching` 和深链接导入共用。
    pub(crate) fn prepare_new_provider(
        app_type: &AppType,
        provider: &mut Provider,
    ) -> Result<(), AppError> {
        // Normalize Claude model keys
        Self::normalize_provider_if_claude(app_type, provider);
        Self::validate_provider_settings(app_type, provider)?;
        Self::normalize_usage_script_credential_overrides(app_type, provider);
        if matches!(app_type, AppType::Claude | AppType::Codex | AppType::Gemini) {
            keep_common_config_for_old_versions(provider);
        }
        Ok(())
    }

    /// 只把供应商存进列表，不写客户端配置、不动当前指针。
    ///
    /// `add` 在还没有当前供应商时会让新的这家直接生效；需要「只保存、不生效」的调用方
    /// （例如确认框里关掉了「导入后立即切换」）走这里，交给用户之后自己切换。
    pub fn add_without_switching(
        state: &AppState,
        app_type: AppType,
        provider: Provider,
    ) -> Result<bool, AppError> {
        let mut provider = provider;
        Self::prepare_new_provider(&app_type, &mut provider)?;
        // 和切换互斥：等着的切换不能看到只存了一半的行
        let _switch_guard = crate::mode::controller::lock_settled_blocking(state, &app_type)?;
        state.db.save_provider(app_type.as_str(), &provider)?;
        Ok(true)
    }

    /// Add a new provider
    pub fn add(
        state: &AppState,
        app_type: AppType,
        provider: Provider,
        _add_to_live: bool,
    ) -> Result<bool, AppError> {
        let mut provider = provider;
        Self::prepare_new_provider(&app_type, &mut provider)?;
        if matches!(app_type, AppType::Codex) {
            return Self::add_codex(state, provider);
        }

        // 还没有当前供应商时这一家会被写进 live：和进入代理、切换互斥（同 `update`）。
        let _switch_guard = crate::mode::controller::lock_settled_blocking(state, &app_type)?;

        // Save to database
        state.db.save_provider(app_type.as_str(), &provider)?;

        // For other apps: Check if sync is needed (if this is current provider, or no current provider)
        let current = crate::mode::current::provider_for(
            &state.db,
            &app_type,
            crate::mode::current::Purpose::Direct,
        )?;
        if current.is_none() {
            // 第一个供应商同样只写关键字段，不覆盖用户已有的配置文件。
            match app_type {
                AppType::Claude => {
                    claude_direct::switch_to(state.db.as_ref(), None, &provider)?;
                    return Ok(true);
                }
                AppType::Gemini => {
                    gemini_direct::switch_to(state.db.as_ref(), &provider)?;
                    return Ok(true);
                }
                AppType::GrokBuild => {
                    grok_direct::switch_to(state.db.as_ref(), None, &provider)?;
                    return Ok(true);
                }
                _ => {}
            }
            // No current provider, set as current and sync. Managed Codex adds
            // use the transactional path above because token resolution can fail.
            state
                .db
                .set_current_provider(app_type.as_str(), &provider.id)?;
            write_live_for_state(state, &app_type, &provider)?;
        }

        Ok(true)
    }

    /// 新增 Codex 供应商。还没有当前供应商时，它就是第一个：行、live 和指针一起提交，
    /// live 只写关键字段，不覆盖用户已有的 config.toml。写 live 失败时撤回刚存的行，
    /// 免得留下一个看得见却用不了的供应商。
    fn add_codex(state: &AppState, provider: Provider) -> Result<bool, AppError> {
        let app_type = AppType::Codex;
        // 和切换互斥：等着的切换不能看到只存了一半的托管账号绑定。
        let _switch_guard = crate::mode::controller::lock_settled_blocking(state, &app_type)?;
        let current = crate::mode::current::provider_for(
            &state.db,
            &app_type,
            crate::mode::current::Purpose::Direct,
        )?;
        if current.is_some() {
            state.db.save_provider(app_type.as_str(), &provider)?;
            return Ok(true);
        }

        // 行有问题（会把官方登录发给第三方等）在存行之前就拒绝。
        codex_direct::preflight(state.db.as_ref(), &provider)?;
        let previous = state
            .db
            .get_provider_by_id(&provider.id, app_type.as_str())?;
        state.db.save_provider(app_type.as_str(), &provider)?;
        let written = codex_direct::write_direct(
            state.db.as_ref(),
            crate::mode::state::op::SWITCH,
            codex_direct::Owner::None,
            Some(&provider),
            crate::mode::state::PendingTarget::pointer(Some(provider.id.clone())),
        );
        if let Err(error) = written {
            // 文件已经发布、只是落定状态失败时，pending 会在下次操作或启动时补完指针，
            // 行要留着；还没发布就失败，撤回刚存的行。
            if crate::mode::operation::has_pending(AppType::Codex.as_str()) {
                return Err(error);
            }
            let rollback = match &previous {
                Some(previous) => state.db.save_provider(app_type.as_str(), previous),
                None => state.db.delete_provider(app_type.as_str(), &provider.id),
            };
            if let Err(rollback) = rollback {
                return Err(AppError::Message(format!(
                    "新增首个 Codex 供应商失败: {error}; 恢复供应商数据同时失败: {rollback}"
                )));
            }
            return Err(error);
        }
        Ok(true)
    }

    /// 供应商编辑器底部配置的显示内容（Claude Code、Codex、Gemini CLI、Grok Build）：切到
    /// 这个供应商之后配置文件会是什么样，以及行里不随切换生效的字段。`category` 用来认出
    /// 官方卡。
    pub fn editor_view(
        state: &AppState,
        app_type: AppType,
        settings_config: &Value,
        category: Option<&str>,
    ) -> Result<EditorView, AppError> {
        Self::editor_view_with_meta(state, app_type, settings_config, category, None, None)
    }

    /// Codex previews retain managed-provider identity. Explicit draft metadata,
    /// including an empty value, takes precedence over the stored card's metadata.
    pub fn editor_view_with_meta(
        state: &AppState,
        app_type: AppType,
        settings_config: &Value,
        category: Option<&str>,
        provider_id: Option<&str>,
        meta: Option<&ProviderMeta>,
    ) -> Result<EditorView, AppError> {
        match app_type {
            AppType::Claude => claude_editor::view(state, settings_config),
            AppType::Codex => {
                let stored_meta = match (meta, provider_id) {
                    (None, Some(id)) => state
                        .db
                        .get_provider_by_id(id, AppType::Codex.as_str())?
                        .and_then(|provider| provider.meta),
                    _ => None,
                };
                codex_editor::view(
                    state,
                    settings_config,
                    category,
                    meta.or(stored_meta.as_ref()),
                )
            }
            AppType::Gemini => gemini_editor::view(state, settings_config, category),
            AppType::GrokBuild => grok_editor::view(state, settings_config, category),
            other => Err(AppError::InvalidInput(format!(
                "{} 的编辑器还不支持按关键字段显示",
                other.as_str()
            ))),
        }
    }

    /// 编辑已有供应商时给 [`Self::editor_view`] 的 `category`：按库里那一行，用切换时同一个
    /// 判断认官方卡。Gemini 还按合作方标记和名字认出旧版没有 `category` 的 Google 卡，Codex
    /// 还认固定 id 和托管账号；只看 `category` 的话，这些卡预览里的登录方式和切换写的不同。
    /// 没有 `provider_id`（新增）或行不存在时原样用 `category`。
    pub fn editor_category(
        state: &AppState,
        app_type: &AppType,
        provider_id: Option<&str>,
        category: Option<String>,
    ) -> Result<Option<String>, AppError> {
        let Some(id) = provider_id else {
            return Ok(category);
        };
        let Some(row) = state.db.get_provider_by_id(id, app_type.as_str())? else {
            return Ok(category);
        };
        let official = match app_type {
            AppType::Codex => codex_direct::is_official(&row),
            AppType::Gemini => gemini_direct::is_official(&row),
            _ => false,
        };
        Ok(if official {
            Some("official".to_string())
        } else {
            category
        })
    }

    /// 从编辑器新增供应商。Claude Code、Codex、Gemini CLI、Grok Build 按关键字段拆开保存
    /// （见各自的 `*_editor`），其余应用和 `add` 一样。
    pub fn add_from_editor(
        state: &AppState,
        app_type: AppType,
        provider: Provider,
        add_to_live: bool,
        editor: Option<EditorSave>,
    ) -> Result<bool, AppError> {
        match (app_type, editor) {
            (AppType::Claude, Some(editor)) => {
                Self::add_claude_from_editor(state, provider, editor)
            }
            (AppType::Codex, Some(editor)) => {
                Self::save_codex_from_editor(state, provider, editor, EditorSaveKind::Add)
            }
            (app_type @ (AppType::Gemini | AppType::GrokBuild), Some(editor)) => {
                Self::save_gemini_or_grok_from_editor(
                    state,
                    app_type,
                    provider,
                    editor,
                    EditorSaveKind::Add,
                )
            }
            (app_type, _) => Self::add(state, app_type, provider, add_to_live),
        }
    }

    /// 从编辑器保存供应商。Claude Code、Codex、Gemini CLI、Grok Build 按关键字段拆开保存
    /// （见各自的 `*_editor`），其余应用和 `update` 一样。
    pub fn update_from_editor(
        state: &AppState,
        app_type: AppType,
        original_id: Option<&str>,
        provider: Provider,
        editor: Option<EditorSave>,
    ) -> Result<bool, AppError> {
        match (app_type, editor) {
            (AppType::Claude, Some(editor))
                if original_id.is_none_or(|original| original == provider.id) =>
            {
                Self::update_claude_from_editor(state, provider, editor)
            }
            (AppType::Codex, Some(editor))
                if original_id.is_none_or(|original| original == provider.id) =>
            {
                Self::save_codex_from_editor(state, provider, editor, EditorSaveKind::Update)
            }
            (app_type @ (AppType::Gemini | AppType::GrokBuild), Some(editor))
                if original_id.is_none_or(|original| original == provider.id) =>
            {
                Self::save_gemini_or_grok_from_editor(
                    state,
                    app_type,
                    provider,
                    editor,
                    EditorSaveKind::Update,
                )
            }
            (app_type, _) => Self::update(state, app_type, original_id, provider),
        }
    }

    /// 从编辑器新增或保存 Codex 供应商：关键字段、独有字段存回行，其余改动作为全局设置
    /// 写进 live（三方比较）。直连模式下编辑当前供应商、或新增第一个供应商时，关键字段在
    /// 同一次写入里换进 live；代理模式下编辑路由那家，全局设置写完后按新行重写代理契约。
    fn save_codex_from_editor(
        state: &AppState,
        provider: Provider,
        editor: EditorSave,
        kind: EditorSaveKind,
    ) -> Result<bool, AppError> {
        let app_type = AppType::Codex;
        let _switch_guard = crate::mode::controller::lock_settled_blocking(state, &app_type)?;
        let existing = state
            .db
            .get_provider_by_id(&provider.id, app_type.as_str())?;
        let mut provider = provider;
        let plan = codex_editor::plan_save(
            existing.as_ref().map(|row| &row.settings_config),
            &provider.settings_config,
            &editor.base,
            &match (existing.as_ref(), editor.draft.as_ref()) {
                (Some(row), _) => codex_editor::Origin::row(&row.settings_config)?,
                (None, Some(draft)) => codex_editor::Origin::row(draft)?,
                (None, None) => codex_editor::Origin::Live(codex_editor::live_exclusive(state)?),
            },
            codex_direct::is_official(&provider),
            false,
            editor.on_conflict,
        )?;
        provider.settings_config = plan.row_settings.clone();
        Self::validate_provider_settings(&app_type, &provider)?;
        Self::normalize_usage_script_credential_overrides(&app_type, &mut provider);
        if kind == EditorSaveKind::Add {
            keep_common_config_for_old_versions(&mut provider);
        }

        let mode = crate::mode::current::mode_state(&app_type);
        let key_fields = kind.writes_key_fields(state, &app_type, &mode, &provider.id)?;

        state.db.save_provider(app_type.as_str(), &provider)?;
        let written = if key_fields {
            codex_editor::write_live(
                state.db.as_ref(),
                &plan.edits,
                codex_editor::KeyFields::Direct {
                    prev: existing.as_ref(),
                    target: &provider,
                    set_pointer: kind == EditorSaveKind::Add,
                },
            )
        } else {
            codex_editor::write_live(
                state.db.as_ref(),
                &plan.edits,
                codex_editor::KeyFields::None,
            )
            .and_then(|()| Self::resync_proxy_for_saved_row(state, &app_type, &provider))
        };
        Self::keep_row_if_written(state, &app_type, &provider.id, existing.as_ref(), written)
    }

    /// 从编辑器新增或保存 Gemini CLI、Grok Build 供应商：关键字段存回行，其余改动作为全局
    /// 设置写进 live（三方比较）。直连模式下编辑当前供应商、或新增第一个供应商时，关键
    /// 字段在同一次写入里换进 live；代理模式下编辑路由那家，全局设置写完后按新行重写代理
    /// 契约。
    fn save_gemini_or_grok_from_editor(
        state: &AppState,
        app_type: AppType,
        provider: Provider,
        editor: EditorSave,
        kind: EditorSaveKind,
    ) -> Result<bool, AppError> {
        let _switch_guard = crate::mode::controller::lock_settled_blocking(state, &app_type)?;
        let existing = state
            .db
            .get_provider_by_id(&provider.id, app_type.as_str())?;
        let stored = existing.as_ref().map(|row| &row.settings_config);
        let mut provider = provider;
        enum Edits {
            Gemini(gemini_editor::GeminiEdits),
            Grok(editor_toml::TomlEdits),
        }
        let edits = if matches!(app_type, AppType::Gemini) {
            let plan = gemini_editor::plan_save(
                stored,
                &provider.settings_config,
                &editor.base,
                editor.on_conflict,
            )?;
            provider.settings_config = plan.row_settings;
            Edits::Gemini(plan.edits)
        } else {
            let plan = grok_editor::plan_save(
                stored,
                &provider.settings_config,
                &editor.base,
                grok_direct::is_official(&provider),
                editor.on_conflict,
            )?;
            provider.settings_config = plan.row_settings;
            Edits::Grok(plan.edits)
        };
        Self::validate_provider_settings(&app_type, &provider)?;
        Self::normalize_usage_script_credential_overrides(&app_type, &mut provider);
        if kind == EditorSaveKind::Add && matches!(app_type, AppType::Gemini) {
            keep_common_config_for_old_versions(&mut provider);
        }

        let mode = crate::mode::current::mode_state(&app_type);
        let key_fields = kind.writes_key_fields(state, &app_type, &mode, &provider.id)?;
        let set_pointer = kind == EditorSaveKind::Add;

        state.db.save_provider(app_type.as_str(), &provider)?;
        let written = match &edits {
            Edits::Gemini(edits) => gemini_editor::write_live(
                state.db.as_ref(),
                edits,
                key_fields.then_some(&provider),
                set_pointer,
            ),
            Edits::Grok(edits) => grok_editor::write_live(
                state.db.as_ref(),
                edits,
                key_fields.then_some((existing.as_ref(), &provider)),
                set_pointer,
            ),
        }
        .and_then(|()| Self::resync_proxy_for_saved_row(state, &app_type, &provider));
        Self::keep_row_if_written(state, &app_type, &provider.id, existing.as_ref(), written)
    }

    /// 编辑器保存先存行、再写 live（`written`）。写 live 失败时：文件已经发布、只是落定
    /// 状态失败的，pending 会在下次操作或启动时补完，行要留着；还没发布就失败（没有
    /// pending），撤回刚存的行。
    fn keep_row_if_written(
        state: &AppState,
        app_type: &AppType,
        provider_id: &str,
        existing: Option<&Provider>,
        written: Result<(), AppError>,
    ) -> Result<bool, AppError> {
        let Err(error) = written else {
            return Ok(true);
        };
        if crate::mode::operation::has_pending(app_type.as_str()) {
            return Err(error);
        }
        let rollback = match existing {
            Some(existing) => state.db.save_provider(app_type.as_str(), existing),
            None => state.db.delete_provider(app_type.as_str(), provider_id),
        };
        if let Err(rollback) = rollback {
            log::warn!(
                "恢复 {} 供应商 '{provider_id}' 失败: {rollback}",
                app_type.as_str()
            );
        }
        Err(error)
    }

    /// 代理模式下存好了这一行：它是代理路由或在 Stack 名单里时按新行重写代理契约，契约没变
    /// 就不碰客户端文件（见 [`crate::mode::controller::resync_saved_row_locked`]）。调用方
    /// 持有这个应用的切换锁。
    fn resync_proxy_for_saved_row(
        state: &AppState,
        app_type: &AppType,
        provider: &Provider,
    ) -> Result<(), AppError> {
        futures::executor::block_on(crate::mode::controller::resync_saved_row_locked(
            state, app_type, provider,
        ))
        .map_err(AppError::Message)
    }

    fn add_claude_from_editor(
        state: &AppState,
        provider: Provider,
        editor: EditorSave,
    ) -> Result<bool, AppError> {
        let app_type = AppType::Claude;
        let _switch_guard = crate::mode::controller::lock_settled_blocking(state, &app_type)?;
        let mut provider = provider;
        Self::normalize_provider_if_claude(&app_type, &mut provider);
        let plan = claude_editor::plan_save(None, &provider.settings_config, &editor.base)?;
        provider.settings_config = plan.row_settings.clone();
        Self::validate_provider_settings(&app_type, &provider)?;
        Self::normalize_usage_script_credential_overrides(&app_type, &mut provider);
        keep_common_config_for_old_versions(&mut provider);

        let existing = state
            .db
            .get_provider_by_id(&provider.id, app_type.as_str())?;
        let first = crate::mode::current::provider_for(
            &state.db,
            &app_type,
            crate::mode::current::Purpose::Direct,
        )?
        .is_none();
        state.db.save_provider(app_type.as_str(), &provider)?;

        let key_fields = first.then_some(claude_editor::KeyFieldWrite {
            prev: None,
            target: &provider,
            set_pointer: true,
        });
        let written =
            claude_editor::write_live(state.db.as_ref(), &plan, editor.on_conflict, key_fields);
        Self::keep_row_if_written(state, &app_type, &provider.id, existing.as_ref(), written)
    }

    /// 和其他编辑器一样先存行、再写 live：写 live 失败且没有 pending 时撤回行；已经发布的
    /// 由 pending 补完，行和 live 不会对不上。
    fn update_claude_from_editor(
        state: &AppState,
        provider: Provider,
        editor: EditorSave,
    ) -> Result<bool, AppError> {
        let app_type = AppType::Claude;
        let _switch_guard = crate::mode::controller::lock_settled_blocking(state, &app_type)?;
        let existing = state
            .db
            .get_provider_by_id(&provider.id, app_type.as_str())?;
        let mut provider = provider;
        Self::normalize_provider_if_claude(&app_type, &mut provider);
        let plan = claude_editor::plan_save(
            existing.as_ref().map(|row| &row.settings_config),
            &provider.settings_config,
            &editor.base,
        )?;
        provider.settings_config = plan.row_settings.clone();
        Self::validate_provider_settings(&app_type, &provider)?;
        Self::normalize_usage_script_credential_overrides(&app_type, &mut provider);

        let mode = crate::mode::current::mode_state(&app_type);
        // 代理模式下 live 的关键字段是代理契约，这里只写全局改动；编辑的是代理路由那家
        // 或 Stack 名单里的一家时，写完按新行重写契约（契约没变就不动）。直连指针那家在
        // 退出代理时写回。
        let key_fields = EditorSaveKind::Update
            .writes_key_fields(state, &app_type, &mode, &provider.id)?
            .then_some(claude_editor::KeyFieldWrite {
                prev: existing.as_ref(),
                target: &provider,
                set_pointer: false,
            });

        state.db.save_provider(app_type.as_str(), &provider)?;
        let written =
            claude_editor::write_live(state.db.as_ref(), &plan, editor.on_conflict, key_fields)
                .and_then(|()| Self::resync_proxy_for_saved_row(state, &app_type, &provider));
        Self::keep_row_if_written(state, &app_type, &provider.id, existing.as_ref(), written)
    }

    /// Update a provider
    pub fn update(
        state: &AppState,
        app_type: AppType,
        original_id: Option<&str>,
        provider: Provider,
    ) -> Result<bool, AppError> {
        let mut provider = provider;
        let original_id = original_id.unwrap_or(provider.id.as_str()).to_string();
        let provider_id_changed = original_id != provider.id;
        // 读旧行、判断谁是当前供应商、存行、写 live 都在切换锁里：进入代理和切换都等这次
        // 保存写完，这里也不会在读完模式之后、写 live 之前被它们改掉模式（否则直连的关键
        // 字段会盖掉刚写的代理契约）。Codex 还要在锁里读旧行，看它是不是托管账号的行。
        let _switch_guard = crate::mode::controller::lock_settled_blocking(state, &app_type)?;
        let existing_provider = state
            .db
            .get_provider_by_id(&original_id, app_type.as_str())?;
        // Normalize Claude model keys
        Self::normalize_provider_if_claude(&app_type, &mut provider);
        Self::validate_provider_settings(&app_type, &provider)?;
        if matches!(app_type, AppType::Codex) && provider.category.as_deref() == Some("official") {
            crate::codex_config::strip_codex_unified_session_bucket_from_settings(
                &mut provider.settings_config,
            )?;
        }
        Self::normalize_usage_script_credential_overrides(&app_type, &mut provider);

        if provider_id_changed {
            return Err(AppError::Message(
                "Provider key cannot be changed".to_string(),
            ));
        }

        // For other apps: 是否是直连指针那家。代理模式下是不是代理路由或 Stack 名单里的一家，
        // 由同步 live 的那一步判断。
        let mode = crate::mode::current::mode_state(&app_type);
        let is_direct_current = crate::mode::current::provider_for(
            &state.db,
            &app_type,
            crate::mode::current::Purpose::Direct,
        )?
        .as_deref()
            == Some(provider.id.as_str());

        if matches!(app_type, AppType::Codex) {
            return Self::update_codex(
                state,
                &provider,
                existing_provider.as_ref(),
                is_direct_current,
            );
        }

        // Save to database
        state.db.save_provider(app_type.as_str(), &provider)?;

        if is_direct_current || mode.is_proxy() {
            let outcome = live::sync_live_for_provider_respecting_mode(
                state,
                &app_type,
                &provider,
                existing_provider.as_ref(),
            )?;
            if outcome == LiveSyncOutcome::WroteLive {
                // MCP is stored in the database and projected after a successful
                // live write. Keep the failure best-effort so the provider save
                // itself is not reported as failed when MCP projection can retry.
                if let Err(err) = McpService::sync_enabled_for_app(state, &app_type) {
                    log::warn!(
                        "保存供应商后重投影 {app_type:?} MCP 失败（将在下次同步时自愈）: {err}"
                    );
                }
            }
        }

        Ok(true)
    }

    /// 保存 Codex 供应商。调用方持有这个应用的切换锁。
    ///
    /// - 直连模式下编辑直连那家：先存行，再只替换 live 里的关键字段和独有字段（换托管
    ///   账号时先采纳、再清掉旧账号的登录）；写 live 失败就把行恢复原样。
    /// - 代理模式下先存行，编辑的是路由那家或 Stack 名单里的一家时按新行重写代理契约（见
    ///   [`Self::resync_proxy_for_saved_row`]），写失败同样把行恢复原样。
    /// - 其余只存行。
    fn update_codex(
        state: &AppState,
        provider: &Provider,
        existing: Option<&Provider>,
        is_direct_current: bool,
    ) -> Result<bool, AppError> {
        let app_type = AppType::Codex;
        let mode = crate::mode::current::mode_state(&app_type);
        if !mode.is_proxy() && !is_direct_current {
            state.db.save_provider(app_type.as_str(), provider)?;
            return Ok(true);
        }
        if !mode.is_proxy() {
            codex_direct::preflight(state.db.as_ref(), provider)?;
        }

        state.db.save_provider(app_type.as_str(), provider)?;
        let written = if mode.is_proxy() {
            Self::resync_proxy_for_saved_row(state, &app_type, provider)
        } else {
            codex_direct::write_direct(
                state.db.as_ref(),
                crate::mode::state::op::APPLY,
                existing.map_or(codex_direct::Owner::None, codex_direct::Owner::Provider),
                Some(provider),
                crate::mode::state::PendingTarget::default(),
            )
            .map(|_| ())
        };
        if let Err(error) = written {
            if crate::mode::operation::has_pending(AppType::Codex.as_str()) {
                return Err(error);
            }
            if let Some(existing) = existing {
                if let Err(rollback) = state.db.save_provider(app_type.as_str(), existing) {
                    return Err(AppError::Message(format!(
                        "更新 Codex 供应商失败: {error}; 恢复供应商数据同时失败: {rollback}"
                    )));
                }
            }
            return Err(error);
        }
        Ok(true)
    }

    /// Delete a provider
    ///
    /// 同时检查本地 settings 和数据库的当前供应商，防止删除任一端正在使用的供应商。
    pub fn delete(state: &AppState, app_type: AppType, id: &str) -> Result<(), AppError> {
        // For other apps: 本地记录、DB、代理路由任何一处指着它都不能删
        if crate::mode::current::is_referenced(&state.db, &app_type, id)? {
            return Err(AppError::Message(
                "无法删除当前正在使用的供应商".to_string(),
            ));
        }

        // Stack 名单里的先移出（和客户端文件同一个操作提交，key 留在登记簿里），成功了再删行。
        // 删行失败时它已经不在名单里，重新加入即可。
        if crate::mode::stack::is_member(&app_type, id)? {
            futures::executor::block_on(crate::mode::controller::set_stack_member(
                state, &app_type, id, false,
            ))
            .map_err(|error| AppError::Message(error.message))?;
        }

        state.db.delete_provider(app_type.as_str(), id)
    }

    /// 切换供应商。
    ///
    /// - 代理模式：只换代理路由，直连指针不变；契约没变时客户端文件不读也不写。
    /// - 直连模式：切换式应用只替换客户端文件里的关键字段（不回填），文件和指针在同一个
    ///   操作里提交；累加式应用按各自的规则写入。
    pub fn switch(state: &AppState, app_type: AppType, id: &str) -> Result<SwitchResult, AppError> {
        let app = app_type.as_str().to_string();
        let result = Self::switch_inner(state, app_type, id);
        match &result {
            Ok(result) if result.warnings.is_empty() => {
                log::info!("[SWITCH] {app} 切到 {id} 完成");
            }
            Ok(result) => {
                log::warn!(
                    "[SWITCH] {app} 切到 {id} 完成，有警告: {:?}",
                    result.warnings
                );
            }
            // 规则拒绝：什么都没改。
            Err(error @ AppError::Localized { key, .. })
                if *key == "switch.official_blocked_by_proxy" =>
            {
                log::info!("[SWITCH] {app} 拒绝切到 {id}: {error}");
            }
            Err(error) => log::error!(
                "[SWITCH] {app} 切到 {id} 失败: {}",
                crate::error_for_log(&error.to_string())
            ),
        }
        result
    }

    fn switch_inner(
        state: &AppState,
        app_type: AppType,
        id: &str,
    ) -> Result<SwitchResult, AppError> {
        // Check if provider exists
        let providers = state.db.get_all_providers(app_type.as_str())?;
        let _provider = providers
            .get(id)
            .ok_or_else(|| AppError::Message(format!("供应商 {id} 不存在")))?;

        if matches!(app_type, AppType::ClaudeDesktop) {
            return Self::switch_normal(state, app_type, id, &providers);
        }

        // 切换和进入 / 退出代理都会改客户端文件和指针。按应用串行，拿到锁、补完上一次
        // 没做完的写入之后再读模式和指针：刚进入代理的应用不会被一次直连写入覆盖，上次
        // 失败后重试也按补完后的指针删上一家的独有字段。
        let _switch_guard = crate::mode::controller::lock_settled_blocking(state, &app_type)?;

        if crate::mode::current::is_proxy(&app_type) {
            // 代理模式：只换代理路由，直连指针不变。契约没变时客户端文件不读也不写；
            // 变了在同一个操作里先改写客户端，再发布路由。
            if _provider.category.as_deref() == Some("official")
                && !official_provider_supports_proxy_takeover(&app_type, _provider)
            {
                return Err(AppError::localized(
                    "switch.official_blocked_by_proxy",
                    "路由模式下不能切换到官方供应商，使用代理访问官方 API 可能导致账号被封禁。请先退出路由模式，或选择第三方供应商。",
                    "Cannot switch to an official provider in routing mode. Using a proxy with official APIs may cause account bans.",
                ));
            }
            log::info!(
                "[SWITCH] {} 代理模式：路由 {} → {}",
                app_type.as_str(),
                crate::mode::current::mode_state(&app_type)
                    .proxy_route
                    .as_deref()
                    .unwrap_or("（无）"),
                id
            );
            futures::executor::block_on(crate::mode::controller::switch_route_locked(
                state, &app_type, _provider,
            ))
            .map_err(|e| AppError::Message(format!("切换路由失败: {e}")))?;
            // MCP 不随路由变：客户端文件没按直连重写。
            return Ok(SwitchResult::default());
        }

        // Normal mode: full switch with Live config write
        Self::switch_normal(state, app_type, id, &providers)
    }

    /// Normal switch flow (non-proxy mode)
    fn switch_normal(
        state: &AppState,
        app_type: AppType,
        id: &str,
        providers: &indexmap::IndexMap<String, Provider>,
    ) -> Result<SwitchResult, AppError> {
        let provider = providers
            .get(id)
            .ok_or_else(|| AppError::Message(format!("供应商 {id} 不存在")))?;

        let previous = crate::mode::current::provider_for(
            &state.db,
            &app_type,
            crate::mode::current::Purpose::Direct,
        )
        .ok()
        .flatten();
        log::info!(
            "[SWITCH] {} 直连：{} → {id}，写客户端配置",
            app_type.as_str(),
            previous.as_deref().unwrap_or("（无）")
        );

        if matches!(app_type, AppType::Claude) {
            return Self::switch_claude_direct(state, provider, providers);
        }
        if matches!(app_type, AppType::Codex) {
            return Self::switch_codex_direct(state, provider, providers);
        }
        if matches!(app_type, AppType::Gemini) {
            gemini_direct::switch_to(state.db.as_ref(), provider)?;
            return Ok(SwitchResult::default());
        }
        if matches!(app_type, AppType::GrokBuild) {
            return Self::switch_grok_direct(state, provider, providers);
        }

        let result = SwitchResult::default();

        crate::settings::set_current_provider(&app_type, Some(id))?;
        state.db.set_current_provider(app_type.as_str(), id)?;

        // 写 live（Claude Desktop；切换式应用在上面各自的分支里写完了）。
        write_live_for_state(state, &app_type, provider)?;

        // 切换重写了目标应用的 live，只重投影该应用的 MCP（Grok Build 的
        // [mcp_servers] 与 live 同文件，整体替换后必须补回；其余应用的
        // MCP 文件独立于 live，投影是幂等维护）。不用全量 sync_all_enabled：
        // 无关应用的 live 损坏（如 ~/.claude.json 坏 JSON）不该阻断切换。
        // 走到这里 DB is_current 与 live 都已落盘，切换事实上已成功；
        // 投影失败上抛会让前端报"切换失败"制造分裂假象，故降级为警告
        // （MCP 投影可自愈：下次切换 / 任一 MCP 启停都会重新投影）。
        if let Err(err) = McpService::sync_enabled_for_app(state, &app_type) {
            log::warn!("切换供应商后重投影 {app_type:?} MCP 失败（将在下次同步时自愈）: {err}");
        }

        Ok(result)
    }

    /// Claude Code 直连切换：只替换关键字段和独有字段，文件和指针在同一个操作里提交。
    ///
    /// 不回填、不同步通用配置片段：用户在 live 里的改动本来就留在原处。发布前的失败
    /// （解析不了、并发冲突）什么都不改；开始发布后由 pending 保证前滚补完。
    fn switch_claude_direct(
        state: &AppState,
        provider: &Provider,
        providers: &IndexMap<String, Provider>,
    ) -> Result<SwitchResult, AppError> {
        let current_id = crate::mode::current::provider_for(
            &state.db,
            &AppType::Claude,
            crate::mode::current::Purpose::Direct,
        )?;
        let prev = current_id
            .as_deref()
            .and_then(|current_id| providers.get(current_id));
        claude_direct::switch_to(state.db.as_ref(), prev, provider)?;

        // MCP 在 ~/.claude.json，和 settings.json 无关；重投影是幂等维护，失败只记警告
        // （切换已经提交，下次同步会自愈）。
        if let Err(err) = McpService::sync_enabled_for_app(state, &AppType::Claude) {
            log::warn!("切换供应商后重投影 claude MCP 失败（将在下次同步时自愈）: {err}");
        }
        Ok(SwitchResult::default())
    }

    /// Codex 直连切换：`config.toml` 只替换关键字段和独有字段；`auth.json`、模型目录、
    /// 托管账号标记和指针在同一个操作里提交。
    ///
    /// 不回填、不同步通用配置片段、不补回 MCP：用户在 live 里的改动（含 `[mcp_servers]`）
    /// 本来就留在原处。行有问题（会把官方登录发给第三方、带 Key 却没地方放）时在写任何
    /// 东西之前报错，指针也不动。
    fn switch_codex_direct(
        state: &AppState,
        provider: &Provider,
        providers: &IndexMap<String, Provider>,
    ) -> Result<SwitchResult, AppError> {
        let current_id = crate::mode::current::provider_for(
            &state.db,
            &AppType::Codex,
            crate::mode::current::Purpose::Direct,
        )?;
        let owner = current_id
            .as_deref()
            .and_then(|current_id| providers.get(current_id))
            .map_or(codex_direct::Owner::None, codex_direct::Owner::Provider);
        codex_direct::write_direct(
            state.db.as_ref(),
            crate::mode::state::op::SWITCH,
            owner,
            Some(provider),
            crate::mode::state::PendingTarget::pointer(Some(provider.id.clone())),
        )?;

        let mut result = SwitchResult::default();
        // 保留登录关闭时切到第三方要删掉 auth.json。删不掉（只读目录、被占用）不让切换
        // 失败：配置和指针都已提交，但要让用户看到官方登录还在盘上。
        if !codex_direct::is_official(provider)
            && !crate::settings::preserve_codex_official_auth_on_switch()
            && crate::codex_config::get_codex_auth_path().exists()
        {
            log::warn!("Codex auth.json still present after a preservation-off third-party switch");
            result
                .warnings
                .push("codex_auth_cleanup_failed".to_string());
        }
        Ok(result)
    }

    /// Grok Build 直连切换：`models.default` 和模型表换成目标的，按写入记录删掉上一家
    /// 的表；文件、指针和写入记录在同一个操作里提交。不回填、不补回 MCP。
    fn switch_grok_direct(
        state: &AppState,
        provider: &Provider,
        providers: &IndexMap<String, Provider>,
    ) -> Result<SwitchResult, AppError> {
        let current_id = crate::mode::current::provider_for(
            &state.db,
            &AppType::GrokBuild,
            crate::mode::current::Purpose::Direct,
        )?;
        let live_owner = current_id
            .as_deref()
            .and_then(|current_id| providers.get(current_id));
        grok_direct::switch_to(state.db.as_ref(), live_owner, provider)?;
        Ok(SwitchResult::default())
    }

    /// Sync current provider to live configuration (re-export)
    pub fn sync_current_to_live(state: &AppState) -> Result<(), AppError> {
        sync_current_to_live(state)
    }

    pub fn sync_current_provider_for_app(
        state: &AppState,
        app_type: AppType,
    ) -> Result<(), AppError> {
        // 没有正在用的那家、或者在代理模式（客户端文件没按直连重写）时不重投影 MCP。
        let outcome = live::sync_current_provider_for_app_respecting_mode(state, &app_type)?;
        if outcome != Some(LiveSyncOutcome::WroteLive) {
            return Ok(());
        }
        McpService::sync_enabled_for_app(state, &app_type)
    }

    pub fn migrate_legacy_common_config_usage(
        state: &AppState,
        app_type: AppType,
        legacy_snippet: &str,
    ) -> Result<(), AppError> {
        if app_type.is_additive_mode() || legacy_snippet.trim().is_empty() {
            return Ok(());
        }

        let providers = state.db.get_all_providers(app_type.as_str())?;

        for provider in providers.values() {
            if provider
                .meta
                .as_ref()
                .and_then(|meta| meta.common_config_enabled)
                .is_some()
            {
                continue;
            }

            if !live::provider_uses_common_config(&app_type, provider, Some(legacy_snippet)) {
                continue;
            }

            let mut updated_provider = provider.clone();
            updated_provider
                .meta
                .get_or_insert_with(Default::default)
                .common_config_enabled = Some(true);

            match live::remove_common_config_from_settings(
                &app_type,
                &updated_provider.settings_config,
                legacy_snippet,
            ) {
                Ok(settings) => updated_provider.settings_config = settings,
                Err(err) => {
                    log::warn!(
                        "Failed to normalize legacy common config for {} provider '{}': {err}",
                        app_type.as_str(),
                        updated_provider.id
                    );
                }
            }

            state
                .db
                .save_provider(app_type.as_str(), &updated_provider)?;
        }

        Ok(())
    }

    pub fn migrate_legacy_common_config_usage_if_needed(
        state: &AppState,
        app_type: AppType,
    ) -> Result<(), AppError> {
        if app_type.is_additive_mode() {
            return Ok(());
        }

        let Some(snippet) = state.db.get_config_snippet(app_type.as_str())? else {
            return Ok(());
        };

        if snippet.trim().is_empty() {
            return Ok(());
        }

        Self::migrate_legacy_common_config_usage(state, app_type, &snippet)
    }

    /// Extract common config snippet from current provider
    ///
    /// Extracts the current provider's configuration and removes provider-specific fields
    /// (API keys, model settings, endpoints) to create a reusable common config snippet.
    pub fn extract_common_config_snippet(
        state: &AppState,
        app_type: AppType,
    ) -> Result<String, AppError> {
        // Get current provider
        let current_id = Self::current(state, app_type.clone())?;
        if current_id.is_empty() {
            return Err(AppError::Message("No current provider".to_string()));
        }

        let providers = state.db.get_all_providers(app_type.as_str())?;
        let provider = providers
            .get(&current_id)
            .ok_or_else(|| AppError::Message(format!("Provider {current_id} not found")))?;

        match app_type {
            AppType::Claude => Self::extract_claude_common_config(&provider.settings_config),
            AppType::ClaudeDesktop => Ok(String::new()),
            AppType::Codex => Self::extract_codex_common_config(&provider.settings_config),
            AppType::Gemini => Self::extract_gemini_common_config(&provider.settings_config),
            AppType::GrokBuild => Ok(String::new()),
        }
    }

    /// Extract common config snippet from a config value (e.g. editor content).
    pub fn extract_common_config_snippet_from_settings(
        app_type: AppType,
        settings_config: &Value,
    ) -> Result<String, AppError> {
        match app_type {
            AppType::Claude => Self::extract_claude_common_config(settings_config),
            AppType::ClaudeDesktop => Ok(String::new()),
            AppType::Codex => Self::extract_codex_common_config(settings_config),
            AppType::Gemini => Self::extract_gemini_common_config(settings_config),
            AppType::GrokBuild => Ok(String::new()),
        }
    }

    /// 判断一个 env / 顶层配置键名是否为凭据/机密：凡命中一律不得写入共享的
    /// 通用配置片段。**故意从严**——多剥一个非机密键只是它不被共享（可恢复的小
    /// 不便），漏剥一个凭据则会把密钥注入到每个供应商（不可恢复的泄漏）。因此用
    /// 模式匹配覆盖整类，而非枚举具体名字（枚举永远会漏掉下一个 `*_API_KEY`）。
    ///
    /// 覆盖：Anthropic / OpenRouter / Google / OpenAI / Gemini 等 `*_API_KEY`
    /// （Claude provider 的凭据见 `Provider::resolve_usage_credentials`，确实支持
    /// `OPENROUTER_API_KEY` / `GOOGLE_API_KEY` 等回退）、各类 `*_AUTH_TOKEN` /
    /// 单数 `*_TOKEN`、AWS Bedrock / Vertex 凭据、通用 secret / password /
    /// 私钥命名，以及发往上游的自定义请求头、Cookie、Authorization。
    pub(crate) fn is_sensitive_config_key(name: &str) -> bool {
        let upper = name.to_ascii_uppercase();

        // 单数 `_TOKEN` 命中 AWS_SESSION_TOKEN 等，但**不**误伤复数 `_TOKENS`
        // （CLAUDE_CODE_MAX_OUTPUT_TOKENS / MAX_THINKING_TOKENS 是正常可共享配置）。
        const SENSITIVE_SUFFIXES: &[&str] = &[
            // 裸 `_KEY` 是最常见的凭据写法（OPENAI_KEY / GROQ_KEY / XAI_KEY…），
            // 必须单列：只枚举 `_API_KEY` / `_ACCESS_KEY` 这些子类，等于把最普通
            // 的那一种漏在外面。下面几条 `_*_KEY` 被它蕴含，保留是为了说明覆盖面。
            "_KEY",
            "_API_KEY",
            "_ACCESS_KEY",
            "_ACCESS_KEY_ID",
            "_KEY_ID",
            "_PRIVATE_KEY",
            // 不带分隔符的复合写法各走各的后缀：`_KEY` 够不着 `..._APIKEY`
            // （倒数第四个字符是 I 不是下划线）。VOLC_ACCESSKEY 是火山引擎文档
            // 里的正式变量名，本仓库就实现了火山 AK/SK 用量查询。
            "_APIKEY",
            "_ACCESSKEY",
            "_SECRETKEY",
            "_APITOKEN",
            "_AUTH_TOKEN",
            "_TOKEN",
            // GITHUB_PAT / GITLAB_PAT 等 personal access token 的惯用写法，
            // 既不含 TOKEN 也不含 KEY，前面每一条规则都够不着。
            "_PAT",
            // 口令类的常见缩写。`_PASS` 不会误伤 `*_BYPASS`（那个以 `_BYPASS`
            // 结尾），`_PWD` 也不会误伤 shell 的 PWD / OLDPWD。
            "_PWD",
            "_PASS",
            "_PASSPHRASE",
            "_CREDS",
            // 发往上游的自定义请求头（ANTHROPIC_CUSTOM_HEADERS、
            // GEMINI_CLI_CUSTOM_HEADERS）：常见写法是 `Authorization: Bearer …`
            // 或 `Cookie: …`，整串就是凭据。只认 `_CUSTOM_HEADERS`，不按 HEADER
            // 一刀切：CLAUDE_CODE_ATTRIBUTION_HEADER 是普通开关，
            // OTEL_EXPORTER_OTLP_HEADERS 发往用户自己的遥测端点，都应照常共享。
            "_CUSTOM_HEADERS",
        ];
        const SENSITIVE_EXACT: &[&str] = &[
            "APIKEY",
            "API_KEY",
            "TOKEN",
            "SECRET",
            "PASSWORD",
            "CREDENTIALS",
            "HEADERS",
        ];
        // contains：覆盖 AWS_SECRET_ACCESS_KEY / *_CLIENT_SECRET /
        // GOOGLE_APPLICATION_CREDENTIALS / AWS_BEARER_TOKEN_BEDROCK 等变体。
        const SENSITIVE_CONTAINS: &[&str] = &[
            "SECRET",
            "PASSWORD",
            "PASSWD",
            "CREDENTIAL",
            "PRIVATE_KEY",
            "BEARER_TOKEN",
            "COOKIE",
            "AUTHORIZATION",
        ];

        SENSITIVE_EXACT.contains(&upper.as_str())
            || SENSITIVE_SUFFIXES.iter().any(|s| upper.ends_with(s))
            || SENSITIVE_CONTAINS.iter().any(|c| upper.contains(c))
    }

    /// Claude `env` 里的键能否进通用配置片段。
    ///
    /// 片段会合并进每一家勾选了它的供应商，所以只能放与供应商无关的设置。关键字段
    /// （请求发到哪、凭什么鉴权、哪个模型名、哪种协议）一旦进了片段，就会跟着切换带给
    /// 下一家：从 Bedrock 切到官方后，官方的 live 里还留着 `CLAUDE_CODE_USE_BEDROCK=1`，
    /// Claude Code 继续走 Bedrock。凭据另由 `is_sensitive_config_key` 统一剥离。
    fn claude_env_key_is_shared(key: &str) -> bool {
        // Context limits follow the actual upstream model. Sharing these
        // across providers can cap GPT/Kimi to the wrong window and make
        // Claude Code compact too early or miss the upstream limit.
        const UPSTREAM_WINDOW_KEYS: &[&str] = &[
            "CLAUDE_CODE_MAX_CONTEXT_TOKENS",
            "CLAUDE_CODE_AUTO_COMPACT_WINDOW",
        ];

        !crate::live::floor::claude_floor_env(key)
            && !UPSTREAM_WINDOW_KEYS.contains(&key)
            && !Self::is_sensitive_config_key(key)
    }

    /// Claude 顶层键能否进通用配置片段，口径同 [`Self::claude_env_key_is_shared`]。
    /// `model` 是 `/model` 保存的选择，属于当时那一家。
    fn claude_top_key_is_shared(key: &str) -> bool {
        !crate::live::floor::claude_floor_top(key) && !Self::is_sensitive_config_key(key)
    }

    /// Extract common config for Claude (JSON format)
    fn extract_claude_common_config(settings: &Value) -> Result<String, AppError> {
        let mut config = settings.clone();

        if let Some(obj) = config.as_object_mut() {
            if let Some(Value::Object(env)) = obj.get_mut("env") {
                env.retain(|key, _| Self::claude_env_key_is_shared(key));
            }
            obj.retain(|key, value| match key.as_str() {
                "env" => !value.as_object().is_some_and(|env| env.is_empty()),
                _ => Self::claude_top_key_is_shared(key),
            });
        }

        // Check if result is empty
        if config.as_object().is_none_or(|obj| obj.is_empty()) {
            return Ok("{}".to_string());
        }

        serde_json::to_string_pretty(&config)
            .map_err(|e| AppError::Message(format!("Serialization failed: {e}")))
    }

    /// Extract common config for Codex (TOML format)
    fn extract_codex_common_config(settings: &Value) -> Result<String, AppError> {
        // Codex config is stored as { "auth": {...}, "config": "toml string" }
        let config_toml = settings
            .get("config")
            .and_then(|v| v.as_str())
            .unwrap_or("");

        if config_toml.is_empty() {
            return Ok(String::new());
        }

        let mut doc = config_toml
            .parse::<toml_edit::DocumentMut>()
            .map_err(|e| AppError::Message(format!("TOML parse error: {e}")))?;

        // 关键字段（选路、模型名、推理档位等，见 `live::floor`）归供应商，不进片段：
        // 片段已冻结，收进去的值会从行里被迁移剥掉、再也写不回 live。
        let root = doc.as_table_mut();
        for key in crate::live::floor::CODEX_FLOOR_TOP {
            root.remove(key);
        }
        for path in crate::live::floor::CODEX_FLOOR_NESTED {
            let [parent, key] = path else { continue };
            if let Some(table) = root
                .get_mut(parent)
                .and_then(|item| item.as_table_like_mut())
            {
                table.remove(key);
                if table.is_empty() {
                    root.remove(parent);
                }
            }
        }

        // Remove entire model_providers table (provider-specific configuration)
        root.remove("model_providers");

        // MCP 服务器归 DB mcp_servers 表所有：进了共享片段会绕过按应用的
        // 启用状态被合并进所有勾选通用配置的供应商，且在通用配置编辑框里
        // 显示为一份"重复"的 MCP 配置。
        root.remove("mcp_servers");
        // 历史错误格式 [mcp.servers] 一并剥离（与 strip_codex_mcp_servers_from_settings
        // 一致）：sync_all_enabled 只管理 [mcp_servers.*]，legacy 形态一旦进了
        // 片段就会被合并进所有供应商，且没有任何同步路径能清掉这个孤儿。
        if let Some(mcp_tbl) = root
            .get_mut("mcp")
            .and_then(|item| item.as_table_like_mut())
        {
            mcp_tbl.remove("servers");
            if mcp_tbl.is_empty() {
                root.remove("mcp");
            }
        }

        // cc-switch 写 live 时注入的产物一律不进共享片段：
        // - experimental_bearer_token 正常写在 [model_providers.<id>] 内（上面
        //   整表已剥），但无活跃路由 / 内建保留 id / 路由表缺失三种 fallback
        //   会落在顶层——不剥等于把 API 密钥写进共享片段。
        root.remove("experimental_bearer_token");
        // - model_catalog_json 指向按供应商生成的 catalog 投影文件（DB 为 SSOT）。
        root.remove("model_catalog_json");
        // - web_search 只剥 cc-switch 注入的 "disabled" 哨兵；用户手设的其它值
        //   属于可共享偏好，保留。
        if root
            .get(crate::codex_config::CODEX_WEB_SEARCH_FIELD)
            .and_then(|item| item.as_str())
            == Some(crate::codex_config::CODEX_WEB_SEARCH_DISABLED)
        {
            root.remove(crate::codex_config::CODEX_WEB_SEARCH_FIELD);
        }

        // Clean up multiple empty lines (keep at most one blank line).
        let mut cleaned = String::new();
        let mut blank_run = 0usize;
        for line in doc.to_string().lines() {
            if line.trim().is_empty() {
                blank_run += 1;
                if blank_run <= 1 {
                    cleaned.push('\n');
                }
                continue;
            }
            blank_run = 0;
            cleaned.push_str(line);
            cleaned.push('\n');
        }

        Ok(cleaned.trim().to_string())
    }

    /// Extract common config for Gemini (JSON format)
    ///
    /// Extracts `.env` values while excluding provider-specific credentials:
    /// - GOOGLE_GEMINI_BASE_URL
    /// - GEMINI_API_KEY
    fn extract_gemini_common_config(settings: &Value) -> Result<String, AppError> {
        let env = settings.get("env").and_then(|v| v.as_object());

        let mut snippet = serde_json::Map::new();
        if let Some(env) = env {
            for (key, value) in env {
                // 关键字段（地址、凭据、模型名、Vertex 选择器）归供应商，不进片段：
                // 片段已冻结，新版不再合并它，收进去的值会从行里被迁移剥掉、再也写不回
                // live（首启导入的 `GEMINI_MODEL` 就是这样丢的）。凭据另交给
                // `is_sensitive_config_key` 统一模式匹配（与 Claude 提取器一致），
                // 兜住关键字段清单以外的 `*_API_KEY`、`*_TOKEN`。
                if crate::live::floor::gemini_floor_env(key) || Self::is_sensitive_config_key(key) {
                    continue;
                }
                let Value::String(v) = value else {
                    continue;
                };
                let trimmed = v.trim();
                if !trimmed.is_empty() {
                    snippet.insert(key.to_string(), Value::String(trimmed.to_string()));
                }
            }
        }

        if snippet.is_empty() {
            return Ok("{}".to_string());
        }

        serde_json::to_string_pretty(&Value::Object(snippet))
            .map_err(|e| AppError::Message(format!("Serialization failed: {e}")))
    }

    /// 一次性清理：把历史泄漏进 Gemini 共享片段的凭据从所有存储位置抹掉。
    ///
    /// 背景：`extract_gemini_common_config` 曾只剥离两个固定键名，`GOOGLE_API_KEY`
    /// 等一等凭据会进入共享片段，再在切换时被深合并进
    /// **其它** Gemini 供应商的 env，随请求发往对方的 base_url。
    ///
    /// 光修提取器不够：Gemini 的片段一旦生成就**永不自动重提取**（启动期
    /// auto-extract 与导入后补提取都要求 `snippet.is_none()`，切换时的回写又只对
    /// Claude / Codex 生效），所以存量片段会一直带着密钥继续注入。
    ///
    /// 两个关键约束：
    ///
    /// 1. **不能只清片段**。合并与剥离是一对靠「值相等」严格抵消的操作：切走供应商时
    ///    `remove_common_config_from_settings` 依据片段内容把注入的键删掉。片段里一旦
    ///    没了这个键，backfill 就会把 live 中残留的密钥原样写进受害供应商的
    ///    `settings_config`——泄漏从瞬时污染变成永久污染。所以片段、各供应商配置、
    ///    live 文件必须一起清。
    /// 2. **按值相等定向删除，不按键名一刀切**。复用 `remove_common_config_from_settings`
    ///    可以只清掉扩散出去的那一份，保留某个供应商自己写的、值不同的同名键。
    ///
    /// 步骤顺序本身是安全属性的一部分：**清片段必须排在最后**。片段是
    /// `remove_common_config_from_settings` 唯一的"该剥哪些键"来源，一旦清空，任何
    /// 残留（live 文件里的、下一轮重试要处理的）都再也无法被识别和剥离。所以所有
    /// 可能失败的步骤都排在它前面，失败即带错返回，让下次启动能原样重来。
    ///
    /// 清理后部分供应商会显示缺少 API Key，需用户重填——这是正确行为：那把密钥本就
    /// 不属于它们。（受害者原有的同名键在合并时已被覆盖，无法恢复。）动手前会往
    /// settings 的 `gemini_common_config_scrub_audit_v1` 写一条审计记录，内容是
    /// **键名与受影响的供应商 id，不含值**：`settings` 会随 WebDAV/S3 同步上传，
    /// 而这里处理的正是必须销毁的凭据，留值等于把一次清除换成一份跨设备扩散、
    /// 没有界面入口、永不过期的明文副本。
    pub async fn scrub_leaked_gemini_common_config(state: &AppState) -> Result<(), AppError> {
        const FLAG: &str = "gemini_common_config_credentials_scrubbed_v1";
        const AUDIT_KEY: &str = "gemini_common_config_scrub_audit_v1";
        let app = AppType::Gemini;

        if state.db.get_bool_flag(FLAG).unwrap_or(false) {
            return Ok(());
        }

        let Some(snippet_text) = state.db.get_config_snippet(app.as_str())? else {
            state.db.set_setting(FLAG, "true")?;
            return Ok(());
        };

        // 片段解析不了就不动它，只标记完成——乱改用户数据比留着更糟
        let Ok(Value::Object(entries)) = serde_json::from_str::<Value>(&snippet_text) else {
            state.db.set_setting(FLAG, "true")?;
            return Ok(());
        };

        let mut poison = serde_json::Map::new();
        let mut clean = serde_json::Map::new();
        for (key, value) in entries {
            if Self::is_sensitive_config_key(&key) {
                poison.insert(key, value);
            } else {
                clean.insert(key, value);
            }
        }

        if poison.is_empty() {
            state.db.set_setting(FLAG, "true")?;
            return Ok(());
        }

        log::warn!(
            "检测到 {} 个凭据键残留在 Gemini 通用配置片段中，开始一次性清理",
            poison.len()
        );

        let poison_keys: Vec<String> = poison.keys().cloned().collect();
        let poison_value = Value::Object(poison);
        let poison_text = serde_json::to_string(&poison_value)
            .map_err(|e| AppError::Message(format!("Serialization failed: {e}")))?;

        // 1) 先算出各供应商清理后的配置，但**先不落库**
        let providers = state.db.get_all_providers(app.as_str())?;
        let mut pending: Vec<(String, Provider, Value)> = Vec::new();
        for (id, provider) in providers {
            let cleaned = match live::remove_common_config_from_settings(
                &app,
                &provider.settings_config,
                &poison_text,
            ) {
                Ok(cleaned) => cleaned,
                Err(err) => {
                    log::warn!("清理供应商 '{id}' 的泄漏凭据失败: {err}");
                    continue;
                }
            };
            if cleaned != provider.settings_config {
                pending.push((id, provider, cleaned));
            }
        }

        // 2) 落库前留一份审计记录：**只记键名与受影响的供应商，不记值**。
        //
        //    「按值相等定向删除」在一种合法场景下也会命中：用户有意在多个供应商里
        //    复用同一把 key。所以必须留下"删了什么、从哪删的"，否则用户只能靠翻
        //    日志。但不能留值——`settings` 表不在 `SYNC_SKIP_TABLES` 里，会随
        //    WebDAV/S3 同步上传，而这里处理的恰恰是必须销毁的泄漏凭据：留值等于
        //    把一次清除换成一份没有界面入口、永不过期、还会跨设备扩散的明文副本。
        //    密钥本来就该轮换，可恢复性不值这个代价。
        let removed_env_keys = |before: &Value, after: &Value| -> Vec<String> {
            let before_env = before.get("env").and_then(Value::as_object);
            let after_env = after.get("env").and_then(Value::as_object);
            match (before_env, after_env) {
                (Some(before_env), Some(after_env)) => before_env
                    .keys()
                    .filter(|key| !after_env.contains_key(*key))
                    .cloned()
                    .collect(),
                (Some(before_env), None) => before_env.keys().cloned().collect(),
                _ => Vec::new(),
            }
        };
        let audit = serde_json::json!({
            "removedFromSnippet": poison_keys,
            "providers": pending
                .iter()
                .map(|(id, provider, cleaned)| serde_json::json!({
                    "id": id,
                    "removedKeys": removed_env_keys(&provider.settings_config, cleaned),
                }))
                .collect::<Vec<_>>(),
        });
        let audit_text = serde_json::to_string(&audit)
            .map_err(|e| AppError::Message(format!("Serialization failed: {e}")))?;
        // 只在没有记录时写。provider 的写入不是一个事务（每次 save_provider 各自
        // 提交），上一轮可能改到一半就中止；此时完成标记没置位，下次启动会重跑，
        // 而重跑看到的"原始状态"已经残缺。无条件 INSERT OR REPLACE 会拿这份残缺
        // 记录盖掉第一轮那份完整的。
        if state.db.get_setting(AUDIT_KEY)?.is_none() {
            state.db.set_setting(AUDIT_KEY, &audit_text)?;
        }

        // 3) 各供应商 settings_config：按值相等定向删除扩散出去的副本
        for (id, provider, cleaned) in pending {
            let mut updated = provider;
            updated.settings_config = cleaned;
            state.db.save_provider(app.as_str(), &updated)?;
            log::info!("已从 Gemini 供应商 '{id}' 中清除泄漏的共享凭据");
        }

        // 4) 旧版接管留下的 live 快照（备份行）里也可能有一份副本。这一步的失败**必须
        //    传播**：
        //
        //    新版启动时不回放这份快照（转存到本机文件后删除），但同一次启动里它排在
        //    这里之后；在那之前降级的话，旧版停代理时会把快照原样写回 `~/.gemini/.env`。
        //    若它仍带毒而我们照样清了片段、置了完成标记，凭据就会复活，而一次性标记又
        //    保证不会再清第二次。
        //
        //    带错返回是安全的失败方式：调用方（lib.rs:1189）只记 warn 不中断启动，
        //    片段和标记都原样留着，下次启动照原样重来。
        if let Some(backup) = state.db.get_live_backup(app.as_str()).await? {
            let original: Value = serde_json::from_str(&backup.original_config)
                .map_err(|e| AppError::Message(format!("解析 Gemini 代理接管备份失败: {e}")))?;
            let cleaned = live::remove_common_config_from_settings(&app, &original, &poison_text)?;
            if cleaned != original {
                let text = serde_json::to_string(&cleaned)
                    .map_err(|e| AppError::Message(format!("Serialization failed: {e}")))?;
                state.db.save_live_backup(app.as_str(), &text).await?;
                log::info!("已从 Gemini 代理接管备份中清除泄漏的共享凭据");
            }
        }

        // 5) `~/.gemini/.env`：**定向**删除，且必须在清片段之前做，失败即中止。
        //
        //    为什么不用 `sync_current_provider_for_app` 重投影：它在没有当前供应商
        //    时直接返回 Ok 而根本不写文件，泄漏值会原样留在 live 里；等片段被清空
        //    之后，下次切换时 `remove_common_config_from_settings` 再也认不出这个
        //    键，backfill 就把它永久写进受害供应商的配置——正是本函数开头说的那个
        //    顺序陷阱，只是由"没修"变成"修了一半更糟"。定向删除还顺带保住了只存在
        //    于 live、与供应商无关的手工 env（重投影会把它们抹掉）。
        //
        //    删除走 `remove_gemini_env_entries` 的**保序**实现而不是 read→HashMap→
        //    write 往返：后者会顺手抹掉注释、空行和无法识别的行，并按键名重排整个
        //    文件。全量投影时那无所谓，但这里是一次用户没主动触发的启动期清理，不该
        //    连带改写与泄漏无关的内容。
        //
        //    失败就带着错误返回：片段此刻还留着毒键，完成标记也没置位，下次启动能
        //    照原样重来。清片段是不可逆的一步，必须排在所有会失败的步骤之后。
        let poison_env: HashMap<String, String> = poison_value
            .as_object()
            .map(|map| {
                map.iter()
                    .filter_map(|(key, value)| {
                        value.as_str().map(|text| (key.clone(), text.to_string()))
                    })
                    .collect()
            })
            .unwrap_or_default();
        if crate::gemini_config::remove_gemini_env_entries(&poison_env)? {
            log::info!("已从 ~/.gemini/.env 中清除泄漏的共享凭据");
        }

        // 6) 片段本身：保留可共享的部分。全部清空时删行而不是写 "{}"——留着空行会让
        //    should_auto_extract_config_snippet 永远为 false，用户的合法共享配置再也
        //    重建不回来。同理绝不置 cleared 标记。
        if clean.is_empty() {
            state.db.set_config_snippet(app.as_str(), None)?;
        } else {
            let cleaned_snippet = serde_json::to_string_pretty(&Value::Object(clean))
                .map_err(|e| AppError::Message(format!("Serialization failed: {e}")))?;
            state
                .db
                .set_config_snippet(app.as_str(), Some(cleaned_snippet))?;
        }

        state.db.set_setting(FLAG, "true")?;
        log::info!("Gemini 通用配置凭据清理完成");
        Ok(())
    }

    /// Import default configuration from live files (re-export)
    ///
    /// Returns `Ok(true)` if imported, `Ok(false)` if skipped.
    pub fn import_default_config(state: &AppState, app_type: AppType) -> Result<bool, AppError> {
        import_default_config(state, app_type)
    }

    pub fn should_import_default_config_on_startup(
        state: &AppState,
        app_type: &AppType,
    ) -> Result<bool, AppError> {
        should_import_default_config_on_startup(state, app_type)
    }

    /// Read current live settings (re-export)
    pub fn read_live_settings(app_type: AppType) -> Result<Value, AppError> {
        read_live_settings(app_type)
    }

    /// Get custom endpoints list (re-export)
    pub fn get_custom_endpoints(
        state: &AppState,
        app_type: AppType,
        provider_id: &str,
    ) -> Result<Vec<CustomEndpoint>, AppError> {
        endpoints::get_custom_endpoints(state, app_type, provider_id)
    }

    /// Add custom endpoint (re-export)
    pub fn add_custom_endpoint(
        state: &AppState,
        app_type: AppType,
        provider_id: &str,
        url: String,
    ) -> Result<(), AppError> {
        endpoints::add_custom_endpoint(state, app_type, provider_id, url)
    }

    /// Remove custom endpoint (re-export)
    pub fn remove_custom_endpoint(
        state: &AppState,
        app_type: AppType,
        provider_id: &str,
        url: String,
    ) -> Result<(), AppError> {
        endpoints::remove_custom_endpoint(state, app_type, provider_id, url)
    }

    /// Update endpoint last used timestamp (re-export)
    pub fn update_endpoint_last_used(
        state: &AppState,
        app_type: AppType,
        provider_id: &str,
        url: String,
    ) -> Result<(), AppError> {
        endpoints::update_endpoint_last_used(state, app_type, provider_id, url)
    }

    /// Update provider sort order
    pub fn update_sort_order(
        state: &AppState,
        app_type: AppType,
        updates: Vec<ProviderSortUpdate>,
    ) -> Result<bool, AppError> {
        let mut providers = state.db.get_all_providers(app_type.as_str())?;

        for update in updates {
            if let Some(provider) = providers.get_mut(&update.id) {
                provider.sort_index = Some(update.sort_index);
                state.db.save_provider(app_type.as_str(), provider)?;
            }
        }

        Ok(true)
    }

    /// Query provider usage (re-export)
    pub async fn query_usage(
        state: &AppState,
        app_type: AppType,
        provider_id: &str,
    ) -> Result<UsageResult, AppError> {
        usage::query_usage(state, app_type, provider_id).await
    }

    /// Test usage script (re-export)
    #[allow(clippy::too_many_arguments)]
    pub async fn test_usage_script(
        state: &AppState,
        app_type: AppType,
        provider_id: &str,
        script_code: &str,
        timeout: u64,
        api_key: Option<&str>,
        base_url: Option<&str>,
        access_token: Option<&str>,
        user_id: Option<&str>,
        template_type: Option<&str>,
    ) -> Result<UsageResult, AppError> {
        usage::test_usage_script(
            state,
            app_type,
            provider_id,
            script_code,
            timeout,
            api_key,
            base_url,
            access_token,
            user_id,
            template_type,
        )
        .await
    }

    fn validate_provider_settings(app_type: &AppType, provider: &Provider) -> Result<(), AppError> {
        icon::validate_icon(provider.icon.as_deref())?;
        match app_type {
            AppType::Claude => {
                if !provider.settings_config.is_object() {
                    return Err(AppError::localized(
                        "provider.claude.settings.not_object",
                        "Claude 配置必须是 JSON 对象",
                        "Claude configuration must be a JSON object",
                    ));
                }
            }
            AppType::ClaudeDesktop => {
                crate::claude_desktop_config::validate_provider(provider)?;
            }
            AppType::Codex => {
                let settings = provider.settings_config.as_object().ok_or_else(|| {
                    AppError::localized(
                        "provider.codex.settings.not_object",
                        "Codex 配置必须是 JSON 对象",
                        "Codex configuration must be a JSON object",
                    )
                })?;

                let auth = settings.get("auth").ok_or_else(|| {
                    AppError::localized(
                        "provider.codex.auth.missing",
                        format!("供应商 {} 缺少 auth 配置", provider.id),
                        format!("Provider {} is missing auth configuration", provider.id),
                    )
                })?;
                if !auth.is_object() {
                    return Err(AppError::localized(
                        "provider.codex.auth.not_object",
                        format!("供应商 {} 的 auth 配置必须是 JSON 对象", provider.id),
                        format!(
                            "Provider {} auth configuration must be a JSON object",
                            provider.id
                        ),
                    ));
                }

                if let Some(config_value) = settings.get("config") {
                    if !(config_value.is_string() || config_value.is_null()) {
                        return Err(AppError::localized(
                            "provider.codex.config.invalid_type",
                            "Codex config 字段必须是字符串",
                            "Codex config field must be a string",
                        ));
                    }
                    if let Some(cfg_text) = config_value.as_str() {
                        crate::codex_config::validate_config_toml(cfg_text)?;
                    }
                }
            }
            AppType::Gemini => {
                use crate::gemini_config::validate_gemini_settings;
                validate_gemini_settings(&provider.settings_config)?
            }
            AppType::GrokBuild => {
                let settings = provider.settings_config.as_object().ok_or_else(|| {
                    AppError::localized(
                        "provider.grokbuild.settings.not_object",
                        "Grok Build 配置必须是 JSON 对象",
                        "Grok Build configuration must be a JSON object",
                    )
                })?;
                let config = settings
                    .get("config")
                    .and_then(Value::as_str)
                    .ok_or_else(|| {
                        AppError::localized(
                            "provider.grokbuild.config.missing",
                            "Grok Build 配置缺少 config 字段",
                            "Grok Build configuration is missing the config field",
                        )
                    })?;
                if provider.category.as_deref() == Some("official") {
                    // 官方条目走 Grok CLI 自带 OAuth：空 config 合法，
                    // 其余内容只要求 TOML 语法合法。
                    crate::grok_config::validate_config_toml_syntax(config)?;
                } else {
                    crate::grok_config::validate_config_toml(config)?;
                }
            }
        }

        // Validate and clean UsageScript configuration (common for all app types)
        if let Some(meta) = &provider.meta {
            if let Some(usage_script) = &meta.usage_script {
                validate_usage_script(usage_script)?;
            }
        }

        Ok(())
    }

    #[allow(dead_code)]
    fn extract_credentials(
        provider: &Provider,
        app_type: &AppType,
    ) -> Result<(String, String), AppError> {
        match app_type {
            AppType::Claude => {
                let env = provider
                    .settings_config
                    .get("env")
                    .and_then(|v| v.as_object())
                    .ok_or_else(|| {
                        AppError::localized(
                            "provider.claude.env.missing",
                            "配置格式错误: 缺少 env",
                            "Invalid configuration: missing env section",
                        )
                    })?;

                let api_key = env
                    .get("ANTHROPIC_AUTH_TOKEN")
                    .or_else(|| env.get("ANTHROPIC_API_KEY"))
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| {
                        AppError::localized(
                            "provider.claude.api_key.missing",
                            "缺少 API Key",
                            "API key is missing",
                        )
                    })?
                    .to_string();

                let base_url = env
                    .get("ANTHROPIC_BASE_URL")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| {
                        AppError::localized(
                            "provider.claude.base_url.missing",
                            "缺少 ANTHROPIC_BASE_URL 配置",
                            "Missing ANTHROPIC_BASE_URL configuration",
                        )
                    })?
                    .to_string();

                Ok((api_key, base_url))
            }
            AppType::GrokBuild => {
                let config_toml = provider
                    .settings_config
                    .get("config")
                    .and_then(Value::as_str)
                    .ok_or_else(|| {
                        AppError::localized(
                            "provider.grokbuild.config.missing",
                            "Grok Build 配置缺少 config 字段",
                            "Grok Build configuration is missing the config field",
                        )
                    })?;
                let (base_url, api_key) = crate::grok_config::extract_credentials(config_toml)
                    .ok_or_else(|| {
                        AppError::localized(
                            "provider.grokbuild.credentials.missing",
                            "Grok Build 配置缺少 Base URL 或 API Key",
                            "Grok Build configuration is missing the base URL or API key",
                        )
                    })?;
                Ok((api_key, base_url))
            }
            AppType::ClaudeDesktop => {
                let credentials =
                    crate::claude_desktop_config::direct_gateway_credentials(provider)?;
                Ok((credentials.api_key, credentials.base_url))
            }
            AppType::Codex => {
                let _auth = provider
                    .settings_config
                    .get("auth")
                    .and_then(|v| v.as_object())
                    .ok_or_else(|| {
                        AppError::localized(
                            "provider.codex.auth.missing",
                            "配置格式错误: 缺少 auth",
                            "Invalid configuration: missing auth section",
                        )
                    })?;

                let config_toml = provider
                    .settings_config
                    .get("config")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");

                let api_key = crate::codex_config::extract_codex_api_key(
                    provider.settings_config.get("auth"),
                    Some(config_toml),
                )
                .ok_or_else(|| {
                    AppError::localized(
                        "provider.codex.api_key.missing",
                        "缺少 API Key",
                        "API key is missing",
                    )
                })?;

                let base_url = if config_toml.contains("base_url") {
                    let re = Regex::new(r#"base_url\s*=\s*["']([^"']+)["']"#).map_err(|e| {
                        AppError::localized(
                            "provider.regex_init_failed",
                            format!("正则初始化失败: {e}"),
                            format!("Failed to initialize regex: {e}"),
                        )
                    })?;
                    re.captures(config_toml)
                        .and_then(|caps| caps.get(1))
                        .map(|m| m.as_str().to_string())
                        .ok_or_else(|| {
                            AppError::localized(
                                "provider.codex.base_url.invalid",
                                "config.toml 中 base_url 格式错误",
                                "base_url in config.toml has invalid format",
                            )
                        })?
                } else {
                    return Err(AppError::localized(
                        "provider.codex.base_url.missing",
                        "config.toml 中缺少 base_url 配置",
                        "base_url is missing from config.toml",
                    ));
                };

                Ok((api_key, base_url))
            }
            AppType::Gemini => {
                use crate::gemini_config::json_to_env;

                let env_map = json_to_env(&provider.settings_config)?;

                let api_key = env_map.get("GEMINI_API_KEY").cloned().ok_or_else(|| {
                    AppError::localized(
                        "gemini.missing_api_key",
                        "缺少 GEMINI_API_KEY",
                        "Missing GEMINI_API_KEY",
                    )
                })?;

                let base_url = env_map
                    .get("GOOGLE_GEMINI_BASE_URL")
                    .cloned()
                    .unwrap_or_else(|| "https://generativelanguage.googleapis.com".to_string());

                Ok((api_key, base_url))
            }
        }
    }
}

/// Normalize Claude model keys in a JSON value
///
/// Reads old key (ANTHROPIC_SMALL_FAST_MODEL), writes new keys (DEFAULT_*), and deletes old key.
pub(crate) fn normalize_claude_models_in_value(settings: &mut Value) -> bool {
    let mut changed = false;
    let env = match settings.get_mut("env").and_then(|v| v.as_object_mut()) {
        Some(obj) => obj,
        None => return changed,
    };

    let model = env
        .get("ANTHROPIC_MODEL")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let small_fast = env
        .get("ANTHROPIC_SMALL_FAST_MODEL")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    let current_haiku = env
        .get("ANTHROPIC_DEFAULT_HAIKU_MODEL")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let current_sonnet = env
        .get("ANTHROPIC_DEFAULT_SONNET_MODEL")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let current_opus = env
        .get("ANTHROPIC_DEFAULT_OPUS_MODEL")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    let target_haiku = current_haiku
        .or_else(|| small_fast.clone())
        .or_else(|| model.clone());
    let target_sonnet = current_sonnet
        .or_else(|| model.clone())
        .or_else(|| small_fast.clone());
    let target_opus = current_opus
        .or_else(|| model.clone())
        .or_else(|| small_fast.clone());

    if env.get("ANTHROPIC_DEFAULT_HAIKU_MODEL").is_none() {
        if let Some(v) = target_haiku {
            env.insert(
                "ANTHROPIC_DEFAULT_HAIKU_MODEL".to_string(),
                Value::String(v),
            );
            changed = true;
        }
    }
    if env.get("ANTHROPIC_DEFAULT_SONNET_MODEL").is_none() {
        if let Some(v) = target_sonnet {
            env.insert(
                "ANTHROPIC_DEFAULT_SONNET_MODEL".to_string(),
                Value::String(v),
            );
            changed = true;
        }
    }
    if env.get("ANTHROPIC_DEFAULT_OPUS_MODEL").is_none() {
        if let Some(v) = target_opus {
            env.insert("ANTHROPIC_DEFAULT_OPUS_MODEL".to_string(), Value::String(v));
            changed = true;
        }
    }

    if env.remove("ANTHROPIC_SMALL_FAST_MODEL").is_some() {
        changed = true;
    }

    changed
}

#[derive(Debug, Clone, Deserialize)]
pub struct ProviderSortUpdate {
    pub id: String,
    #[serde(rename = "sortIndex")]
    pub sort_index: usize,
}

// ============================================================================
// 统一供应商（Universal Provider）服务方法
// ============================================================================

use crate::provider::UniversalProvider;
use std::collections::HashMap;

impl ProviderService {
    /// 获取所有统一供应商
    pub fn list_universal(
        state: &AppState,
    ) -> Result<HashMap<String, UniversalProvider>, AppError> {
        state.db.get_all_universal_providers()
    }

    /// 获取单个统一供应商
    pub fn get_universal(
        state: &AppState,
        id: &str,
    ) -> Result<Option<UniversalProvider>, AppError> {
        state.db.get_universal_provider(id)
    }

    /// 添加或更新统一供应商（不自动同步，需手动调用 sync_universal_to_apps）
    pub fn upsert_universal(
        state: &AppState,
        provider: UniversalProvider,
    ) -> Result<bool, AppError> {
        // 统一供应商没有 per-app 开关：任何来源（旧版本前端、导入的数据）都归一成
        // 「五个应用全开」，免得留一份只同步到部分应用、却又没有界面能改回来的记录。
        let mut provider = provider;
        if !provider.apps.is_all() {
            provider.apps = crate::provider::UniversalProviderApps::all();
        }
        icon::validate_icon(provider.icon.as_deref())?;
        // 保存统一供应商
        state.db.save_universal_provider(&provider)?;

        Ok(true)
    }

    /// 拉取可用模型列表：`/v1/models` 出 id，oh-my-pi 模型目录补名称与上下文窗口。
    ///
    /// 传 `id` 时用已存的那家供应商的地址/Key 并写回它自己；不传时用传入的地址与 Key
    /// 现拉（新建场景：记录还没落库，没有 id 可用）。
    pub async fn refresh_universal_models(
        state: &AppState,
        id: Option<&str>,
        base_url: Option<&str>,
        api_key: Option<&str>,
    ) -> Result<Vec<crate::provider::AvailableModel>, AppError> {
        let stored = match id {
            Some(id) => Some(
                state
                    .db
                    .get_universal_provider(id)?
                    .ok_or_else(|| AppError::Message(format!("统一供应商 {id} 不存在")))?,
            ),
            None => None,
        };

        let base_url = stored
            .as_ref()
            .map(|provider| provider.base_url.clone())
            .or_else(|| base_url.map(str::to_string))
            .ok_or_else(|| AppError::Message("缺少 API 地址，无法获取模型列表".to_string()))?;
        let api_key = stored
            .as_ref()
            .map(|provider| provider.api_key.clone())
            .or_else(|| api_key.map(str::to_string))
            .unwrap_or_default();

        // 与单应用表单一致：协议、完整 URL、UA、请求头都取 Claude 那一份配置。
        let claude = stored
            .as_ref()
            .and_then(|provider| provider.models.claude.clone())
            .unwrap_or_default();
        let ua = crate::provider::parse_custom_user_agent(claude.custom_user_agent.as_deref())
            .ok()
            .flatten();
        let headers: Option<std::collections::BTreeMap<String, String>> = claude
            .request_overrides
            .as_ref()
            .map(|overrides| overrides.headers.clone())
            .filter(|headers| !headers.is_empty())
            .map(|headers| headers.into_iter().collect());

        let fetched = crate::services::model_fetch::fetch_models(
            &base_url,
            &api_key,
            claude.is_full_url.unwrap_or(false),
            None,
            ua,
            claude.api_format.as_deref(),
            headers.as_ref(),
        )
        .await
        .map_err(|e| AppError::Message(format!("获取模型列表失败: {e}")))?;

        // 去重保序：聚合站常有重复 id。
        let mut ids: Vec<String> = Vec::with_capacity(fetched.len());
        for model in fetched {
            if !ids.iter().any(|existing| existing == &model.id) {
                ids.push(model.id);
            }
        }

        let models = crate::services::model_catalog::build_available_models(ids).await;
        // 只有已存的那家才写回；新建时列表留在表单里，保存时随供应商一起提交。
        if let Some(mut provider) = stored {
            provider.available_models = models.clone();
            state.db.save_universal_provider(&provider)?;
        }
        Ok(models)
    }

    /// 删除统一供应商
    pub fn delete_universal(state: &AppState, id: &str) -> Result<bool, AppError> {
        // 获取统一供应商（用于删除生成的子供应商）
        let provider = state.db.get_universal_provider(id)?;

        // 删除统一供应商
        state.db.delete_universal_provider(id)?;

        // 删除生成的子供应商
        if let Some(p) = provider {
            if p.apps.claude {
                let claude_id = format!("universal-claude-{id}");
                let _ = state.db.delete_provider("claude", &claude_id);
            }
            if p.apps.codex {
                let codex_id = format!("universal-codex-{id}");
                let _ = state.db.delete_provider("codex", &codex_id);
            }
            if p.apps.gemini {
                let gemini_id = format!("universal-gemini-{id}");
                let _ = state.db.delete_provider("gemini", &gemini_id);
            }
            if p.apps.claude_desktop {
                let desktop_id = format!("universal-claude-desktop-{id}");
                let _ = state.db.delete_provider("claude-desktop", &desktop_id);
            }
            if p.apps.grokbuild {
                let grokbuild_id = format!("universal-grokbuild-{id}");
                let _ = state.db.delete_provider("grokbuild", &grokbuild_id);
            }
        }

        Ok(true)
    }

    /// 统一供应商在子卡 meta 上拥有的键：这些由 per-app 配置驱动，同步时以生成的值为准
    /// （用户清空某个选项后，子卡不能留着旧值）。其余键属于子卡自己（用量脚本、排序、
    /// 通用配置退出、端点列表等），必须原样保留。
    const UNIVERSAL_OWNED_META_KEYS: &'static [&'static str] = &[
        "apiFormat",
        "apiKeyField",
        "isFullUrl",
        "customUserAgent",
        "localProxyRequestOverrides",
        "codexChatReasoning",
        "promptCacheRouting",
        "impersonateClaudeCode",
        "maxOutputTokens",
        "claudeDesktopMode",
        "claudeDesktopModelRoutes",
    ];

    /// 把生成的子卡 meta 合进已有的那份。
    ///
    /// 统一供应商驱动的键以生成的值为准（用户清空某个选项后，子卡不能留着旧值）；其余
    /// 键属于子卡自己（用量脚本、排序、通用配置退出、端点列表等），已有值必须原样保留。
    /// 父级 meta 里的其他键只在子卡还没有时作为初值。
    fn merge_child_meta(
        existing: Option<ProviderMeta>,
        generated: Option<ProviderMeta>,
    ) -> Option<ProviderMeta> {
        let Some(existing) = existing else {
            return generated;
        };
        let mut merged = serde_json::to_value(&existing).unwrap_or_default();
        let generated = serde_json::to_value(generated.unwrap_or_default()).unwrap_or_default();
        if let (Some(base), Some(patch)) = (merged.as_object_mut(), generated.as_object()) {
            for (key, value) in patch {
                // 父级 meta 是默认值（统一供应商通常不带 meta）时不能抹掉子卡自己的设置。
                if value.is_null() {
                    continue;
                }
                if Self::UNIVERSAL_OWNED_META_KEYS.contains(&key.as_str()) {
                    base.insert(key.clone(), value.clone());
                } else {
                    base.entry(key.clone()).or_insert_with(|| value.clone());
                }
            }
        }
        serde_json::from_value(merged).ok()
    }

    /// 同步统一供应商到各应用
    pub fn sync_universal_to_apps(state: &AppState, id: &str) -> Result<bool, AppError> {
        let provider = state
            .db
            .get_universal_provider(id)?
            .ok_or_else(|| AppError::Message(format!("统一供应商 {id} 不存在")))?;
        // 子供应商经 save_provider 直接入库，不走保存校验，图标在这里把关
        icon::validate_icon(provider.icon.as_deref())?;

        // Keep DB and live projections in sync independently per application:
        // one broken config file must not prevent the other two apps from being
        // updated, but it must still be reported instead of returning success.
        let mut live_failures = Vec::new();

        // 同步到 Claude
        if let Some(mut claude_provider) = provider.to_claude_provider() {
            // 合并已有配置。署名与自动升级由统一供应商驱动（用户取消勾选后子卡不能
            // 留着旧值），已有行里的旧值要在合并时删掉；其余键保持「已有为底、
            // 生成为补」的合并。
            let generated = claude_provider.settings_config.clone();
            if let Some(existing) = state.db.get_provider_by_id(&claude_provider.id, "claude")? {
                let mut merged = existing.settings_config.clone();
                if generated.get("attribution").is_none() {
                    if let Some(obj) = merged.as_object_mut() {
                        obj.remove("attribution");
                    }
                }
                if generated["env"].get("DISABLE_AUTOUPDATER").is_none() {
                    if let Some(env) = merged.get_mut("env").and_then(|e| e.as_object_mut()) {
                        env.remove("DISABLE_AUTOUPDATER");
                    }
                }
                Self::merge_json(&mut merged, &generated);
                claude_provider.settings_config = merged;
                // 子卡自己的设置（用量脚本、排序等）保留，统一供应商拥有的键以生成的为准。
                claude_provider.meta = Self::merge_child_meta(existing.meta, claude_provider.meta);
                claude_provider.created_at = existing.created_at;
                claude_provider.sort_index = existing.sort_index;
            }
            state.db.save_provider("claude", &claude_provider)?;
            Self::project_universal_child_to_live(
                state,
                AppType::Claude,
                &claude_provider.id,
                &mut live_failures,
            );
        } else {
            // 如果禁用了 Claude，删除对应的子供应商
            let claude_id = format!("universal-claude-{id}");
            let _ = state.db.delete_provider("claude", &claude_id);
        }

        // 同步到 Codex
        if let Some(mut codex_provider) = provider.to_codex_provider() {
            // 合并已有配置
            if let Some(existing) = state.db.get_provider_by_id(&codex_provider.id, "codex")? {
                let mut merged = existing.settings_config.clone();
                Self::merge_json(&mut merged, &codex_provider.settings_config);
                codex_provider.settings_config = merged;
                // 子卡自己的设置（用量脚本、排序等）保留，统一供应商拥有的键以生成的为准。
                codex_provider.meta = Self::merge_child_meta(existing.meta, codex_provider.meta);
                codex_provider.created_at = existing.created_at;
                codex_provider.sort_index = existing.sort_index;
            }
            state.db.save_provider("codex", &codex_provider)?;
            Self::project_universal_child_to_live(
                state,
                AppType::Codex,
                &codex_provider.id,
                &mut live_failures,
            );
        } else {
            let codex_id = format!("universal-codex-{id}");
            let _ = state.db.delete_provider("codex", &codex_id);
        }

        // 同步到 Gemini
        if let Some(mut gemini_provider) = provider.to_gemini_provider() {
            // 合并已有配置
            if let Some(existing) = state.db.get_provider_by_id(&gemini_provider.id, "gemini")? {
                let mut merged = existing.settings_config.clone();
                Self::merge_json(&mut merged, &gemini_provider.settings_config);
                gemini_provider.settings_config = merged;
                // 子卡自己的设置（用量脚本、排序等）保留，统一供应商拥有的键以生成的为准。
                gemini_provider.meta = Self::merge_child_meta(existing.meta, gemini_provider.meta);
                gemini_provider.created_at = existing.created_at;
                gemini_provider.sort_index = existing.sort_index;
            }
            state.db.save_provider("gemini", &gemini_provider)?;
            Self::project_universal_child_to_live(
                state,
                AppType::Gemini,
                &gemini_provider.id,
                &mut live_failures,
            );
        } else {
            let gemini_id = format!("universal-gemini-{id}");
            let _ = state.db.delete_provider("gemini", &gemini_id);
        }

        // 同步到 Claude Desktop
        if let Some(mut desktop_provider) = provider.to_claude_desktop_provider() {
            if let Some(existing) = state
                .db
                .get_provider_by_id(&desktop_provider.id, "claude-desktop")?
            {
                let mut merged = existing.settings_config.clone();
                Self::merge_json(&mut merged, &desktop_provider.settings_config);
                desktop_provider.settings_config = merged;
                // 子卡自己的设置（用量脚本、排序等）保留，统一供应商拥有的键以生成的为准。
                desktop_provider.meta =
                    Self::merge_child_meta(existing.meta, desktop_provider.meta);
                desktop_provider.created_at = existing.created_at;
                desktop_provider.sort_index = existing.sort_index;
            }
            state
                .db
                .save_provider("claude-desktop", &desktop_provider)?;
            Self::project_universal_child_to_live(
                state,
                AppType::ClaudeDesktop,
                &desktop_provider.id,
                &mut live_failures,
            );
        } else {
            let desktop_id = format!("universal-claude-desktop-{id}");
            let _ = state.db.delete_provider("claude-desktop", &desktop_id);
        }

        // 同步到 Grok Build
        if let Some(mut grokbuild_provider) = provider.to_grokbuild_provider() {
            if let Some(existing) = state
                .db
                .get_provider_by_id(&grokbuild_provider.id, "grokbuild")?
            {
                let mut merged = existing.settings_config.clone();
                Self::merge_json(&mut merged, &grokbuild_provider.settings_config);
                grokbuild_provider.settings_config = merged;
                // 子卡自己的设置（用量脚本、排序等）保留，统一供应商拥有的键以生成的为准。
                grokbuild_provider.meta =
                    Self::merge_child_meta(existing.meta, grokbuild_provider.meta);
                grokbuild_provider.created_at = existing.created_at;
                grokbuild_provider.sort_index = existing.sort_index;
            }
            state.db.save_provider("grokbuild", &grokbuild_provider)?;
            Self::project_universal_child_to_live(
                state,
                AppType::GrokBuild,
                &grokbuild_provider.id,
                &mut live_failures,
            );
        } else {
            let grokbuild_id = format!("universal-grokbuild-{id}");
            let _ = state.db.delete_provider("grokbuild", &grokbuild_id);
        }

        if live_failures.is_empty() {
            Ok(true)
        } else {
            Err(AppError::Message(format!(
                "统一供应商已保存到数据库，但以下应用的配置文件未能写入，仍是旧内容：{}。请重试同步，或切换一次该应用的供应商。",
                live_failures.join("、")
            )))
        }
    }

    /// Re-project a generated universal child only when it is the effective
    /// current provider for that app. Failures are collected by the caller so
    /// the other applications can continue syncing.
    fn project_universal_child_to_live(
        state: &AppState,
        app_type: AppType,
        child_id: &str,
        failures: &mut Vec<String>,
    ) {
        // 正在用的那家（代理模式下是代理路由）才需要重投影。
        let is_current = match crate::mode::current::provider_for(
            &state.db,
            &app_type,
            crate::mode::current::Purpose::InUse,
        ) {
            Ok(current) => current.as_deref() == Some(child_id),
            Err(err) => {
                log::warn!(
                    "读取 {} 当前供应商失败，跳过统一供应商的 live 重投影: {err}",
                    app_type.as_str()
                );
                failures.push(app_type.as_str().to_string());
                return;
            }
        };
        if !is_current {
            return;
        }

        if let Err(err) = Self::sync_current_provider_for_app(state, app_type.clone()) {
            log::warn!(
                "统一供应商同步后重写 {} live 配置失败: {err}",
                app_type.as_str()
            );
            failures.push(app_type.as_str().to_string());
        }
    }

    /// 递归合并 JSON：base 为底，patch 覆盖同名字段
    fn merge_json(base: &mut serde_json::Value, patch: &serde_json::Value) {
        use serde_json::Value;

        match (base, patch) {
            (Value::Object(base_map), Value::Object(patch_map)) => {
                for (k, v_patch) in patch_map {
                    match base_map.get_mut(k) {
                        Some(v_base) => Self::merge_json(v_base, v_patch),
                        None => {
                            base_map.insert(k.clone(), v_patch.clone());
                        }
                    }
                }
            }
            // 其它类型：直接覆盖
            (base_val, patch_val) => {
                *base_val = patch_val.clone();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(any(target_os = "macos", windows, target_os = "linux"))]
    use crate::claude_desktop_config::PROFILE_ID;
    use crate::config::{get_claude_settings_path, read_json_file, write_json_file};
    use crate::database::Database;
    #[cfg(any(target_os = "macos", windows, target_os = "linux"))]
    use crate::provider::{ClaudeDesktopMode, ClaudeDesktopModelRoute};
    use crate::provider::{ClaudeModelConfig, ProviderMeta, UniversalProvider, UsageScript};
    use crate::proxy::types::ProxyConfig;
    use crate::store::AppState;
    use serde_json::json;
    use serial_test::serial;
    use std::env;
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::sync::{Arc, Mutex, OnceLock};
    use tempfile::TempDir;

    struct TempHome {
        #[allow(dead_code)]
        dir: TempDir,
        original_home: Option<String>,
        #[cfg(windows)]
        original_local_app_data: Option<String>,
        original_userprofile: Option<String>,
        original_test_home: Option<String>,
        #[cfg(target_os = "linux")]
        original_xdg_config_home: Option<std::ffi::OsString>,
    }

    impl TempHome {
        fn new() -> Self {
            let dir = TempDir::new().expect("failed to create temp home");
            let original_home = env::var("HOME").ok();
            #[cfg(windows)]
            let original_local_app_data = env::var("LOCALAPPDATA").ok();
            let original_userprofile = env::var("USERPROFILE").ok();
            let original_test_home = env::var("CC_SWITCH_TEST_HOME").ok();
            #[cfg(target_os = "linux")]
            let original_xdg_config_home = env::var_os("XDG_CONFIG_HOME");

            env::set_var("HOME", dir.path());
            #[cfg(windows)]
            env::set_var("LOCALAPPDATA", dir.path().join("AppData").join("Local"));
            env::set_var("USERPROFILE", dir.path());
            env::set_var("CC_SWITCH_TEST_HOME", dir.path());
            // Claude Desktop Linux paths follow XDG_CONFIG_HOME; pin them under the temp home.
            #[cfg(target_os = "linux")]
            env::remove_var("XDG_CONFIG_HOME");

            Self {
                dir,
                original_home,
                #[cfg(windows)]
                original_local_app_data,
                original_userprofile,
                original_test_home,
                #[cfg(target_os = "linux")]
                original_xdg_config_home,
            }
        }
    }

    impl Drop for TempHome {
        fn drop(&mut self) {
            match &self.original_home {
                Some(value) => env::set_var("HOME", value),
                None => env::remove_var("HOME"),
            }

            #[cfg(windows)]
            {
                match &self.original_local_app_data {
                    Some(value) => env::set_var("LOCALAPPDATA", value),
                    None => env::remove_var("LOCALAPPDATA"),
                }
            }

            match &self.original_userprofile {
                Some(value) => env::set_var("USERPROFILE", value),
                None => env::remove_var("USERPROFILE"),
            }

            match &self.original_test_home {
                Some(value) => env::set_var("CC_SWITCH_TEST_HOME", value),
                None => env::remove_var("CC_SWITCH_TEST_HOME"),
            }

            #[cfg(target_os = "linux")]
            {
                match &self.original_xdg_config_home {
                    Some(value) => env::set_var("XDG_CONFIG_HOME", value),
                    None => env::remove_var("XDG_CONFIG_HOME"),
                }
            }
        }
    }

    #[cfg(windows)]
    fn claude_desktop_profile_path(home: &Path) -> PathBuf {
        home.join("AppData")
            .join("Local")
            .join("Claude-3p")
            .join("configLibrary")
            .join(format!("{PROFILE_ID}.json"))
    }

    #[cfg(target_os = "macos")]
    fn claude_desktop_profile_path(home: &Path) -> PathBuf {
        home.join("Library")
            .join("Application Support")
            .join("Claude-3p")
            .join("configLibrary")
            .join(format!("{PROFILE_ID}.json"))
    }

    #[cfg(target_os = "linux")]
    fn claude_desktop_profile_path(home: &Path) -> PathBuf {
        home.join(".config")
            .join("Claude-3p")
            .join("configLibrary")
            .join(format!("{PROFILE_ID}.json"))
    }

    fn test_guard() -> std::sync::MutexGuard<'static, ()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
            .lock()
            .unwrap_or_else(|err| err.into_inner())
    }

    fn with_test_home<T>(test: impl FnOnce(&AppState, &Path) -> T) -> T {
        let _guard = test_guard();
        let temp = tempfile::tempdir().expect("tempdir");
        let old_test_home = std::env::var_os("CC_SWITCH_TEST_HOME");
        let old_home = std::env::var_os("HOME");
        std::env::set_var("CC_SWITCH_TEST_HOME", temp.path());
        std::env::set_var("HOME", temp.path());

        let db = Arc::new(Database::memory().expect("in-memory database"));
        let state = AppState::new(db);
        let result = test(&state, temp.path());

        match old_test_home {
            Some(value) => std::env::set_var("CC_SWITCH_TEST_HOME", value),
            None => std::env::remove_var("CC_SWITCH_TEST_HOME"),
        }
        match old_home {
            Some(value) => std::env::set_var("HOME", value),
            None => std::env::remove_var("HOME"),
        }

        result
    }

    fn codex_settings(base_url: &str, api_key: &str) -> Value {
        json!({
            "auth": {
                "OPENAI_API_KEY": api_key
            },
            "config": format!(
                "model_provider = \"custom\"\n\
                 [model_providers.custom]\n\
                 name = \"custom\"\n\
                 base_url = \"{base_url}\"\n\
                 wire_api = \"chat\"\n"
            )
        })
    }

    fn usage_script_with_credentials(
        api_key: Option<&str>,
        base_url: Option<&str>,
        template_type: Option<&str>,
    ) -> UsageScript {
        UsageScript {
            enabled: true,
            language: "javascript".to_string(),
            code: "return { remaining: 1, unit: 'USD' };".to_string(),
            timeout: Some(10),
            api_key: api_key.map(str::to_string),
            base_url: base_url.map(str::to_string),
            access_token: None,
            user_id: None,
            template_type: template_type.map(str::to_string),
            auto_query_interval: None,
            coding_plan_provider: None,
            access_key_id: Some("ak-test".to_string()),
            secret_access_key: Some("sk-test".to_string()),
            team_organization_id: None,
            team_project_id: None,
        }
    }

    fn codex_provider_with_usage(
        id: &str,
        base_url: &str,
        api_key: &str,
        usage_api_key: Option<&str>,
        usage_base_url: Option<&str>,
        template_type: Option<&str>,
    ) -> Provider {
        let mut provider = Provider::with_id(
            id.to_string(),
            format!("Provider {id}"),
            codex_settings(base_url, api_key),
            None,
        );
        provider.meta = Some(ProviderMeta {
            usage_script: Some(usage_script_with_credentials(
                usage_api_key,
                usage_base_url,
                template_type,
            )),
            ..Default::default()
        });
        provider
    }

    #[test]
    #[serial]
    fn add_clears_usage_credentials_that_match_provider_config() {
        with_test_home(|state, _| {
            let provider = codex_provider_with_usage(
                "codex-a",
                "https://api.a.example/v1/",
                "sk-a",
                Some(" sk-a "),
                Some(" https://api.a.example/v1/ "),
                None,
            );

            ProviderService::add(state, AppType::Codex, provider, false).expect("add provider");

            let saved = state
                .db
                .get_provider_by_id("codex-a", AppType::Codex.as_str())
                .expect("query saved provider")
                .expect("saved provider should exist");
            let script = saved
                .meta
                .as_ref()
                .and_then(|meta| meta.usage_script.as_ref())
                .expect("usage script should remain");

            assert_eq!(script.api_key, None);
            assert_eq!(script.base_url, None);
        });
    }

    #[test]
    #[serial]
    fn update_preserves_usage_credentials_that_only_match_previous_config() {
        with_test_home(|state, _| {
            let provider = codex_provider_with_usage(
                "codex-usage-old",
                "https://api.a.example/v1/",
                "sk-a",
                Some("sk-a"),
                Some("https://api.a.example/v1/"),
                None,
            );
            state
                .db
                .save_provider(AppType::Codex.as_str(), &provider)
                .expect("seed provider with explicit usage credentials");

            let mut updated = provider.clone();
            updated.settings_config = codex_settings("https://api.b.example/v1/", "sk-b");

            ProviderService::update(state, AppType::Codex, None, updated)
                .expect("update provider main credentials");

            let saved = state
                .db
                .get_provider_by_id("codex-usage-old", AppType::Codex.as_str())
                .expect("query updated provider")
                .expect("updated provider should exist");
            let script = saved
                .meta
                .as_ref()
                .and_then(|meta| meta.usage_script.as_ref())
                .expect("usage script should remain");

            assert_eq!(script.api_key.as_deref(), Some("sk-a"));
            assert_eq!(
                script.base_url.as_deref(),
                Some("https://api.a.example/v1/")
            );
            assert_eq!(
                saved.resolve_usage_credentials(&AppType::Codex),
                ("https://api.b.example/v1".to_string(), "sk-b".to_string())
            );
        });
    }

    #[test]
    #[serial]
    fn copied_provider_uses_edited_credentials_after_add_clears_mirrored_usage_credentials() {
        with_test_home(|state, _| {
            let copied_provider = codex_provider_with_usage(
                "codex-copy",
                "https://api.a.example/v1/",
                "sk-a",
                Some("sk-a"),
                Some("https://api.a.example/v1/"),
                None,
            );

            ProviderService::add(state, AppType::Codex, copied_provider, false)
                .expect("add copied provider");

            let saved_after_add = state
                .db
                .get_provider_by_id("codex-copy", AppType::Codex.as_str())
                .expect("query copied provider")
                .expect("copied provider should exist");
            let script_after_add = saved_after_add
                .meta
                .as_ref()
                .and_then(|meta| meta.usage_script.as_ref())
                .expect("usage script should remain");
            assert_eq!(script_after_add.api_key, None);
            assert_eq!(script_after_add.base_url, None);

            let mut edited_provider = saved_after_add.clone();
            edited_provider.settings_config = codex_settings("https://api.b.example/v1/", "sk-b");

            ProviderService::update(state, AppType::Codex, None, edited_provider)
                .expect("edit copied provider credentials");

            let saved_after_update = state
                .db
                .get_provider_by_id("codex-copy", AppType::Codex.as_str())
                .expect("query edited provider")
                .expect("edited provider should exist");
            let script_after_update = saved_after_update
                .meta
                .as_ref()
                .and_then(|meta| meta.usage_script.as_ref())
                .expect("usage script should remain");

            assert_eq!(script_after_update.api_key, None);
            assert_eq!(script_after_update.base_url, None);
            assert_eq!(
                saved_after_update.resolve_usage_credentials(&AppType::Codex),
                ("https://api.b.example/v1".to_string(), "sk-b".to_string())
            );
        });
    }

    #[test]
    #[serial]
    fn update_clears_usage_credentials_that_match_current_config() {
        with_test_home(|state, _| {
            let provider = codex_provider_with_usage(
                "codex-current",
                "https://api.a.example/v1",
                "sk-a",
                Some("sk-usage"),
                Some("https://usage.example/api"),
                None,
            );
            state
                .db
                .save_provider(AppType::Codex.as_str(), &provider)
                .expect("seed provider with distinct usage credentials");

            let mut updated = provider.clone();
            updated.settings_config = codex_settings("https://api.b.example/v1/", "sk-b");
            updated.meta = Some(ProviderMeta {
                usage_script: Some(usage_script_with_credentials(
                    Some(" sk-b "),
                    Some(" https://api.b.example/v1/ "),
                    None,
                )),
                ..Default::default()
            });

            ProviderService::update(state, AppType::Codex, None, updated)
                .expect("update provider with redundant usage credentials");

            let saved = state
                .db
                .get_provider_by_id("codex-current", AppType::Codex.as_str())
                .expect("query updated provider")
                .expect("updated provider should exist");
            let script = saved
                .meta
                .as_ref()
                .and_then(|meta| meta.usage_script.as_ref())
                .expect("usage script should remain");

            assert_eq!(script.api_key, None);
            assert_eq!(script.base_url, None);
        });
    }

    #[test]
    #[serial]
    fn add_preserves_distinct_usage_credentials() {
        with_test_home(|state, _| {
            let provider = codex_provider_with_usage(
                "codex-distinct",
                "https://api.main.example/v1",
                "sk-main",
                Some("sk-usage"),
                Some("https://usage.example/api"),
                None,
            );

            ProviderService::add(state, AppType::Codex, provider, false).expect("add provider");

            let saved = state
                .db
                .get_provider_by_id("codex-distinct", AppType::Codex.as_str())
                .expect("query saved provider")
                .expect("saved provider should exist");
            let script = saved
                .meta
                .as_ref()
                .and_then(|meta| meta.usage_script.as_ref())
                .expect("usage script should remain");

            assert_eq!(script.api_key.as_deref(), Some("sk-usage"));
            assert_eq!(
                script.base_url.as_deref(),
                Some("https://usage.example/api")
            );
        });
    }

    #[test]
    #[serial]
    fn add_does_not_clear_token_plan_credentials() {
        with_test_home(|state, _| {
            let provider = codex_provider_with_usage(
                "codex-token-plan",
                "https://api.plan.example/v1",
                "sk-plan",
                Some("sk-plan"),
                Some("https://api.plan.example/v1"),
                Some("token_plan"),
            );

            ProviderService::add(state, AppType::Codex, provider, false).expect("add provider");

            let saved = state
                .db
                .get_provider_by_id("codex-token-plan", AppType::Codex.as_str())
                .expect("query saved provider")
                .expect("saved provider should exist");
            let script = saved
                .meta
                .as_ref()
                .and_then(|meta| meta.usage_script.as_ref())
                .expect("usage script should remain");

            assert_eq!(script.api_key.as_deref(), Some("sk-plan"));
            assert_eq!(
                script.base_url.as_deref(),
                Some("https://api.plan.example/v1")
            );
            assert_eq!(script.access_key_id.as_deref(), Some("ak-test"));
            assert_eq!(script.secret_access_key.as_deref(), Some("sk-test"));
        });
    }

    #[test]
    fn validate_provider_settings_rejects_missing_auth() {
        let provider = Provider::with_id(
            "codex".into(),
            "Codex".into(),
            json!({ "config": "base_url = \"https://example.com\"" }),
            None,
        );
        let err = ProviderService::validate_provider_settings(&AppType::Codex, &provider)
            .expect_err("missing auth should be rejected");
        assert!(
            err.to_string().contains("auth"),
            "expected auth error, got {err:?}"
        );
    }

    #[test]
    #[serial]
    fn add_accepts_multiple_unbound_codex_official_cards() {
        with_test_home(|state, _| {
            crate::settings::reload_settings().expect("reload settings");
            state
                .db
                .init_default_official_providers()
                .expect("seed official providers");
            let fixed_id = crate::database::CODEX_OFFICIAL_PROVIDER_ID;
            state
                .db
                .set_current_provider(AppType::Codex.as_str(), fixed_id)
                .expect("set database current");
            crate::settings::set_current_provider(&AppType::Codex, Some(fixed_id))
                .expect("set local current");

            for id in ["follow-login-a", "follow-login-b"] {
                let mut provider = Provider::with_id(
                    id.to_string(),
                    id.to_string(),
                    json!({ "auth": {}, "config": "" }),
                    None,
                );
                provider.category = Some("official".to_string());
                ProviderService::add(state, AppType::Codex, provider, false)
                    .expect("add unbound Official card");
            }

            let providers = state
                .db
                .get_all_providers(AppType::Codex.as_str())
                .expect("read providers");
            assert!(providers.contains_key("follow-login-a"));
            assert!(providers.contains_key("follow-login-b"));
        });
    }

    #[test]
    fn extract_gemini_common_config_strips_credentials_keeps_shareable() {
        // Gemini 的共享片段会被 deep-merge 回**其它** Gemini 供应商的 env
        // (旧版切换时合并片段；新版片段冻结只给旧版读)，因此任何凭据都不得进入片段。
        // 之前这里只硬编码跳过 GEMINI_API_KEY/GOOGLE_GEMINI_BASE_URL，而
        // GOOGLE_API_KEY 是 provider.rs 认可的一等 Gemini 凭据 → 会泄露到别的供应商。
        let settings = json!({
            "env": {
                "GEMINI_API_KEY": "g-gem",
                "GOOGLE_API_KEY": "g-legacy-real-key",
                "GOOGLE_GEMINI_BASE_URL": "https://gemini.example",
                "GOOGLE_APPLICATION_CREDENTIALS": "/path/creds.json",
                "SOME_PROXY_AUTH_TOKEN": "tok-proxy",
                // 关键字段归供应商：片段已冻结，收进去会从行里剥掉、再也写不回 live
                "GEMINI_MODEL": "gemini-2.5-pro",
                "GOOGLE_GENAI_USE_VERTEXAI": "true",
                // 可共享的非机密配置必须保留
                "GEMINI_TIMEOUT_MS": "30000"
            }
        });

        let snippet =
            ProviderService::extract_gemini_common_config(&settings).expect("extract should work");
        let value: Value = serde_json::from_str(&snippet).expect("snippet is valid JSON");

        for leaked in [
            "GEMINI_API_KEY",
            "GOOGLE_API_KEY",
            "GOOGLE_APPLICATION_CREDENTIALS",
            "SOME_PROXY_AUTH_TOKEN",
            "GEMINI_MODEL",
            "GOOGLE_GENAI_USE_VERTEXAI",
        ] {
            assert!(
                value.get(leaked).is_none(),
                "{leaked} must not go into the shared Gemini snippet"
            );
        }
        assert_eq!(
            value.get("GEMINI_TIMEOUT_MS").and_then(|v| v.as_str()),
            Some("30000"),
            "shareable non-secret config must be preserved"
        );
    }

    /// 造一个「已被污染」的现场：片段里带 A 账号的凭据 + 一个合法可共享键。
    #[test]
    fn sensitive_key_matcher_covers_common_credential_namings() {
        for key in [
            // 裸 `_KEY`：最常见的写法，却曾被"只枚举 `_API_KEY` 这些子类"漏在外面
            "OPENAI_KEY",
            "GROQ_KEY",
            "XAI_KEY",
            // 不带分隔符的复合写法
            "VOLC_ACCESSKEY",
            "ALIYUN_SECRETKEY",
            "SOME_APITOKEN",
            // personal access token：既不含 TOKEN 也不含 KEY
            "GITHUB_PAT",
            "gitlab_pat",
            // 口令类缩写
            "MYSQL_PWD",
            "DB_PASS",
            "GPG_PASSPHRASE",
            "AWS_CREDS",
            // 发往上游的自定义请求头、Cookie、Authorization
            "ANTHROPIC_CUSTOM_HEADERS",
            "GEMINI_CLI_CUSTOM_HEADERS",
            "headers",
            "UPSTREAM_COOKIE",
            "PROXY_AUTHORIZATION",
        ] {
            assert!(
                ProviderService::is_sensitive_config_key(key),
                "{key} must be treated as a credential"
            );
        }

        // 后缀必须带下划线，不能把正常配置一起卷进来
        for key in [
            "PATH",
            "OLDPWD",
            "GEMINI_COMPAT",
            "SSL_BYPASS",
            "GEMINI_TIMEOUT_MS",
            "CLAUDE_CODE_MAX_OUTPUT_TOKENS",
            // 名字带 HEADER 但不是发往上游的凭据：普通开关、用户自己的遥测端点
            "CLAUDE_CODE_ATTRIBUTION_HEADER",
            "OTEL_EXPORTER_OTLP_HEADERS",
        ] {
            assert!(
                !ProviderService::is_sensitive_config_key(key),
                "{key} is ordinary shareable config and must not be stripped"
            );
        }
    }

    /// 测试夹具：按键排序写一份 `~/.gemini/.env`。
    fn write_gemini_env(map: &HashMap<String, String>) -> Result<(), AppError> {
        let mut keys: Vec<&String> = map.keys().collect();
        keys.sort();
        let text = keys
            .into_iter()
            .map(|key| format!("{key}={}", map[key]))
            .collect::<Vec<_>>()
            .join("\n");
        crate::gemini_config::write_gemini_env_text_atomic(&text)
    }

    fn seed_leaked_gemini_state(db: &Arc<Database>) {
        db.set_config_snippet(
            "gemini",
            Some(
                json!({
                    "GOOGLE_API_KEY": "key-A-leaked",
                    "SOME_PROXY_AUTH_TOKEN": "tok-A-leaked",
                    "GEMINI_TIMEOUT_MS": "30000"
                })
                .to_string(),
            ),
        )
        .expect("seed snippet");

        // 受害者 B：泄漏的密钥已经被合并进它的 env
        let victim = Provider::with_id(
            "b".into(),
            "Relay B".into(),
            json!({ "env": {
                "GOOGLE_GEMINI_BASE_URL": "https://relay-b.example",
                "GOOGLE_API_KEY": "key-A-leaked",
                "GEMINI_TIMEOUT_MS": "30000"
            }}),
            None,
        );
        db.save_provider("gemini", &victim).expect("save victim");

        // 供应商 C：自己写了同名键但值不同，不能被误删
        let unrelated = Provider::with_id(
            "c".into(),
            "Own Key C".into(),
            json!({ "env": {
                "GOOGLE_GEMINI_BASE_URL": "https://c.example",
                "GOOGLE_API_KEY": "key-C-owned"
            }}),
            None,
        );
        db.save_provider("gemini", &unrelated).expect("save c");
    }

    /// Saving the active provider while takeover has never been enabled must
    /// rewrite the real live file immediately.
    #[tokio::test]
    #[serial]
    async fn update_current_claude_provider_writes_live_when_proxy_never_enabled() {
        let _home = TempHome::new();
        crate::settings::reload_settings().expect("reload settings");

        let db = Arc::new(Database::memory().expect("init db"));
        let state = AppState::new(db.clone());
        let original = Provider::with_id(
            "p1".into(),
            "Claude A".into(),
            json!({
                "env": {
                    "ANTHROPIC_AUTH_TOKEN": "token-a",
                    "ANTHROPIC_BASE_URL": "https://api.old.example"
                }
            }),
            None,
        );
        db.save_provider("claude", &original)
            .expect("save provider");
        db.set_current_provider("claude", "p1")
            .expect("set current provider");
        crate::settings::set_current_provider(&AppType::Claude, Some("p1"))
            .expect("set local current provider");
        write_live_for_state(&state, &AppType::Claude, &original).expect("seed live file");

        let mut updated = original.clone();
        updated.settings_config["env"]["ANTHROPIC_BASE_URL"] =
            Value::String("https://api.new.example".into());
        ProviderService::update(&state, AppType::Claude, None, updated)
            .expect("update current provider");

        let live: Value = read_json_file(&get_claude_settings_path()).expect("read live");
        assert_eq!(
            live["env"]["ANTHROPIC_BASE_URL"].as_str(),
            Some("https://api.new.example")
        );
    }

    /// 编辑当前供应商、去掉它的独有字段：live 里 CC Switch 写进去的那个值随之删掉，
    /// 用户自己的键不动。
    #[tokio::test]
    #[serial]
    async fn update_current_claude_provider_drops_the_compat_switch_it_no_longer_has() {
        let _home = TempHome::new();
        crate::settings::reload_settings().expect("reload settings");

        let db = Arc::new(Database::memory().expect("init db"));
        let state = AppState::new(db.clone());
        let original = Provider::with_id(
            "ds".into(),
            "DeepSeek".into(),
            json!({ "env": {
                "ANTHROPIC_BASE_URL": "https://api.deepseek.example/anthropic",
                "CLAUDE_CODE_DISABLE_ARTIFACT": "1"
            }}),
            None,
        );
        db.save_provider("claude", &original)
            .expect("save provider");
        ProviderService::switch(&state, AppType::Claude, "ds").expect("switch");
        let mut live: Value = read_json_file(&get_claude_settings_path()).expect("read live");
        live["env"]["DEBUG"] = json!("1");
        write_json_file(&get_claude_settings_path(), &live).expect("user edit");

        let mut updated = original.clone();
        updated.settings_config["env"]
            .as_object_mut()
            .expect("env")
            .remove("CLAUDE_CODE_DISABLE_ARTIFACT");
        ProviderService::update(&state, AppType::Claude, None, updated).expect("update");

        let live: Value = read_json_file(&get_claude_settings_path()).expect("read live");
        assert_eq!(
            live,
            json!({ "env": {
                "ANTHROPIC_BASE_URL": "https://api.deepseek.example/anthropic",
                "DEBUG": "1"
            }})
        );
    }

    /// 文件已经写成目标供应商、指针还没改时崩溃：下次启动按 pending 补完指针，
    /// 文件和指针重新一致。
    #[tokio::test]
    #[serial]
    async fn claude_switch_interrupted_before_the_pointer_rolls_forward() {
        use crate::mode::operation::failpoint;

        let _home = TempHome::new();
        crate::settings::reload_settings().expect("reload settings");

        let db = Arc::new(Database::memory().expect("init db"));
        let state = AppState::new(db.clone());
        for (id, url) in [("a", "https://a.example"), ("b", "https://b.example")] {
            let provider = Provider::with_id(
                id.into(),
                id.into(),
                json!({ "env": { "ANTHROPIC_BASE_URL": url } }),
                None,
            );
            db.save_provider("claude", &provider)
                .expect("save provider");
        }
        ProviderService::switch(&state, AppType::Claude, "a").expect("switch to a");

        failpoint::crash_at(Some("published:0"));
        let result = ProviderService::switch(&state, AppType::Claude, "b");
        failpoint::crash_at(None);
        assert!(result.is_err(), "the injected crash surfaces");

        let live: Value = read_json_file(&get_claude_settings_path()).expect("read live");
        assert_eq!(
            live["env"]["ANTHROPIC_BASE_URL"],
            json!("https://b.example")
        );
        assert_eq!(
            db.get_current_provider("claude")
                .expect("current")
                .as_deref(),
            Some("a"),
            "the pointer has not moved yet"
        );

        crate::mode::operation::recover_on_startup(&db);
        assert_eq!(
            db.get_current_provider("claude")
                .expect("current")
                .as_deref(),
            Some("b")
        );
        assert_eq!(
            crate::settings::get_current_provider(&AppType::Claude).as_deref(),
            Some("b")
        );
    }

    /// A stale backup row must be refreshed but must not divert the live write.
    #[tokio::test]
    #[serial]
    async fn update_current_claude_provider_writes_live_when_backup_row_is_stale() {
        let _home = TempHome::new();
        crate::settings::reload_settings().expect("reload settings");

        let db = Arc::new(Database::memory().expect("init db"));
        let state = AppState::new(db.clone());
        let original = Provider::with_id(
            "p1".into(),
            "Claude A".into(),
            json!({
                "env": {
                    "ANTHROPIC_AUTH_TOKEN": "token-a",
                    "ANTHROPIC_BASE_URL": "https://api.old.example"
                }
            }),
            None,
        );
        db.save_provider("claude", &original)
            .expect("save provider");
        db.set_current_provider("claude", "p1")
            .expect("set current provider");
        crate::settings::set_current_provider(&AppType::Claude, Some("p1"))
            .expect("set local current provider");
        write_live_for_state(&state, &AppType::Claude, &original).expect("seed live file");
        db.save_live_backup(
            "claude",
            &serde_json::to_string(&original.settings_config).expect("serialize backup"),
        )
        .await
        .expect("seed stale backup");
        assert!(!state.proxy_service.is_running().await);

        let mut updated = original.clone();
        updated.settings_config["env"]["ANTHROPIC_BASE_URL"] =
            Value::String("https://api.new.example".into());
        ProviderService::update(&state, AppType::Claude, None, updated)
            .expect("update current provider");

        let live: Value = read_json_file(&get_claude_settings_path()).expect("read live");
        assert_eq!(
            live["env"]["ANTHROPIC_BASE_URL"].as_str(),
            Some("https://api.new.example"),
            "a leftover backup row is not proxy mode: the edit goes to live"
        );
    }

    /// An enabled flag left behind by an interrupted teardown is not enough to
    /// suppress a live write when neither placeholder nor backup evidence exists.
    #[tokio::test]
    #[serial]
    async fn update_current_claude_provider_ignores_enabled_flag_without_evidence() {
        let _home = TempHome::new();
        crate::settings::reload_settings().expect("reload settings");

        let db = Arc::new(Database::memory().expect("init db"));
        let state = AppState::new(db.clone());
        let original = Provider::with_id(
            "p1".into(),
            "Claude A".into(),
            json!({
                "env": {
                    "ANTHROPIC_AUTH_TOKEN": "token-a",
                    "ANTHROPIC_BASE_URL": "https://api.old.example"
                }
            }),
            None,
        );
        db.save_provider("claude", &original)
            .expect("save provider");
        db.set_current_provider("claude", "p1")
            .expect("set current provider");
        crate::settings::set_current_provider(&AppType::Claude, Some("p1"))
            .expect("set local current provider");
        write_live_for_state(&state, &AppType::Claude, &original).expect("seed live file");
        let mut config = db
            .get_proxy_config_for_app("claude")
            .await
            .expect("read proxy config");
        config.enabled = true;
        db.update_proxy_config_for_app(config)
            .await
            .expect("leave enabled flag set");
        assert!(!state.proxy_service.is_running().await);

        let mut updated = original.clone();
        updated.settings_config["env"]["ANTHROPIC_BASE_URL"] =
            Value::String("https://api.new.example".into());
        ProviderService::update(&state, AppType::Claude, None, updated)
            .expect("update current provider");

        let live: Value = read_json_file(&get_claude_settings_path()).expect("read live");
        assert_eq!(
            live["env"]["ANTHROPIC_BASE_URL"].as_str(),
            Some("https://api.new.example")
        );
    }

    #[tokio::test]
    #[serial]
    async fn scrub_gemini_removes_leaked_credentials_from_snippet_and_providers() {
        let _home = TempHome::new();
        crate::settings::reload_settings().expect("reload settings");
        let db = Arc::new(Database::memory().expect("init db"));
        let state = AppState::new(db.clone());
        seed_leaked_gemini_state(&db);

        ProviderService::scrub_leaked_gemini_common_config(&state)
            .await
            .expect("scrub must succeed");

        // 片段：凭据清掉，可共享配置保留
        let snippet = db
            .get_config_snippet("gemini")
            .expect("read snippet")
            .expect("snippet must still exist");
        let snippet: Value = serde_json::from_str(&snippet).expect("valid json");
        assert!(snippet.get("GOOGLE_API_KEY").is_none());
        assert!(snippet.get("SOME_PROXY_AUTH_TOKEN").is_none());
        assert_eq!(
            snippet.get("GEMINI_TIMEOUT_MS").and_then(Value::as_str),
            Some("30000"),
            "shareable config must survive the scrub"
        );

        // 受害者 B：扩散过去的那一份被清掉
        let providers = db.get_all_providers("gemini").expect("providers");
        let victim_env = &providers["b"].settings_config["env"];
        assert!(
            victim_env.get("GOOGLE_API_KEY").is_none(),
            "leaked key must be removed from the victim provider"
        );
        assert_eq!(
            victim_env.get("GEMINI_TIMEOUT_MS").and_then(Value::as_str),
            Some("30000"),
            "non-credential config must not be touched"
        );
    }

    #[tokio::test]
    #[serial]
    async fn scrub_gemini_keeps_a_providers_own_differently_valued_key() {
        let _home = TempHome::new();
        crate::settings::reload_settings().expect("reload settings");
        let db = Arc::new(Database::memory().expect("init db"));
        let state = AppState::new(db.clone());
        seed_leaked_gemini_state(&db);

        ProviderService::scrub_leaked_gemini_common_config(&state)
            .await
            .expect("scrub must succeed");

        // 这条最容易写错成「按键名一刀切」：C 自己的密钥值与片段不同，是它自己的凭据
        let providers = db.get_all_providers("gemini").expect("providers");
        assert_eq!(
            providers["c"].settings_config["env"]
                .get("GOOGLE_API_KEY")
                .and_then(Value::as_str),
            Some("key-C-owned"),
            "a provider's own key must not be deleted by name matching"
        );
    }

    #[tokio::test]
    #[serial]
    async fn scrub_gemini_audit_records_key_names_but_never_values() {
        let _home = TempHome::new();
        crate::settings::reload_settings().expect("reload settings");
        let db = Arc::new(Database::memory().expect("init db"));
        let state = AppState::new(db.clone());
        seed_leaked_gemini_state(&db);

        ProviderService::scrub_leaked_gemini_common_config(&state)
            .await
            .expect("scrub must succeed");

        let audit_text = db
            .get_setting("gemini_common_config_scrub_audit_v1")
            .expect("read audit")
            .expect("an audit record must exist so the deletion is not silent");

        // 值绝不能进这条记录：`settings` 会随 WebDAV/S3 同步上传，留值等于把一次
        // 清除换成一份跨设备扩散、没有界面入口、永不过期的明文副本。
        assert!(
            !audit_text.contains("key-A-leaked") && !audit_text.contains("tok-A-leaked"),
            "the audit record must never carry credential values: {audit_text}"
        );

        // 但必须说清楚删了什么、从哪删的，否则用户只能靠翻日志
        let audit: Value = serde_json::from_str(&audit_text).expect("audit is JSON");
        let removed: Vec<&str> = audit["removedFromSnippet"]
            .as_array()
            .expect("removedFromSnippet array")
            .iter()
            .filter_map(Value::as_str)
            .collect();
        assert!(
            removed.contains(&"GOOGLE_API_KEY") && removed.contains(&"SOME_PROXY_AUTH_TOKEN"),
            "every key removed from the snippet must be named: {audit}"
        );
        let victim = audit["providers"]
            .as_array()
            .expect("providers array")
            .iter()
            .find(|entry| entry["id"] == json!("b"))
            .expect("every provider whose config gets rewritten must be recorded");
        assert_eq!(
            victim["removedKeys"],
            json!(["GOOGLE_API_KEY"]),
            "the record must name what was taken from each provider: {audit}"
        );
    }

    #[tokio::test]
    #[serial]
    async fn scrub_gemini_never_overwrites_an_existing_audit_record() {
        let _home = TempHome::new();
        crate::settings::reload_settings().expect("reload settings");
        let db = Arc::new(Database::memory().expect("init db"));
        let state = AppState::new(db.clone());
        seed_leaked_gemini_state(&db);

        // 上一轮改到一半就中止的情形：完成标记没置位，下次启动会重跑，但那时
        // 读到的"原始状态"已经残缺。无条件覆盖会拿残缺记录盖掉第一轮那份完整的。
        db.set_setting(
            "gemini_common_config_scrub_audit_v1",
            "{\"from\":\"an earlier, complete run\"}",
        )
        .expect("seed an existing audit record");

        ProviderService::scrub_leaked_gemini_common_config(&state)
            .await
            .expect("scrub must succeed");

        assert_eq!(
            db.get_setting("gemini_common_config_scrub_audit_v1")
                .expect("read audit")
                .as_deref(),
            Some("{\"from\":\"an earlier, complete run\"}"),
            "an audit record from an earlier run must survive a retry"
        );
    }

    #[tokio::test]
    #[serial]
    async fn scrub_gemini_cleans_the_live_env_without_a_current_provider() {
        let _home = TempHome::new();
        crate::settings::reload_settings().expect("reload settings");
        let db = Arc::new(Database::memory().expect("init db"));
        let state = AppState::new(db.clone());
        seed_leaked_gemini_state(&db);

        // 没有当前供应商——这正是 sync_current_provider_for_app 直接返回 Ok 而
        // 根本不写文件的分支。此时 live 若清不掉，片段又已被清空，下次切换的
        // backfill 就会把残留永久写进受害供应商的配置。
        write_gemini_env(&HashMap::from([
            ("GOOGLE_API_KEY".to_string(), "key-A-leaked".to_string()),
            ("GEMINI_TIMEOUT_MS".to_string(), "30000".to_string()),
            // 只存在于 live 的手工修改：定向删除必须保住它，全量重投影会抹掉
            (
                "HTTPS_PROXY".to_string(),
                "http://127.0.0.1:7890".to_string(),
            ),
        ]))
        .expect("seed live env");

        ProviderService::scrub_leaked_gemini_common_config(&state)
            .await
            .expect("scrub must succeed");

        let live = crate::gemini_config::read_gemini_env().expect("read live env");
        assert!(
            !live.contains_key("GOOGLE_API_KEY"),
            "the leaked credential must be gone from ~/.gemini/.env: {live:?}"
        );
        assert_eq!(
            live.get("HTTPS_PROXY").map(String::as_str),
            Some("http://127.0.0.1:7890"),
            "a hand-added live-only var must survive targeted removal: {live:?}"
        );
    }

    #[tokio::test]
    #[serial]
    async fn scrub_gemini_live_cleanup_preserves_the_rest_of_the_env_file() {
        let _home = TempHome::new();
        crate::settings::reload_settings().expect("reload settings");
        let db = Arc::new(Database::memory().expect("init db"));
        let state = AppState::new(db.clone());
        seed_leaked_gemini_state(&db);

        // 这是一次用户没主动触发的启动期清理，不该顺手重写与泄漏无关的内容。
        // read→HashMap→write 的往返会把注释、空行、无法识别的行全丢掉并按键名重排。
        let original = "\
# my own notes
GOOGLE_API_KEY=key-C-owned

GOOGLE_API_KEY=key-A-leaked
this line is not KEY=VALUE at all
GEMINI_TIMEOUT_MS=30000
";
        crate::gemini_config::write_gemini_env_text_atomic(original).expect("seed live env");

        ProviderService::scrub_leaked_gemini_common_config(&state)
            .await
            .expect("scrub must succeed");

        let raw = std::fs::read_to_string(crate::gemini_config::get_gemini_env_path())
            .expect("read live env");
        assert!(
            !raw.contains("key-A-leaked"),
            "the leaked line must be gone: {raw:?}"
        );
        assert!(
            raw.contains("# my own notes"),
            "comments must survive a targeted removal: {raw:?}"
        );
        assert!(
            raw.contains("this line is not KEY=VALUE at all"),
            "unparseable lines must survive a targeted removal: {raw:?}"
        );
        // 被泄漏值遮住的那条重新生效——正是想要的结果，遮住它的恰恰是泄漏值
        assert_eq!(
            crate::gemini_config::read_gemini_env()
                .expect("read live env")
                .get("GOOGLE_API_KEY")
                .map(String::as_str),
            Some("key-C-owned"),
            "only the matching line may be dropped: {raw:?}"
        );
    }

    #[tokio::test]
    #[serial]
    async fn scrub_gemini_aborts_before_clearing_the_snippet_when_the_live_backup_fails() {
        let _home = TempHome::new();
        crate::settings::reload_settings().expect("reload settings");
        let db = Arc::new(Database::memory().expect("init db"));
        let state = AppState::new(db.clone());
        seed_leaked_gemini_state(&db);

        // 关代理时这份快照会被原样写回 live。若清不动它却照样清了片段、置了完成标记，
        // 代理一停凭据就复活，而一次性标记保证不会再清第二次。
        db.save_live_backup("gemini", "}not json{")
            .await
            .expect("seed backup");

        let result = ProviderService::scrub_leaked_gemini_common_config(&state).await;
        assert!(
            result.is_err(),
            "a backup that cannot be cleaned must abort the scrub"
        );

        // 片段是「该剥哪些键」的唯一知识来源，中止后必须原样留着，否则下次重试
        // 会因为 poison 为空而直接短路，反倒把标记置上
        let snippet = db
            .get_config_snippet("gemini")
            .expect("read snippet")
            .expect("snippet must still exist");
        assert!(
            snippet.contains("key-A-leaked"),
            "the snippet must be left intact so the next boot can retry: {snippet}"
        );
        assert!(
            db.get_setting("gemini_common_config_credentials_scrubbed_v1")
                .expect("read flag")
                .is_none(),
            "the one-shot flag must not be set when the scrub aborted"
        );
    }

    #[tokio::test]
    #[serial]
    async fn scrub_gemini_leaves_no_residue_for_backfill_to_persist() {
        let _home = TempHome::new();
        crate::settings::reload_settings().expect("reload settings");
        let db = Arc::new(Database::memory().expect("init db"));
        let state = AppState::new(db.clone());
        seed_leaked_gemini_state(&db);

        ProviderService::scrub_leaked_gemini_common_config(&state)
            .await
            .expect("scrub must succeed");

        // 顺序陷阱回归：如果只清了片段，切走供应商时 remove_common_config_from_settings
        // 就不再认识这个键，live 里的残留会被 backfill 永久写进供应商配置。
        // 清理必须是原子的——清完之后，任何地方都不该再有那个值。
        let snippet = db
            .get_config_snippet("gemini")
            .expect("read snippet")
            .unwrap_or_default();
        assert!(!snippet.contains("key-A-leaked"));

        for (id, provider) in db.get_all_providers("gemini").expect("providers") {
            assert!(
                !provider
                    .settings_config
                    .to_string()
                    .contains("key-A-leaked"),
                "provider '{id}' still carries the leaked value"
            );
        }
    }

    #[tokio::test]
    #[serial]
    async fn scrub_gemini_is_idempotent_and_skips_on_second_run() {
        let _home = TempHome::new();
        crate::settings::reload_settings().expect("reload settings");
        let db = Arc::new(Database::memory().expect("init db"));
        let state = AppState::new(db.clone());
        seed_leaked_gemini_state(&db);

        ProviderService::scrub_leaked_gemini_common_config(&state)
            .await
            .expect("first run");

        // 第二次必须是 no-op：用户清理后重新填的凭据不能被再抹一遍
        db.set_config_snippet(
            "gemini",
            Some(json!({"GOOGLE_API_KEY": "restored"}).to_string()),
        )
        .expect("user re-adds a value");

        ProviderService::scrub_leaked_gemini_common_config(&state)
            .await
            .expect("second run");

        let snippet = db
            .get_config_snippet("gemini")
            .expect("read snippet")
            .expect("snippet exists");
        assert!(
            snippet.contains("restored"),
            "the one-shot flag must prevent a second scrub: {snippet}"
        );
    }

    #[test]
    fn extract_claude_common_config_strips_all_credentials_keeps_shareable() {
        // env 混入多种凭据（Anthropic/OpenRouter/Google/OpenAI/Gemini + AWS/Vertex）
        // 与可共享配置；顶层混入非标准的 apiKey/api_key 凭据与正常设置。
        let settings = json!({
            "env": {
                "ANTHROPIC_API_KEY": "sk-ant",
                "ANTHROPIC_AUTH_TOKEN": "tok-ant",
                "OPENROUTER_API_KEY": "sk-or",
                "GOOGLE_API_KEY": "g-key",
                "OPENAI_API_KEY": "sk-oai",
                "GEMINI_API_KEY": "g-gem",
                "AWS_ACCESS_KEY_ID": "AKIA",
                "AWS_SECRET_ACCESS_KEY": "secret",
                "AWS_SESSION_TOKEN": "sess",
                "GOOGLE_APPLICATION_CREDENTIALS": "/path/creds.json",
                "AWS_BEARER_TOKEN_BEDROCK": "bedrock-tok",
                "ANTHROPIC_BASE_URL": "https://example.com",
                "ANTHROPIC_MODEL": "claude-x",
                "CLAUDE_CODE_SUBAGENT_MODEL": "gpt-5.4-mini",
                "CLAUDE_CODE_MAX_CONTEXT_TOKENS": "400000",
                "CLAUDE_CODE_AUTO_COMPACT_WINDOW": "400000",
                // 可共享、非机密配置（复数 _TOKENS 不应被误剥）
                "ENABLE_TOOL_SEARCH": "true",
                "CLAUDE_CODE_MAX_OUTPUT_TOKENS": "8192"
            },
            "apiKey": "sk-top",
            "api_key": "sk-top2",
            "theme": "dark",
            "includeCoAuthoredBy": false
        });

        let snippet = ProviderService::extract_claude_common_config(&settings)
            .expect("extract should succeed");
        let value: Value = serde_json::from_str(&snippet).expect("snippet is valid JSON");

        // 所有凭据都不得出现在共享片段里
        let env = value.get("env");
        for leaked in [
            "ANTHROPIC_API_KEY",
            "ANTHROPIC_AUTH_TOKEN",
            "OPENROUTER_API_KEY",
            "GOOGLE_API_KEY",
            "OPENAI_API_KEY",
            "GEMINI_API_KEY",
            "AWS_ACCESS_KEY_ID",
            "AWS_SECRET_ACCESS_KEY",
            "AWS_SESSION_TOKEN",
            "GOOGLE_APPLICATION_CREDENTIALS",
            "AWS_BEARER_TOKEN_BEDROCK",
        ] {
            assert!(
                env.and_then(|e| e.get(leaked)).is_none(),
                "credential {leaked} must not leak into common config"
            );
        }
        assert!(
            value.get("apiKey").is_none() && value.get("api_key").is_none(),
            "top-level credentials must be stripped"
        );

        // 端点/模型（provider-specific 非机密）也应剥掉
        assert!(env.and_then(|e| e.get("ANTHROPIC_BASE_URL")).is_none());
        assert!(env.and_then(|e| e.get("ANTHROPIC_MODEL")).is_none());
        assert!(env
            .and_then(|e| e.get("CLAUDE_CODE_SUBAGENT_MODEL"))
            .is_none());
        assert!(env
            .and_then(|e| e.get("CLAUDE_CODE_MAX_CONTEXT_TOKENS"))
            .is_none());
        assert!(env
            .and_then(|e| e.get("CLAUDE_CODE_AUTO_COMPACT_WINDOW"))
            .is_none());

        // 可共享的非机密配置必须保留（含复数 _TOKENS 不被误剥）
        assert_eq!(
            env.and_then(|e| e.get("ENABLE_TOOL_SEARCH"))
                .and_then(|v| v.as_str()),
            Some("true")
        );
        assert_eq!(
            env.and_then(|e| e.get("CLAUDE_CODE_MAX_OUTPUT_TOKENS"))
                .and_then(|v| v.as_str()),
            Some("8192")
        );
        assert_eq!(value.get("theme").and_then(|v| v.as_str()), Some("dark"));
        assert_eq!(value.get("includeCoAuthoredBy"), Some(&json!(false)));
    }

    /// 关键字段（协议选择器、Bedrock/Vertex 区域、`/model` 的选择等）不进共享片段；
    /// 同在 `CLAUDE_CODE_USE_` 前缀下、与供应商无关的开关照常共享。
    #[test]
    fn extract_claude_common_config_keeps_key_fields_per_provider() {
        let settings = json!({
            "env": {
                "CLAUDE_CODE_USE_BEDROCK": "1",
                "CLAUDE_CODE_USE_VERTEX": "1",
                "CLAUDE_CODE_SKIP_BEDROCK_AUTH": "1",
                "AWS_REGION": "us-west-2",
                "AWS_PROFILE": "work",
                "CLOUD_ML_REGION": "us-east5",
                "VERTEX_REGION_CLAUDE_4_0_OPUS": "europe-west1",
                "ANTHROPIC_VERTEX_PROJECT_ID": "my-project",
                "ANTHROPIC_SMALL_FAST_MODEL": "haiku",
                "CLAUDE_CODE_SUBAGENT_MODEL_FORCE": "1",
                "CLAUDE_CODE_USE_POWERSHELL_TOOL": "1",
                "CLAUDE_CODE_ATTRIBUTION_HEADER": "0",
                "DISABLE_TELEMETRY": "1"
            },
            "model": "opus",
            "fallbackModel": "sonnet",
            "apiKeyHelper": "~/bin/key.sh",
            "awsAuthRefresh": "aws sso login",
            "hooks": { "Stop": [] },
            "theme": "dark"
        });

        let snippet = ProviderService::extract_claude_common_config(&settings)
            .expect("extract should succeed");
        let value: Value = serde_json::from_str(&snippet).expect("snippet is valid JSON");

        assert_eq!(
            value,
            json!({
                "env": {
                    "CLAUDE_CODE_USE_POWERSHELL_TOOL": "1",
                    "CLAUDE_CODE_ATTRIBUTION_HEADER": "0",
                    "DISABLE_TELEMETRY": "1"
                },
                "hooks": { "Stop": [] },
                "theme": "dark"
            })
        );
        assert!(
            snippet.find("CLAUDE_CODE_USE_POWERSHELL_TOOL") < snippet.find("DISABLE_TELEMETRY"),
            "removing keys must not reorder the ones that stay: {snippet}"
        );
    }

    /// Regression for issue #4272: Fable tier env keys must not enter the shared
    /// Claude common-config snippet (same class as haiku/sonnet/opus model pins).
    #[test]
    fn extract_claude_common_config_strips_fable_model_env_keys() {
        let settings = json!({
            "env": {
                "ANTHROPIC_DEFAULT_HAIKU_MODEL": "haiku-mapped",
                "ANTHROPIC_DEFAULT_HAIKU_MODEL_NAME": "Haiku Mapped",
                "ANTHROPIC_DEFAULT_SONNET_MODEL": "sonnet-mapped[1M]",
                "ANTHROPIC_DEFAULT_SONNET_MODEL_NAME": "Sonnet Mapped",
                "ANTHROPIC_DEFAULT_OPUS_MODEL": "opus-mapped[1M]",
                "ANTHROPIC_DEFAULT_OPUS_MODEL_NAME": "Opus Mapped",
                "ANTHROPIC_DEFAULT_FABLE_MODEL": "deepseek-v4-flash[1M]",
                "ANTHROPIC_DEFAULT_FABLE_MODEL_NAME": "deepseek-v4-flash",
                "ANTHROPIC_MODEL": "default-mapped",
                "ENABLE_TOOL_SEARCH": "true"
            },
            "theme": "dark"
        });

        let snippet = ProviderService::extract_claude_common_config(&settings)
            .expect("extract should succeed");
        let value: Value = serde_json::from_str(&snippet).expect("snippet is valid JSON");
        let env = value.get("env");

        for stripped in [
            "ANTHROPIC_DEFAULT_HAIKU_MODEL",
            "ANTHROPIC_DEFAULT_HAIKU_MODEL_NAME",
            "ANTHROPIC_DEFAULT_SONNET_MODEL",
            "ANTHROPIC_DEFAULT_SONNET_MODEL_NAME",
            "ANTHROPIC_DEFAULT_OPUS_MODEL",
            "ANTHROPIC_DEFAULT_OPUS_MODEL_NAME",
            "ANTHROPIC_DEFAULT_FABLE_MODEL",
            "ANTHROPIC_DEFAULT_FABLE_MODEL_NAME",
            "ANTHROPIC_MODEL",
        ] {
            assert!(
                env.and_then(|e| e.get(stripped)).is_none(),
                "provider-specific model key {stripped} must not enter common config"
            );
        }

        assert_eq!(
            env.and_then(|e| e.get("ENABLE_TOOL_SEARCH"))
                .and_then(|v| v.as_str()),
            Some("true")
        );
        assert_eq!(value.get("theme").and_then(|v| v.as_str()), Some("dark"));
    }

    #[test]
    fn extract_credentials_returns_expected_values() {
        let provider = Provider::with_id(
            "claude".into(),
            "Claude".into(),
            json!({
                "env": {
                    "ANTHROPIC_AUTH_TOKEN": "token",
                    "ANTHROPIC_BASE_URL": "https://claude.example"
                }
            }),
            None,
        );
        let (api_key, base_url) =
            ProviderService::extract_credentials(&provider, &AppType::Claude).unwrap();
        assert_eq!(api_key, "token");
        assert_eq!(base_url, "https://claude.example");
    }

    #[test]
    fn extract_codex_common_config_strips_provider_fields_and_injected_artifacts() {
        // 顶层 experimental_bearer_token 模拟无活跃路由时的 fallback 注入；
        // web_search = "disabled" 是 cc-switch 对黑名单网关注入的哨兵；
        // 顶层 wire_api 模拟无 model_provider 时的 fallback 写法；
        // [mcp.servers] 是历史错误格式，sync_all_enabled 清不掉它。
        let config_toml = r#"model_provider = "azure"
model = "gpt-4"
wire_api = "chat"
disable_response_storage = true
model_reasoning_effort = "high"
approval_policy = "on-request"
experimental_bearer_token = "sk-live-secret"
model_catalog_json = "cc-switch-model-catalog.json"
web_search = "disabled"

[agents]
default_subagent_model = "gpt-4-mini"
max_threads = 4

[model_providers.azure]
name = "Azure OpenAI"
base_url = "https://azure.example/v1"
wire_api = "responses"

[mcp_servers.my_server]
base_url = "http://localhost:8080"

[mcp.servers.legacy_server]
command = "legacy-cmd"
"#;

        let settings = json!({ "config": config_toml });
        let extracted = ProviderService::extract_codex_common_config(&settings)
            .expect("extract_codex_common_config should succeed");

        assert!(
            !extracted
                .lines()
                .any(|line| line.trim_start().starts_with("model_provider")),
            "should remove top-level model_provider"
        );
        assert!(
            !extracted
                .lines()
                .any(|line| line.trim_start().starts_with("model =")),
            "should remove top-level model"
        );
        assert!(
            !extracted.contains("[model_providers"),
            "should remove entire model_providers table"
        );
        // MCP 归 DB mcp_servers 表所有，不得进共享片段（含历史错误格式 [mcp.servers]）
        assert!(
            !extracted.contains("mcp_servers") && !extracted.contains("http://localhost:8080"),
            "should strip mcp_servers from the shared snippet, got: {extracted}"
        );
        assert!(
            !extracted.contains("[mcp") && !extracted.contains("legacy-cmd"),
            "should strip the legacy [mcp.servers] form from the shared snippet, got: {extracted}"
        );
        // 顶层 wire_api 是供应商路由语义（model_providers 整表已剥，
        // 剩余任何 wire_api 都意味着泄漏）
        assert!(
            !extracted.contains("wire_api"),
            "should strip top-level wire_api from the shared snippet, got: {extracted}"
        );
        // 注入产物不得进共享片段（bearer token 泄漏为密钥级问题）
        assert!(
            !extracted.contains("experimental_bearer_token")
                && !extracted.contains("sk-live-secret"),
            "should strip top-level fallback bearer token, got: {extracted}"
        );
        assert!(
            !extracted.contains("model_catalog_json"),
            "should strip catalog projection pointer, got: {extracted}"
        );
        assert!(
            !extracted.contains("web_search"),
            "should strip the cc-switch web_search disabled sentinel, got: {extracted}"
        );
        // 关键字段归供应商（片段已冻结，收进去会从行里剥掉、再也写不回 live）
        for key in [
            "disable_response_storage",
            "model_reasoning_effort",
            "default_subagent_model",
        ] {
            assert!(
                !extracted.contains(key),
                "key field {key} must stay with the provider, got: {extracted}"
            );
        }
        // 真正可共享的键保留
        assert!(
            extracted.contains("approval_policy = \"on-request\"")
                && extracted.contains("max_threads = 4"),
            "shareable keys must survive extraction, got: {extracted}"
        );
    }

    #[test]
    fn extract_codex_common_config_keeps_user_set_web_search() {
        let config_toml = "web_search = \"enabled\"\ndisable_response_storage = true\n";
        let settings = json!({ "config": config_toml });
        let extracted = ProviderService::extract_codex_common_config(&settings)
            .expect("extract should succeed");
        assert!(
            extracted.contains("web_search = \"enabled\""),
            "a user-set web_search value is a shareable preference, got: {extracted}"
        );
    }

    #[tokio::test]
    #[serial]
    async fn editing_the_routed_claude_provider_rewrites_the_proxy_contract() {
        let _home = TempHome::new();
        crate::settings::reload_settings().expect("reload settings");

        let db = Arc::new(Database::memory().expect("init db"));
        let state = AppState::new(db.clone());

        let original = Provider::with_id(
            "p1".into(),
            "Claude A".into(),
            json!({
                "env": {
                    "ANTHROPIC_API_KEY": "token-a",
                    "ANTHROPIC_BASE_URL": "https://api.a.example",
                    "ANTHROPIC_MODEL": "model-a"
                }
            }),
            None,
        );
        db.save_provider("claude", &original)
            .expect("save provider");
        db.set_current_provider("claude", "p1")
            .expect("set current provider");
        crate::settings::set_current_provider(&AppType::Claude, Some("p1"))
            .expect("set local current provider");
        write_json_file(
            &get_claude_settings_path(),
            &json!({
                "env": {
                    "ANTHROPIC_BASE_URL": "https://api.a.example",
                    "ANTHROPIC_API_KEY": "token-a",
                    "ANTHROPIC_MODEL": "model-a"
                },
                "permissions": { "allow": ["Bash"] }
            }),
        )
        .expect("seed live file");

        db.update_proxy_config(ProxyConfig {
            listen_port: 0,
            ..Default::default()
        })
        .await
        .expect("update proxy config");
        crate::mode::controller::enter(&state, &AppType::Claude, false)
            .await
            .expect("enter routing mode");
        let proxy_url = state
            .proxy_service
            .build_proxy_urls()
            .await
            .expect("proxy url")
            .0;

        let updated = Provider::with_id(
            "p1".into(),
            "Claude A".into(),
            json!({
                "env": {
                    "ANTHROPIC_API_KEY": "token-updated",
                    "ANTHROPIC_BASE_URL": "https://api.updated.example",
                    "ANTHROPIC_MODEL": "model-updated"
                }
            }),
            None,
        );
        ProviderService::update(&state, AppType::Claude, None, updated)
            .expect("update routed provider");

        let live: Value = read_json_file(&get_claude_settings_path()).expect("read live");
        let env = &live["env"];
        assert_eq!(env["ANTHROPIC_API_KEY"], "PROXY_MANAGED");
        assert_eq!(env["ANTHROPIC_BASE_URL"], proxy_url.as_str());
        assert!(env.get("ANTHROPIC_MODEL").is_none(), "{live}");
        assert_eq!(
            env["ANTHROPIC_DEFAULT_SONNET_MODEL_NAME"], "model-updated",
            "the contract's display names follow the edited provider"
        );
        assert_eq!(
            live["permissions"],
            json!({ "allow": ["Bash"] }),
            "user settings in live are left alone"
        );

        state
            .proxy_service
            .stop()
            .await
            .expect("stop proxy service");
    }

    #[tokio::test]
    #[serial]
    async fn update_current_codex_provider_refreshes_and_clears_catalog_during_takeover() {
        let _home = TempHome::new();
        crate::settings::reload_settings().expect("reload settings");

        let db = Arc::new(Database::memory().expect("init db"));
        let state = AppState::new(db.clone());

        let mut original = Provider::with_id(
            "p1".into(),
            "Codex A".into(),
            json!({
                "auth": { "OPENAI_API_KEY": "token-a" },
                "config": r#"model_provider = "custom"
model = "old-model"

[model_providers.custom]
name = "Codex A"
base_url = "https://api.a.example/v1"
wire_api = "responses"
requires_openai_auth = true
"#,
                "modelCatalog": {
                    "models": [{ "model": "old-model" }]
                }
            }),
            None,
        );
        original.meta = Some(ProviderMeta {
            api_format: Some("openai_responses".into()),
            ..Default::default()
        });
        db.save_provider("codex", &original).expect("save provider");
        db.set_current_provider("codex", "p1")
            .expect("set current provider");
        crate::settings::set_current_provider(&AppType::Codex, Some("p1"))
            .expect("set local current provider");

        db.update_proxy_config(ProxyConfig {
            listen_port: 0,
            ..Default::default()
        })
        .await
        .expect("update proxy config");
        crate::mode::controller::enter(&state, &AppType::Codex, false)
            .await
            .expect("enter routing mode");
        assert!(
            state
                .proxy_service
                .live_has_proxy_placeholder(&AppType::Codex),
            "Codex live config should carry the proxy contract"
        );

        let mut updated = original.clone();
        updated.settings_config["config"] = json!(
            r#"model_provider = "custom"
model = "gpt-5.4"

[model_providers.custom]
name = "Codex A"
base_url = "https://api.updated.example/v1"
wire_api = "responses"
requires_openai_auth = true
"#
        );
        updated.settings_config["modelCatalog"] = json!({
            "models": [{ "model": "gpt-5.4", "displayName": "GPT 5.4" }]
        });

        ProviderService::update(&state, AppType::Codex, None, updated.clone())
            .expect("update current Codex provider mapping");

        let catalog_path = crate::codex_config::get_codex_model_catalog_path();
        let catalog: Value = read_json_file(&catalog_path).expect("read generated catalog");
        assert_eq!(catalog["models"][0]["slug"], "gpt-5.4");
        assert_eq!(
            catalog["models"][0]["input_modalities"],
            json!(["text", "image"]),
            "unknown/GPT models must fail open to image input"
        );
        let live_config = fs::read_to_string(crate::codex_config::get_codex_config_path())
            .expect("read Codex config.toml");
        assert!(live_config.contains("model_catalog_json"));

        updated.settings_config["modelCatalog"] = json!({ "models": [] });
        ProviderService::update(&state, AppType::Codex, None, updated)
            .expect("remove current Codex provider mapping");

        let live_config = fs::read_to_string(crate::codex_config::get_codex_config_path())
            .expect("read Codex config.toml after mapping removal");
        assert!(
            !live_config.contains("model_catalog_json"),
            "removing mappings during takeover must clear the stale catalog pointer"
        );

        state
            .proxy_service
            .stop()
            .await
            .expect("stop proxy service");
    }

    #[cfg(any(target_os = "macos", windows, target_os = "linux"))]
    #[tokio::test]
    #[serial]
    async fn update_current_claude_desktop_provider_syncs_profile_when_proxy_takeover_is_active() {
        let home = TempHome::new();
        crate::settings::reload_settings().expect("reload settings");

        let db = Arc::new(Database::memory().expect("init db"));
        let state = AppState::new(db.clone());

        let mut original = Provider::with_id(
            "p1".into(),
            "Desktop A".into(),
            json!({
                "env": {
                    "ANTHROPIC_AUTH_TOKEN": "token-a",
                    "ANTHROPIC_BASE_URL": "https://opencode.ai/zen/go"
                }
            }),
            None,
        );
        original.meta = Some(ProviderMeta {
            api_format: Some("openai_chat".into()),
            claude_desktop_mode: Some(ClaudeDesktopMode::Proxy),
            claude_desktop_model_routes: std::collections::HashMap::from([(
                "claude-sonnet-4-6".into(),
                ClaudeDesktopModelRoute {
                    model: "deepseek-v4-flash".into(),
                    label_override: Some("DeepSeek V4 Flash".into()),
                    supports_1m: None,
                },
            )]),
            ..Default::default()
        });
        db.save_provider("claude-desktop", &original)
            .expect("save provider");
        db.set_current_provider("claude-desktop", "p1")
            .expect("set current provider");
        crate::settings::set_current_provider(&AppType::ClaudeDesktop, Some("p1"))
            .expect("set local current provider");

        // Claude Desktop keeps backup state from takeover startup; this sentinel only
        // marks takeover as active so provider updates rewrite the 3P profile.
        db.save_live_backup("claude-desktop", "{}")
            .await
            .expect("seed live backup");
        {
            let mut config = db
                .get_proxy_config_for_app("claude-desktop")
                .await
                .expect("get app proxy config");
            config.enabled = true;
            db.update_proxy_config_for_app(config)
                .await
                .expect("update app proxy config");
        }

        state
            .proxy_service
            .start()
            .await
            .expect("start proxy service");

        let mut updated = Provider::with_id(
            "p1".into(),
            "Desktop A".into(),
            json!({
                "env": {
                    "ANTHROPIC_AUTH_TOKEN": "token-updated",
                    "ANTHROPIC_BASE_URL": "https://opencode.ai/zen/go"
                }
            }),
            None,
        );
        updated.meta = Some(ProviderMeta {
            api_format: Some("openai_chat".into()),
            claude_desktop_mode: Some(ClaudeDesktopMode::Proxy),
            claude_desktop_model_routes: std::collections::HashMap::from([(
                "claude-sonnet-4-6".into(),
                ClaudeDesktopModelRoute {
                    model: "deepseek-v4-flash".into(),
                    label_override: Some("DeepSeek V4 Flash Updated".into()),
                    supports_1m: Some(true),
                },
            )]),
            ..Default::default()
        });

        ProviderService::update(&state, AppType::ClaudeDesktop, None, updated.clone())
            .expect("update current provider");

        let backup = db
            .get_live_backup("claude-desktop")
            .await
            .expect("get live backup")
            .expect("backup exists");
        assert_eq!(
            backup.original_config, "{}",
            "Claude Desktop provider edits should not rewrite takeover backup"
        );

        let profile_path = claude_desktop_profile_path(home.dir.path());
        let profile: Value = read_json_file(&profile_path).expect("read desktop profile");
        assert_eq!(
            profile["inferenceGatewayBaseUrl"],
            json!("http://127.0.0.1:15721/claude-desktop"),
            "desktop profile should stay pointed at the local gateway during takeover"
        );
        assert_eq!(profile["inferenceGatewayAuthScheme"], json!("bearer"));
        assert_eq!(
            profile["inferenceModels"],
            json!([{ "name": "claude-sonnet-4-6", "labelOverride": "DeepSeek V4 Flash Updated", "supports1m": true }]),
            "provider edits should propagate into the Claude Desktop 3P profile during takeover"
        );
    }

    #[test]
    #[serial]
    fn sync_universal_to_apps_preserves_child_metadata() {
        with_test_home(|state, _home| {
            let mut universal = UniversalProvider::new(
                "metadata".into(),
                "Original".into(),
                "https://old.example".into(),
                "old-key".into(),
            );
            universal.apps.claude = true;
            universal.apps.codex = true;
            universal.apps.gemini = true;
            universal.meta = Some(
                serde_json::from_value(json!({
                    "usage_script": {"enabled": false, "language": "javascript", "code": "parent"}
                }))
                .unwrap(),
            );
            state.db.save_universal_provider(&universal).unwrap();
            ProviderService::sync_universal_to_apps(state, &universal.id).unwrap();

            let mut expected = Vec::new();
            for (index, app) in ["claude", "codex", "gemini"].iter().enumerate() {
                let id = format!("universal-{app}-metadata");
                let mut child = state.db.get_provider_by_id(&id, app).unwrap().unwrap();
                assert_eq!(
                    serde_json::to_value(&child.meta).unwrap(),
                    serde_json::to_value(&universal.meta).unwrap()
                );
                child.meta = Some(
                    serde_json::from_value(json!({
                        "usage_script": {"enabled": true, "language": "javascript", "code": app,
                            "apiKey": "usage-only-key", "autoQueryInterval": 15},
                        "commonConfigEnabled": false,
                        "endpointAutoSelect": true
                    }))
                    .unwrap(),
                );
                child.created_at = Some(123 + index as i64);
                child.sort_index = Some(10 + index);
                child.settings_config["local_setting"] = json!(app);
                state.db.save_provider(app, &child).unwrap();
                state
                    .db
                    .add_custom_endpoint(app, &id, "https://extra.example")
                    .unwrap();
                expected.push(child);
            }

            universal.name = "Updated".into();
            universal.base_url = "https://new.example".into();
            universal.api_key = "new-key".into();
            universal.notes = Some("shared note".into());
            universal.models = serde_json::from_value(json!({
                "claude": {"model": "claude-new"},
                "codex": {"model": "codex-new", "reasoningEffort": "low"},
                "gemini": {"model": "gemini-new"}
            }))
            .unwrap();
            // Both absent and present parent metadata must not overwrite child settings.
            for parent_meta in [None, universal.meta.clone()] {
                universal.meta = parent_meta;
                state.db.save_universal_provider(&universal).unwrap();
                ProviderService::sync_universal_to_apps(state, &universal.id).unwrap();
                for (app, before) in ["claude", "codex", "gemini"].iter().zip(&expected) {
                    let after = state
                        .db
                        .get_provider_by_id(&before.id, app)
                        .unwrap()
                        .unwrap();
                    assert_eq!(
                        serde_json::to_value(&after.meta).unwrap(),
                        serde_json::to_value(&before.meta).unwrap(),
                        "{app}"
                    );
                    assert_eq!(after.created_at, before.created_at, "{app}");
                    assert_eq!(after.sort_index, before.sort_index, "{app}");
                    assert_eq!(after.name, "Updated");
                    assert_eq!(after.notes.as_deref(), Some("shared note"));
                    assert_eq!(after.settings_config["local_setting"], json!(app));
                    let generated = match *app {
                        "claude" => universal.to_claude_provider(),
                        "codex" => universal.to_codex_provider(),
                        _ => universal.to_gemini_provider(),
                    }
                    .unwrap();
                    let mut expected_settings = before.settings_config.clone();
                    ProviderService::merge_json(&mut expected_settings, &generated.settings_config);
                    assert_eq!(after.settings_config, expected_settings);
                    assert_eq!(
                        state.db.get_all_providers(app).unwrap()[&before.id]
                            .meta
                            .as_ref()
                            .unwrap()
                            .custom_endpoints
                            .len(),
                        1
                    );
                }
            }
        });
    }

    #[test]
    #[serial]
    fn universal_provider_icon_is_validated_on_save_and_sync() {
        with_test_home(|state, _home| {
            let mut universal = UniversalProvider::new(
                "icon".into(),
                "Icon".into(),
                "https://api.example".into(),
                "key".into(),
            );
            universal.apps.claude = true;
            // 只有文件头、没有像素数据的「PNG」
            universal.icon =
                Some("data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAGAAAABgCAYAAAA=".into());
            assert!(ProviderService::upsert_universal(state, universal.clone()).is_err());
            assert!(state.db.get_universal_provider("icon").unwrap().is_none());

            // 绕过保存直接写进库的坏图标，同步到应用时也不会复制给子供应商
            state.db.save_universal_provider(&universal).unwrap();
            assert!(ProviderService::sync_universal_to_apps(state, "icon").is_err());
            assert!(state
                .db
                .get_provider_by_id("universal-claude-icon", "claude")
                .unwrap()
                .is_none());
        });
    }

    #[test]
    #[serial]
    fn sync_universal_to_apps_reprojects_current_child_to_live() {
        with_test_home(|state, _home| {
            let mut universal = UniversalProvider::new(
                "shared".to_string(),
                "Shared Relay".to_string(),
                "https://api.new.example".to_string(),
                "new-key".to_string(),
            );
            universal.apps.claude = true;
            universal.models.claude = Some(ClaudeModelConfig {
                model: Some("claude-sonnet-4".to_string()),
                ..Default::default()
            });
            state
                .db
                .save_universal_provider(&universal)
                .expect("save universal provider");

            let child = universal
                .to_claude_provider()
                .expect("claude child provider");
            state
                .db
                .save_provider("claude", &child)
                .expect("seed child provider");
            state
                .db
                .set_current_provider("claude", &child.id)
                .expect("set current child");
            crate::settings::set_current_provider(&AppType::Claude, Some(&child.id))
                .expect("set local current child");

            let mut old_live = child.settings_config.clone();
            old_live["env"]["ANTHROPIC_BASE_URL"] =
                Value::String("https://api.old.example".to_string());
            write_json_file(&get_claude_settings_path(), &old_live).expect("seed old live");

            ProviderService::sync_universal_to_apps(state, "shared")
                .expect("sync universal provider");

            let live: Value = read_json_file(&get_claude_settings_path()).expect("read live");
            assert_eq!(
                live["env"]["ANTHROPIC_BASE_URL"].as_str(),
                Some("https://api.new.example")
            );
            assert_eq!(
                live["env"]["ANTHROPIC_AUTH_TOKEN"].as_str(),
                Some("new-key")
            );
        });
    }

    #[test]
    #[serial]
    fn sync_universal_to_apps_writes_claude_switches_into_child_row_and_live() {
        // 署名与自动升级归供应商所有：写进子供应商行，同步时随行投影到 live。
        with_test_home(|state, _home| {
            let mut universal = UniversalProvider::new(
                "globals".to_string(),
                "Globals".to_string(),
                "https://api.globals.example".to_string(),
                "globals-key".to_string(),
            );
            universal.apps.claude = true;
            universal.models.claude = Some(ClaudeModelConfig {
                model: Some("claude-sonnet-4".to_string()),
                hide_attribution: Some(true),
                disable_auto_upgrade: Some(true),
                ..Default::default()
            });
            state
                .db
                .save_universal_provider(&universal)
                .expect("save universal provider");
            // 子卡是当前供应商，同步才会把行投影到 live
            state
                .db
                .set_current_provider("claude", "universal-claude-globals")
                .expect("set current provider");
            crate::settings::set_current_provider(
                &AppType::Claude,
                Some("universal-claude-globals"),
            )
            .expect("set local current provider");

            ProviderService::sync_universal_to_apps(state, "globals").expect("sync");

            // 子卡行带着这两个键：这是这一家的设置
            let child = state
                .db
                .get_provider_by_id("universal-claude-globals", "claude")
                .expect("read child")
                .expect("child exists");
            assert_eq!(
                child.settings_config["attribution"],
                json!({ "commit": "", "pr": "", "sessionUrl": false })
            );
            assert_eq!(
                child.settings_config["env"]["DISABLE_AUTOUPDATER"],
                json!("1")
            );

            let live: Value = read_json_file(&get_claude_settings_path()).expect("read live");
            assert_eq!(
                live["attribution"],
                json!({ "commit": "", "pr": "", "sessionUrl": false })
            );
            assert_eq!(live["env"]["DISABLE_AUTOUPDATER"].as_str(), Some("1"));
        });
    }

    #[test]
    #[serial]
    fn switching_away_from_provider_with_claude_switches_clears_them() {
        // 切到别的供应商：这两个键跟着当前那家走，上一家的值不再留在 live。
        with_test_home(|state, _home| {
            let mut universal = UniversalProvider::new(
                "globals".to_string(),
                "Globals".to_string(),
                "https://api.globals.example".to_string(),
                "globals-key".to_string(),
            );
            universal.apps.claude = true;
            universal.models.claude = Some(ClaudeModelConfig {
                model: Some("claude-sonnet-4".to_string()),
                hide_attribution: Some(true),
                disable_auto_upgrade: Some(true),
                ..Default::default()
            });
            state
                .db
                .save_universal_provider(&universal)
                .expect("save universal provider");
            state
                .db
                .set_current_provider("claude", "universal-claude-globals")
                .expect("set current provider");
            crate::settings::set_current_provider(
                &AppType::Claude,
                Some("universal-claude-globals"),
            )
            .expect("set local current provider");
            ProviderService::sync_universal_to_apps(state, "globals").expect("sync");

            // 把两个开关去掉再同步：子卡行清掉，live 也跟着清掉
            let mut updated = universal;
            updated.models.claude = Some(ClaudeModelConfig {
                model: Some("claude-sonnet-4".to_string()),
                ..Default::default()
            });
            state
                .db
                .save_universal_provider(&updated)
                .expect("save updated");
            ProviderService::sync_universal_to_apps(state, "globals").expect("sync 2");

            let child = state
                .db
                .get_provider_by_id("universal-claude-globals", "claude")
                .expect("read child")
                .expect("child exists");
            assert!(child.settings_config.get("attribution").is_none());
            assert!(child.settings_config["env"]
                .get("DISABLE_AUTOUPDATER")
                .is_none());

            let live: Value = read_json_file(&get_claude_settings_path()).expect("read live");
            assert!(live.get("attribution").is_none(), "{live}");
            assert!(live["env"].get("DISABLE_AUTOUPDATER").is_none(), "{live}");
        });
    }
}
