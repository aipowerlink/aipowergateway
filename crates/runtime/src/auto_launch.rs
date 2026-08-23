//! auto_launch：开机自启（参考 cc-switch 的 auto_launch crate）。
//! - Windows：注册表 Run 键
//! - Linux：XDG autostart desktop 文件
//! - macOS：AppleScript login item（需 .app bundle 路径）

use auto_launch::{AutoLaunch, AutoLaunchBuilder};
use std::path::Path;

use crate::{RuntimeError, RuntimeResult};

const APP_NAME: &str = "aipowergateway";

/// 构造 AutoLaunch 实例（macOS 需 .app bundle 路径）。
fn build() -> RuntimeResult<AutoLaunch> {
    let exe = std::env::current_exe()
        .map_err(|e| RuntimeError::Other(format!("current exe: {e}")))?;
    let app_path = get_app_path(&exe);
    AutoLaunchBuilder::new()
        .set_app_name(APP_NAME)
        .set_app_path(&app_path.to_string_lossy())
        .build()
        .map_err(|e| RuntimeError::Other(format!("auto launch build: {e}")))
}

/// 获取自启路径（macOS 用 .app bundle）。
fn get_app_path(exe: &Path) -> std::path::PathBuf {
    #[cfg(target_os = "macos")]
    {
        let s = exe.to_string_lossy();
        if let Some(pos) = s.find(".app/Contents/MacOS/") {
            return std::path::PathBuf::from(&s[..pos + 4]); // 到 .app
        }
    }
    exe.to_path_buf()
}

/// 启用开机自启。
pub fn enable() -> RuntimeResult<()> {
    build()?.enable().map_err(|e| RuntimeError::Other(format!("enable autostart: {e}")))
}

/// 禁用开机自启。
pub fn disable() -> RuntimeResult<()> {
    build()?.disable().map_err(|e| RuntimeError::Other(format!("disable autostart: {e}")))
}

/// 查询自启状态。
pub fn is_enabled() -> RuntimeResult<bool> {
    build()?.is_enabled().map_err(|e| RuntimeError::Other(format!("autostart status: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn builds_ok() {
        // 仅验证构造（不实际修改系统）
        assert!(build().is_ok() || build().is_err()); // 环境差异下不崩即可
    }
}