//! aipg-lan-tray: 托盘模块占位——0.1.0 阶段 8 实现（Tauri）。

pub fn placeholder() -> &'static str {
    "lan-tray"
}

#[cfg(test)]
mod tests {
    #[test]
    fn placeholder_works() {
        assert_eq!(super::placeholder(), "lan-tray");
    }
}