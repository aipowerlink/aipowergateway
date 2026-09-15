//! lan-strategy：成员机本地策略摘要缓存（共享者 → 使用者只读镜像）。
//!
//! 场景（策略同步到需求人节点）：组长 `GET /api/strategy/me` 下发只读策略摘要
//! （规则名 + 本机配额 + 拉黑状态），成员机 `--role client` 本地：
//! - `/v1/models` 有缓存时直接返回规则名列表（不跨网请求，App 本地即可见可选规则）；
//! - 请求前预检：已拉黑 → 403、配额已超限 → 429（提前拦截，权威仍在组长）；
//! - 拉取失败保留上次缓存降级（未命中拦截时照常透传组长，组长按真实状态兜底）。
//!
//! 零知识边界（09 号文档 §1）：只同步**规则名**（能力可见），绝不下发真实上游候选模型。
//! 持久化：`data_dir/strategy-cache.json`。

use std::path::PathBuf;
use std::sync::{Arc, RwLock};

use serde::{Deserialize, Serialize};

use crate::gateway::MemberGateway;

/// 配额快照（组长 `/api/strategy/me` 的 `quota` 对象）。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase")]
pub struct QuotaSnapshot {
    /// 配额上限（0 = 不限）。
    pub limit: u64,
    /// 共享者侧累计用量。
    pub used: u64,
}

/// 策略摘要（组长 `/api/strategy/me` 响应体，字段名与组长 JSON 对齐）。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase")]
pub struct StrategySummary {
    /// 策略指纹（规则名+配额+拉黑变化的简单哈希；变化 → 刷新）。
    pub version: u64,
    /// 规则名列表（使用者可见可选规则；零知识：无真实模型候选）。
    pub rules: Vec<String>,
    /// 本机配额。
    pub quota: QuotaSnapshot,
    /// 本机是否被拉黑。
    pub banned: bool,
    /// 本地拉取时间（unix 秒，仅本地字段；组长响应无此字段）。
    #[serde(default)]
    pub fetched_at: u64,
}

/// 本地提前拦截类别。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PreflightBlock {
    /// 已拉黑 → 403（零知识 message，不回显原因细节）。
    Banned,
    /// 配额已超限 → 429（形状与组长 quota_exceeded 一致）。
    QuotaExceeded { limit: u64 },
}

/// 请求前预检（纯函数，便于单测）：缓存摘要 → 拦截决定。
/// 未命中任何拦截 → `None`（请求照常透传组长，权威在组长）。
/// 配额语义与组长 `quota.check` 一致：limit>0 且 used>=limit → 超限。
pub fn preflight_block(summary: &StrategySummary) -> Option<PreflightBlock> {
    if summary.banned {
        return Some(PreflightBlock::Banned);
    }
    let limit = summary.quota.limit;
    if limit > 0 && summary.quota.used >= limit {
        return Some(PreflightBlock::QuotaExceeded { limit });
    }
    None
}

/// 策略摘要缓存（内存 + 持久化，仿 UsageView 模式）。
#[derive(Clone)]
pub struct StrategyCache {
    inner: Arc<RwLock<Option<StrategySummary>>>,
    persist_path: Option<PathBuf>,
}

impl StrategyCache {
    /// 新建缓存；`persist_path` 存在时启动即加载上次拉取结果（失败 → 空，不阻塞启动）。
    pub fn new(persist_path: Option<PathBuf>) -> Self {
        let loaded = persist_path.as_ref().and_then(|p| {
            std::fs::read(p)
                .ok()
                .and_then(|bytes| serde_json::from_slice::<StrategySummary>(&bytes).ok())
        });
        Self {
            inner: Arc::new(RwLock::new(loaded)),
            persist_path,
        }
    }

    pub fn get(&self) -> Option<StrategySummary> {
        self.inner.read().ok().and_then(|g| g.clone())
    }

    /// 更新缓存并持久化（拉取成功时调用；失败不调用 → 天然保留上次缓存降级）。
    pub fn set(&self, summary: StrategySummary) {
        if let Ok(mut g) = self.inner.write() {
            *g = Some(summary.clone());
        }
        if let Some(p) = &self.persist_path {
            if let Ok(bytes) = serde_json::to_vec(&summary) {
                let _ = std::fs::write(p, bytes);
            }
        }
    }

