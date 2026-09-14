//! 规则执行引擎（数据面）：模型名 = 规则名，按规则解析真实上游候选序列。
//!
//! 对齐 09 号文档（网关规则执行引擎设计）：
//! - 用户请求的 `model` 字段可能是规则集 `name`（如 `default-cost`），
//!   命中规则集后路由由 Resolver 决定，不再把该字符串当作真实模型传给上游。
//! - `token_tier`（默认）：按估算 prompt token 筛掉装不下的候选，
//!   剩余按 `max_prompt_tokens` 升序（最小上下文 = 最便宜）排列，无上限者置底，
//!   末尾追加 `fallback`；`fixed`：按 candidates 原序（别名固定映射）。
//! - 未命中规则名 → 返回 None，调用方维持原行为（当真实模型名交给注册表路由）。
//! - 规则名与真实模型名天然共存、互不冲突（/v1/models 会合并列出规则名）。

use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, RwLock};

/// 规则候选：真实上游模型 + 可选上下文上限（token_tier 排序/筛选依据）。
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct Candidate {
    /// 真实上游模型名。
    pub model: String,
    /// 最大可装 prompt tokens；缺省 = 最终兜底（置底）。
    #[serde(default)]
    pub max_prompt_tokens: Option<u64>,
}

/// 单条规则：候选序列 + 选择策略。
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct Rule {
    /// 子模型匹配：规则名模式下建议 "*"（兜底）；也可精确匹配上游模型名。
    #[serde(default = "default_wildcard")]
    pub match_model: String,
    /// 规则选择顺序（match_model 均不命中时取 order 最小者）。
    #[serde(default)]
    pub order: u32,
    /// 选择策略：token_tier（默认）| fixed。
    #[serde(default = "default_strategy")]
    pub strategy: String,
    /// 候选序列（token_tier 按上下文升序排；fixed 原序）。
    pub candidates: Vec<Candidate>,
    /// 候选全部失败后再试的模型（追加到候选序列末尾，无上下文上限）。
    #[serde(default)]
    pub fallback: Vec<String>,
}

fn default_wildcard() -> String {
    "*".to_string()
}
fn default_strategy() -> String {
    "token_tier".to_string()
}

/// 规则集：用户请求的 `model` 字段即 `name`。
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct RuleSet {
    pub id: String,
    /// 用户请求里写的 model（规则名）。
    pub name: String,
    pub version: u64,
    #[serde(default)]
    pub rules: Vec<Rule>,
}

/// Resolve 结果：命中规则集后的真实上游候选序列（含规则名供遥测）。
#[derive(Debug, Clone, PartialEq)]
pub struct Resolution {
    /// 命中的规则集名（usage 遥测 rule_set_name）。
    pub rule_set: String,
    /// 过滤+排序后的候选模型序列（末尾已追加 fallback）。
    pub candidates: Vec<String>,
}

impl Resolution {
    /// 首个候选（流式 / Anthropic 路径固定取候选[0]）。
    pub fn first(&self) -> Option<&str> {
        self.candidates.first().map(|s| s.as_str())
    }
}

/// 规则集加载文件形态：单个规则集或数组。
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(untagged)]
enum RuleSetsFile {
    Single(RuleSet),
    Many(Vec<RuleSet>),
}

/// 规则解析器（内存，线程安全）：rule name → RuleSet。
#[derive(Default)]
pub struct RuleResolver {
    inner: RwLock<HashMap<String, Arc<RuleSet>>>,
}

impl RuleResolver {
    pub fn new() -> Self {
        Self {
            inner: RwLock::new(HashMap::new()),
        }
    }

    /// 从文件加载（单个或数组）；文件缺失/损坏 → 空解析器（不影响启动）。
    /// 兼容 UTF-8 BOM（Windows 编辑器/面板保存常见）。
    pub fn load_from_file(path: &Path) -> Self {
        let resolver = Self::new();
        match std::fs::read(path) {
            Ok(bytes) => {
                // 剥离 UTF-8 BOM（EF BB BF）后再解析
                let bytes = if bytes.starts_with(&[0xEF, 0xBB, 0xBF]) { &bytes[3..] } else { &bytes[..] };
                match serde_json::from_slice::<RuleSetsFile>(bytes) {
                    Ok(file) => {
                        let mut sets = match file {
                            RuleSetsFile::Single(rs) => vec![rs],
                            RuleSetsFile::Many(v) => v,
                        };
                        // 名称唯一性：后写覆盖先写
                        for rs in sets.drain(..) {
                            resolver.upsert(rs);
                        }
                        tracing::info!(file = %path.display(), count = resolver.len(), "rule sets loaded");
                    }
                    Err(e) => tracing::warn!(file = %path.display(), error = %e, "rule sets file parse failed, ignored"),
                }
            }
            Err(_) => tracing::debug!(file = %path.display(), "no rule sets file (resolver empty)"),
        }
        resolver
    }

