//! lan-tray：系统托盘（tauri-apps tray-icon 独立库，参考 cc-switch）。
//!
//! - 服务端（组长）菜单：打开管理面板 / 开启共享 / 暂停共享 / 修改密码 / 退出
//! - 消费端（组员）菜单：组长列表（子菜单分层，点击接入）/ 接入状态 / 改名 / 用量 / 退出
//! - 动态菜单：经控制通道重建（cc-switch set_menu 模式），组长列表变化即时反映
//! - 关闭不退出（最小侵入）；--no-tray 纯 CLI 兜底
//!
//! **线程模型**：TrayIcon 内部是 Rc<RefCell>（非 Send），必须留在创建它的原生线程。
//! 托盘 + 事件循环整体跑在一个 std::thread 里；TrayService 经两条 channel 通信：
//! - 动作 channel（托盘 → 宿主：菜单点击）
//! - 控制 channel（宿主 → 托盘：重建菜单 / 更新状态）

use std::sync::mpsc;
use std::sync::{Arc, Mutex};

use tray_icon::menu::{Menu, MenuEvent, MenuItem, PredefinedMenuItem, Submenu};
use tray_icon::{Icon, TrayIconBuilder, TrayIconEvent};

use aipg_runtime::{RuntimeError, RuntimeResult};

/// 托盘菜单动作（宿主侧消费）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TrayAction {
    /// 打开管理面板（浏览器）。
    OpenConsole,
    /// 开启共享。
    StartSharing,
    /// 暂停共享。
    PauseSharing,
    /// 修改密码。
    ChangePassword,
    /// 接入某组长（参数：leader id）。
    ConnectLeader(String),
    /// 修改显示名。
    Rename,
    /// 查看个人用量。
    ShowUsage,
    /// 退出。
    Quit,
    /// 无操作。
    None,
}

/// 托盘模式。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrayMode {
    /// 服务端（组长）。
    Server,
    /// 消费端（组员）。
    Client,
}

/// 宿主 → 托盘 控制指令（cc-switch set_menu 模式）。
#[derive(Debug, Clone)]
pub enum TrayCommand {
    /// 更新消费端组长列表（重建"发现的组长"子菜单）。
    UpdateLeaders(Vec<LeaderEntry>),
    /// 更新接入状态。
    SetConnected(bool),
    /// 更新共享状态（服务端）。
    SetSharing(bool),
}

/// 组长条目（消费端菜单显示）。
#[derive(Debug, Clone)]
pub struct LeaderEntry {
    pub id: String,
    pub name: String,
    pub online: bool,
}

/// 托盘服务：跨线程安全句柄。
#[derive(Clone)]
pub struct TrayService {
    /// 动作接收端（宿主轮询）。
    rx: Arc<Mutex<mpsc::Receiver<TrayAction>>>,
    /// 控制发送端（宿主发指令到托盘线程）。
    cmd_tx: mpsc::Sender<TrayCommand>,
}

