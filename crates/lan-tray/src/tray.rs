//! lan-tray：系统托盘（tauri-apps tray-icon 独立库，参考 cc-switch）。
//!
//! - 服务端（组长）菜单：打开管理面板 / 开启共享 / 暂停共享 / 修改密码 / 退出
//! - 消费端（组员）菜单：组长列表（点击接入）/ 接入状态 / 改名 / 用量 / 退出
//! - 关闭不退出（最小侵入）；--no-tray 纯 CLI 兜底
//!
//! **线程模型**：TrayIcon 内部是 Rc<RefCell>（非 Send），必须留在创建它的原生线程。
//! 因此托盘 + 事件循环整体跑在一个 std::thread 里，TrayService 仅经 mpsc channel
//! 与外部通信（TrayService 本身 Send，可进 tokio::spawn）。

use std::sync::mpsc;
use std::sync::{Arc, Mutex};

use tray_icon::menu::{Menu, MenuEvent, MenuItem, PredefinedMenuItem};
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

/// 托盘服务：跨线程安全句柄（内部线程持有 TrayIcon）。
#[derive(Clone)]
pub struct TrayService {
    /// 动作接收端（宿主轮询）。
    rx: Arc<Mutex<mpsc::Receiver<TrayAction>>>,
}

impl TrayService {
    /// 创建托盘：原生线程持有 TrayIcon + 事件循环。
    pub fn new(mode: TrayMode) -> RuntimeResult<Self> {
        let (tx, rx) = mpsc::channel::<TrayAction>();
        let (ready_tx, ready_rx) = mpsc::channel::<RuntimeResult<()>>();

        // 托盘线程：创建图标 + 跑事件循环（不返回，线程持活）
        std::thread::spawn(move || {
            // 事件接收器必须在 build 之前获取
            let menu_receiver = MenuEvent::receiver().clone();
            let icon_receiver = TrayIconEvent::receiver().clone();

            let items: Arc<Mutex<Vec<(String, TrayAction)>>> = Arc::new(Mutex::new(Vec::new()));
            let leader_items: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));

            // 构建菜单
            let menu = match build_menu(mode, &items, &leader_items) {
                Ok(m) => m,
                Err(e) => { let _ = ready_tx.send(Err(e)); return; }
            };

            // 托盘图标
            let icon = match build_icon() {
                Ok(i) => i,
                Err(e) => { let _ = ready_tx.send(Err(e)); return; }
            };
            let _tray = match TrayIconBuilder::new()
                .with_menu(Box::new(menu))
                .with_tooltip("AIPowerLink")
                .with_icon(icon)
                .build()
            {
                Ok(t) => t,
                Err(e) => { let _ = ready_tx.send(Err(RuntimeError::Other(format!("tray build: {e}")))); return; }
            };

            // 托盘创建成功，通知主线程
            let _ = ready_tx.send(Ok(()));

            // 事件循环：菜单点击 → 动作
            let items_clone = items.clone();
            let tx_clone = tx.clone();
            loop {
                // 非阻塞合并检查两个 channel（简单轮询）
                if let Ok(event) = menu_receiver.try_recv() {
                    let id = event.id().0.clone();
                    let action = items_clone.lock().unwrap().iter()
                        .find(|(i, _)| *i == id)
                        .map(|(_, a)| a.clone())
                        .unwrap_or(TrayAction::None);
                    let _ = tx_clone.send(action);
                }
                if let Ok(_event) = icon_receiver.try_recv() {
                    // 点击图标不处理（菜单为主）
                }
                std::thread::sleep(std::time::Duration::from_millis(100));
            }
        });

        // 等待托盘创建结果
        match ready_rx.recv() {
            Ok(Ok(())) => {}
            Ok(Err(e)) => return Err(e),
            Err(_) => return Err(RuntimeError::Other("tray thread died".to_string())),
        }

        Ok(Self { rx: Arc::new(Mutex::new(rx)) })
    }

    /// 接收下一个动作（阻塞；返回 None 表示通道关闭）。
    pub fn recv(&self) -> TrayAction {
        self.rx.lock().unwrap().recv().unwrap_or(TrayAction::None)
    }

    /// 尝试接收（非阻塞）。
    pub fn try_recv(&self) -> Option<TrayAction> {
        self.rx.lock().unwrap().try_recv().ok()
    }
}

/// 构建菜单（按模式）。
fn build_menu(
    mode: TrayMode,
    items: &Arc<Mutex<Vec<(String, TrayAction)>>>,
    _leader_items: &Arc<Mutex<Vec<String>>>,
) -> RuntimeResult<Menu> {
    let menu = Menu::new();
    match mode {
        TrayMode::Server => {
            add_item(&menu, items, "打开管理面板", TrayAction::OpenConsole)?;
            menu.append(&PredefinedMenuItem::separator())
                .map_err(|e| RuntimeError::Other(format!("tray sep: {e}")))?;
            add_item(&menu, items, "开启共享", TrayAction::StartSharing)?;
            add_item(&menu, items, "暂停共享", TrayAction::PauseSharing)?;
            add_item(&menu, items, "修改密码", TrayAction::ChangePassword)?;
            menu.append(&PredefinedMenuItem::separator())
                .map_err(|e| RuntimeError::Other(format!("tray sep: {e}")))?;
            add_item(&menu, items, "退出", TrayAction::Quit)?;
        }
        TrayMode::Client => {
            add_item(&menu, items, "发现的组长：", TrayAction::None)?;
            add_item(&menu, items, "  （启动后自动发现）", TrayAction::None)?;
            menu.append(&PredefinedMenuItem::separator())
                .map_err(|e| RuntimeError::Other(format!("tray sep: {e}")))?;
            add_item(&menu, items, "接入状态", TrayAction::None)?;
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