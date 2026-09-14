//! Provider 级健康轮询（pair-integration P1，auto-discovery 前置）。
//!
//! `HealthMonitor` 周期探测各后端的 OpenAI 兼容端点（`GET {base_url}/models`，复用
//! `crate::api::probe`），把结果汇入状态机，供注册表「降权/摘除」与面板「四态状态点」消费：
//!
//! - `ok` →（连续失败 ≥ fail_threshold，默认 3）→ `degraded`（路由降权）
//! - `degraded` →（连续失败 ≥ remove_threshold，默认 10）→ `removed`（退出候选序列）
//! - 任一次成功 → 立即回 `ok`（连续失败计数清零）
//!
//! 设计约束：
//! - 状态按 `backend_id` 独立保存（热替换注册表不丢健康状态）
//! - 无轮询条目不 spawn——`ensure_running()` 仅在存在启用条目时才启动调度循环，
//!   循环在全部条目被关闭后自行退出（零开销）
//! - 默认只轮询执行体预设（pair/agent）；官方/自定义后端默认不轮询，
//!   通过 `PUT /api/backends/:id/polling` 显式开启

use std::collections::HashMap;
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant};

use tokio::task::JoinHandle;

use crate::api::{probe, test_target};
use crate::backend_store::BackendStore;

/// 健康等级（四态状态点的来源：ok=绿 / degraded=黄 / removed=红 / untested=灰）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HealthLevel {
    /// 最近一次探测成功（或从未失败）。
    Ok,
    /// 连续失败达到 fail_threshold：仍可路由但降权标记。
    Degraded,
    /// 连续失败达到 remove_threshold：退出候选序列（路由/模型目录剔除）。
    Removed,
}

impl HealthLevel {
    pub fn as_str(&self) -> &'static str {
        match self {
            HealthLevel::Ok => "ok",
            HealthLevel::Degraded => "degraded",
            HealthLevel::Removed => "removed",
        }
    }
}

/// 单后端的健康状态（进程内存；随轮询/探活刷新）。
#[derive(Debug, Clone)]
pub struct ProviderState {
    pub level: HealthLevel,
    /// 连续失败次数（成功清零）。
    pub failures: u32,
    /// 最近一次成功的延迟（ms）。
    pub latency_ms: Option<u64>,
    /// 最近一次失败原因。
    pub last_error: Option<String>,
}

impl ProviderState {
    /// 状态机推进：ok → 清零；fail → 计数并在阈值处降级/摘除。
    pub fn observe(&mut self, ok: bool, latency_ms: Option<u64>, error: Option<String>, fail_threshold: u32, remove_threshold: u32) {
        if ok {
            self.level = HealthLevel::Ok;
            self.failures = 0;
            self.latency_ms = latency_ms;
            self.last_error = None;
        } else {
            self.failures += 1;
            self.level = if self.failures >= remove_threshold {
                HealthLevel::Removed
            } else if self.failures >= fail_threshold {
                HealthLevel::Degraded
            } else {
                HealthLevel::Ok
            };
            self.last_error = error;
        }
    }
}

/// 轮询配置（PUT /api/backends/:id/polling；未显式配置时执行体预设用默认值）。
#[derive(Debug, Clone, Copy)]
pub struct PollConfig {
    pub enabled: bool,
    /// 轮询间隔（秒）；默认 15。
    pub poll_interval_secs: u64,
    /// 连续失败多少次进入 degraded；默认 3。
    pub fail_threshold: u32,
    /// 连续失败多少次进入 removed；默认 10。
    pub remove_threshold: u32,
}

impl Default for PollConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            poll_interval_secs: 15,
            fail_threshold: 3,
            remove_threshold: 10,
        }
    }
}

/// 健康监控器：状态表 + 轮询配置 + 调度循环。
pub struct HealthMonitor {
    states: Arc<RwLock<HashMap<String, ProviderState>>>,
    configs: Arc<RwLock<HashMap<String, PollConfig>>>,
    /// 调度循环句柄（None = 未运行/已退出）。
    task: Mutex<Option<JoinHandle<()>>>,
}