impl TrayService {
    /// 创建托盘：原生线程持有 TrayIcon + 事件循环。
    pub fn new(mode: TrayMode) -> RuntimeResult<Self> {
        let (tx, rx) = mpsc::channel::<TrayAction>();
        let (cmd_tx, cmd_rx) = mpsc::channel::<TrayCommand>();
        let (ready_tx, ready_rx) = mpsc::channel::<RuntimeResult<()>>();

        // 托盘线程：创建图标 + 事件循环 + 处理控制指令
        std::thread::spawn(move || {
            // 事件接收器必须在 build 之前获取
            let menu_receiver = MenuEvent::receiver().clone();
            let icon_receiver = TrayIconEvent::receiver().clone();

            let items: Arc<Mutex<Vec<(String, TrayAction)>>> = Arc::new(Mutex::new(Vec::new()));
            let mut leaders: Vec<LeaderEntry> = Vec::new();
            let mut connected = false;
            let mut sharing = true;

            // 初始菜单
            let menu = match build_menu(mode, &items, &leaders, connected, sharing) {
                Ok(m) => m,
                Err(e) => { let _ = ready_tx.send(Err(e)); return; }
            };

            let icon = match build_icon() {
                Ok(i) => i,
                Err(e) => { let _ = ready_tx.send(Err(e)); return; }
            };
            let tray = match TrayIconBuilder::new()
                .with_menu(Box::new(menu))
                .with_tooltip("AIPowerLink")
                .with_icon(icon)
                .build()
            {
                Ok(t) => t,
                Err(e) => { let _ = ready_tx.send(Err(RuntimeError::Other(format!("tray build: {e}")))); return; }
            };

            // 创建成功
            let _ = ready_tx.send(Ok(()));

            // 事件 + 控制循环
            loop {
                // 处理控制指令（重建菜单）
                while let Ok(cmd) = cmd_rx.try_recv() {
                    match cmd {
                        TrayCommand::UpdateLeaders(new_leaders) => {
                            leaders = new_leaders;
                            if let Ok(new_menu) = build_menu(mode, &items, &leaders, connected, sharing) {
                                let _ = tray.set_menu(Some(Box::new(new_menu)));
                            }
                        }
                        TrayCommand::SetConnected(c) => {
                            connected = c;
                            if let Ok(new_menu) = build_menu(mode, &items, &leaders, connected, sharing) {
                                let _ = tray.set_menu(Some(Box::new(new_menu)));
                            }
                        }
                        TrayCommand::SetSharing(s) => {
                            sharing = s;
                            if let Ok(new_menu) = build_menu(mode, &items, &leaders, connected, sharing) {
                                let _ = tray.set_menu(Some(Box::new(new_menu)));
                            }
                        }
                    }
                }
                // 菜单点击
                if let Ok(event) = menu_receiver.try_recv() {
                    let id = event.id().0.clone();
                    let action = items.lock().unwrap().iter()
                        .find(|(i, _)| *i == id)
                        .map(|(_, a)| a.clone())
                        .unwrap_or(TrayAction::None);
                    let _ = tx.send(action);
                }
                // 图标点击
                if let Ok(_event) = icon_receiver.try_recv() {
                    // 0.1.0：点击图标不处理（菜单为主）
                }
                std::thread::sleep(std::time::Duration::from_millis(100));
            }
        });

        match ready_rx.recv() {
            Ok(Ok(())) => {}
            Ok(Err(e)) => return Err(e),
            Err(_) => return Err(RuntimeError::Other("tray thread died".to_string())),
        }

        Ok(Self { rx: Arc::new(Mutex::new(rx)), cmd_tx })
    }

    /// 接收下一个动作（阻塞）。
    pub fn recv(&self) -> TrayAction {
        self.rx.lock().unwrap().recv().unwrap_or(TrayAction::None)
    }

    /// 尝试接收（非阻塞）。
    pub fn try_recv(&self) -> Option<TrayAction> {
        self.rx.lock().unwrap().try_recv().ok()
    }

    /// 更新组长列表（消费端菜单）。
    pub fn update_leaders(&self, leaders: Vec<LeaderEntry>) {
        let _ = self.cmd_tx.send(TrayCommand::UpdateLeaders(leaders));
    }

    /// 更新接入状态。
    pub fn set_connected(&self, connected: bool) {
        let _ = self.cmd_tx.send(TrayCommand::SetConnected(connected));
    }

    /// 更新共享状态（服务端）。
    pub fn set_sharing(&self, sharing: bool) {
        let _ = self.cmd_tx.send(TrayCommand::SetSharing(sharing));
    }
}

