//! 统一供应商的可用模型列表：`/v1/models` 出 id，oh-my-pi 模型目录补名称、上下文窗口、
//! 最大输出和 effort 档位。
//!
//! 用 `@oh-my-pi/pi-catalog` 的 `models.json`：它是 `供应商 -> {模型 id -> 元数据}` 的
//! 嵌套表，同名模型会在多家供应商下重复出现。这里把它压扁成 `模型 id -> 元数据`：同名
//! 条目按字段合并（非空值优先、effort 档位取并集），所以任何一个供应商声明的能力都能
//! 查到。拉取顺序是：先用供应商自己的凭据取 id 列表，再拿这份表去补全；表里没有的 id
//! 只填 id 本身。

use std::collections::BTreeMap;
use std::time::Duration;

use serde::Deserialize;

use crate::provider::AvailableModel;

/// oh-my-pi 模型目录（`@oh-my-pi/pi-catalog` 的 `models.json`）。`@latest` 让 CDN
/// 跟随包的最新发布；jsDelivr 不可用时退到 unpkg。
const CATALOG_URLS: &[&str] = &[
    "https://cdn.jsdelivr.net/npm/@oh-my-pi/pi-catalog@latest/src/models.json",
    "https://unpkg.com/@oh-my-pi/pi-catalog@latest/src/models.json",
];
const FETCH_TIMEOUT_SECS: u64 = 25;
/// 解压后的目录体积上限（目录原文约 12 MB），防御压缩炸弹。
const MAX_CATALOG_BYTES: usize = 64 * 1024 * 1024;

/// 一条压扁后的模型元数据。
#[derive(Debug, Default, Clone)]
pub(crate) struct CatalogEntry {
    name: Option<String>,
    context_window: Option<u64>,
    /// `maxTokens`（最大输出 tokens）。
    max_output: Option<u64>,
    /// `thinking.efforts` 的并集（保持声明顺序去重）。目录里的 effort / budget /
    /// google-level / anthropic-adaptive 都表示用户可选的思考档位。
    efforts: Vec<String>,
}

/// `models.json` 一条模型里我们关心的字段。
#[derive(Debug, Deserialize)]
struct CatalogRawModel {
    name: Option<String>,
    #[serde(rename = "contextWindow")]
    context_window: Option<u64>,
    #[serde(rename = "maxTokens")]
    max_tokens: Option<u64>,
    thinking: Option<CatalogThinking>,
}

#[derive(Debug, Deserialize)]
struct CatalogThinking {
    efforts: Option<Vec<String>>,
}

/// `models.json` 的顶层形状：`供应商 id -> {模型 id -> 元数据}`。
type CatalogRaw = BTreeMap<String, BTreeMap<String, CatalogRawModel>>;

/// 模型显示名的合并规则：先用第一个非空名字；若它只是模型 id 本身（有些转售平台
/// 原样抄 id），后面出现的人类可读名字优先替换，避免把 `deepseek-v4-flash` 当成
/// 显示名写进菜单。
fn merge_name(into: &mut Option<String>, candidate: Option<&str>, model_id: &str) {
    let Some(name) = candidate.map(str::trim).filter(|name| !name.is_empty()) else {
        return;
    };
    let replace = match into.as_deref() {
        None => true,
        Some(existing) => {
            existing.eq_ignore_ascii_case(model_id) && !name.eq_ignore_ascii_case(model_id)
        }
    };
    if replace {
        *into = Some(name.to_string());
    }
}

/// effort 档位的合并顺序：先声明的供应商排前面，去重；排序交给 `canonical_efforts`。
fn merge_efforts(into: &mut Vec<String>, values: &[String]) {
    for value in values {
        let value = value.trim();
        if value.is_empty() || into.iter().any(|existing| existing == value) {
            continue;
        }
        into.push(value.to_string());
    }
}

