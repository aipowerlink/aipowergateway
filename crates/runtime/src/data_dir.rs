//! 跨平台用户数据目录（参考 cc-switch 最小侵入：配置存用户数据目录，不写系统全局）。

use std::path::PathBuf;

/// 应用数据目录名。
pub const APP_DIR: &str = ".aipowerlink";

/// 返回跨平台用户数据目录：
/// - Windows: %APPDATA%/.aipowerlink
/// - Linux:   ~/.config/.aipowerlink
/// - macOS:   ~/Library/Application Support/.aipowerlink
pub fn default_data_dir() -> PathBuf {
    let base = if cfg!(target_os = "windows") {
        dirs::config_dir()
    } else if cfg!(target_os = "macos") {
        dirs::data_dir()
    } else {
        dirs::config_dir()
    };
    base.unwrap_or_else(|| PathBuf::from(".")).join(APP_DIR)
}

/// 角色目录（自定义角色存放处）。
pub fn roles_dir(data_dir: &PathBuf) -> PathBuf {
    data_dir.join("roles")
}

/// 配置库路径。
pub fn db_path(data_dir: &PathBuf) -> PathBuf {
    data_dir.join("aipowerlink.db")
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn roles_and_db_under_data_dir() {
        let d = PathBuf::from("/tmp/aip");
        assert_eq!(roles_dir(&d), PathBuf::from("/tmp/aip/roles"));
        assert_eq!(db_path(&d), PathBuf::from("/tmp/aip/aipowerlink.db"));
    }
}