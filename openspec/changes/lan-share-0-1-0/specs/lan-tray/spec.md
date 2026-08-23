## Purpose

双角色共用系统托盘（Tauri，参考 cc-switch）：服务端/消费端均以托盘常驻，托盘菜单提供快捷操作，关闭窗口不退出服务，实现最小侵入的后台运行。支持 Windows / Linux / macOS 三平台。

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

### Requirement: 跨平台支持
系统 SHALL 在 Windows、Linux、macOS 三平台同等提供托盘常驻与管理面板打开能力。

#### Scenario: Windows 运行
- **WHEN** 程序在 Windows 运行
- **THEN** 托盘图标出现，管理面板经系统浏览器打开

#### Scenario: Linux 运行
- **WHEN** 程序在 Linux 运行
- **THEN** 托盘图标出现，管理面板经系统浏览器打开

#### Scenario: macOS 运行
- **WHEN** 程序在 macOS 运行
- **THEN** 菜单栏托盘出现，管理面板经系统浏览器打开

### Requirement: 多语言支持
系统 SHALL 提供中英双语（默认跟随系统，可手动切换），覆盖托盘菜单与管理面板文案。

#### Scenario: 中文系统默认中文
- **WHEN** 系统 locale 为中文
- **THEN** 托盘菜单与管理面板以中文显示

#### Scenario: 英文系统默认英文
- **WHEN** 系统 locale 为英文
- **THEN** 托盘菜单与管理面板以英文显示

#### Scenario: 手动切换语言
- **WHEN** 用户在设置中切换语言
- **THEN** 文案即时切换并持久化偏好