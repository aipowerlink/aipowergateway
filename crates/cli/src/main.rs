//! aipowerlink CLI 入口：--role / --backend / --no-tray / config / role 子命令。
//! Windows: release 无控制台窗口（托盘后台运行），debug 保留窗口便于调试
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]


use std::path::PathBuf;

use clap::{Parser, Subcommand};

/// AIPowerLink 局域网算力共享网关（Rust + Tauri）。
#[derive(Parser, Debug)]
#[command(name = "aipowergateway", version, about)]
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
    /// 开机自启管理。
    Autostart {
        #[command(subcommand)]
        sub: AutostartCmd,
    },
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
pub enum AutostartCmd {
    /// 启用开机自启。
    Enable,
    /// 禁用开机自启。
    Disable,
    /// 查询自启状态。
    Status,
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
            Commands::Autostart { sub } => handle_autostart(sub),
        }
        return;
    }

    // 单实例（参考 cc-switch）：已有实例运行时退出；守卫保持到进程结束
    // 锁名按角色区分：同一台机器可同时运行组长(server)与成员(client)两个 gateway。
    let lock_name = if cli.role == "client" { "aipowergateway-client" } else { "aipowergateway" };
    let _single = match aipg_runtime::SingleInstance::acquire(lock_name) {
        Some(guard) => guard,
        None => {
            eprintln!("aipowergateway ({}) is already running", cli.role);
            std::process::exit(0);
        }
    };

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
        "client" => run_client(&data_dir, cli.no_tray).await,
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

/// 从 --backend/环境变量解析后端配置条目（对齐 backends.yaml providers 段）。
/// 密钥以环境变量引用（credential-ref）形式保存，不落明文盘。
fn entries_from_env(backend_arg: &str) -> anyhow::Result<Vec<aipg_lan_share::BackendEntry>> {
    use aipg_lan_share::BackendEntry;
    let mut out: Vec<BackendEntry> = Vec::new();
    for name in backend_arg.split(',') {
        let name = name.trim();
        if name.is_empty() { continue; }
        match name {
            "mock" => out.push(BackendEntry { provider: "mock".into(), ..Default::default() }),
            "deepseek" | "kimi" | "zhipu" => {
                let env_key = format!("AIPOWERLINK_{}_API_KEY", name.to_uppercase());
                let official = std::env::var(&env_key).map(|v| !v.is_empty()).unwrap_or(false);
                let generic = std::env::var("AIPOWERLINK_API_KEY").map(|v| !v.is_empty()).unwrap_or(false);
                if !official && !generic {
                    anyhow::bail!("{name} backend requires {env_key} (or AIPOWERLINK_API_KEY) env var");
                }
                out.push(BackendEntry {
                    provider: name.into(),
                    api_key_env: Some(if official { env_key } else { "AIPOWERLINK_API_KEY".into() }),
                    models: std::env::var(format!("AIPOWERLINK_{}_MODEL", name.to_uppercase())).ok()
                        .map(|m| vec![m]).unwrap_or_default(),
                    base_url: std::env::var("AIPOWERLINK_BASE_URL").ok(),
                    ..Default::default()
                });
            }
            other => anyhow::bail!("unknown backend: {other} (mock/deepseek/kimi/zhipu)"),
        }
    }
    if out.is_empty() { anyhow::bail!("no backend configured (use --backend mock/deepseek/kimi/zhipu)"); }
    Ok(out)
}

