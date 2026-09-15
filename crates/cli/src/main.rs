//! aipowerlink CLI 入口：--role / --backend / --no-tray / config / role 子命令。
//! Windows: release 无控制台窗口（托盘后台运行），debug 保留窗口便于调试。
//!   窗口中不可见的一切日志均写入 {data_dir}/logs/aipowergateway.log（滚动），
//!   无窗口运行（release）时文件日志是唯一可见渠道，务必输出关键状态。
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

    /// 执行后端（mock / deepseek / kimi / zhipu / codebuddy；逗号分隔可多后端：deepseek,kimi）。
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
    /// 登录插件(可选增强,匿名优先)。
    Account {
        #[command(subcommand)]
        sub: AccountCmd,
    },
}

#[derive(Subcommand, Debug)]
pub enum AccountCmd {
    /// 查看登录状态(匿名 or 已登录)。
    Status,
    /// 启用登录插件(默认关闭 = 匿名优先)。
    Enable,
    /// 禁用登录插件并登出(恢复匿名,核心功能不受影响)。
    Disable,
    /// 注册账号(发动态密码到邮箱)。
    Register { username: String, email: String },
    /// 登录(动态密码 → 会话)。
    Login { username: String, otp: String },
    /// 绑定本机设备到账号(1:1,跨机找回凭据的前提)。
    Bind,
    /// 登出(立即恢复匿名)。
    Logout,
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
    let cli = Cli::parse();

    // 数据目录先于日志确定：日志写入 {data_dir}/logs/aipowergateway.log
    let data_dir = cli.data_dir.clone().unwrap_or_else(aipg_runtime::data_dir::default_data_dir);
    // guard 必须存活到进程结束，否则日志线程被回收（此处绑定为 _log_guard 保持）
    let _log_guard = init_logging(&data_dir);

    // 初始化 i18n（语言偏好持久化）
    let i18n = aipg_runtime::I18n::new(&data_dir);

    if let Some(cmd) = &cli.command {
        match cmd {
            Commands::Config { sub } => handle_config(sub, &cli.data_dir),
            Commands::Role { sub } => handle_role(sub, &cli.data_dir, &i18n),
            Commands::Version => {
                println!("aipowerlink {}", aipg_runtime::VERSION);
            }
            Commands::Autostart { sub } => handle_autostart(sub, &cli),
            Commands::Account { sub } => handle_account(sub, &cli.data_dir).await,
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
            tracing::warn!("single instance lock held: {} is already running", cli.role);
            std::process::exit(0);
        }
    };

