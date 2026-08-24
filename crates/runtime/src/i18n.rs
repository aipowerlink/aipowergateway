//! i18n：轻量多语言（zh-CN / en），运行时切换 + 偏好持久化。
//! 参考实现 aitokengateway/internal/i18n（JSON bundle 模式）与 DSH locale。

use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, RwLock};

/// 语言键。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Lang {
    Zh,
    En,
}

impl Lang {
    pub fn code(&self) -> &'static str {
        match self { Lang::Zh => "zh-CN", Lang::En => "en" }
    }

    /// 从系统 locale 推断（默认英文——面向全球用户；仅显式 zh 环境默认中文）。
    pub fn from_system() -> Self {
        let locale = std::env::var("LANG")
            .or_else(|_| std::env::var("LC_ALL"))
            .unwrap_or_default();
        let l = locale.to_lowercase();
        if l.starts_with("zh") {
            Lang::Zh
        } else {
            // 默认英文（全球用户基线）
            Lang::En
        }
    }
}

/// 本地化管理器。
#[derive(Clone)]
pub struct I18n {
    /// 当前语言。
    lang: Arc<RwLock<Lang>>,
    /// 字典（语言 → key → 文案）。
    dicts: Arc<HashMap<Lang, &'static [(&'static str, &'static str)]>>,
    /// 偏好持久化路径。
    persist_path: Arc<Option<std::path::PathBuf>>,
}

impl I18n {
    /// 新建（加载偏好或跟随系统）。
    pub fn new(data_dir: &Path) -> Self {
        let _ = std::fs::create_dir_all(data_dir);
        let persist_path = data_dir.join("i18n.json");
        let lang = Self::load_pref(&persist_path).unwrap_or_else(Lang::from_system);
        Self {
            lang: Arc::new(RwLock::new(lang)),
            dicts: Arc::new(build_dicts()),
            persist_path: Arc::new(Some(persist_path)),
        }
    }

    /// 无持久化（测试用）。
    pub fn without_persist() -> Self {
        Self {
            lang: Arc::new(RwLock::new(Lang::En)),
            dicts: Arc::new(build_dicts()),
            persist_path: Arc::new(None),
        }
    }

    fn load_pref(path: &Path) -> Option<Lang> {
        let data = std::fs::read_to_string(path).ok()?;
        if data.contains("\"en\"") || data.trim() == "en" { Some(Lang::En) } else { Some(Lang::Zh) }
    }

    pub fn lang(&self) -> Lang {
        *self.lang.read().unwrap()
    }

    /// 切换语言并持久化。
    pub fn set_lang(&self, lang: Lang) {
        *self.lang.write().unwrap() = lang;
        if let Some(p) = self.persist_path.as_ref() {
            if let Some(dir) = p.parent() { let _ = std::fs::create_dir_all(dir); }
            let data = format!("{{\"lang\":\"{}\"}}", lang.code());
            let _ = std::fs::write(p, data);
        }
    }

    /// 翻译（key 缺失回退英文，再缺失回退 key 本身）。
    pub fn tr(&self, key: &str) -> String {
        let lang = self.lang();
        let dict = self.dicts.get(&lang).copied().unwrap_or(&[]);
        if let Some((_, v)) = dict.iter().find(|(k, _)| *k == key) {
            return v.to_string();
        }
        // 回退英文
        let en = self.dicts.get(&Lang::En).copied().unwrap_or(&[]);
        if let Some((_, v)) = en.iter().find(|(k, _)| *k == key) {
            return v.to_string();
        }
        key.to_string()
    }

    /// 带参数翻译（{name} 替换）。
    pub fn tr_args(&self, key: &str, args: &[(&str, &str)]) -> String {
        let mut s = self.tr(key);
        for (k, v) in args {
            s = s.replace(&format!("{{{k}}}"), v);
        }
        s
    }
}

fn build_dicts() -> HashMap<Lang, &'static [(&'static str, &'static str)]> {
    let mut m = HashMap::new();
    m.insert(Lang::Zh, ZH_DICT);
    m.insert(Lang::En, EN_DICT);
    m
}

