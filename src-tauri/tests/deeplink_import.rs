use std::sync::Arc;

use cc_switch_lib::{
    import_provider_from_deeplink, parse_deeplink_url, AppState, AppType, Database, ProviderService,
};

#[path = "support.rs"]
mod support;
use support::{ensure_test_home, reset_test_fs, test_mutex};

/// 深链接导入现在一律生成统一供应商：父级进 settings 里的统一供应商列表，五个应用
/// 各有一张由它同步出来的子卡。子卡 id 的格式由 `UniversalProvider::to_*_provider` 决定。
fn child_id(app: &str, universal_id: &str) -> String {
    format!("universal-{app}-{universal_id}")
}

#[test]
fn deeplink_import_claude_provider_becomes_a_universal_provider() {
    let _guard = test_mutex().lock().unwrap_or_else(|e| e.into_inner());
    reset_test_fs();
    let _home = ensure_test_home();

    let url = "ccswitch://v1/import?resource=provider&app=claude&name=DeepLink%20Claude&homepage=https%3A%2F%2Fexample.com&endpoint=https%3A%2F%2Fapi.example.com%2Fv1&apiKey=sk-test-claude-key&model=claude-sonnet-4&icon=claude";
    let request = parse_deeplink_url(url).expect("parse deeplink url");

    let db = Arc::new(Database::memory().expect("create memory db"));
    let state = AppState::new(db.clone());

    let universal_id = import_provider_from_deeplink(&state, request.clone())
        .expect("import provider from deeplink");

    // 父级：链接里的连接信息与模型块都被保留下来。
    let universal = db
        .get_universal_provider(&universal_id)
        .expect("read universal provider")
        .expect("universal provider created via deeplink");
    assert_eq!(universal.name, request.name.clone().unwrap());
    assert_eq!(
        universal.website_url.as_deref(),
        request.homepage.as_deref()
    );
    assert_eq!(universal.icon.as_deref(), Some("claude"));
    assert_eq!(universal.base_url, "https://api.example.com/v1");
    assert_eq!(universal.api_key, "sk-test-claude-key");
    assert_eq!(
        universal
            .models
            .claude
            .as_ref()
            .and_then(|claude| claude.model.as_deref()),
        Some("claude-sonnet-4")
    );
    assert!(universal.apps.is_all(), "统一供应商同步到全部应用");

    // 子卡：五个应用都有一份，Claude 的那份带着链接里的模型与凭据。
    let claude_id = child_id("claude", &universal_id);
    for app in AppType::all() {
        let providers = db.get_all_providers(app.as_str()).expect("get providers");
        assert!(
            providers.contains_key(&child_id(app.as_str(), &universal_id)),
            "{} should receive a synced child row",
            app.as_str()
        );
    }
    let claude = db
        .get_provider_by_id(&claude_id, "claude")
        .expect("read claude child")
        .expect("claude child created");
    assert_eq!(claude.name, request.name.clone().unwrap());
    assert_eq!(claude.website_url.as_deref(), request.homepage.as_deref());
    assert_eq!(claude.icon.as_deref(), Some("claude"));
    let auth_token = claude
        .settings_config
        .pointer("/env/ANTHROPIC_AUTH_TOKEN")
        .and_then(|v| v.as_str());
    let base_url = claude
        .settings_config
        .pointer("/env/ANTHROPIC_BASE_URL")
        .and_then(|v| v.as_str());
    let model = claude
        .settings_config
        .pointer("/env/ANTHROPIC_MODEL")
        .and_then(|v| v.as_str());
    assert_eq!(auth_token, request.api_key.as_deref());
    assert_eq!(base_url, request.endpoint.as_deref());
    assert_eq!(model, Some("claude-sonnet-4"));
}