/// 把嵌套的 `models.json` 压扁：`provider/model -> 元数据` 合并成 `model -> 元数据`。
///
/// 合并规则逐字段独立：`name` 取第一个非空值（模型 id 原样不算名字，见
/// [`merge_name`]），`context_window` / `max_output` 取第一个非零值（遍历序稳定，
/// 供应商 id 是 BTreeMap 序），`efforts` 取并集。
fn flatten_catalog(raw: CatalogRaw) -> BTreeMap<String, CatalogEntry> {
    let mut flat: BTreeMap<String, CatalogEntry> = BTreeMap::new();
    for models in raw.values() {
        for (model_id, model) in models {
            let entry = flat.entry(model_id.clone()).or_default();
            merge_name(&mut entry.name, model.name.as_deref(), model_id);
            if entry.context_window.is_none() {
                entry.context_window = model.context_window.filter(|context| *context > 0);
            }
            if entry.max_output.is_none() {
                entry.max_output = model.max_tokens.filter(|output| *output > 0);
            }
            if let Some(efforts) = model
                .thinking
                .as_ref()
                .and_then(|thinking| thinking.efforts.as_ref())
            {
                merge_efforts(&mut entry.efforts, efforts);
            }
        }
    }
    flat
}

/// 拉取 oh-my-pi 的模型目录并压扁。失败时返回空表：补全只是锦上添花，
/// 供应商自己给的 id 列表照常可用。
///
/// 全局 HTTP 客户端禁用了自动解压（代理转发路径按 `content-encoding` 手动处理，
/// 见 `http_client.rs`），所以这里显式要压缩响应，再按 `content-encoding` 自己解一次。
async fn fetch_catalog() -> BTreeMap<String, CatalogEntry> {
    let client = crate::proxy::http_client::get();
    for url in CATALOG_URLS {
        match fetch_catalog_from(&client, url).await {
            Ok(raw) => return flatten_catalog(raw),
            Err(e) => log::warn!("拉取 oh-my-pi 模型目录失败（{url}），尝试下一个源: {e}"),
        }
    }
    BTreeMap::new()
}

async fn fetch_catalog_from(client: &reqwest::Client, url: &str) -> Result<CatalogRaw, String> {
    let response = client
        .get(url)
        .header(reqwest::header::ACCEPT_ENCODING, "br, gzip")
        .timeout(Duration::from_secs(FETCH_TIMEOUT_SECS))
        .send()
        .await
        .map_err(|e| format!("请求失败: {e}"))?;
    if !response.status().is_success() {
        return Err(format!("返回 {}", response.status()));
    }
    let encoding = crate::proxy::content_encoding::get_content_encoding(response.headers())
        .unwrap_or_default();
    let raw_bytes = response
        .bytes()
        .await
        .map_err(|e| format!("读取响应失败: {e}"))?;
    let body = crate::proxy::content_encoding::decompress_body_with_limit(
        &encoding,
        raw_bytes.as_ref(),
        MAX_CATALOG_BYTES,
    )
    .map_err(|e| format!("解压失败（{encoding}）: {e}"))?
    // 没有 content-encoding：原样就是 JSON。
    .unwrap_or_else(|| raw_bytes.to_vec());
    serde_json::from_slice::<CatalogRaw>(&body).map_err(|e| format!("解析失败: {e}"))
}

/// 模型 id 切成 token：按**任意**非字母数字分隔符切开，不限于 `/` 和 `-`。
///
/// 匹配单位是 token（用户定义的「按 token 划分」），所以 `gpt-5.6-luna` 是
/// `["gpt","5","6","luna"]`，`openai/gpt-5.6-luna` 只多一个 `openai`。
/// 大小写与 1M 标记在这里统一。
fn tokens(model_id: &str) -> Vec<String> {
    let lowered = model_id.trim().to_lowercase();
    let without_marker = lowered
        .strip_suffix("[1m]")
        .map(str::trim_end)
        .unwrap_or(&lowered);
    without_marker
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|token| !token.is_empty())
        .map(str::to_string)
        .collect()
}

/// 两个 token 序列里最长的**连续公共片段**长度。
///
/// 这就是「核心段」：两边各自忽略任意数量的开头/结尾 token 后剩下的部分。
fn longest_common_token_run(query: &[String], candidate: &[String]) -> usize {
    let mut best = 0usize;
    for i in 0..query.len() {
        for j in 0..candidate.len() {
            let mut run = 0usize;
            while i + run < query.len()
                && j + run < candidate.len()
                && query[i + run] == candidate[j + run]
            {
                run += 1;
            }
            best = best.max(run);
        }
    }
    best
}

