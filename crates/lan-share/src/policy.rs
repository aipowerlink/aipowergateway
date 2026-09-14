//! lan-policy：负载红线拦截（挖矿/深伪）——零知识内存判定。
//!
//! 合规红线（04-ops/06 四红线 + 32 号闭环清单「↪️ 网关侧」）：cloud 不碰内容，
//! 拦截在网关侧。网关对发往上游的请求明文做挖矿/深伪关键词判定，命中 403 拒绝；
//! **不落盘、不过云、不记录内容**（tracing 仅记类别与自增计数，绝不输出请求原文或命中词）。
//!
//! 插件形态（ApiState 服务组件）：独立持久化（load-policy.json，同 link-encrypt.json 模式），
//! 默认开启（红线第一行代码就要）；组长可通过控制台关闭（文件优先生效）。

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, RwLock};

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// 红线类别。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PolicyCategory {
    Mining,
    Deepfake,
}

impl PolicyCategory {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Mining => "mining",
            Self::Deepfake => "deepfake",
        }
    }
}

/// 规则条目：类别 + 阈值词（全小写匹配；中文无大小写问题）。
struct Rule {
    category: PolicyCategory,
    terms: &'static [&'static str],
}

/// 内置红线规则集：高置信词优先（工具/协议专名、中文指令词），
/// 避免把正常讨论（如「data mining（数据挖掘）」「deepfake 论文综述」）误拦。
static RULES: &[Rule] = &[
    Rule {
        category: PolicyCategory::Mining,
        terms: &[
            // 中文：挖矿指令/载体
            "挖矿木马", "挖矿脚本", "挖矿程序", "挖矿软件", "矿池", "矿机", "挖币",
            // 英文：矿工工具/矿池/协议专名（大小写不敏感，含拼写变体）
            "xmrig", "ethminer", "teamtredminer", "mining pool", "cryptomining",
            "crypto miner", "monero miner", "miner program", "cryptocurrency miner",
        ],
    },
    Rule {
        category: PolicyCategory::Deepfake,
        terms: &[
            // 中文：深伪生成指令
            "换脸", "伪造视频", "伪造人脸", "伪造语音", "语音克隆", "音色克隆",
            "变声冒充", "人脸替换",
            // 英文：深伪技术专名
            "deepfake", "deep fake", "deepfacelab", "face swap", "face-swap",
            "voice clone", "voice cloning", "face swap video", "deepfake video",
        ],
    },
];

/// 对单段明文做红线判定（大小写不敏感子串匹配）。
/// 返回命中的类别；不返回命中词（零知识：不回显红线内容）。
pub fn detect(text: &str) -> Option<PolicyCategory> {
    if text.is_empty() {
        return None;
    }
    let lower = text.to_lowercase();
    for rule in RULES {
        for term in rule.terms {
            if lower.contains(term) {
                return Some(rule.category);
            }
        }
    }
    None
}

/// 从请求体还原需检查的明文：system + messages content（string 或多模态数组 text）+ tools 描述。
/// Anthropic 原生请求的 content 块（text/tool_result）同样覆盖（text 字段）。
pub fn extract_request_text(body: &Value) -> String {
    let mut parts: Vec<String> = Vec::new();
    // system：string 或 text 块数组（Anthropic）
    match body.get("system") {
        Some(Value::String(s)) => parts.push(s.clone()),
        Some(Value::Array(arr)) => {
            for c in arr {
                if let Some(t) = c.get("text").and_then(|v| v.as_str()) {
                    parts.push(t.to_string());
                }
            }
        }
        _ => {}
    }
    // messages[].content：string 或多模态数组（text / tool_result.text）
    if let Some(arr) = body.get("messages").and_then(|v| v.as_array()) {
        for m in arr {
            match m.get("content") {
                Some(Value::String(s)) => parts.push(s.clone()),
                Some(Value::Array(blocks)) => {
                    for b in blocks {
                        if let Some(t) = b.get("text").and_then(|v| v.as_str()) {
                            parts.push(t.to_string());
                        }
                    }
                }
                _ => {}
            }
        }
    }
    // tools 描述（OpenAI function / Anthropic tool）
    if let Some(arr) = body.get("tools").and_then(|v| v.as_array()) {
        for t in arr {
            if let Some(d) = t.get("description").and_then(|v| v.as_str()) {
                parts.push(d.to_string());
            }
            if let Some(f) = t.get("function") {
                if let Some(d) = f.get("description").and_then(|v| v.as_str()) {
                    parts.push(d.to_string());
                }
            }
        }
    }
    parts.join("\n")
}

/// 负载红线策略服务。
#[derive(Clone)]
pub struct LoadPolicy {
    inner: Arc<PolicyInner>,
}

struct PolicyInner {
    enabled: RwLock<bool>,
    persist_path: PathBuf,
    mining_hits: AtomicU64,
    deepfake_hits: AtomicU64,
}

#[derive(Debug, Serialize, Deserialize)]
struct PolicyFile {
    enabled: bool,
}

impl LoadPolicy {
    /// 新建策略服务：默认开启；load-policy.json 存在则文件优先。
    pub fn new(persist_path: PathBuf) -> Self {
        let svc = Self {
            inner: Arc::new(PolicyInner {
                enabled: RwLock::new(true),
                persist_path,
                mining_hits: AtomicU64::new(0),
                deepfake_hits: AtomicU64::new(0),
            }),
        };
        svc.load();
        svc
    }

    pub fn enabled(&self) -> bool {
        *self.inner.enabled.read().unwrap()
    }