#[test]
fn deeplink_import_codex_provider_keeps_auth_and_config_in_the_child() {
    let _guard = test_mutex().lock().unwrap_or_else(|e| e.into_inner());
    reset_test_fs();
    let _home = ensure_test_home();

    let url = "ccswitch://v1/import?resource=provider&app=codex&name=DeepLink%20Codex&homepage=https%3A%2F%2Fopenai.example&endpoint=https%3A%2F%2Fapi.openai.example%2Fv1&apiKey=sk-test-codex-key&model=gpt-4o&icon=openai";
    let request = parse_deeplink_url(url).expect("parse deeplink url");

    let db = Arc::new(Database::memory().expect("create memory db"));
    let state = AppState::new(db.clone());

    let universal_id = import_provider_from_deeplink(&state, request.clone())
        .expect("import provider from deeplink");

    let universal = db
        .get_universal_provider(&universal_id)
        .expect("read universal provider")
        .expect("universal provider created via deeplink");
    assert_eq!(universal.base_url, "https://api.openai.example/v1");
    assert_eq!(universal.api_key, "sk-test-codex-key");
    assert_eq!(
        universal
            .models
            .codex
            .as_ref()
            .and_then(|codex| codex.model.as_deref()),
        Some("gpt-4o")
    );

    let codex = db
        .get_provider_by_id(&child_id("codex", &universal_id), "codex")
        .expect("read codex child")
        .expect("codex child created");
    assert_eq!(codex.name, request.name.clone().unwrap());
    assert_eq!(codex.website_url.as_deref(), request.homepage.as_deref());
    assert_eq!(codex.icon.as_deref(), Some("openai"));
    let auth_value = codex
        .settings_config
        .pointer("/auth/OPENAI_API_KEY")
        .and_then(|v| v.as_str());
    let config_text = codex
        .settings_config
        .get("config")
        .and_then(|v| v.as_str())
        .unwrap_or_default();
    assert_eq!(auth_value, request.api_key.as_deref());
    assert!(
        config_text.contains(request.endpoint.as_deref().unwrap()),
        "config.toml content should contain endpoint"
    );
    assert!(
        config_text.contains("model = \"gpt-4o\""),
        "config.toml content should contain model setting"
    );
}

#[test]
fn deeplink_import_records_extra_endpoints_on_the_universal_provider() {
    let _guard = test_mutex().lock().unwrap_or_else(|e| e.into_inner());
    reset_test_fs();
    let _home = ensure_test_home();

    let url = "ccswitch://v1/import?resource=provider&app=claude&name=Multi%20Endpoint&homepage=https%3A%2F%2Fexample.com&endpoint=https%3A%2F%2Fapi.example.com%2Chttps%3A%2F%2Fbackup.example.com&apiKey=sk-test";
    let request = parse_deeplink_url(url).expect("parse deeplink url");

    let db = Arc::new(Database::memory().expect("create memory db"));
    let state = AppState::new(db.clone());

    let universal_id =
        import_provider_from_deeplink(&state, request).expect("import provider from deeplink");
    let universal = db
        .get_universal_provider(&universal_id)
        .expect("read universal provider")
        .expect("universal provider created");

    let endpoints = universal
        .meta
        .as_ref()
        .expect("meta present")
        .custom_endpoints
        .keys()
        .cloned()
        .collect::<Vec<_>>();
    assert_eq!(endpoints, vec!["https://backup.example.com".to_string()]);
}

#[test]
fn deeplink_import_enabled_switches_only_the_source_app() {
    let _guard = test_mutex().lock().unwrap_or_else(|e| e.into_inner());
    reset_test_fs();
    let _home = ensure_test_home();

    let url = "ccswitch://v1/import?resource=provider&app=claude&name=Switch%20Me&homepage=https%3A%2F%2Fexample.com&endpoint=https%3A%2F%2Fapi.example.com&apiKey=sk-test&enabled=true";
    let request = parse_deeplink_url(url).expect("parse deeplink url");

    let db = Arc::new(Database::memory().expect("create memory db"));
    let state = AppState::new(db.clone());

    let universal_id =
        import_provider_from_deeplink(&state, request).expect("import provider from deeplink");

    let current = ProviderService::current(&state, AppType::Claude).expect("read current");
    assert_eq!(current, child_id("claude", &universal_id));
    for app in [
        AppType::Codex,
        AppType::Gemini,
        AppType::ClaudeDesktop,
        AppType::GrokBuild,
    ] {
        assert_eq!(
            ProviderService::current(&state, app.clone()).expect("read current"),
            "",
            "{} must keep its current provider",
            app.as_str()
        );
    }
}