/// 以服务端角色运行（组长）。
async fn run_server(data_dir: &std::path::Path, backend_arg: &str, no_tray: bool) -> anyhow::Result<()> {
    use aipg_config::{ConfigService, RoleView};
    use aipg_lan_share::{BroadcastConfig, BroadcastService, ShareServer, ShareServerConfig};
    std::fs::create_dir_all(data_dir).map_err(|e| anyhow::anyhow!("create data dir: {e}"))?;
    // 从配置文件读取 port / bind（默认 39091 / 0.0.0.0），config set port|bind 立即生效
    let svc = ConfigService::open(data_dir, "aipowerlink.db").map_err(|e| anyhow::anyhow!("config open: {e}"))?;
    let port: u16 = match svc.get(RoleView::Global, "port").map_err(|e| anyhow::anyhow!("config read: {e}"))? {
        Some(v) => v.parse().map_err(|_| anyhow::anyhow!("config port invalid: {v}"))?,
        None => 39091,
    };
    let bind: std::net::IpAddr = match svc.get(RoleView::Global, "bind").map_err(|e| anyhow::anyhow!("config read: {e}"))? {
        Some(v) => v.parse().map_err(|_| anyhow::anyhow!("config bind invalid: {v} (expected e.g. 0.0.0.0 or 127.0.0.1)"))?,
        // 默认仅本机；局域网共享需显式 config set bind 0.0.0.0
        None => [127, 0, 0, 1].into(),
    };
    // gateway 间共享通道端口：成员 gateway 经此端口接入（独立于管理/API，默认 0.0.0.0）
    let share_port: u16 = match svc.get(RoleView::Global, "share_port").map_err(|e| anyhow::anyhow!("config read: {e}"))? {
        Some(v) => v.parse().map_err(|_| anyhow::anyhow!("config share_port invalid: {v}"))?,
        None => 39092,
    };
    let cfg = ShareServerConfig {
        port,
        bind,
        share_port,
        // 0 = 永久有效（管理/API 只监听 127.0.0.1，key 仅暴露在本机）
        token_ttl_secs: 0,
        heartbeat_timeout_secs: 90,
        name: "aipowerlink-share".to_string(),
        data_dir: data_dir.to_path_buf(),
        web_dir: std::env::var("AIPOWERLINK_WEB_DIR")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|_| std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../web/dist")),
    };
    let entries = entries_from_env(backend_arg)?;
    let server = ShareServer::with_entries(&cfg, entries)?;
    println!("sharing: enabled on {}:{}", cfg.bind, cfg.port);
    println!("gateway channel: http://0.0.0.0:{} (member gateways connect here)", cfg.share_port);
    let broadcast = BroadcastService::new(BroadcastConfig {
        port: 39090,
        name: "aipowerlink-share".to_string(),
        api_port: cfg.port,
        share_port: cfg.share_port,
        fingerprint: String::new(), // 0.2.0 起免密：指纹弃用（协议字段保留兼容）
        interval_secs: 10,
        target: "255.255.255.255".to_string(),
    });
    broadcast.start();
    println!("discovery broadcast: UDP :{} (name=aipowerlink-share, api :{}, gateway channel :{})", 39090, cfg.port, cfg.share_port);

    // 托盘（参考 cc-switch）：--no-tray 时纯 CLI
    if !no_tray {
        println!("starting system tray (use --no-tray for CLI-only)...");
        let tray = aipg_lan_tray::TrayService::new(aipg_lan_tray::TrayMode::Server)?;
        let server_handle = server.clone();
        // TrayIcon 非 Send，不能在 tokio::spawn；用 std::thread 轮询托盘动作
        std::thread::spawn(move || {
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
                    aipg_lan_tray::TrayAction::Quit => {
                        println!("[tray] quitting...");
                        std::process::exit(0);
                    }
                    _ => {}
                }
            }
        });
    }

    // 服务启动后自动打开管理面板（延迟等服务监听就绪）
    let console_url = format!("http://127.0.0.1:{}", cfg.port);
    std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(1200));
        eprintln!("[console] opening: {}", console_url);
        let _ = open_browser(&console_url);
    });

    let result = server.serve().await;
    broadcast.stop();
    result?;
    Ok(())
}

