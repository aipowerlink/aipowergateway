//! aipowerlink CLI 入口：--role / --backend / --no-tray / config / role 子命令。

use std::path::PathBuf;
use std::sync::Arc;

use clap::{Parser, Subcommand};

/// AIPowerLink 局域网算力共享网关（Rust + Tauri）。
#[derive(Parser, Debug)]
#[command(name = "aipowerlink", version, about)]
pub struct Cli {
    /// 运行角色（内置：server/client；或自定义角色 id）。
    #[arg(long, default_value = "server")]
    pub role: String,

    /// 执行后端（mock / deepseek / kimi / zhipu；逗号分隔可多后端：deepseek,kimi）。
    #[arg(long, default_value = "mock")]
    pub backend: String,

    /// 无托盘模式（纯命令行）。
    #[arg(long)]
    pub no_tray: bool,

    /// 数据目录覆盖（默认跨平台用户数据目录）。
    #[arg(long)]
    pub data_dir: Option<PathBuf>,

    /// 子命令。
    #[command(subcommand)]
    pub command: Option<Commands>,
}

#[derive(Subcommand, Debug)]
pub enum Commands {
    /// 配置读写。
    Config {
        #[command(subcommand)]
        sub: ConfigCmd,
    },
    /// 角色管理。
    Role {
        #[command(subcommand)]
        sub: RoleCmd,
    },
    /// 版本信息。
    Version,
}

#[derive(Subcommand, Debug)]
pub enum ConfigCmd {
    /// 读取配置。
    Get { key: String },
    /// 写入配置。
    Set { key: String, value: String },
    /// 列出配置（已脱敏）。
    List,
}

#[derive(Subcommand, Debug)]
pub enum RoleCmd {
    /// 列出角色（内置标 system，自定义标 user）。
    List,
    /// 显示角色详情。
    Show { id: String },
    /// 复制内置角色为自定义。
    Clone { from: String, to: String },
    /// 新建自定义角色。
    New { id: String },
    /// 编辑角色（CLI 编辑模块清单）。
    Edit { id: String },
    /// 删除自定义角色。
    Rm { id: String },
}

#[tokio::main]
async fn main() {
    init_logging();
    let cli = Cli::parse();

    // 初始化 i18n（语言偏好持久化）
    let data_dir_for_i18n = cli.data_dir.clone().unwrap_or_else(aipg_runtime::data_dir::default_data_dir);
    let i18n = aipg_runtime::I18n::new(&data_dir_for_i18n);

    if let Some(cmd) = &cli.command {
        match cmd {
            Commands::Config { sub } => handle_config(sub, &cli.data_dir),
            Commands::Role { sub } => handle_role(sub, &cli.data_dir, &i18n),
            Commands::Version => {
                println!("aipowerlink {}", aipg_runtime::VERSION);
            }
        }
        return;
    }

    // 无子命令：装配角色并运行
    let data_dir = cli.data_dir.clone().unwrap_or_else(aipg_runtime::data_dir::default_data_dir);
    println!("aipowerlink {}", aipg_runtime::VERSION);
    println!("role: {}", cli.role);
    println!("backend: {}", cli.backend);
    println!("tray: {}", if cli.no_tray { "disabled" } else { "enabled" });
    println!("data_dir: {}", data_dir.display());

    // 自定义角色解析（server/client 为内置）
    let role_name = cli.role.clone();
    let role_modules = {
        use aipg_runtime::RoleManager;
        let mgr = RoleManager::new(&data_dir);
        match role_name.as_str() {
            "server" | "client" => None,
            _ => {
                match mgr.enabled_modules(&role_name) {
                    Ok(mods) if !mods.is_empty() => Some(mods),
                    Ok(_) => {
                        eprintln!("role {role_name} has no enabled modules");
                        std::process::exit(2);
                    }
                    Err(e) => {
                        eprintln!("role error: {e}");
                        std::process::exit(2);
                    }
                }
            }
        }
    };

    let result = match role_name.as_str() {
        "server" => run_server(&data_dir, &cli.backend, cli.no_tray).await,
        "client" => {
            println!("[client] consumer role — stage 4 (not yet implemented)");
            Ok(())
        }
        _ => {
            println!("custom role modules: {}", role_modules.clone().unwrap_or_default().join(", "));
            run_server(&data_dir, &cli.backend, cli.no_tray).await
        }
    };
    if let Err(e) = result {
        eprintln!("error: {e}");
        std::process::exit(1);
    }
}

fn init_logging() {
    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"));
    tracing_subscriber::fmt().with_env_filter(filter).init();
}

