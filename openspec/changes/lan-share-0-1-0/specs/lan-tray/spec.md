## Purpose

双角色共用系统托盘（Tauri，参考 cc-switch）：服务端/消费端均以托盘常驻，托盘菜单提供快捷操作，关闭窗口不退出服务，实现最小侵入的后台运行。

## ADDED Requirements

### Requirement: 托盘常驻
系统 SHALL 以系统托盘图标常驻运行，启动即进托盘，关闭窗口/网页不退出服务。

#### Scenario: 启动进托盘
- **WHEN** 组长/组员启动程序
- **THEN** 托盘图标出现，服务后台运行

#### Scenario: 关闭不退出
- **WHEN** 用户关闭管理网页/窗口
- **THEN** 服务继续后台运行

### Requirement: 服务端托盘菜单
服务端托盘菜单 SHALL 提供：打开管理面板、开启/暂停共享、修改密码、退出。

#### Scenario: 托盘操作
- **WHEN** 组长通过托盘菜单操作
- **THEN** 对应操作生效

### Requirement: 消费端托盘菜单
消费端托盘菜单 SHALL 提供：发现的组长列表（点击接入）、接入状态、修改显示名、查看个人用量、退出。

#### Scenario: 托盘接入
- **WHEN** 组员通过托盘选择组长
- **THEN** 尝试接入该组长

### Requirement: 无托盘启动
系统 SHALL 支持 --no-tray 纯命令行启动（不创建托盘图标）。

#### Scenario: 无托盘运行
- **WHEN** 用户以 --no-tray 启动
- **THEN** 服务以命令行方式运行