impl Default for HealthMonitor {
    fn default() -> Self {
        Self {
            states: Arc::new(RwLock::new(HashMap::new())),
            configs: Arc::new(RwLock::new(HashMap::new())),
            task: Mutex::new(None),
        }
    }
}

impl HealthMonitor {
    pub fn new() -> Self {
        Self::default()
    }

    /// 状态表（注册表 route/models_catalog 通过该 Arc 读降权/摘除）。
    pub fn states(&self) -> Arc<RwLock<HashMap<String, ProviderState>>> {
        self.states.clone()
    }

    /// 读取某后端轮询配置（显式配置优先；执行体预设回落默认开启，其他回落关闭）。
    pub fn config(&self, id: &str, is_executor: bool) -> PollConfig {
        self.configs.read().unwrap().get(id).copied().unwrap_or_else(|| {
            if is_executor { PollConfig::default() } else { PollConfig { enabled: false, ..PollConfig::default() } }
        })
    }

    /// 更新轮询配置（PUT polling）。
    pub fn set_config(&self, id: &str, cfg: PollConfig) {
        self.configs.write().unwrap().insert(id.to_string(), cfg);
    }

    /// 清除某后端状态与配置（DELETE backend 后调用）。
    pub fn remove(&self, id: &str) {
        self.states.write().unwrap().remove(id);
        self.configs.write().unwrap().remove(id);
    }

    /// 是否存在启用轮询的条目（executor 预设回落默认开启 → 也计入）。
    fn has_enabled(&self, store: &BackendStore) -> bool {
        store.list().iter().any(|e| {
            let is_executor = crate::backend::is_executor_provider(&e.provider);
            self.config(&e.backend_id(), is_executor).enabled
        })
    }

    /// 确保调度循环在跑：无启用条目（或已运行）时不 spawn——零开销。
    pub fn ensure_running(self: &Arc<Self>, store: Arc<BackendStore>) {
        if self.task.lock().unwrap().is_some() {
            return;
        }
        if !self.has_enabled(&store) {
            return;
        }
        let this = self.clone();
        let store_clone = store.clone();
        let handle = tokio::spawn(async move {
            this.run(store_clone).await;
        });
        *self.task.lock().unwrap() = Some(handle);
    }