/// 两个模型 id 的匹配分数：`(核心 token 数, 被忽略的 token 数)`。
///
/// 规则（「可忽略前后缀 + 最长匹配」，按 token 划分）：
/// - 两边各自去掉**任意**开头与结尾 token（分隔符不限，`/`、`-`、`.`、`_` 都行，
///   因为切分只按非字母数字），剩下的核心 token 段相等才算命中。
/// - 多条命中时，**核心 token 数最多**者胜。
/// - 核心一样多时，**被忽略的 token 数最少**者胜（`zhipuai/glm-5` 胜过
///   `zhipuai/glm-5.3`：后者还要多丢一个 `3`）。
/// - 核心只有一个 token 太弱，除非它就是较短那一侧的全部（`o1` 这类单 token id
///   才匹配得上；`gpt-reserve` 里的 `gpt` 不算）。
///
/// 返回 `None` 表示不匹配。
fn match_score(query: &str, candidate: &str) -> Option<(usize, usize)> {
    let query = tokens(query);
    let candidate = tokens(candidate);
    if query.is_empty() || candidate.is_empty() {
        return None;
    }
    let core = longest_common_token_run(&query, &candidate);
    let shorter = query.len().min(candidate.len());
    if core == 0 || (core < 2 && core != shorter) {
        return None;
    }
    Some((core, query.len() + candidate.len() - 2 * core))
}

/// 在目录表里给 `query` 找最合适的一条。
///
/// 排序即胜负：核心 token 更多优先，其次被忽略的 token 更少。完全并列时保留表内先
/// 出现的（表是稳定序，多次调用结果一致）。
fn best_catalog_match<'a>(
    query: &str,
    table: &'a BTreeMap<String, CatalogEntry>,
) -> Option<&'a CatalogEntry> {
    let mut best: Option<((usize, usize), &CatalogEntry)> = None;
    for (key, entry) in table {
        let Some(score) = match_score(query, key) else {
            continue;
        };
        let better = match best {
            None => true,
            Some((best_score, _)) => {
                score.0 > best_score.0 || (score.0 == best_score.0 && score.1 < best_score.1)
            }
        };
        if better {
            best = Some((score, entry));
        }
    }
    best.map(|(_, entry)| entry)
}

/// 把 ids 逐条补上目录里的名称、上下文窗口、最大输出和 effort 档位。
///
/// effort 档位按预置升序（none…ultra）写出，不管目录的声明顺序（同名模型跨供应商
/// 并集时是首现序）。目录写入、代理查表钳制、UI 勾选集合都假设这个顺序；未知档位保留在
/// 末尾（旧数据可能带非预置值，下游过滤不丢数据）。
pub(crate) fn enrich_with_catalog(
    ids: Vec<String>,
    table: &BTreeMap<String, CatalogEntry>,
) -> Vec<AvailableModel> {
    ids.into_iter()
        .map(|id| {
            let hit = best_catalog_match(&id, table);
            AvailableModel {
                display_name: hit
                    .and_then(|entry| entry.name.clone())
                    .filter(|name| !name.trim().is_empty()),
                context_window: hit.and_then(|entry| entry.context_window),
                max_output: hit.and_then(|entry| entry.max_output),
                efforts: hit
                    .map(|entry| canonical_efforts(&entry.efforts))
                    .unwrap_or_default(),
                default_effort: None,
                id,
            }
        })
        .collect()
}

/// 预置 effort 升序（none < minimal < … < ultra），与前端 REASONING_LEVELS / EFFORT_PRESETS
/// 和 Codex 目录的 CODEX_REASONING_LEVEL_DESCRIPTIONS 同序。已知档位按此序排列，未知档位
/// （目录里的扩展值或旧数据）按字典序殿后，绝不静默丢弃。
const CANONICAL_EFFORTS: &[&str] = &[
    "none", "minimal", "low", "medium", "high", "xhigh", "max", "ultra",
];

fn canonical_efforts(efforts: &[String]) -> Vec<String> {
    let mut known: Vec<&str> = Vec::with_capacity(efforts.len());
    let mut unknown: Vec<String> = Vec::new();
    for effort in efforts {
        let effort = effort.trim();
        if effort.is_empty() {
            continue;
        }
        if CANONICAL_EFFORTS.contains(&effort) {
            if !known.contains(&effort) {
                known.push(effort);
            }
        } else if !unknown.iter().any(|existing| existing == effort) {
            unknown.push(effort.to_string());
        }
    }
    known.sort_by_key(|effort| {
        CANONICAL_EFFORTS
            .iter()
            .position(|preset| preset == effort)
            .expect("known effort is in the presets")
    });
    unknown.sort();
    known
        .into_iter()
        .map(str::to_string)
        .chain(unknown)
        .collect()
}

