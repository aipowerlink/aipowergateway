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
    println!("backend: {}", cli.backend);
    println!("tray: {}", if cli.no_tray { "disabled" } else { "enabled" });
    println!("data_dir: {}", data_dir.display());

    let result = match cli.role.as_str() {
        "server" => run_server(&data_dir, &cli.backend, cli.no_tray).await,
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

/// 构造多后端注册表（mock / deepseek / kimi / zhipu，可逗号分隔）。
fn build_registry(backend_arg: &str) -> anyhow::Result<aipg_lan_share::BackendRegistry> {
    use aipg_lan_share::{BackendRegistry, MockBackend, OpenAICompatBackend, OpenAICompatConfig, Provider};
    let mut registry = BackendRegistry::new();
    for name in backend_arg.split(',') {
        let name = name.trim();
        if name.is_empty() {
            continue;
        }
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
        let cfg = OpenAICompatConfig {
            provider,
            api_key,
            model,
            base_url,
            timeout_secs: 60,
        };
        registry.register(Arc::new(OpenAICompatBackend::new(cfg)));
    }
    if registry.backend_count() == 0 {
        anyhow::bail!("no backend configured");
    }
    Ok(registry)
}

/// 以服务端角色运行（组长）。
async fn run_server(data_dir: &std::path::Path, backend_arg: &str, _no_tray: bool) -> anyhow::Result<()> {
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