//! aipowerlink CLI 入口：--role / --no-tray / config / role 子命令。

use std::path::PathBuf;

use clap::{Parser, Subcommand};

/// AIPowerLink 局域网算力共享网关（Rust + Tauri）。
#[derive(Parser, Debug)]
#[command(name = "aipowerlink", version, about)]
pub struct Cli {
    /// 运行角色（内置：server/client；或自定义角色 id）。
    #[arg(long, default_value = "server")]
    pub role: String,

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

    if let Some(cmd) = &cli.command {
        match cmd {
            Commands::Config { sub } => handle_config(sub),
            Commands::Role { sub } => handle_role(sub),
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
    println!("tray: {}", if cli.no_tray { "disabled" } else { "enabled" });
    println!("data_dir: {}", data_dir.display());

    let result = match cli.role.as_str() {
        "server" => run_server(&data_dir, cli.no_tray).await,
        "client" => {
            println!("[client] consumer role — stage 4 (not yet implemented)");
            Ok(())
        }
        other => {
            eprintln!("unknown role: {other}");
            std::process::exit(2);
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

/// 以服务端角色运行（组长）。
async fn run_server(data_dir: &std::path::Path, _no_tray: bool) -> anyhow::Result<()> {
    use aipg_lan_share::{BroadcastConfig, BroadcastService, MockBackend, ShareServer, ShareServerConfig};
    std::fs::create_dir_all(data_dir).map_err(|e| anyhow::anyhow!("create data dir: {e}"))?;
    let cfg = ShareServerConfig {
        port: 39091,
        password: std::env::var("AIPOWERLINK_PASSWORD").unwrap_or_else(|_| "aipowerlink".to_string()),
        token_ttl_secs: 12 * 3600,
        heartbeat_timeout_secs: 90,
        data_dir: data_dir.to_path_buf(),
    };
    let server = ShareServer::new(&cfg, std::sync::Arc::new(MockBackend::default()));
    println!("sharing: enabled on :{}", cfg.port);
    let fingerprint = server.fingerprint(8);
    println!("fingerprint: {}", fingerprint);
    // 启动 UDP 周期广播（组员端自动发现）
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
    let result = server.serve().await;
    broadcast.stop();
    result?;
    Ok(())
}

fn handle_config(sub: &ConfigCmd) {
    match sub {
        ConfigCmd::Get { key } => println!("config get {key} (stage 6)"),
        ConfigCmd::Set { key, value } => println!("config set {key}={value} (stage 6)"),
        ConfigCmd::List => println!("config list (stage 6, redacted)"),
    }
}

fn handle_role(sub: &RoleCmd) {
    match sub {
        RoleCmd::List => println!("roles: server (system), client (system) — stage 7"),
        RoleCmd::Show { id } => println!("role show {id} — stage 7"),
        RoleCmd::Clone { from, to } => println!("role clone {from} -> {to} — stage 7"),
        RoleCmd::New { id } => println!("role new {id} — stage 7"),
        RoleCmd::Edit { id } => println!("role edit {id} — stage 7"),
        RoleCmd::Rm { id } => println!("role rm {id} — stage 7"),
    }
}