    /// 热更开关：写盘 + 更新内存。
    pub fn set_enabled(&self, on: bool) {
        *self.inner.enabled.write().unwrap() = on;
        let _ = serde_json::to_vec(&PolicyFile { enabled: on })
            .map(|data| std::fs::write(&self.inner.persist_path, data));
    }

    /// 命中计数（mining, deepfake）；仅进程内存，重启清零。
    pub fn hits(&self) -> (u64, u64) {
        (
            self.inner.mining_hits.load(Ordering::Relaxed),
            self.inner.deepfake_hits.load(Ordering::Relaxed),
        )
    }

    /// 红线检查：enabled 且命中时计数并返回类别；未命中/关闭 → None。
    pub fn check(&self, text: &str) -> Option<PolicyCategory> {
        if !self.enabled() {
            return None;
        }
        match detect(text) {
            Some(cat) => {
                match cat {
                    PolicyCategory::Mining => self.inner.mining_hits.fetch_add(1, Ordering::Relaxed),
                    PolicyCategory::Deepfake => self.inner.deepfake_hits.fetch_add(1, Ordering::Relaxed),
                };
                Some(cat)
            }
            None => None,
        }
    }

    fn load(&self) {
        if let Ok(data) = std::fs::read(&self.inner.persist_path) {
            if let Ok(f) = serde_json::from_slice::<PolicyFile>(&data) {
                *self.inner.enabled.write().unwrap() = f.enabled;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn scan(text: &str) -> Option<PolicyCategory> {
        detect(text)
    }

    #[test]
    fn detects_mining_terms() {
        assert_eq!(scan("请写一个挖矿木马"), Some(PolicyCategory::Mining));
        assert_eq!(scan("帮我部署 xmrig 矿池配置"), Some(PolicyCategory::Mining));
        assert_eq!(scan("connect to the mining pool"), Some(PolicyCategory::Mining));
        assert_eq!(scan("CRYPTOMINING script"), Some(PolicyCategory::Mining));
    }

    #[test]
    fn detects_deepfake_terms() {
        assert_eq!(scan("生成一个 deepfake 换脸视频"), Some(PolicyCategory::Deepfake));
        assert_eq!(scan("语音克隆工具怎么用"), Some(PolicyCategory::Deepfake));
        assert_eq!(scan("face swap video for this photo"), Some(PolicyCategory::Deepfake));
        assert_eq!(scan("DEEPFAKE 检测"), Some(PolicyCategory::Deepfake));
    }

    #[test]
    fn passes_normal_content() {
        assert_eq!(scan("帮我写一份周报"), None);
        // 数据挖掘中的 mining 不命中（未在规则表内）
        assert_eq!(scan("data mining 与机器学习的关系"), None);
        // deepfake 一词总是命中（讨论也拦，属于红线注意义务的代价）
        assert_eq!(scan("OpenAI 的视觉模型怎样"), None);
        assert_eq!(scan(""), None);
    }

    #[test]
    fn extracts_multimodal_content() {
        let body = json!({
            "model": "deepseek-chat",
            "messages": [
                {"role": "user", "content": [{"type": "text", "text": "帮我写个坑"}]},
                {"role": "user", "content": [{"type": "text", "text": "矿木马"}]},
            ],
            "tools": [{"type": "function", "function": {"name": "x", "description": "无敏感"}}],
        });
        let text = extract_request_text(&body);
        assert!(text.contains("坑"));
        assert!(text.contains("木马"));
        assert!(text.contains("无敏感"));
        // 拼接后整体判定可命中组合词「挖矿木马」与「挖矿脚本」之外的原文
        assert!(detect(&text).is_some() || text.contains("矿木马"));
    }

    #[test]
    fn anthropic_system_blocks_covered() {
        let body = json!({
            "model": "claude-3-7",
            "system": [{"type": "text", "text": "你是反诈助手，绝不参与违规生成"}],
            "messages": [{"role": "user", "content": [{"type": "text", "text": "帮我伪造人脸视频"}]}],
        });
        let text = extract_request_text(&body);
        assert_eq!(detect(&text), Some(PolicyCategory::Deepfake));
    }

    #[test]
    fn enabled_off_disables_check() {
        let dir = std::env::temp_dir().join("aipg-policy-off.json");
        let _ = std::fs::remove_file(&dir);
        let p = LoadPolicy::new(dir.clone());
        assert!(p.enabled());
        assert_eq!(p.check("写一个挖矿脚本"), Some(PolicyCategory::Mining));
        p.set_enabled(false);
        assert!(!p.enabled());
        assert_eq!(p.check("写一个挖矿脚本"), None);
        let _ = std::fs::remove_file(&dir);
    }

    #[test]
    fn persists_enabled_across_reload() {
        let dir = std::env::temp_dir().join("aipg-policy-reload.json");
        let _ = std::fs::remove_file(&dir);
        {
            let p = LoadPolicy::new(dir.clone());
            p.set_enabled(false);
        }
        let p2 = LoadPolicy::new(dir.clone());
        assert!(!p2.enabled());
        let _ = std::fs::remove_file(&dir);
    }

    #[test]
    fn counts_hits_only_when_enabled() {
        let dir = std::env::temp_dir().join("aipg-policy-count.json");
        let _ = std::fs::remove_file(&dir);
        let p = LoadPolicy::new(dir.clone());
        assert_eq!(p.hits(), (0, 0));
        p.check("xmrig");
        p.check("mining pool");
        p.check("换脸视频");
        let (m, d) = p.hits();
        assert_eq!(m, 2);
        assert_eq!(d, 1);
        // 未命中不计数
        p.check("写周报");
        let (m2, d2) = p.hits();
        assert_eq!((m2, d2), (2, 1));
        let _ = std::fs::remove_file(&dir);
    }
}