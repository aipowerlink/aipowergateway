//! lan-tray：系统托盘（tauri-apps tray-icon 独立库，参考 cc-switch）。
//!
//! - 服务端（组长）菜单：打开管理面板 / 开启共享 / 暂停共享 / 修改密码 / 退出
//! - 消费端（组员）菜单：组长列表（点击接入）/ 接入状态 / 改名 / 用量 / 退出
//! - 关闭不退出（最小侵入）；--no-tray 纯 CLI 兜底

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

/// 托盘服务：图标 + 菜单 + 事件接收。
pub struct TrayService {
    /// 动作接收端（宿主轮询）。
    rx: mpsc::Receiver<TrayAction>,
    /// 菜单项映射（menu id → 动作）。
    #[allow(dead_code)]
    items: Arc<Mutex<Vec<(String, TrayAction)>>>,
    /// 消费端动态组长菜单项（重建用）。
    leader_items: Arc<Mutex<Vec<String>>>,
}

impl TrayService {
    /// 创建托盘（阻塞运行事件循环；返回前已完成初始化）。
    pub fn new(mode: TrayMode) -> RuntimeResult<Self> {
        let (tx, rx) = mpsc::channel::<TrayAction>();
        let items: Arc<Mutex<Vec<(String, TrayAction)>>> = Arc::new(Mutex::new(Vec::new()));
        let leader_items: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));

        // 构建菜单
        let menu = build_menu(mode, &items, &leader_items)?;

        // 托盘图标（内置简单图标）
        let icon = build_icon()?;
        let _tray = TrayIconBuilder::new()
            .with_menu(Box::new(menu))
            .with_tooltip("AIPowerLink")
            .with_icon(icon)
            .build()
            .map_err(|e| RuntimeError::Other(format!("tray build: {e}")))?;

        // 事件转发线程：菜单点击 + 托盘事件
        let items_clone = items.clone();
        let tx_clone = tx.clone();
        std::thread::spawn(move || {
            // 菜单事件监听（crossbeam 通道）
            let receiver = MenuEvent::receiver();
            loop {
                if let Ok(event) = receiver.recv() {
                    let event = event.clone();
                    let id = event.id().0.clone();
                    let action = items_clone.lock().unwrap().iter()
                        .find(|(i, _)| *i == id)
                        .map(|(_, a)| a.clone())
                        .unwrap_or(TrayAction::None);
                    let _ = tx_clone.send(action);
                }
            }
        });

        // 托盘图标事件（点击/退出）监听
        std::thread::spawn(move || {
            let receiver = TrayIconEvent::receiver();
            loop {
                if let Ok(_event) = receiver.recv() {
                    // 0.1.0：点击图标不处理（菜单为主）
                }
            }
        });

        Ok(Self { rx, items, leader_items })
    }

    /// 接收下一个动作（阻塞；超时返回 None）。供宿主轮询。
    pub fn try_recv(&self) -> Option<TrayAction> {
        self.rx.try_recv().ok()
    }

    /// 阻塞接收动作。
    pub fn recv(&self) -> TrayAction {
        self.rx.recv().unwrap_or(TrayAction::None)
    }

    /// 消费端：动态更新组长列表菜单。
    pub fn update_leaders(&self, _leaders: Vec<String>) {
        // 0.1.0：菜单重建较复杂，组长列表静态展示提示（1.x 动态）
        drop(self.leader_items.lock().unwrap());
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

/// 简单内置图标（16x16 RGBA 填充）。
fn build_icon() -> RuntimeResult<Icon> {
    let mut rgba = vec![0u8; 16 * 16 * 4];
    for px in rgba.chunks_exact_mut(4) {
        px.copy_from_slice(&[37, 99, 235, 255]); // 蓝色
    }
    // 画个简单 ⚡ 形状：中心竖条
    for y in 4..12 {
        for x in 7..9 {
            let idx = (y * 16 + x) * 4;
            rgba[idx..idx + 4].copy_from_slice(&[255, 255, 255, 255]);
        }
    }
    Icon::from_rgba(rgba, 16, 16).map_err(|e| RuntimeError::Other(format!("tray icon: {e}")))
}

/// 保持托盘事件循环存活（Windows 需要消息循环）。
pub fn run_event_loop() {
    // tray-icon 内部启动了消息循环线程；此处阻塞主线程保持进程
    loop {
        std::thread::sleep(std::time::Duration::from_secs(3600));
    }
}