    // 无子命令：装配角色并运行
    println!("aipowerlink {}", aipg_runtime::VERSION);
    println!("role: {}", cli.role);
    println!("backend: {}", cli.backend);
    println!("tray: {}", if cli.no_tray { "disabled" } else { "enabled" });
    println!("data_dir: {}", data_dir.display());
    tracing::info!(
        version = aipg_runtime::VERSION,
        role = %cli.role,
        backend = %cli.backend,
        tray = if cli.no_tray { "disabled" } else { "enabled" },
        data_dir = %data_dir.display(),
        "gateway starting"
    );

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

/// 双层日志：{data_dir}/logs/aipowergateway.log（按天滚动）+ 控制台（有窗口时）。
/// Windows release 无控制台（windows_subsystem=windows），文件日志是唯一可见渠道；
/// debug/其他平台保留控制台输出便于终端调试。
/// 返回 WorkerGuard：调用方必须持有到进程结束，否则日志线程被回收。
fn init_logging(data_dir: &std::path::Path) -> tracing_appender::non_blocking::WorkerGuard {
    use tracing_subscriber::layer::SubscriberExt;
    use tracing_subscriber::util::SubscriberInitExt;

    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"));

    let log_dir = data_dir.join("logs");
    let _ = std::fs::create_dir_all(&log_dir);
    let file_appender = tracing_appender::rolling::daily(&log_dir, "aipowergateway.log");
    let (non_blocking, guard) = tracing_appender::non_blocking(file_appender);
    let file_layer = tracing_subscriber::fmt::layer()
        .with_writer(non_blocking)
        .with_ansi(false)
        .with_target(false);

    // 有控制台（debug 构建或非 windows）时双写终端；无控制台 release 仅文件
    if cfg!(any(debug_assertions, not(windows))) {
        tracing_subscriber::registry()
            .with(filter)
            .with(file_layer)
            .with(tracing_subscriber::fmt::layer().with_ansi(true))
            .init();
    } else {
        tracing_subscriber::registry()
            .with(filter)
            .with(file_layer)
            .init();
    }
    guard
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
            "deepseek" | "kimi" | "zhipu" | "codebuddy" => {
                let env_key = format!("AIPOWERLINK_{}_API_KEY", name.to_uppercase());
                let official = std::env::var(&env_key).map(|v| !v.is_empty()).unwrap_or(false);
                let generic = std::env::var("AIPOWERLINK_API_KEY").map(|v| !v.is_empty()).unwrap_or(false);
                // CodeBuddy 兼容 DSH 的凭证变量名 CODEBUDDY_API_KEY
                let cb_alias = name == "codebuddy" && std::env::var("CODEBUDDY_API_KEY").map(|v| !v.is_empty()).unwrap_or(false);
                if !official && !generic && !cb_alias {
                    anyhow::bail!("{name} backend requires {env_key} (or AIPOWERLINK_API_KEY) env var");
                }
                out.push(BackendEntry {
                    provider: name.into(),
                    api_key_env: Some(if official { env_key } else if cb_alias { "CODEBUDDY_API_KEY".into() } else { "AIPOWERLINK_API_KEY".into() }),
                    models: std::env::var(format!("AIPOWERLINK_{}_MODEL", name.to_uppercase())).ok()
                        .map(|m| vec![m]).unwrap_or_default(),
                    base_url: std::env::var("AIPOWERLINK_BASE_URL").ok(),
                    ..Default::default()
                });
            }
            other => anyhow::bail!("unknown backend: {other} (mock/deepseek/kimi/zhipu/codebuddy)"),
        }
    }
    if out.is_empty() { anyhow::bail!("no backend configured (use --backend mock/deepseek/kimi/zhipu/codebuddy)"); }
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
    // 链路加密策略（M3 组长端）：link.encrypt = off | aes-gcm | enforce（缺省 aes-gcm=协商式）
    let link_encrypt = match svc.get(RoleView::Global, "link.encrypt").map_err(|e| anyhow::anyhow!("config read link.encrypt: {e}"))? {
        Some(v) => aipg_link_crypto::LinkEncryptMode::parse(&v),
        None => aipg_link_crypto::LinkEncryptMode::AesGcm,
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
        link_encrypt,
    };
    let entries = entries_from_env(backend_arg)?;
    let server = ShareServer::with_entries(&cfg, entries)?;
    println!("sharing: enabled on {}:{}", cfg.bind, cfg.port);
    println!("gateway channel: http://0.0.0.0:{} (member gateways connect here)", cfg.share_port);
    if link_encrypt != aipg_link_crypto::LinkEncryptMode::Off {
        println!("link-encrypt: {} (aes-gcm link encryption; enforce rejects unencrypted /v1/* with 426)", link_encrypt.as_str());
    }
    tracing::info!(bind = %cfg.bind, port = cfg.port, share_port = cfg.share_port, link_encrypt = ?link_encrypt, "sharing enabled (member gateways connect via gateway channel)");
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
    tracing::info!(port = 39090, api_port = cfg.port, share_port = cfg.share_port, "discovery broadcast started");

    // 协调服务器（跨网络互联 + 遥测）：AIPOWERLINK_COORD_URL 配置后启用（默认关闭 = 纯局域网零服务器）
    if let Ok(coord_url) = std::env::var("AIPOWERLINK_COORD_URL") {
        if !coord_url.is_empty() {
            let client = aipg_coord_client::DeviceClient::new(aipg_coord_client::DeviceClientConfig {
                base_url: coord_url.clone(),
                heartbeat_interval_s: 60,
                timeout_s: 10,
            });
            let node = aipg_coord_client::NodeInfo {
                name: "aipowerlink-share".to_string(),
                platform: std::env::consts::OS.to_string(),
                version: aipg_runtime::VERSION.to_string(),
                public_ip: std::env::var("AIPOWERLINK_PUBLIC_IP").unwrap_or_default(), // 跨网络接入需配置本机公网/可达地址
                api_port: cfg.port,
                region_hint: std::env::var("AIPOWERLINK_REGION").ok(),
            };
            let telemetry = aipg_coord_client::HeartbeatTelemetry {
                enabled: std::env::var("AIPOWERLINK_TELEMETRY").map(|v| v == "1").unwrap_or(false),
                platform: std::env::consts::OS.to_string(),
                version: aipg_runtime::VERSION.to_string(),
                region_hint: std::env::var("AIPOWERLINK_REGION").unwrap_or_default(),
            };
            let node2 = node.clone();
            let telemetry2 = telemetry.clone();
            let client2 = client.clone();
            let data_dir_owned = std::path::PathBuf::from(data_dir);

            // 登录插件(coord-account):可选增强,默认关闭(匿名优先——全部核心功能零登录依赖)。
            // 显式开关 account.enabled,或已存在登录会话(重启后免重复 enable)时装配。
            let account_session = aipg_coord_client::AccountStore::new(data_dir).load();
            let account_enabled = svc.get(RoleView::Global, "account.enabled").map(|v| v.as_deref() == Some("true")).unwrap_or(false)
                || account_session.is_logged_in();
            match aipg_coord_client::AccountPlugin::assemble(data_dir, &coord_url, account_enabled) {
                Some(p) => println!("account plugin: enabled — {}", p.session.summary()),
                None => tracing::info!("coord-account disabled — 匿名模式,全部核心功能可用(登录为可选增强)"),
            }

            tokio::spawn(async move {
                match client.register(&node2).await {
                    Ok(resp) => {
                        // 设备凭据持久化(device.json):供 account bind(绑定设备)与跨机找回凭据
                        let _ = aipg_coord_client::DeviceStore::new(&data_dir_owned)
                            .save(&resp.device_token, &resp.share_id);
                        println!("coord registered: share_id={}", resp.share_id);
                        println!("deep-link: aipowerlink://share?shareId={}", resp.share_id);
                        tracing::info!(share_id = %resp.share_id, "coord registered (deep-link ready)");
                        // 跨网 P2P（路径2）：后台打洞会话，收成员候选 → 打洞 → QUIC 承载 →
                        // 流桥接到本机共享通道（127.0.0.1:{share_port}），零服务器留存。
                        let punch_cfg = aipg_lan_share::LeaderPunchConfig {
                            tunnel_target: ([127, 0, 0, 1], share_port).into(),
                            cert_dir: data_dir_owned,
                            stun_addr: std::env::var("AIPOWERLINK_STUN_ADDR").ok()
                                .filter(|v| !v.is_empty()),
                            ..Default::default()
                        };
                        aipg_lan_share::spawn_leader_punch(client.clone(), punch_cfg);
                        println!("p2p: cross-network punch session started (QUIC tunnel → 127.0.0.1:{share_port})");
                        let _ = client.heartbeat_loop(telemetry2).await;
                    }
                    Err(e) => {
                        tracing::warn!(%e, "coord register failed (LAN-only mode continues)");
                    }
                }
            });
            let _ = client2;
            println!("coord-client: enabled ({coord_url})");
            tracing::info!(coord_url = %coord_url, "coord-client enabled");
        }
    }

    // 托盘（参考 cc-switch）：--no-tray 时纯 CLI
    if !no_tray {
        println!("starting system tray (use --no-tray for CLI-only)...");
        tracing::info!("system tray started (use --no-tray for CLI-only)");
        let tray = aipg_lan_tray::TrayService::new(aipg_lan_tray::TrayMode::Server)?;
        let server_handle = server.clone();
        // TrayIcon 非 Send，不能在 tokio::spawn；用 std::thread 轮询托盘动作
        std::thread::spawn(move || {
            loop {
                match tray.recv() {
                    aipg_lan_tray::TrayAction::OpenConsole => {
                        println!("[tray] open console: http://127.0.0.1:{}", 39091);
                        tracing::info!("[tray] open console");
                        let _ = open_browser(&format!("http://127.0.0.1:{}", 39091));
                    }
                    aipg_lan_tray::TrayAction::StartSharing => {
                        server_handle.set_sharing(true);
                        println!("[tray] sharing started");
                        tracing::info!("[tray] sharing started");
                    }
                    aipg_lan_tray::TrayAction::PauseSharing => {
                        server_handle.set_sharing(false);
                        println!("[tray] sharing paused");
                        tracing::info!("[tray] sharing paused");
                    }
                    aipg_lan_tray::TrayAction::Quit => {
                        println!("[tray] quitting...");
                        tracing::info!("[tray] quit requested");
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
        tracing::info!(console_url = %console_url, "opening management console");
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
    use aipg_lan_client::{DiscoveryClient, DiscoveryConfig, PreflightBlock, StrategyCache, fetch_strategy};
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
    // 链路加密：link.encrypt = off | aes-gcm | tls（缺省 off；跨网络深链建议 aes-gcm）
    let encrypt = match svc.get(RoleView::Global, "link.encrypt").map_err(|e| anyhow::anyhow!("config read link.encrypt: {e}"))? {
        Some(v) => aipg_link_crypto::LinkEncryptMode::parse(&v),
        None => aipg_link_crypto::LinkEncryptMode::Off,
    };
    let gateway = MemberGateway::with_encrypt(discovery.clone(), encrypt);
    if encrypt != aipg_link_crypto::LinkEncryptMode::Off {
        println!("link-encrypt: enabled ({encrypt:?}) — 跨网络深链请求/响应将加密传输");
        tracing::info!(encrypt = ?encrypt, "link encryption enabled (cross-network deep-link traffic encrypted)");
    }

    // 协调服务器（组员端同样注册 + 心跳）：AIPOWERLINK_COORD_URL 配置后启用（默认关闭 = 纯局域网）
    if let Ok(coord_url) = std::env::var("AIPOWERLINK_COORD_URL") {
        if !coord_url.is_empty() {
            // Deep Link 跨网络接入：AIPOWERLINK_JOIN_SHARE_ID = 组长分享的 shareId（可选）
            let join_share = std::env::var("AIPOWERLINK_JOIN_SHARE_ID")
                .ok()
                .map(|v| v.trim().to_string())
                .filter(|v| !v.is_empty());
            let client = aipg_coord_client::DeviceClient::new(aipg_coord_client::DeviceClientConfig {
                base_url: coord_url.clone(),
                heartbeat_interval_s: 60,
                timeout_s: 10,
            });
            let node = aipg_coord_client::NodeInfo {
                name: format!("member-{}", hostname_fallback()),
                platform: std::env::consts::OS.to_string(),
                version: aipg_runtime::VERSION.to_string(),
                public_ip: String::new(),
                api_port: port,
                region_hint: std::env::var("AIPOWERLINK_REGION").ok(),
            };
            let telemetry = aipg_coord_client::HeartbeatTelemetry {
                enabled: std::env::var("AIPOWERLINK_TELEMETRY").map(|v| v == "1").unwrap_or(false),
                platform: std::env::consts::OS.to_string(),
                version: aipg_runtime::VERSION.to_string(),
                region_hint: std::env::var("AIPOWERLINK_REGION").unwrap_or_default(),
            };
            let gw = gateway.clone();
            let data_dir_owned = std::path::PathBuf::from(data_dir);

            // 登录插件(coord-account):可选增强,默认关闭(匿名优先——接入/代理/策略零登录依赖)。
            let account_session = aipg_coord_client::AccountStore::new(data_dir).load();
            let account_enabled = svc.get(RoleView::Global, "account.enabled").map(|v| v.as_deref() == Some("true")).unwrap_or(false)
                || account_session.is_logged_in();
            match aipg_coord_client::AccountPlugin::assemble(data_dir, &coord_url, account_enabled) {
                Some(p) => println!("account plugin: enabled — {}", p.session.summary()),
                None => tracing::info!("coord-account disabled — 匿名模式,全部核心功能可用(登录为可选增强)"),
            }

            tokio::spawn(async move {
                match client.register(&node).await {
                    Ok(resp) => {
                        // 设备凭据持久化(device.json):供 account bind(绑定设备)与跨机找回凭据
                        let _ = aipg_coord_client::DeviceStore::new(&data_dir_owned)
                            .save(&resp.device_token, &resp.share_id);
                        println!("coord registered (member): share_id={}", resp.share_id);
                        tracing::info!(share_id = %resp.share_id, "coord registered (member)");
                        let hb = client.clone();
                        tokio::spawn(async move { let _ = hb.heartbeat_loop(telemetry).await; });

                        // Deep Link：解析组长 shareId → 注入静态组长（直连）；随后后台打洞建 QUIC 隧道。
                        if let Some(join_share) = &join_share {
                            let join_share = join_share.clone();
                            let join_share2 = join_share.clone();
                            let tunnel_gw = gw.clone();
                            let resolve_client = client.clone();
                            tokio::spawn(async move {
                                match resolve_client.resolve(&join_share).await {
                                    Ok(node) => {
                                        let leader = aipg_lan_client::LeaderInfo {
                                            name: node.name.clone(),
                                            api_port: node.api_port,
                                            share_port: Some(node.api_port), // 跨网络直连组长 API 端口
                                            fingerprint: node.fingerprint.clone(),
                                            address: node.public_ip.clone(),
                                            last_seen: 0,
                                            online: node.online,
                                        };
                                        tunnel_gw.set_static_leader(leader);
                                        println!("deep-link: joined {} ({}) via shareId {join_share}", node.name, node.public_ip);
                                        tracing::info!(name = %node.name, ip = %node.public_ip, share_id = %join_share, "deep-link joined (static leader injected)");

                                        // 跨网 P2P（路径2）：同一已注册 client（带 token）打洞。成功后把代理目标
                                        // 切到本机 QUIC 隧道端口（127.0.0.1:{port} → QUIC → 组长共享通道）；
                                        // 打洞失败 → 明确报“无法连接”，无中继兜底（pin 在 resolve 直连作为常规路径）。
                                        let leader_name = node.name.clone();
                                        let stun_addr = std::env::var("AIPOWERLINK_STUN_ADDR").ok()
                                            .filter(|v| !v.is_empty());
                                        match aipg_lan_client::punch_join(&resolve_client, &join_share2, stun_addr).await {
                                            Ok(tunnel) => {
                                                // 切代理目标到本地隧道端口（link_base = http://127.0.0.1:{port}）
                                                let tunnel_leader = aipg_lan_client::LeaderInfo {
                                                    name: leader_name,
                                                    api_port: tunnel.local_port,
                                                    share_port: Some(tunnel.local_port),
                                                    fingerprint: String::new(),
                                                    address: "127.0.0.1".to_string(),
                                                    last_seen: 0,
                                                    online: true,
                                                };
                                                tunnel_gw.set_static_leader(tunnel_leader);
                                                println!("p2p: QUIC tunnel up — switching proxy to 127.0.0.1:{} (组长直连，服务器仅信令零留存)", tunnel.local_port);
                                                tracing::info!(port = tunnel.local_port, share_id = %join_share2, "p2p tunnel established, proxy switched to local tunnel");
                                            }
                                            Err(e) => {
                                                eprintln!("p2p: 无法连接（打洞失败）: {e} — 保持 resolve 直连，无中继兜底");
                                                tracing::warn!(share_id = %join_share2, error = %e, "punch failed: cannot connect (no relay fallback)");
                                            }
                                        }
                                    }
                                    Err(e) => {
                                        eprintln!("deep-link: resolve {join_share} failed: {e} (fallback to LAN discovery)");
                                        tracing::warn!(share_id = %join_share, error = %e, "deep-link resolve failed, falling back to LAN discovery");
                                    }
                                }
                            });
                        }
                    }
                    Err(e) => {
                        tracing::warn!(%e, "coord register failed (LAN-only mode continues)");
                    }
                }
            });
            println!("coord-client: enabled ({coord_url})");
            tracing::info!(coord_url = %coord_url, "coord-client enabled (member)");
        }
    }

    async fn proxy_resp(g: &MemberGateway, path: &str, auth: Option<&str>, body: Option<Vec<u8>>) -> Response {
        match g.proxy(path, auth, body).await {
            Ok((status, bytes)) => (
                StatusCode::from_u16(status).unwrap_or(StatusCode::BAD_GATEWAY),
                bytes,
            ).into_response(),
            Err(e) => (StatusCode::SERVICE_UNAVAILABLE, Json(json!({ "error": { "message": e } }))).into_response(),
        }
    }

    // 请求前置状态（成员 gateway + 本地只读策略镜像；策略由后台周期拉取刷新）
    #[derive(Clone)]
    struct ClientState {
        gateway: MemberGateway,
        strategy: StrategyCache,
    }

    async fn h_token(State(s): State<ClientState>, body: Bytes) -> Response {
        proxy_resp(&s.gateway, "/auth/token", None, Some(body.to_vec())).await
    }
    async fn h_models(State(s): State<ClientState>) -> Response {
        // 本地策略镜像命中（规则名非空）→ 直返规则名列表：App 本地即可见可选规则（零知识：无真实模型）
        let rules = s.strategy.rule_names();
        if !rules.is_empty() {
            let data: Vec<serde_json::Value> = rules
                .into_iter()
                .map(|name| json!({ "id": name, "object": "model", "created": 0, "owned_by": "aipowerlink-rule" }))
                .collect();
            return Json(json!({ "object": "list", "data": data })).into_response();
        }
        // 无缓存/组长无规则 → 透传组长（按真实模型目录返回，权威兜底）
        proxy_resp(&s.gateway, "/v1/models", None, None).await
    }
    async fn h_chat(State(s): State<ClientState>, headers: axum::http::HeaderMap, body: Bytes) -> Response {
        // 请求前预检：被拉黑 403 / 配额已超限 429（形状与组长一致）；未命中拦截 → 照常透传
        match s.strategy.preflight() {
            Some(PreflightBlock::Banned) => return member_banned_blocked(),
            Some(PreflightBlock::QuotaExceeded { limit }) => return quota_exceeded_block(limit),
            None => {}
        }
        let auth = headers.get(header::AUTHORIZATION).and_then(|v| v.to_str().ok()).map(|s| s.to_string());
        proxy_resp(&s.gateway, "/v1/chat/completions", auth.as_deref(), Some(body.to_vec())).await
    }
    async fn h_messages(State(s): State<ClientState>, headers: axum::http::HeaderMap, body: Bytes) -> Response {
        match s.strategy.preflight() {
            Some(PreflightBlock::Banned) => return member_banned_blocked(),
            Some(PreflightBlock::QuotaExceeded { limit }) => return quota_exceeded_block(limit),
            None => {}
        }
        let auth = headers.get(header::AUTHORIZATION).and_then(|v| v.to_str().ok()).map(|s| s.to_string());
        proxy_resp(&s.gateway, "/v1/messages", auth.as_deref(), Some(body.to_vec())).await
    }
    async fn h_status(State(s): State<ClientState>) -> Response {
        let strategy = match s.strategy.get() {
            Some(m) => json!({
                "version": m.version,
                "rules": m.rules.len(),
                "quota": { "limit": m.quota.limit, "used": m.quota.used },
                "banned": m.banned,
                "fetched_at": m.fetched_at,
            }),
            None => serde_json::Value::Null,
        };
        Json(json!({
            "role": "client",
            "leaders": s.gateway.leader_count(),
            "leader": s.gateway.leader_summary(),
            "strategy": strategy,
        })).into_response()
    }

    // 预检拦截响应（形状与组长错误对齐，本地提前拦截专用）
    fn member_banned_blocked() -> Response {
        (StatusCode::FORBIDDEN, Json(json!({
            "error": {
                "message": "blocked: member banned",
                "type": "member_banned",
                "code": "banned",
            }
        }))).into_response()
    }
    fn quota_exceeded_block(limit: u64) -> Response {
        (StatusCode::TOO_MANY_REQUESTS, Json(json!({
            "error": {
                "message": format!("quota exceeded: limit {limit} tokens"),
                "type": "insufficient_quota",
                "code": "quota_exceeded",
                "quota_limit": limit,
            }
        }))).into_response()
    }

    // 策略镜像后台刷新：启动即拉 + 每 60s（无组长/失败 → 保留上次缓存降级；权威仍在组长）
    let strategy = StrategyCache::new(Some(data_dir.join("strategy-cache.json")));
    {
        let gw = gateway.clone();
        let strat = strategy.clone();
        let machine = hostname_fallback();
        tokio::spawn(async move {
            loop {
                match fetch_strategy(&gw, &machine).await {
                    Ok(summary) => {
                        tracing::info!(version = summary.version, rules = summary.rules.len(), "strategy mirror refreshed");
                        strat.set(summary);
                    }
                    Err(e) => {
                        tracing::debug!(%e, "strategy refresh deferred (keep last cache)");
                    }
                }
                tokio::time::sleep(std::time::Duration::from_secs(60)).await;
            }
        });
    }

    let app = Router::new()
        .route("/", get(h_status))
        .route("/auth/token", post(h_token))
        .route("/v1/models", get(h_models))
        .route("/v1/chat/completions", post(h_chat))
        .route("/v1/messages", post(h_messages))
        .with_state(ClientState { gateway: gateway.clone(), strategy });

    let addr: std::net::SocketAddr = std::net::SocketAddr::from(([127, 0, 0, 1], port));
    let listener = tokio::net::TcpListener::bind(addr).await.map_err(|e| anyhow::anyhow!("member gateway bind {addr}: {e}"))?;
    println!("member gateway: listening on http://127.0.0.1:{}", port);
    println!("discovery: UDP :{} (auto-discover leader, forward via gateway channel)", 39090);
    tracing::info!(port, "member gateway listening");
    if !no_tray {
        eprintln!("[tray] client role: tray not provided, use --no-tray (default behavior overrides)");
        tracing::warn!("client role has no tray; run with --no-tray");
    }

    axum::serve(listener, app).await.map_err(|e| anyhow::anyhow!("member gateway serve: {e}"))?;
    Ok(())
}

/// 获取本机主机名（失败时回退 "unknown"，供协调服务器节点命名）。
fn hostname_fallback() -> String {
    std::env::var("COMPUTERNAME")
        .or_else(|_| std::env::var("HOSTNAME"))
        .unwrap_or_else(|_| "unknown".to_string())
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

fn handle_autostart(sub: &AutostartCmd, cli: &Cli) {
    use aipg_runtime::auto_launch;

    // 开机启动时带上与当前启动一致的参数：--no-tray / --role / --backend / --data-dir，
    // 保证开机启动实例与手动启动实例行为一致（数据目录、托盘、后端）。
    let launch_args = || {
        let mut args: Vec<String> = Vec::new();
        if cli.no_tray { args.push("--no-tray".to_string()); }
        if cli.role != "server" {
            args.push("--role".to_string());
            args.push(cli.role.clone());
        }
        if cli.backend != "mock" {
            args.push("--backend".to_string());
            args.push(cli.backend.clone());
        }
        if let Some(d) = &cli.data_dir {
            args.push("--data-dir".to_string());
            args.push(d.to_string_lossy().to_string());
        }
        args
    };

    match sub {
        AutostartCmd::Enable => {
            let args = launch_args();
            match auto_launch::build_with_args(&args).and_then(|a| {
                a.enable().map_err(|e| aipg_runtime::RuntimeError::Other(format!("enable autostart: {e}")))
            }) {
                Ok(()) => println!(
                    "autostart: enabled (boot args: {})",
                    if args.is_empty() { "(none)".to_string() } else { args.join(" ") }
                ),
                Err(e) => { eprintln!("error: {e}"); std::process::exit(1); }
            }
        }
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

/// 登录插件命令(可选增强,匿名优先)。
///
/// 产品三原则落地:
/// - 不登录完整可用:插件默认关闭,所有命令与核心功能零登录依赖;
/// - 登录仅增强、可随时关闭:`login/logout` 即时切换;`disable` 同时关闭插件与会话;
/// - 插件化可卸载:遵循 coord-account 模块语义,自定义角色模块清单移除即彻底卸载。
async fn handle_account(sub: &AccountCmd, data_dir_override: &Option<PathBuf>) {
    use aipg_coord_client::{AccountClient, AccountClientConfig, AccountStore, DeviceStore};
    let data_dir = data_dir_override.clone().unwrap_or_else(aipg_runtime::data_dir::default_data_dir);
    std::fs::create_dir_all(&data_dir).ok();
    let store = AccountStore::new(&data_dir);

    // 账号服务器地址:显式环境变量 > 协调服务器地址 > config(account.base_url)
    let base_url = std::env::var("AIPOWERLINK_ACCOUNT_URL")
        .ok()
        .filter(|v| !v.is_empty())
        .or_else(|| std::env::var("AIPOWERLINK_COORD_URL").ok().filter(|v| !v.is_empty()))
        .or_else(|| {
            aipg_config::ConfigService::open(&data_dir, "aipowerlink.db")
                .ok()
                .and_then(|svc| svc.get(aipg_config::RoleView::Global, "account.base_url").ok().flatten())
        });

    // account.enabled 插件开关(默认关闭 = 匿名优先)
    let enabled = aipg_config::ConfigService::open(&data_dir, "aipowerlink.db")
        .ok()
        .and_then(|svc| svc.get(aipg_config::RoleView::Global, "account.enabled").ok().flatten())
        .map(|v| v == "true")
        .unwrap_or(false);

    match sub {
        AccountCmd::Status => {
            let s = store.load();
            println!("account: {}", s.summary());
            println!("account plugin: {} (登录为可选增强)", if enabled { "enabled" } else { "disabled" });
            match &base_url {
                Some(u) => println!("account server: {u}"),
                None => println!("account server: (未配置 — 设 AIPOWERLINK_ACCOUNT_URL / AIPOWERLINK_COORD_URL 或 config set account.base_url)"),
            }
        }
        AccountCmd::Enable => {
            if !enabled {
                if let Ok(svc) = aipg_config::ConfigService::open(&data_dir, "aipowerlink.db") {
                    let _ = svc.set(aipg_config::RoleView::Global, "account.enabled", "true", false);
                }
            }
            println!("account plugin: enabled — 登录为可选增强(account register / login / bind)");
        }
        AccountCmd::Disable => {
            let _ = store.clear(); // 关闭插件同时登出
            if let Ok(svc) = aipg_config::ConfigService::open(&data_dir, "aipowerlink.db") {
                let _ = svc.set(aipg_config::RoleView::Global, "account.enabled", "false", false);
            }
            println!("account plugin: disabled — 已登出,恢复匿名模式,全部核心功能可用");
        }
        AccountCmd::Register { username, email } => {
            let url = match &base_url {
                Some(u) => u.clone(),
                None => { eprintln!("error: 未配置账号服务器(AIPOWERLINK_ACCOUNT_URL / AIPOWERLINK_COORD_URL / account.base_url)"); std::process::exit(1); }
            };
            if !enabled { println!("note: 插件未启用 — 注册后可 account login,并 account enable 保持登录增强"); }
            let client = AccountClient::new(AccountClientConfig { base_url: url, ..Default::default() });
            match client.register(username, email).await {
                Ok(()) => println!("account: 注册请求已发送 — 动态密码已发往 {email}(邮箱 OTP)"),
                Err(e) => { eprintln!("error: {e}"); std::process::exit(1); }
            }
        }
        AccountCmd::Login { username, otp } => {
            let url = match &base_url {
                Some(u) => u.clone(),
                None => { eprintln!("error: 未配置账号服务器(AIPOWERLINK_ACCOUNT_URL / AIPOWERLINK_COORD_URL / account.base_url)"); std::process::exit(1); }
            };
            if !enabled {
                if let Ok(svc) = aipg_config::ConfigService::open(&data_dir, "aipowerlink.db") {
                    let _ = svc.set(aipg_config::RoleView::Global, "account.enabled", "true", false);
                }
                println!("account plugin: enabled (登录即自动启用插件)");
            }
            let client = AccountClient::new(AccountClientConfig { base_url: url, ..Default::default() });
            match client.login(username, otp).await {
                Ok(resp) => {
                    let mut s = store.load();
                    s.username = Some(username.clone());
                    s.token = Some(resp.token);
                    s.device_bound = resp.device_bound;
                    if let Err(e) = store.save(&s) {
                        eprintln!("error: 会话保存失败: {e}");
                        std::process::exit(1);
                    }
                    println!("account: 已登录({username}) — 会话已持久化,重启后自动恢复;可 account bind 绑定本机设备");
                }
                Err(e) => { eprintln!("error: {e}"); std::process::exit(1); }
            }
        }
        AccountCmd::Bind => {
            let s = store.load();
            if !s.is_logged_in() {
                eprintln!("error: 未登录 — 先 account login(绑定设备需要有效会话)");
                std::process::exit(1);
            }
            let device = match DeviceStore::new(&data_dir).load() {
                Some(d) => d,
                None => {
                    eprintln!("error: 本机尚未注册设备(device.json 缺失 — 需启用协调服务器 AIPOWERLINK_COORD_URL 并启动 gateway 完成注册)");
                    std::process::exit(1);
                }
            };
            let url = match &base_url {
                Some(u) => u.clone(),
                None => { eprintln!("error: 未配置账号服务器"); std::process::exit(1); }
            };
            let client = AccountClient::new(AccountClientConfig { base_url: url, ..Default::default() });
            if let Some(token) = s.token.as_deref() {
                client.restore_session(token);
            }
            match client.bind_device(&device.device_token).await {
                Ok(()) => {
                    let mut updated = s;
                    updated.device_bound = true;
                    updated.share_id = Some(device.share_id.clone());
                    if let Err(e) = store.save(&updated) {
                        eprintln!("error: 会话保存失败: {e}");
                        std::process::exit(1);
                    }
                    println!("account: 本机设备已绑定到账号(shareId={}) — 跨机找回凭据已登记", device.share_id);
                }
                Err(e) => { eprintln!("error: {e}"); std::process::exit(1); }
            }
        }
        AccountCmd::Logout => {
            let was = store.load().is_logged_in();
            match store.clear() {
                Ok(()) => {
                    if was {
                        println!("account: 已登出 — 恢复匿名模式,全部核心功能不受影响");
                    } else {
                        println!("account: 当前即匿名模式(无需登出)");
                    }
                }
                Err(e) => { eprintln!("error: 登出清理失败: {e}"); std::process::exit(1); }
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