    /// 调度循环：每 1s tick，对「到间隔」的启用条目探活并推进状态机；
    /// 全部条目关闭后自行退出（句柄置 None，后续 PUT 可重新拉起）。
    async fn run(self: Arc<Self>, store: Arc<BackendStore>) {
        let mut last_poll: HashMap<String, Instant> = HashMap::new();
        tracing::info!("health monitor loop started");
        loop {
            // 收集启用条目的后端 id（executor 回落默认开启同样计入）
            let enabled: Vec<String> = store.list().iter()
                .filter(|e| {
                    let is_executor = crate::backend::is_executor_provider(&e.provider);
                    self.config(&e.backend_id(), is_executor).enabled
                })
                .map(|e| e.backend_id())
                .collect();
            if enabled.is_empty() {
                break;
            }
            let entries = store.list();
            for id in &enabled {
                let entry = match entries.iter().find(|e| e.backend_id() == *id) {
                    Some(e) => e.clone(),
                    None => continue, // 条目已删除（配置残留可能未清）→ 跳过
                };
                let is_executor = crate::backend::is_executor_provider(&entry.provider);
                let cfg = self.config(id, is_executor);
                let due = match last_poll.get(id) {
                    None => true,
                    Some(t) => t.elapsed() >= Duration::from_secs(cfg.poll_interval_secs.max(1)),
                };
                if !due {
                    continue;
                }
                last_poll.insert(id.clone(), Instant::now());
                let (ok, latency, error) = match test_target(&entry) {
                    Ok(Some(target)) => match probe(&target).await {
                        Ok(out) => (true, Some(out.latency_ms), None),
                        Err(msg) => (false, None, Some(msg)),
                    },
                    Ok(None) => (true, None, None), // mock：本地直通视为健康
                    Err(msg) => (false, None, Some(msg)),
                };
                // 写入前复查配置：probe 期间可能被 PUT 关闭/变更——避免旧 tick 收官把状态写回
                if !self.config(id, is_executor).enabled {
                    continue;
                }
                let mut states = self.states.write().unwrap();
                states.entry(id.clone()).or_insert_with(|| ProviderState {
                    level: HealthLevel::Ok,
                    failures: 0,
                    latency_ms: None,
                    last_error: None,
                }).observe(ok, latency, error, cfg.fail_threshold, cfg.remove_threshold);
            }
            tokio::time::sleep(Duration::from_secs(1)).await;
        }
        *self.task.lock().unwrap() = None;
        tracing::info!("health monitor loop stopped (no enabled entries)");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fresh() -> ProviderState {
        ProviderState { level: HealthLevel::Ok, failures: 0, latency_ms: None, last_error: None }
    }

    fn fail(st: &mut ProviderState, n: u32) {
        for _ in 0..n {
            st.observe(false, None, Some("boom".into()), 3, 10);
        }
    }

    /// 连续失败降权 → 摘除 → 恢复回归（P1 核心状态机）。
    #[test]
    fn state_machine_degrade_remove_recover() {
        let mut st = fresh();
        // 前 2 次失败仍未降级（fail_threshold=3）
        fail(&mut st, 2);
        assert_eq!(st.level, HealthLevel::Ok);
        assert_eq!(st.failures, 2);
        // 第 3 次 → degraded
        fail(&mut st, 1);
        assert_eq!(st.level, HealthLevel::Degraded);
        // 继续失败到 10 → removed（退出候选）
        fail(&mut st, 7);
        assert_eq!(st.level, HealthLevel::Removed);
        assert_eq!(st.failures, 10);
        // 任一次成功 → 立即回 ok，连续失败计数清零
        st.observe(true, Some(42), None, 3, 10);
        assert_eq!(st.level, HealthLevel::Ok);
        assert_eq!(st.failures, 0);
        assert_eq!(st.latency_ms, Some(42));
        assert_eq!(st.last_error, None);
    }

    /// 任一次成功中断连续失败序列（不累积）。
    #[test]
    fn success_resets_failure_run() {
        let mut st = fresh();
        fail(&mut st, 2);
        st.observe(true, Some(10), None, 3, 10);
        assert_eq!(st.failures, 0);
        assert_eq!(st.level, HealthLevel::Ok);
        fail(&mut st, 2);
        assert_eq!(st.level, HealthLevel::Ok, "失败计数应从 0 重新累计");
    }

    /// 无轮询条目的 config 回落（executor 默认开启，其他默认关闭）。
    #[test]
    fn config_defaults_by_executor() {
        let mon = HealthMonitor::new();
        let pair = mon.config("pair-home-1", true);
        assert!(pair.enabled);
        assert_eq!(pair.poll_interval_secs, 15);
        assert_eq!(pair.fail_threshold, 3);
        assert_eq!(pair.remove_threshold, 10);
        let custom = mon.config("ollama", false);
        assert!(!custom.enabled, "非执行体默认不轮询");
    }

    /// set_config 覆盖回落默认。
    #[test]
    fn set_config_overrides_default() {
        let mon = HealthMonitor::new();
        mon.set_config("pair-home-1", PollConfig { enabled: false, poll_interval_secs: 60, fail_threshold: 5, remove_threshold: 20 });
        let cfg = mon.config("pair-home-1", true);
        assert!(!cfg.enabled);
        assert_eq!(cfg.poll_interval_secs, 60);
        assert_eq!(cfg.fail_threshold, 5);
        assert_eq!(cfg.remove_threshold, 20);
    }
}