/// 构建菜单（cc-switch 分层模式：子菜单折叠 + 动态项）。
fn build_menu(
    mode: TrayMode,
    items: &Arc<Mutex<Vec<(String, TrayAction)>>>,
    leaders: &[LeaderEntry],
    connected: bool,
    sharing: bool,
) -> RuntimeResult<Menu> {
    let menu = Menu::new();
    match mode {
        TrayMode::Server => {
            // 顶部：打开管理面板（对应 cc-switch show_main）
            add_item(&menu, items, "打开管理面板", TrayAction::OpenConsole)?;
            menu.append(&PredefinedMenuItem::separator())
                .map_err(|e| RuntimeError::Other(format!("tray sep: {e}")))?;
            // 共享状态 + 操作
            add_item(&menu, items,
                if sharing { "✓ 共享中" } else { "○ 已暂停" },
                TrayAction::None)?;
            add_item(&menu, items, "开启共享", TrayAction::StartSharing)?;
            add_item(&menu, items, "暂停共享", TrayAction::PauseSharing)?;
            add_item(&menu, items, "修改密码", TrayAction::ChangePassword)?;
            menu.append(&PredefinedMenuItem::separator())
                .map_err(|e| RuntimeError::Other(format!("tray sep: {e}")))?;
            add_item(&menu, items, "退出", TrayAction::Quit)?;
        }
        TrayMode::Client => {
            // 顶部：接入状态
            add_item(&menu, items,
                if connected { "✓ 已接入" } else { "○ 未接入" },
                TrayAction::None)?;
            // 发现的组长（子菜单分层，对应 cc-switch 分应用子菜单）
            let leaders_sub = Submenu::new("发现的组长", true);
            if leaders.is_empty() {
                let item = MenuItem::new("（未发现组长）", true, None);
                let id = item.id().0.clone();
                items.lock().unwrap().push((id, TrayAction::None));
                leaders_sub.append(&item).map_err(|e| RuntimeError::Other(format!("tray sub item: {e}")))?;
            } else {
                for l in leaders {
                    let label = format!("{} {}", if l.online { "🟢" } else { "⚪" }, l.name);
                    let item = MenuItem::new(&label, true, None);
                    let id = item.id().0.clone();
                    items.lock().unwrap().push((id, TrayAction::ConnectLeader(l.id.clone())));
                    leaders_sub.append(&item).map_err(|e| RuntimeError::Other(format!("tray sub item: {e}")))?;
                }
            }
            menu.append(&leaders_sub)
                .map_err(|e| RuntimeError::Other(format!("tray submenu: {e}")))?;
            menu.append(&PredefinedMenuItem::separator())
                .map_err(|e| RuntimeError::Other(format!("tray sep: {e}")))?;
            add_item(&menu, items, "修改显示名", TrayAction::Rename)?;
            add_item(&menu, items, "查看个人用量", TrayAction::ShowUsage)?;
            menu.append(&PredefinedMenuItem::separator())
                .map_err(|e| RuntimeError::Other(format!("tray sep: {e}")))?;
            add_item(&menu, items, "退出", TrayAction::Quit)?;
        }
    }
    Ok(menu)
}

fn add_item(menu: &Menu, items: &Arc<Mutex<Vec<(String, TrayAction)>>>, label: &str, action: TrayAction) -> RuntimeResult<()> {
    let item = MenuItem::new(label, true, None);
    let id = item.id().0.clone();
    items.lock().unwrap().push((id, action));
    menu.append(&item).map_err(|e| RuntimeError::Other(format!("tray item: {e}")))?;
    Ok(())
}

/// 从嵌入的 PNG 加载图标（assets/icon-32.png，include_bytes!）。
fn build_icon() -> RuntimeResult<Icon> {
    const ICON_PNG: &[u8] = include_bytes!("../../../assets/png/icon-32.png");
    let image = image::load_from_memory(ICON_PNG)
        .map_err(|e| RuntimeError::Other(format!("icon decode: {e}")))?;
    let rgba = image.to_rgba8();
    let (w, h) = rgba.dimensions();
    Icon::from_rgba(rgba.into_raw(), w, h).map_err(|e| RuntimeError::Other(format!("icon build: {e}")))
}