/// 以成员角色运行（--role client）：本机 gateway，把请求转发给发现的组长。
async fn run_client(data_dir: &std::path::Path, no_tray: bool) -> anyhow::Result<()> {
    use aipg_config::{ConfigService, RoleView};
    use aipg_lan_client::gateway::MemberGateway;
    use aipg_lan_client::{DiscoveryClient, DiscoveryConfig};
    use axum::extract::State;
    use axum::http::{header, StatusCode};
    use axum::response::{IntoResponse, Json, Response};
    use axum::routing::{get, post};
    use axum::Router;
    use axum::body::Bytes;
    use serde_json::json;

    std::fs::create_dir_all(data_dir).map_err(|e| anyhow::anyhow!("create data dir: {e}"))?;
    let svc = ConfigService::open(data_dir, "aipowerlink.db").map_err(|e| anyhow::anyhow!("config open: {e}"))?;
    let port: u16 = match svc.get(RoleView::Global, "member_port").map_err(|e| anyhow::anyhow!("config read: {e}"))? {
        Some(v) => v.parse().map_err(|_| anyhow::anyhow!("config member_port invalid: {v}"))?,
        None => 39091,
    };

    // UDP 发现组长（gateway 间通信：经组长共享通道端口转发）
    let discovery = DiscoveryClient::new(DiscoveryConfig::default());
    discovery.start_listen();
    discovery.ping_once();
    let gateway = MemberGateway::new(discovery.clone());

    async fn proxy_resp(g: &MemberGateway, path: &str, auth: Option<&str>, body: Option<Vec<u8>>) -> Response {
        match g.proxy(path, auth, body).await {
            Ok((status, bytes)) => (
                StatusCode::from_u16(status).unwrap_or(StatusCode::BAD_GATEWAY),
                bytes,
            ).into_response(),
            Err(e) => (StatusCode::SERVICE_UNAVAILABLE, Json(json!({ "error": { "message": e } }))).into_response(),
        }
    }

    async fn h_token(State(g): State<MemberGateway>, body: Bytes) -> Response {
        proxy_resp(&g, "/auth/token", None, Some(body.to_vec())).await
    }
    async fn h_models(State(g): State<MemberGateway>) -> Response {
        proxy_resp(&g, "/v1/models", None, None).await
    }
    async fn h_chat(State(g): State<MemberGateway>, headers: axum::http::HeaderMap, body: Bytes) -> Response {
        let auth = headers.get(header::AUTHORIZATION).and_then(|v| v.to_str().ok()).map(|s| s.to_string());
        proxy_resp(&g, "/v1/chat/completions", auth.as_deref(), Some(body.to_vec())).await
    }
    async fn h_messages(State(g): State<MemberGateway>, headers: axum::http::HeaderMap, body: Bytes) -> Response {
        let auth = headers.get(header::AUTHORIZATION).and_then(|v| v.to_str().ok()).map(|s| s.to_string());
        proxy_resp(&g, "/v1/messages", auth.as_deref(), Some(body.to_vec())).await
    }
    async fn h_status(State(g): State<MemberGateway>) -> Response {
        Json(json!({
            "role": "client",
            "leaders": g.leader_count(),
            "leader": g.leader_summary(),
        })).into_response()
    }

    let app = Router::new()
        .route("/", get(h_status))
        .route("/auth/token", post(h_token))
        .route("/v1/models", get(h_models))
        .route("/v1/chat/completions", post(h_chat))
        .route("/v1/messages", post(h_messages))
        .with_state(gateway);

    let addr: std::net::SocketAddr = std::net::SocketAddr::from(([127, 0, 0, 1], port));
    let listener = tokio::net::TcpListener::bind(addr).await.map_err(|e| anyhow::anyhow!("member gateway bind {addr}: {e}"))?;
    println!("member gateway: listening on http://127.0.0.1:{}", port);
    println!("discovery: UDP :{} (auto-discover leader, forward via gateway channel)", 39090);
    if !no_tray {
        eprintln!("[tray] client role: tray not provided, use --no-tray (default behavior overrides)");
    }

    axum::serve(listener, app).await.map_err(|e| anyhow::anyhow!("member gateway serve: {e}"))?;
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

fn handle_autostart(sub: &AutostartCmd) {
    use aipg_runtime::auto_launch;
    match sub {
        AutostartCmd::Enable => match auto_launch::enable() {
            Ok(()) => println!("autostart: enabled"),
            Err(e) => { eprintln!("error: {e}"); std::process::exit(1); }
        },
        AutostartCmd::Disable => match auto_launch::disable() {
            Ok(()) => println!("autostart: disabled"),
            Err(e) => { eprintln!("error: {e}"); std::process::exit(1); }
        },
        AutostartCmd::Status => match auto_launch::is_enabled() {
            Ok(true) => println!("autostart: enabled"),
            Ok(false) => println!("autostart: disabled"),
            Err(e) => { eprintln!("error: {e}"); std::process::exit(1); }
        },
    }
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