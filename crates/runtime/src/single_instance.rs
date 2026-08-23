//! single_instance：单实例锁（参考 cc-switch 的 tauri_plugin_single_instance）。
//! - Windows：命名 Mutex（跨进程互斥）
//! - Linux/macOS：锁文件 + 独占创建（O_EXCL 检测）

#[cfg(not(target_os = "windows"))]
use std::path::PathBuf;

/// 单实例守卫（持有期间独占运行权）。
pub struct SingleInstance {
    #[cfg(target_os = "windows")]
    _mutex: windows_sys::Win32::Foundation::HANDLE,
    #[cfg(not(target_os = "windows"))]
    _lock_path: PathBuf,
}

impl SingleInstance {
    /// 尝试获取单实例锁。已有实例在运行则返回 None。
    pub fn acquire(app_name: &str) -> Option<Self> {
        #[cfg(target_os = "windows")]
        {
            use windows_sys::Win32::Foundation::{CloseHandle, ERROR_ACCESS_DENIED, ERROR_ALREADY_EXISTS, GetLastError};
            use windows_sys::Win32::System::Threading::CreateMutexW;

            fn create_mutex(name: &str) -> (windows_sys::Win32::Foundation::HANDLE, u32) {
                let wide: Vec<u16> = name.encode_utf16().chain(std::iter::once(0)).collect();
                unsafe {
                    let h = CreateMutexW(std::ptr::null_mut(), 0, wide.as_ptr());
                    (h, GetLastError())
                }
            }

            // 全局命名空间优先；受限会话无权限（如标准用户）时回退会话命名空间
            let (mut handle, mut err) = create_mutex(&format!("Global\\{app_name}-single-instance"));
            if handle.is_null() && err == ERROR_ACCESS_DENIED {
                let (h2, e2) = create_mutex(&format!("Local\\{app_name}-single-instance"));
                handle = h2;
                err = e2;
            }
            if handle.is_null() {
                return None;
            }
            if err == ERROR_ALREADY_EXISTS {
                unsafe { CloseHandle(handle) };
                return None; // 已有实例
            }
            Some(Self { _mutex: handle })
        }
        #[cfg(not(target_os = "windows"))]
        {
            // 锁文件 + 独占创建：已存在说明有实例
            let dir = std::env::temp_dir();
            let lock_path = dir.join(format!(".{app_name}.lock"));
            match std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&lock_path)
            {
                Ok(mut f) => {
                    use std::io::Write;
                    let _ = writeln!(f, "{}", std::process::id());
                    Some(Self { _lock_path: lock_path })
                }
                Err(_) => None, // 文件已存在 = 已有实例
            }
        }
    }

    /// 是否已有实例在运行。
    pub fn is_running(app_name: &str) -> bool {
        Self::acquire(app_name).is_none()
    }
}

impl Drop for SingleInstance {
    fn drop(&mut self) {
        #[cfg(target_os = "windows")]
        unsafe {
            use windows_sys::Win32::Foundation::CloseHandle;
            CloseHandle(self._mutex);
        }
        #[cfg(not(target_os = "windows"))]
        {
            let _ = std::fs::remove_file(&self._lock_path);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn acquire_returns_some() {
        let guard = SingleInstance::acquire("test-aipg-single-instance");
        assert!(guard.is_some());
        drop(guard);
    }

    #[test]
    fn drop_releases() {
        {
            let _g = SingleInstance::acquire("test-aipg-single-release");
            // 持有中
            #[cfg(not(target_os = "windows"))]
            {
                let p = std::env::temp_dir().join(".test-aipg-single-release.lock");
                assert!(p.exists());
            }
        }
        // drop 后再次获取应成功（Unix 锁文件删除）
        #[cfg(not(target_os = "windows"))]
        {
            let g2 = SingleInstance::acquire("test-aipg-single-release");
            assert!(g2.is_some());
            drop(g2);
        }
    }
}