/// 中文文案。
const ZH_DICT: &[(&str, &str)] = &[
    ("app.name", "AIPowerLink"),
    ("tray.server.menu", "组长端"),
    ("tray.client.menu", "组员端"),
    ("tray.open_console", "打开管理面板"),
    ("tray.start_sharing", "开启共享"),
    ("tray.pause_sharing", "暂停共享"),
    ("tray.quit", "退出"),
    ("tray.leaders", "发现的组长"),
    ("tray.auto_discover", "（启动后自动发现）"),
    ("tray.connection_status", "接入状态"),
    ("tray.rename", "修改显示名"),
    ("tray.show_usage", "查看个人用量"),
    ("cli.starting", "启动中..."),
    ("cli.sharing_enabled", "共享已开启"),
    ("cli.sharing_paused", "共享已暂停"),
    ("cli.broadcast", "发现广播"),
    ("cli.tray_started", "系统托盘已启动"),
    ("cli.tray_disabled", "无托盘模式（纯命令行）"),
    ("cli.open_console", "打开管理面板"),
    ("role.builtin_server", "内置组长"),
    ("role.builtin_client", "内置组员"),
    ("role.custom_server", "自定义组长"),
    ("role.custom_client", "自定义组员"),
    ("role.readonly", "内置角色只读，请先 clone 后修改"),
    ("usage.total", "总用量"),
    ("usage.calls", "调用次数"),
];

/// 英文文案。
const EN_DICT: &[(&str, &str)] = &[
    ("app.name", "AIPowerLink"),
    ("tray.server.menu", "Leader"),
    ("tray.client.menu", "Member"),
    ("tray.open_console", "Open console"),
    ("tray.start_sharing", "Start sharing"),
    ("tray.pause_sharing", "Pause sharing"),
    ("tray.quit", "Quit"),
    ("tray.leaders", "Discovered leaders"),
    ("tray.auto_discover", "(auto-discover on start)"),
    ("tray.connection_status", "Connection status"),
    ("tray.rename", "Rename"),
    ("tray.show_usage", "Show usage"),
    ("cli.starting", "starting..."),
    ("cli.sharing_enabled", "sharing enabled"),
    ("cli.sharing_paused", "sharing paused"),
    ("cli.broadcast", "discovery broadcast"),
    ("cli.tray_started", "system tray started"),
    ("cli.tray_disabled", "no-tray mode (CLI only)"),
    ("cli.open_console", "open console"),
    ("role.builtin_server", "Built-in Leader"),
    ("role.builtin_client", "Built-in Member"),
    ("role.custom_server", "Custom Leader"),
    ("role.custom_client", "Custom Member"),
    ("role.readonly", "builtin role is read-only; clone it to customize"),
    ("usage.total", "Total usage"),
    ("usage.calls", "Calls"),
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tr_zh_en() {
        let i = I18n::without_persist();
        // 默认英文（全球用户）
        assert_eq!(i.tr("tray.quit"), "Quit");
        i.set_lang(Lang::Zh);
        assert_eq!(i.tr("tray.quit"), "退出");
    }

    #[test]
    fn tr_args_replaces() {
        let i = I18n::without_persist();
        // 默认英文
        assert_eq!(i.tr_args("tray.open_console", &[]), "Open console");
        i.set_lang(Lang::Zh);
        assert_eq!(i.tr_args("tray.open_console", &[]), "打开管理面板");
    }

    #[test]
    fn missing_key_fallback() {
        let i = I18n::without_persist();
        i.set_lang(Lang::Zh);
        assert_eq!(i.tr("unknown.key"), "unknown.key");
        i.set_lang(Lang::En);
        assert_eq!(i.tr("unknown.key"), "unknown.key");
    }

    #[test]
    fn persist_roundtrip() {
        use std::sync::atomic::{AtomicU64, Ordering};
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("aipg-i18n-{}-{}", std::process::id(), n));
        let _ = std::fs::remove_dir_all(&dir);
        let i = I18n::new(&dir);
        i.set_lang(Lang::En);
        let i2 = I18n::new(&dir);
        assert_eq!(i2.lang(), Lang::En);
        let _ = std::fs::remove_dir_all(&dir);
    }
}