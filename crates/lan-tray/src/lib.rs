//! aipg-lan-tray: 系统托盘（参考 cc-switch，tauri-apps tray-icon 独立库）。

pub mod tray;

pub use tray::{TrayAction, TrayMode, TrayService};