fn main() {
    // Windows 版本信息资源：嵌入 exe（消除杀软误报 + 正常元数据）
    #[cfg(target_os = "windows")]
    {
        let rc = "resources/app.rc";
        if std::path::Path::new(rc).exists() {
            println!("cargo:rerun-if-changed=resources/app.rc");
            // 用 windres（mingw）编译 .rc → .res，避免依赖 SDK rc.exe 路径查找
            let out_dir = std::env::var("OUT_DIR").unwrap_or_default();
            let res_out = format!("{}\\app.res", out_dir);
            // 尝试用 rc.exe（MSVC）——通过 vcvars 环境的 PATH 已包含 rc.exe
            let status = std::process::Command::new("rc.exe")
                .args(["/fo", &res_out, rc])
                .status();
            let ok = match status {
                Ok(s) => s.success(),
                Err(_) => false,
            };
            if ok {
                println!("cargo:rustc-link-arg={}", res_out);
            } else {
                // 回退：windres（mingw64）
                let windres = "D:\\AppSpaces\\GreenApp\\mingw64\\bin\\windres.exe";
                if std::path::Path::new(windres).exists() {
                    let status = std::process::Command::new(windres)
                        .args(["-i", rc, "-o", &res_out, "--output-format=coff"])
                        .status();
                    if let Ok(s) = status {
                        if s.success() {
                            println!("cargo:rustc-link-arg={}", res_out);
                        }
                    }
                }
            }
        }
    }
}