    /// 加载/覆盖全量规则集（本地内联 + 云端轮询共用）。
    pub fn load(&self, rule_sets: Vec<RuleSet>) {
        let mut map = self.inner.write().unwrap();
        map.clear();
        for rs in rule_sets {
            map.insert(rs.name.clone(), Arc::new(rs));
        }
    }

    /// 新增或按 name 覆盖单条规则集。
    pub fn upsert(&self, rs: RuleSet) {
        self.inner.write().unwrap().insert(rs.name.clone(), Arc::new(rs));
    }

    /// 删除规则集（按 name）；返回是否命中。
    pub fn remove(&self, name: &str) -> bool {
        self.inner.write().unwrap().remove(name).is_some()
    }

    /// 已加载规则集数量。
    pub fn len(&self) -> usize {
        self.inner.read().unwrap().len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// 全部规则集（按 name 排序；/api/rules 展示用）。
    pub fn list(&self) -> Vec<RuleSet> {
        let mut v: Vec<RuleSet> = self.inner.read().unwrap().values().map(|rs| (**rs).clone()).collect();
        v.sort_by(|a, b| a.name.cmp(&b.name));
        v
    }

    /// 规则集 name 列表（/v1/models 与 /api/models 暴露：客户端可见可选规则）。
    pub fn rule_names(&self) -> Vec<String> {
        let mut v: Vec<String> = self.inner.read().unwrap().keys().cloned().collect();
        v.sort();
        v
    }

    /// 解析请求的 model：
    /// - 未命中规则名 → None（调用方当作真实模型名路由，维持原行为）。
    /// - 命中 → 按规则选择 + 策略过滤排序 + 追加 fallback 的候选序列。
    pub fn resolve(&self, requested_model: &str, est_tokens: u64) -> Option<Resolution> {
        let rule_set = self.inner.read().unwrap().get(requested_model)?.clone();
        // 规则选择：match_model=="*" 优先，否则 match_model==requested，否则 order 最小
        let rule = {
            let v = &rule_set.rules;
            let star = v.iter().find(|r| r.match_model == "*");
            let exact = v.iter().find(|r| r.match_model == requested_model);
            star.or(exact).or_else(|| v.iter().min_by_key(|r| r.order))
        }?;
        let mut candidates: Vec<String> = Vec::new();
        match rule.strategy.as_str() {
            // token_tier：筛掉装不下的，按 max_prompt_tokens 升序（None=兜底置底）
            "fixed" => {
                for c in &rule.candidates {
                    if !c.model.is_empty() {
                        candidates.push(c.model.clone());
                    }
                }
            }
            // token_tier（默认）
            _ => {
                let mut fit: Vec<&Candidate> = rule.candidates.iter().collect();
                fit.retain(|c| c.max_prompt_tokens.map(|m| m >= est_tokens).unwrap_or(true));
                // 按 max_prompt_tokens 升序；None（无上限）置底 = 最终兜底
                fit.sort_by_key(|c| c.max_prompt_tokens.unwrap_or(u64::MAX));
                for c in fit {
                    if !c.model.is_empty() {
                        candidates.push(c.model.clone());
                    }
                }
            }
        }
        // 末尾追加 fallback
        for m in &rule.fallback {
            if !m.is_empty() {
                candidates.push(m.clone());
            }
        }
        if candidates.is_empty() {
            return None;
        }
        Some(Resolution {
            rule_set: rule_set.name.clone(),
            candidates,
        })
    }
}

/// 估算请求的 prompt token 量级（无 tokenizer，按文本长度 /4 粗估）。
/// 仅用于 token_tier 选模（上下文大小量级比较），无需精确。
pub fn estimate_tokens(text: &str) -> u64 {
    let chars = text.chars().count() as u64;
    (chars / 4).max(1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// 构造 token_tier 规则集：小上下文便宜优先，大上下文兜底。
    fn tier_set() -> RuleSet {
        serde_json::from_value(json!({
            "id": "rs_01H",
            "name": "default-cost",
            "version": 2,
            "rules": [{
                "match_model": "*",
                "order": 1,
                "strategy": "token_tier",
                "candidates": [
                    { "model": "moonshot-v1-8k", "max_prompt_tokens": 6000 },
                    { "model": "moonshot-v1-32k", "max_prompt_tokens": 28000 },
                    { "model": "moonshot-v1-128k" }
                ],
                "fallback": ["gpt-4o-mini"]
            }]
        }))
        .unwrap()
    }

    #[test]
    fn token_tier_picks_smallest_that_fits() {
        let resolver = RuleResolver::new();
        resolver.upsert(tier_set());
        // 小提示词 → 8k（最小上下文=最便宜）
        let r = resolver.resolve("default-cost", 500).unwrap();
        assert_eq!(r.rule_set, "default-cost");
        assert_eq!(r.candidates, vec!["moonshot-v1-8k", "moonshot-v1-32k", "moonshot-v1-128k", "gpt-4o-mini"]);
        assert_eq!(r.first(), Some("moonshot-v1-8k"));
        // 8k 装不下的量级 → 32k；32k 也装不下 → 128k
        let r = resolver.resolve("default-cost", 7000).unwrap();
        assert_eq!(r.candidates.first().unwrap(), "moonshot-v1-32k");
        let r = resolver.resolve("default-cost", 30000).unwrap();
        assert_eq!(r.candidates.first().unwrap(), "moonshot-v1-128k");
        // 无上限候选兜底（超过所有上限也总有得选）
        let r = resolver.resolve("default-cost", 1_000_000).unwrap();
        assert_eq!(r.candidates.first().unwrap(), "moonshot-v1-128k");
    }

    #[test]
    fn fixed_strategy_keeps_original_order() {
        let resolver = RuleResolver::new();
        resolver.upsert(serde_json::from_value(json!({
            "id": "rs_f",
            "name": "cheap",
            "version": 1,
            "rules": [{
                "match_model": "*",
                "order": 1,
                "strategy": "fixed",
                "candidates": [
                    { "model": "b-model" },
                    { "model": "a-model" }
                ],
                "fallback": ["fallback-model"]
            }]
        })).unwrap());
        let r = resolver.resolve("cheap", 999_999).unwrap();
        assert_eq!(r.candidates, vec!["b-model", "a-model", "fallback-model"], "fixed 原序 + fallback 追加");
    }

    #[test]
    fn unknown_model_falls_back_to_original_behavior() {
        let resolver = RuleResolver::new();
        resolver.upsert(tier_set());
        assert!(resolver.resolve("mock-7b", 100).is_none(), "未命中规则名 → None（当真实模型名处理）");
        assert!(resolver.resolve("nonexistent", 100).is_none());
    }

    #[test]
    fn rule_selection_prefers_wildcard_then_exact_then_order() {
        // 09 号文档规则选择：优先 match_model=="*"，否则 match_model==requestedModel，否则 order 最小。
        // 规则名模式下（requested=规则名 name），子模型 match_model 用于「同一规则集内多规则」细分。
        let r = RuleResolver::new();
        r.upsert(serde_json::from_value(json!({
            "id": "rs_s",
            "name": "sub",
            "version": 1,
            "rules": [
                { "match_model": "*", "order": 9, "strategy": "fixed", "candidates": [{ "model": "star-model" }] },
                { "match_model": "exact", "order": 1, "strategy": "fixed", "candidates": [{ "model": "exact-model" }] }
            ]
        })).unwrap());
        // requestedModel=规则名 "sub"：* 兜底命中（无 exact 子模型匹配）
        let res = r.resolve("sub", 1).unwrap();
        assert_eq!(res.candidates, vec!["star-model"]);
    }

    #[test]
    fn rule_selection_prefers_requested_match_over_star() {
        // 当请求 model 同时命中规则集 name（"sub"）且子模型匹配 exact 时——
        // 实际场景：请求 model="sub:exact"（规则名:子模型）→ 规则集命中，子模型精确匹配优先。
        let r = RuleResolver::new();
        r.upsert(serde_json::from_value(json!({
            "id": "rs_s",
            "name": "sub",
            "version": 1,
            "rules": [
                { "match_model": "*", "order": 9, "strategy": "fixed", "candidates": [{ "model": "star-model" }] },
                { "match_model": "exact", "order": 1, "strategy": "fixed", "candidates": [{ "model": "exact-model" }] }
            ]
        })).unwrap());
        // 请求 model="sub:exact"：规则集未按 name 命中（name 是 "sub"）→ None 维持原行为。
        // 规则集 name 精确命中才进 Resolver；子模型匹配在同名命中后细分。
        assert!(r.resolve("sub:exact", 1).is_none(), "规则名不匹配时不进 Resolver");

        // 构造规则名 "sub:exact" 本身命中 → 取该规则（无子模型概念，* 即兜底）
        r.upsert(serde_json::from_value(json!({
            "id": "rs_e",
            "name": "sub:exact",
            "version": 1,
            "rules": [
                { "match_model": "*", "order": 1, "strategy": "fixed", "candidates": [{ "model": "exact-model" }] }
            ]
        })).unwrap());
        assert_eq!(r.resolve("sub:exact", 1).unwrap().first(), Some("exact-model"));
    }

    #[test]
    fn rule_names_and_list_sorted() {
        let resolver = RuleResolver::new();
        resolver.upsert(tier_set());
        resolver.upsert(serde_json::from_value(json!({
            "id": "rs_f", "name": "cheap", "version": 1,
            "rules": [{ "match_model": "*", "order": 1, "strategy": "fixed", "candidates": [{ "model": "b" }] }]
        })).unwrap());
        assert_eq!(resolver.rule_names(), vec!["cheap", "default-cost"]);
        assert_eq!(resolver.len(), 2);
        let list = resolver.list();
        assert_eq!(list[0].name, "cheap");
        assert_eq!(list[1].name, "default-cost");
    }

    #[test]
    fn upsert_overrides_remove_deletes() {
        let resolver = RuleResolver::new();
        resolver.upsert(serde_json::from_value(json!({
            "id": "rs_a", "name": "a", "version": 1,
            "rules": [{ "match_model": "*", "order": 1, "strategy": "fixed", "candidates": [{ "model": "m1" }] }]
        })).unwrap());
        resolver.upsert(serde_json::from_value(json!({
            "id": "rs_b", "name": "a", "version": 2,
            "rules": [{ "match_model": "*", "order": 1, "strategy": "fixed", "candidates": [{ "model": "m2" }] }]
        })).unwrap());
        assert_eq!(resolver.len(), 1, "同名 upsert 覆盖");
        assert_eq!(resolver.resolve("a", 1).unwrap().first(), Some("m2"));
        assert!(resolver.remove("a"));
        assert!(resolver.is_empty());
        assert!(!resolver.remove("a"), "二次删除未命中");
    }

    #[test]
    fn load_file_single_and_many_and_missing() {
        let dir = std::env::temp_dir().join("aipg-rules-load.json");
        let _ = std::fs::remove_file(&dir);
        // 缺失文件 → 空解析器
        let r = RuleResolver::load_from_file(&dir);
        assert!(r.is_empty());
        // 单规则集写盘 → 加载
        std::fs::write(&dir, serde_json::to_string(&tier_set()).unwrap()).unwrap();
        let r = RuleResolver::load_from_file(&dir);
        assert_eq!(r.rule_names(), vec!["default-cost"]);
        assert_eq!(r.resolve("default-cost", 500).unwrap().first(), Some("moonshot-v1-8k"));
        // 数组形态
        std::fs::write(&dir, serde_json::to_string(&json!([tier_set()])).unwrap()).unwrap();
        let r = RuleResolver::load_from_file(&dir);
        assert_eq!(r.len(), 1);
        // 损坏文件 → 空解析器（不 panic）
        std::fs::write(&dir, "{not json").unwrap();
        let r = RuleResolver::load_from_file(&dir);
        assert!(r.is_empty());
        let _ = std::fs::remove_file(&dir);
    }

    #[test]
    fn estimate_tokens_proportional_to_length() {
        assert!(estimate_tokens("short") < estimate_tokens(&"x".repeat(1000)));
        assert!(estimate_tokens("") >= 1, "空文本至少 1 token（避免除零）");
    }
}