    /// 当前规则名列表（无缓存 / 组长无规则 → 空；调用方据此决定本地直返或透传）。
    pub fn rule_names(&self) -> Vec<String> {
        self.get().map(|s| s.rules).unwrap_or_default()
    }

    /// 预检（无缓存 → None 透传组长，权威兜底）。
    pub fn preflight(&self) -> Option<PreflightBlock> {
        self.get().and_then(|s| preflight_block(&s))
    }
}

/// 经成员 gateway 拉取策略摘要：先免密换本机 token，再 Bearer `GET /api/strategy/me`。
/// `machine_name` 即本机成员身份（组长 auth.issue 幂等：同机复用同一 token）。
pub async fn fetch_strategy(gateway: &MemberGateway, machine_name: &str) -> Result<StrategySummary, String> {
    // 1) 免密换本机 token（/auth/token 为豁免端点，明文可达；组长侧幂等）
    let body = serde_json::json!({ "machineName": machine_name, "displayName": machine_name });
    let body = serde_json::to_vec(&body).map_err(|e| e.to_string())?;
    let (status, bytes) = gateway.proxy("/auth/token", None, Some(body)).await?;
    if status != 200 {
        return Err(format!("auth token status {status}"));
    }
    let v: serde_json::Value = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
    let token = v
        .get("token")
        .and_then(|t| t.as_str())
        .ok_or_else(|| "auth token missing in response".to_string())?;
    // 2) Bearer 拉策略摘要（跨网络深链时经链路加密往返，gateway 自动协商）
    let auth = format!("Bearer {token}");
    let (status, bytes) = gateway.proxy("/api/strategy/me", Some(&auth), None).await?;
    if status != 200 {
        return Err(format!("strategy status {status}"));
    }
    let mut summary: StrategySummary = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
    summary.fetched_at = now_unix_secs();
    Ok(summary)
}

fn now_unix_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> StrategySummary {
        StrategySummary {
            version: 42,
            rules: vec!["deepseek-r1".to_string(), "moonshot-lite".to_string()],
            quota: QuotaSnapshot { limit: 1000, used: 0 },
            banned: false,
            fetched_at: 0,
        }
    }

    #[test]
    fn preflight_allows_within_quota() {
        let s = sample();
        assert_eq!(preflight_block(&s), None, "未超限不拦截");
    }

    #[test]
    fn preflight_blocks_banned() {
        let mut s = sample();
        s.banned = true;
        assert_eq!(preflight_block(&s), Some(PreflightBlock::Banned));
    }

    #[test]
    fn preflight_blocks_at_quota_boundary() {
        let mut s = sample();
        s.quota.used = 1000;
        assert_eq!(preflight_block(&s), Some(PreflightBlock::QuotaExceeded { limit: 1000 }));
    }

    #[test]
    fn preflight_zero_limit_never_blocks() {
        let mut s = sample();
        s.quota.limit = 0;
        s.quota.used = 999999;
        assert_eq!(preflight_block(&s), None, "limit=0 不限量");
    }

    #[test]
    fn cache_persists_across_reload() {
        let p = std::env::temp_dir().join("aipg-strategy-cache-test.json");
        let _ = std::fs::remove_file(&p);
        {
            let c = StrategyCache::new(Some(p.clone()));
            assert!(c.get().is_none(), "首次无缓存");
            c.set(sample());
            assert_eq!(c.rule_names(), sample().rules);
        }
        {
            let c = StrategyCache::new(Some(p.clone()));
            let got = c.get().expect("重启后应从磁盘恢复");
            assert_eq!(got.version, sample().version);
            assert_eq!(got.rules, sample().rules);
            assert_eq!(got.quota.limit, 1000);
        }
        let _ = std::fs::remove_file(&p);
    }

    #[test]
    fn summary_round_trips_leaders_json_shape() {
        // 与组长 /api/strategy/me JSON 形状对齐（camelCase + quota 嵌套）
        let json = serde_json::json!({
            "version": 7,
            "rules": ["deepseek-r1"],
            "quota": { "limit": 500, "used": 120 },
            "banned": false,
        });
        let s: StrategySummary = serde_json::from_value(json).expect("组长响应应可解析");
        assert_eq!(s.version, 7);
        assert_eq!(s.rules, vec!["deepseek-r1"]);
        assert_eq!(s.quota.limit, 500);
        assert_eq!(s.quota.used, 120);
        assert_eq!(s.banned, false);
        assert_eq!(s.fetched_at, 0, "组长无 fetched_at 字段 → 默认 0");
    }
}