/// 取 id 列表（已按调用方去重），补全后返回。
pub async fn build_available_models(ids: Vec<String>) -> Vec<AvailableModel> {
    let table = fetch_catalog().await;
    enrich_with_catalog(ids, &table)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(name: Option<&str>, context: Option<u64>) -> CatalogEntry {
        CatalogEntry {
            name: name.map(str::to_string),
            context_window: context,
            max_output: None,
            efforts: Vec::new(),
        }
    }

    fn raw_model(
        name: Option<&str>,
        context: Option<u64>,
        output: Option<u64>,
        efforts: Option<Vec<&str>>,
    ) -> CatalogRawModel {
        CatalogRawModel {
            name: name.map(str::to_string),
            context_window: context,
            max_tokens: output,
            thinking: efforts.map(|values| CatalogThinking {
                efforts: Some(values.into_iter().map(str::to_string).collect()),
            }),
        }
    }

    #[test]
    fn catalog_source_is_the_oh_my_pi_package() {
        // 目录来源锁定在 `@oh-my-pi/pi-catalog`：models.dev 的 api.json 字段形状不同
        // （limit.context / reasoning_options），换回去会静默补不到任何元数据。
        for url in CATALOG_URLS {
            assert!(
                url.contains("@oh-my-pi/pi-catalog"),
                "unexpected catalog source: {url}"
            );
        }
    }

    #[test]
    fn parses_pi_catalog_rows() {
        // `@oh-my-pi/pi-catalog` 的字段是 camelCase，思考档位在 thinking.efforts。
        let raw: CatalogRaw = serde_json::from_str(
            r#"{
                "anthropic": {
                    "claude-opus-4-7": {
                        "id": "claude-opus-4-7",
                        "name": "Claude Opus 4.7",
                        "api": "anthropic-messages",
                        "provider": "anthropic",
                        "reasoning": true,
                        "contextWindow": 1000000,
                        "maxTokens": 128000,
                        "thinking": {
                            "mode": "anthropic-adaptive",
                            "efforts": ["low", "medium", "high", "xhigh", "max"]
                        }
                    }
                }
            }"#,
        )
        .expect("catalog parses");

        let table = flatten_catalog(raw);
        let out = enrich_with_catalog(vec!["claude-opus-4-7".to_string()], &table);
        assert_eq!(out[0].display_name.as_deref(), Some("Claude Opus 4.7"));
        assert_eq!(out[0].context_window, Some(1_000_000));
        assert_eq!(out[0].max_output, Some(128_000));
        assert_eq!(
            out[0].efforts,
            vec!["low", "medium", "high", "xhigh", "max"]
        );
    }

    #[test]
    fn flattens_catalog_merging_provider_duplicates() {
        let mut raw = CatalogRaw::new();
        let mut deepinfra_models = BTreeMap::new();
        deepinfra_models.insert(
            "tencent/hy3".to_string(),
            raw_model(
                Some("Hy3"),
                Some(262_144),
                Some(32_768),
                Some(vec!["low", "high"]),
            ),
        );
        raw.insert("deepinfra".to_string(), deepinfra_models);
        let mut hf_models = BTreeMap::new();
        hf_models.insert(
            "tencent/hy3".to_string(),
            raw_model(None, Some(999_999), None, Some(vec!["high", "max"])),
        );
        raw.insert("huggingface".to_string(), hf_models);

        let flat = flatten_catalog(raw);
        let hy3 = flat.get("tencent/hy3").expect("flattened");
        // 名称、窗口与最大输出取第一个非空；effort 取并集（保持首现顺序去重）。
        assert_eq!(hy3.name.as_deref(), Some("Hy3"));
        assert_eq!(hy3.context_window, Some(262_144));
        assert_eq!(hy3.max_output, Some(32_768));
        assert_eq!(hy3.efforts, vec!["low", "high", "max"]);
    }

    #[test]
    fn prefers_a_human_name_over_the_bare_model_id() {
        // 有些转售平台把 id 原样当名字；后面供应商给的人类可读名要顶掉它。
        let mut raw = CatalogRaw::new();
        let mut reseller = BTreeMap::new();
        reseller.insert(
            "deepseek-v4-flash".to_string(),
            raw_model(Some("deepseek-v4-flash"), Some(1_000_000), None, None),
        );
        raw.insert("aimlapi".to_string(), reseller);
        let mut vendor = BTreeMap::new();
        vendor.insert(
            "deepseek-v4-flash".to_string(),
            raw_model(Some("DeepSeek V4 Flash"), Some(1_048_576), None, None),
        );
        raw.insert("deepseek".to_string(), vendor);

        let flat = flatten_catalog(raw);
        assert_eq!(
            flat.get("deepseek-v4-flash")
                .and_then(|e| e.name.as_deref()),
            Some("DeepSeek V4 Flash")
        );
    }

    #[test]
    fn enrich_carries_efforts_from_the_table() {
        let mut table = BTreeMap::new();
        let mut glm = entry(Some("GLM-5"), Some(200_000));
        glm.efforts = vec!["low".to_string(), "high".to_string()];
        table.insert("zhipuai/glm-5".to_string(), glm);
        table.insert("gpt-reserve".to_string(), entry(None, None));

        let out = enrich_with_catalog(vec!["glm-5".to_string(), "gpt-reserve".to_string()], &table);
        assert_eq!(out[0].efforts, vec!["low", "high"]);
        assert!(out[1].efforts.is_empty(), "未收录的模型没有档位");
    }

    #[test]
    fn enrich_writes_efforts_in_preset_order_not_declaration_order() {
        // 同名模型跨供应商并集（flatten_catalog 的 merge_efforts 保持首现序）：
        // 声明顺序可能是乱的，写出的档位必须是预置升序——UI 勾选集合、Codex 目录、
        // 代理查表钳制都按这个顺序读。
        let mut table = BTreeMap::new();
        let mut entry_ = entry(Some("Kimi K2.6"), None);
        entry_.efforts = vec![
            "none".to_string(),
            "high".to_string(),
            "minimal".to_string(),
            "low".to_string(),
            "medium".to_string(),
            "xhigh".to_string(),
            "max".to_string(),
        ];
        table.insert("moonshot/kimi-k2.6".to_string(), entry_);

        let out = enrich_with_catalog(vec!["kimi-k2.6".to_string()], &table);
        assert_eq!(
            out[0].efforts,
            vec!["none", "minimal", "low", "medium", "high", "xhigh", "max"]
        );
    }

    #[test]
    fn enrich_keeps_unknown_efforts_after_the_known_ones() {
        // 目录里的扩展档位 / 旧数据的非预置值不能丢：排在已知档位之后，下游
        // 会过滤但数据还在。
        let mut table = BTreeMap::new();
        let mut entry_ = entry(Some("Custom"), None);
        entry_.efforts = vec![
            "max".to_string(),
            "turbo".to_string(),
            "low".to_string(),
            "lite".to_string(),
            "max".to_string(),
            "".to_string(),
        ];
        table.insert("custom/model".to_string(), entry_);

        let out = enrich_with_catalog(vec!["model".to_string()], &table);
        assert_eq!(out[0].efforts, vec!["low", "max", "lite", "turbo"]);
    }

    #[test]
    fn fills_name_and_context_from_catalog() {
        let mut table = BTreeMap::new();
        table.insert(
            "zhipuai/glm-5".to_string(),
            entry(Some("GLM-5"), Some(200_000)),
        );

        let out = enrich_with_catalog(vec!["glm-5".to_string()], &table);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].id, "glm-5");
        assert_eq!(out[0].display_name.as_deref(), Some("GLM-5"));
        assert_eq!(out[0].context_window, Some(200_000));
    }

    #[test]
    fn keeps_the_id_when_catalog_has_no_match() {
        let table = BTreeMap::new();
        let out = enrich_with_catalog(vec!["my-local-model".to_string()], &table);
        assert_eq!(out[0].id, "my-local-model");
        assert_eq!(out[0].display_name, None);
        assert_eq!(out[0].context_window, None);
    }

    #[test]
    fn matches_through_arbitrary_affixes_and_delimiters() {
        let mut table = BTreeMap::new();
        table.insert(
            "vendor/Model-X:free".to_string(),
            entry(Some("Model X"), Some(128_000)),
        );

        // 分隔符不限：`/`、`-`、`.`、`_`、`:` 都只是切分点，前后缀可任意忽略
        for probe in [
            "Model-X",
            "model-x",
            "vendor/Model-X:free",
            "VENDOR/MODEL.X_FREE.PREVIEW",
            "someprefix-model-x-somesuffix",
        ] {
            let out = enrich_with_catalog(vec![probe.to_string()], &table);
            assert_eq!(
                out[0].context_window,
                Some(128_000),
                "probe {probe} should match"
            );
        }
    }

    #[test]
    fn prefers_the_fewest_ignored_tokens_when_cores_tie() {
        let mut table = BTreeMap::new();
        table.insert(
            "zhipuai/glm-5".to_string(),
            entry(Some("GLM-5"), Some(200_000)),
        );
        table.insert(
            "zhipuai/glm-5.3".to_string(),
            entry(Some("GLM-5.3"), Some(300_000)),
        );

        // 两条核心都是 `glm 5`；`zhipuai/glm-5` 只丢命名空间，`glm-5.3` 还多丢一个 `3`。
        let out = enrich_with_catalog(vec!["glm-5".to_string()], &table);
        assert_eq!(out[0].display_name.as_deref(), Some("GLM-5"));
        assert_eq!(out[0].context_window, Some(200_000));
    }

    #[test]
    fn prefers_the_longer_core_when_the_query_is_the_longer_one() {
        let mut table = BTreeMap::new();
        table.insert("openai/o1".to_string(), entry(Some("o1"), Some(200_000)));
        table.insert(
            "openai/o1-pro".to_string(),
            entry(Some("o1-pro"), Some(200_000)),
        );

        let out = enrich_with_catalog(vec!["o1-pro".to_string()], &table);
        assert_eq!(out[0].display_name.as_deref(), Some("o1-pro"));
    }

    #[test]
    fn does_not_match_on_a_single_shared_token() {
        let mut table = BTreeMap::new();
        // `gpt-reserve` 与它只共用 `gpt` 一个 token；`codex-auto-review` 与会话条目
        // 只共用 `review` 一个：都不算命中，免得写入错的名称和窗口。
        table.insert(
            "openai/gpt-realtime-2.1".to_string(),
            entry(Some("GPT-Realtime-2.1"), Some(128_000)),
        );
        table.insert(
            "tencent/hy3-preview".to_string(),
            entry(Some("Hy3 preview"), Some(256_000)),
        );

        let out = enrich_with_catalog(
            vec!["gpt-reserve".to_string(), "codex-auto-review".to_string()],
            &table,
        );
        assert_eq!(out[0].display_name, None);
        assert_eq!(out[0].context_window, None);
        assert_eq!(out[1].display_name, None);
        assert_eq!(out[1].context_window, None);
    }

    #[test]
    fn still_matches_a_single_token_id_that_is_the_whole_shorter_side() {
        let mut table = BTreeMap::new();
        table.insert("openai/o1".to_string(), entry(Some("o1"), Some(200_000)));
        let out = enrich_with_catalog(vec!["o1".to_string()], &table);
        assert_eq!(out[0].display_name.as_deref(), Some("o1"));
    }

    #[test]
    fn fills_real_gateway_ids_by_tokens() {
        // 用户网关里真实出现过的 id：后缀 `-thinking` / `-high` / `-medium` 要被忽略。
        let mut table = BTreeMap::new();
        table.insert(
            "anthropic/claude-opus-4-6".to_string(),
            entry(Some("Claude Opus 4.6"), Some(200_000)),
        );
        table.insert(
            "google/gemini-3.8-flash".to_string(),
            entry(Some("Gemini 3.8 Flash"), Some(1_000_000)),
        );
        table.insert(
            "openai/gpt-oss-120b".to_string(),
            entry(Some("GPT-OSS 120B"), Some(128_000)),
        );

        let out = enrich_with_catalog(
            vec![
                "claude-opus-4-6-thinking".to_string(),
                "gemini-3.8-flash-high".to_string(),
                "gpt-oss-120b-medium".to_string(),
            ],
            &table,
        );
        assert_eq!(out[0].context_window, Some(200_000));
        assert_eq!(out[1].context_window, Some(1_000_000));
        assert_eq!(out[2].context_window, Some(128_000));
    }

    #[test]
    fn treats_zero_context_as_missing() {
        // 目录里有少量 0 占位：压扁时（flatten_catalog）就被当作没填。
        let mut raw = CatalogRaw::new();
        let mut models = BTreeMap::new();
        models.insert(
            "m".to_string(),
            raw_model(Some("M"), Some(0), Some(0), None),
        );
        raw.insert("vendor".to_string(), models);
        let table = flatten_catalog(raw);
        let out = enrich_with_catalog(vec!["m".to_string()], &table);
        assert_eq!(out[0].context_window, None);
        assert_eq!(out[0].max_output, None);
        assert_eq!(out[0].display_name.as_deref(), Some("M"));
    }
}