/// 构造多后端注册表（mock / deepseek / kimi / zhipu，可逗号分隔）。
fn build_registry(backend_arg: &str) -> anyhow::Result<aipg_lan_share::BackendRegistry> {
    use aipg_lan_share::{BackendRegistry, MockBackend, OpenAICompatBackend, OpenAICompatConfig, Provider};
    let mut registry = BackendRegistry::new();
    for name in backend_arg.split(',') {
        let name = name.trim();
        if name.is_empty() { continue; }
        let provider = match name {
            "mock" => Provider::Mock,
            "deepseek" => Provider::DeepSeek,
            "kimi" => Provider::Kimi,
            "zhipu" => Provider::Zhipu,
            other => anyhow::bail!("unknown backend: {other} (mock/deepseek/kimi/zhipu)"),
        };
        if provider == Provider::Mock {
            registry.register(Arc::new(MockBackend::default()));
            continue;
        }
        let env_key = format!("AIPOWERLINK_{}_API_KEY", provider.name().to_uppercase());
        let api_key = std::env::var(&env_key)
            .or_else(|_| std::env::var("AIPOWERLINK_API_KEY"))
            .map_err(|_| anyhow::anyhow!("{name} backend requires {env_key} (or AIPOWERLINK_API_KEY) env var"))?;
        let model_env = format!("AIPOWERLINK_{}_MODEL", provider.name().to_uppercase());
        let model = std::env::var(&model_env).ok();
        let base_url = std::env::var("AIPOWERLINK_BASE_URL").ok();
        let cfg = OpenAICompatConfig { provider, api_key, model, base_url, timeout_secs: 60 };
        registry.register(Arc::new(OpenAICompatBackend::new(cfg)));
    }
    if registry.backend_count() == 0 { anyhow::bail!("no backend configured"); }
    Ok(registry)
}

/// 以服务端角色运行（组长）。
async fn run_server(data_dir: &std::path::Path, backend_arg: &str, no_tray: bool) -> anyhow::Result<()> {
    use aipg_lan_share::{BroadcastConfig, BroadcastService, ShareServer, ShareServerConfig};
    std::fs::create_dir_all(data_dir).map_err(|e| anyhow::anyhow!("create data dir: {e}"))?;
    let cfg = ShareServerConfig {
        port: 39091,
        password: std::env::var("AIPOWERLINK_PASSWORD").unwrap_or_else(|_| "aipowerlink".to_string()),
        token_ttl_secs: 12 * 3600,
        heartbeat_timeout_secs: 90,
        data_dir: data_dir.to_path_buf(),
        web_dir: std::env::var("AIPOWERLINK_WEB_DIR")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|_| std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../web/dist")),
    };
    let registry = build_registry(backend_arg)?;
    let server = ShareServer::new(&cfg, registry);
    println!("sharing: enabled on :{}", cfg.port);
    let fingerprint = server.fingerprint(8);
    println!("fingerprint: {}", fingerprint);
    let broadcast = BroadcastService::new(BroadcastConfig {
        port: 39090,
        name: "aipowerlink-share".to_string(),
        api_port: cfg.port,
        fingerprint,
        interval_secs: 10,
        target: "255.255.255.255".to_string(),
    });
    broadcast.start();
    println!("discovery broadcast: UDP :{} (name=aipowerlink-share, api :{})", 39090, cfg.port);

    // 托盘（参考 cc-switch）：--no-tray 时纯 CLI
    if !no_tray {
        println!("starting system tray (use --no-tray for CLI-only)...");
        let tray = aipg_lan_tray::TrayService::new(aipg_lan_tray::TrayMode::Server)?;
        let server_handle = server.clone();
        tokio::spawn(async move {
            loop {
                match tray.recv() {
                    aipg_lan_tray::TrayAction::OpenConsole => {
                        println!("[tray] open console: http://127.0.0.1:{}", 39091);
                        let _ = open_browser(&format!("http://127.0.0.1:{}", 39091));
                    }
                    aipg_lan_tray::TrayAction::StartSharing => {
                        server_handle.set_sharing(true);
                        println!("[tray] sharing started");
                    }
                    aipg_lan_tray::TrayAction::PauseSharing => {
                        server_handle.set_sharing(false);
                        println!("[tray] sharing paused");
                    }
                    aipg_lan_tray::TrayAction::ChangePassword => {
                        println!("[tray] change password: use `aipowerlink config set password <new>`");
                    }
                    aipg_lan_tray::TrayAction::Quit => {
                        println!("[tray] quitting...");
                        std::process::exit(0);
                    }
                    _ => {}
                }
            }
        });
    }

    let result = server.serve().await;
    broadcast.stop();
    result?;
    Ok(())
}

/// 打开系统浏览器（跨平台）。
#[cfg(target_os = "windows")]
fn open_browser(url: &str) -> std::io::Result<()> {
    std::process::Command::new("cmd").args(["/c", "start", "", url]).spawn().map(|_| ())
}

#[cfg(target_os = "linux")]
fn open_browser(url: &str) -> std::io::Result<()> {
    std::process::Command::new("xdg-open").arg(url).spawn().map(|_| ())
}

#[cfg(target_os = "macos")]
fn open_browser(url: &str) -> std::io::Result<()> {
    std::process::Command::new("open").arg(url).spawn().map(|_| ())
}

fn handle_config(sub: &ConfigCmd, data_dir_override: &Option<PathBuf>) {
    use aipg_config::{ConfigService, RoleView};
    let data_dir = data_dir_override.clone().unwrap_or_else(aipg_runtime::data_dir::default_data_dir);
    let svc = match ConfigService::open(&data_dir, "aipowerlink.db") {
        Ok(s) => s,
        Err(e) => { eprintln!("config error: {e}"); std::process::exit(1); }
    };
    match sub {
        ConfigCmd::Get { key } => {
            match svc.get(RoleView::Global, key) {
                Ok(Some(v)) => println!("{key} = {v}"),
                Ok(None) => println!("{key} = (not set)"),
                Err(e) => { eprintln!("error: {e}"); std::process::exit(1); }
            }
        }
        ConfigCmd::Set { key, value } => {
            let secret = key.contains("password") || key.contains("token") || key.contains("api_key") || key.contains("secret");
            match svc.set(RoleView::Global, key, value, secret) {
                Ok(()) => println!("{key} set ({}secret)", if secret { "" } else { "non-" }),
                Err(e) => { eprintln!("error: {e}"); std::process::exit(1); }
            }
        }
        ConfigCmd::List => {
            match svc.list(RoleView::Global) {
                Ok(entries) => { for e in entries { println!("{} = {}", e.key, e.value); } }
                Err(e) => { eprintln!("error: {e}"); std::process::exit(1); }
            }
        }
    }
}

fn handle_role(sub: &RoleCmd, data_dir_override: &Option<PathBuf>, i18n: &aipg_runtime::I18n) {
    use aipg_runtime::{RoleManager, Trust};
    let data_dir = data_dir_override.clone().unwrap_or_else(aipg_runtime::data_dir::default_data_dir);
    let mgr = RoleManager::new(&data_dir);
    match sub {
        RoleCmd::List => {
            for (r, trust) in mgr.all() {
                let tag = match trust { Trust::System => "system", Trust::User => "user" };
                let name = r.name.clone().unwrap_or_else(|| r.id.clone());
                // 内置角色名本地化
                let name = match (r.id.as_str(), trust) {
                    ("server", Trust::System) => i18n.tr("role.builtin_server"),
                    ("client", Trust::System) => i18n.tr("role.builtin_client"),
                    (_, _) => name,
                };
                let count = mgr.enabled_modules(&r.id).map(|m| m.len()).unwrap_or(0);
                println!("{:<16} {:<8} modules={}  {}", r.id, tag, count, name);
            }
        }
        RoleCmd::Show { id } => {
            match mgr.find(id) {
                Some((r, trust)) => {
                    println!("role: {} ({:?})", r.id, trust);
                    println!("name: {}", r.name.clone().unwrap_or_default());
                    println!("base: {}", r.base.clone().unwrap_or_default());
                    println!("modules:");
                    for m in mgr.enabled_modules(id).unwrap_or_default() { println!("  - {m}"); }
                }
                None => { eprintln!("role not found: {id}"); std::process::exit(1); }
            }
        }
        RoleCmd::Clone { from, to } => {
            match mgr.clone_role(from, to) {
                Ok(p) => println!("cloned {from} -> {} (user)", p.id),
                Err(e) => { eprintln!("error: {e}"); std::process::exit(1); }
            }
        }
        RoleCmd::New { id } => {
            match mgr.new_role(id) {
                Ok(p) => println!("created role {} (user)", p.id),
                Err(e) => { eprintln!("error: {e}"); std::process::exit(1); }
            }
        }
        RoleCmd::Edit { id } => {
            match mgr.find(id) {
                Some((_, Trust::System)) => {
                    eprintln!("{}: aipowerlink role clone {id} my-{id}", i18n.tr("role.readonly"));
                    std::process::exit(1);
                }
                Some(_) => {
                    println!("editing role {id} (full module editor in 0.1.x; use role.json directly for now)");
                    println!("  role file: {}", mgr.user_roles_dir().join(id).join("role.json").display());
                }
                None => { eprintln!("role not found: {id}"); std::process::exit(1); }
            }
        }
        RoleCmd::Rm { id } => {
            match mgr.delete_role(id) {
                Ok(()) => println!("removed role {id}"),
                Err(e) => { eprintln!("error: {e}"); std::process::exit(1); }
            }
        }
    }
}