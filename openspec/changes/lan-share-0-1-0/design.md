# Design: lan-share-0-1-0（aipowergateway Rust 架构）

## Context

现状（见 proposal.md — Why）：aipowergateway 为全新 Rust 工程（目前仅 README + openspec）。本 change 从零建设 0.1.0 局域网算力共享。参考：06 文档（角色运行时选择 §十、模块契约 §11.4）、DSH（代码/网页结构）、cc-switch（Tauri 托盘运行形态）。

## Goals / Non-Goals

**Goals:**
- Rust 实现 gateway 双角色（服务端/消费端）局域网共享，模块化装配
- Tauri 托盘常驻（参考 cc-switch）；管理网页三栏布局 + 组件化 + CSS Modules（参考 DSH web 管理面：AppFrame/SidebarRoot 模式）
- **跨平台（Win / Linux / macOS）**：三平台同等支持（参考 cc-switch 跨平台发布）
- **国际化（i18n）**：多语言支持（托盘菜单/管理网页/日志），默认 中/英，架构支持扩展
- 应用层 OpenAI 兼容 HTTP + 传输分层（0.1.0 TCP，1.x QUIC/P2P）
- 零云端依赖，局域网开箱即用

**Non-Goals:**
- 不做远程接入/打洞（0.1.0 仅 LAN；QUIC 路径 1.x）
- 不做计费/额度分配/模型白名单（单密码 + 可见性）
- 不做 SaaS 同步
- 不做长任务状态机（0.1.0 请求-响应式 API；任务状态机 1.x）

## Decisions

### D1：语言与工具链——Rust 1.94.1 + Tauri 2.2.7（用户确认）
- 用户选定 Rust（替代参考实现 Go）；工具链已确认可用（D:\AppSpaces\GreenApp\Rust，rustup toolchains: stable/1.94.1/nightly）
- Tauri CLI 2.2.7 已装——托盘/窗口/WebView 原生支持（cc-switch 同款）
- 注意：host 为 x86_64-pc-windows-gnu；Tauri 打包 Windows 安装器建议 MSVC，1.x 评估切换或 GNU 直发

### D2：模块化架构——微内核 + 模块契约（参考 DSH/Cordis 与 06 §11.4）
- **微内核**（对应 Go 参考的 internal/runtime 与 DSH Cordis Context）：
  - trait Module: { name(), requires(), optional(), apply(host) }
  - Host：服务注册表（provide/get）+ 事件总线（emit/subscribe）+ 依赖序装配（拓扑排序，Boot/Stop 逆序回收）
- **模块清单**（按角色装配，对应 06 §十）：
  | 模块 | 角色 | 职责 |
  |------|------|------|
  | lan-share-server | 服务端 | 双协议 HTTP API（OpenAI /v1/chat/completions + Anthropic /v1/messages SSE，axum/hyper） |
  | lan-auth | 服务端 | 密码→Bearer token、改密、禁止名单 |
  | lan-member-registry | 服务端 | 成员表、改名同步 |
  | lan-usage | 服务端 | token 计量（消费 OpenAI usage）、SQLite 持久化 |
  | lan-discovery-broadcast | 服务端 | UDP 周期广播 |
  | lan-web-console | 服务端 | 静态网页托管（React 产物） |
  | lan-discovery-client | 消费端 | UDP 监听/扫描、组长列表 |
  | lan-share-client | 消费端 | 换 token、OpenAI 兼容调用 |
  | lan-identity | 消费端 | 机器名/显示名 |
  | lan-usage-view | 消费端 | 个人用量 |
  | lan-tray | 双角色 | Tauri 托盘 |
  | lan-runtime | 双角色 | 微内核装配（自举） |
- **装配**：入口按角色/配置选择模块集（Runtime::boot(role)），Optional 可跳过

### D2.1：插件语言策略——内置 Rust、第三方 WASM（语言无关）【补充：06 §九 决策落地】
- **内置模块（我们开发）**：**Rust**——与网关同语言，`trait Module` 编译期装配进二进制（crates/lan-*）
- **第三方/社区插件**：**不限语言**——TS / Python / Rust / Go 均可编译为 WASM（WASI），经沙箱加载、审核制 registry 分发
- **依据（06 文档 §九 已定）**：
  - 语言无关：开发者不需要会 Rust，用自己熟悉的语言写插件
  - Windows 支持：WASM 无平台 dylib 限制（Go plugin/Rust cdylib 均跨平台受限）
  - 沙箱隔离：网关承载零知识密钥，第三方代码必须 WASM 沙箱化（WASI）
  - 安全与扩展平衡：开放生态 + 密钥体系不暴露
- **契约对齐（参考实现 pkg/plugin 印证）**：语言中立的 Plugin 契约 `Name()/Requires()/Optional()/Apply(host)` + Host（Provide/On/Config）——
  - Rust 侧：`trait Module`（编译期装配，同契约语义）
  - WASM 侧：同契约经 ABI/接口对齐（WASI exports）
  - 契约不绑定实现语言——两路共享同一语义
- **落地节奏**：
  | 阶段 | 插件加载 |
  |------|---------|
  | 0.1.0 | 仅内置 Rust 模块（编译期装配），不做 WASM 加载 |
  | 1.x | WASM 沙箱插件（wasmtime/wasmer）+ 审核 registry + 权限声明（最小授权） |
- **不引入**：Rust 动态库（cdylib）插件——跨平台脆弱 + 无沙箱，与网关安全定位冲突

### D3：传输层——HTTP（双协议）+ UDP（广播）
- **应用层双协议（用户确认）**：
  - **OpenAI 兼容**：POST /v1/chat/completions（axum handler），标准 OpenAI 请求/响应（含 usage）——通用工具接入
  - **Anthropic/Claude Code 兼容**：POST /v1/messages（含 SSE 流式 stream:true）——Claude Code CLI 经 LLM Gateway 接入（ANTHROPIC_BASE_URL 指向本网关 + ANTHROPIC_AUTH_TOKEN = 组员 Bearer token）
  - **协议翻译层**（参考实现 aitokengateway/internal/anthropic 对照复用）：OpenAI 请求 ↔ Anthropic 请求互转；Anthropic SSE 流 ← OpenAI StreamChunks（StreamTranslator 状态机：message_start/content_block_delta/message_delta/message_stop）
  - 用量统一：两协议最终都归一为 token 计量（Anthropic usage → input/output_tokens）
- **单 HTTP 端口**托管 API（/v1/*）+ 管理网页（/，静态）
- **UDP 广播**（tokio UdpSocket）做发现，与 HTTP 分离
- **备选：自研 TCP 协议——弃用**（同前复审结论：生态兼容、零定制客户端）
- **P2P 演进（1.x）**：Rust QUIC 生态成熟（quinn）——跨网打洞后 HTTP/3 over UDP 直连，中继兜底；应用层不变

### D4：鉴权——密码 → Bearer token（HTTP 标准）
- POST /auth/token（body: {password, machineName, displayName}）→ {token, expiresAt}
- 请求带 Authorization: Bearer <token>；踢人=吊销 token + 禁止名单；改密=旧 token 全失效
- token 存储：内存表 + 重启失效（0.1.0 简单）；1.x 持久化

### D5：成员与用量
- 成员：换 token 时登记（机器名/IP/显示名/在线）；在线靠心跳（复用参考实现 peerOnlineThreshold 思路，90s）
- 用量：OpenAI 标准响应 usage 字段按 member_id 累积，SQLite 持久化（rusqlite/sqlx）

### D6：托盘——Tauri（参考 cc-switch）
- **服务端托盘菜单**：打开管理面板（系统浏览器 or Tauri WebView）/ 开启共享 / 暂停共享 / 修改密码 / 退出
- **消费端托盘菜单**：发现的组长列表（点击接入）/ 接入状态 / 修改显示名 / 查看个人用量 / 退出
- 关闭不退出（最小侵入）；托盘与宿主经 Tauri command/event 通信
- 备选：纯原生托盘库（tray-icon）——Tauri 已含，不引入额外依赖

### D6.1：跨平台支持——Win / Linux / macOS（参考 cc-switch）【补充：用户确认】
- **平台目标**：Windows、Linux、macOS 三平台同等支持（桌面 + 服务端角色）
- **框架依据**：Tauri 2.x 原生跨平台（cc-switch 同款）——托盘/窗口/WebView 三平台一致 API
  - 参考实现 aitokengateway/internal/tray 已有平台适配可对照：tray_darwin/linux/windows.go + browser_darwin/linux/windows.go（systray 库 + 系统浏览器打开）
- **平台差异处理**：
  | 能力 | Windows | Linux | macOS |
  |------|---------|-------|-------|
  | 托盘 | Tauri tray（原生） | Tauri tray | Tauri tray（菜单栏） |
  | 管理面板打开 | 系统浏览器 | 系统浏览器 | 系统浏览器（open 命令） |
  | 开机自启 | 注册表/任务计划 | autostart desktop | LaunchAgent |
  | 权限/沙箱 | 无特殊 | AppArmor 可选 | 无特殊 |
  | 打包 | MSI/NSIS | deb/rpm/AppImage | dmg/App |
  | 构建环境 | windows-gnu（当前）→ 1.x MSVC | Linux CI | macOS CI（需 Apple 环境/签名） |
- **执行后端差异**（06 文档 §八 已定矩阵）：Windows 走 WSL2、macOS 原生（MLX/Metal）、Linux 原生（CUDA）——契约与计量不变
- **开发/验证节奏**：
  | 阶段 | 平台覆盖 |
  |------|---------|
  | 0.1.0 | 主平台 Windows 开发 + Linux 验证（跨平台代码一次编写） |
  | 0.1.x | Linux 正式支持 + macOS 基础验证 |
  | 1.0 | 三平台发布矩阵（CI 构建 Win/Linux/macOS 产物） |
- **注意**：当前本机 Rust host 为 x86_64-pc-windows-gnu——Windows 开发可用；macOS 产物需在 macOS/CI 环境构建（交叉编译 Tauri 不支持，需平台原生构建）
- **不引入**：平台特化业务逻辑（业务在 crates 通用层，平台差异只在 src-tauri 壳层隔离）

### D6.2：国际化（i18n）——多语言支持【补充：用户确认】
- **范围**：托盘菜单文案、管理网页文案、CLI 提示/日志（关键文案）
- **机制**：
  | 层 | 方案 | 参考 |
  |----|------|------|
  | Rust 侧（托盘/CLI/日志） | 嵌入式 bundle（JSON per-locale，include_str! 嵌入）+ 运行时语言切换 | 参考实现 aitokengateway/internal/i18n（en/zh-CN/fr/es 4 语言 bundle，go:embed 嵌入） |
  | 网页侧（React） | locales.ts 字典（zh/en）+ 语言上下文切换 | DSH ui-* 的 LocaleNamespaceMap + locales.ts 模式 |
  | 存储 | 用户语言偏好（配置持久化） | cc-switch 设置系统同款 |
- **语言集合**：
  | 阶段 | 语言 |
  |------|------|
  | 0.1.0 | **zh-CN + en**（默认跟随系统，可手动切换） |
  | 0.1.x | fr / es / ja / ko（按需增补，bundle 追加即可） |
  | 1.x | 完整 i18n 框架（复数/变量插值/热切换） |
- **契约/技术文案**：协议（OpenAI/Anthropic）、API 错误信息保持英文（生态标准），仅 UI/托盘/提示本地化
- **不引入**：重 i18n 框架（0.1.0 轻量 bundle 即可）；不本地化调试日志内部信息

### D6.3：cc-switch 其余可学习点（完整借鉴清单）【补充：用户询问】

已吸收：托盘（D6）、跨平台（D6.1）、i18n（D6.2）。其余 cc-switch 成熟设计，按价值分批引入：

| # | cc-switch 能力 | 我们的借鉴 | 落地 |
|---|---------------|-----------|------|
| 1 | **Minimal Intrusion（最小侵入）**：卸载 app 后 CLI 工具不受影响 | 配置独立存储（用户数据目录），不写系统全局；卸载/删除后仅失去管理面，共享服务行为不受影响 | **0.1.0 必做**（已隐含于 D6 关闭不退出，明确为设计原则） |
| 2 | **Deep Link（`ccswitch://` URL 协议）**：一键导入供应商/MCP/提示词 | `aipowerlink://share?password=...&server=...` 或局域网内一键加入（组员点链接自动配置并接入组长） | 1.x（0.1.0 先做二维码/邀请码手动输入） |
| 3 | **配置持久化 + 导入导出**：数据目录 + 备份恢复 | 配置（密码/成员/用量）存数据目录；导入导出 JSON 便于迁移/多机同步 | 0.1.x（用量持久化已有；导入导出追加） |
| 4 | **Provider Health Monitoring（供应商健康监控）**：延迟/可用性检测 | 组长端检测本机执行后端健康（mock/llama.cpp 可达性），托盘/网页显示状态 | 0.1.x |
| 5 | **轻量模式（Lightweight）**：低资源占用形态 | 托盘 + 无网页模式（仅 CLI/托盘，管理页按需启动 HTTP） | 0.1.0 可做（HTTP 惰性启动，非 P0） |
| 6 | **Proxy & Failover（代理与故障转移）** | 组长端多执行后端 failover（主后端失败自动切备用）；1.x 网关间中继兜底 | 1.x |
| 7 | **多工具统一管理（All-in-One）**：管理多个 CLI 工具配置 | 本 0.1.0 已双协议（OpenAI/Anthropic）覆盖 Claude Code/通用工具；1.x 扩展更多协议 | 1.x |
| 8 | **设置系统（Settings）**：语言/主题/数据目录/代理 | 管理网页设置页（语言/主题/数据目录/端口）；0.1.0 语言已有（D6.2），主题/数据目录 0.1.x | 0.1.x |
| 9 | **快捷切换**：托盘/快捷键快速切换供应商 | 托盘菜单切换执行后端/组长（已有菜单骨架）；快捷键 1.x | 0.1.0 部分 |

**取舍原则**（克制）：0.1.0 只纳入与核心闭环强相关的（Minimal Intrusion、配置独立存储）；其余按价值/成本排入 0.1.x/1.x，不一次性全做。

### D7：管理网页——参考 DSH web（AppFrame 三栏 + 组件化 + CSS Modules）【用户确认：管理页面参考 DSH】
- **整体形态对应 DSH 管理面**（ui-layout 的 AppFrame 三栏 + ui-sidebar + 中部主区）——组长管理页为**三栏布局**：
  | 栏 | 对应 DSH | 本页面内容 |
  |----|---------|-----------|
  | 左侧栏 sidebar | ui-sidebar（SidebarRoot：brand/导航/settings 入口） | 导航（成员/用量/设置）+ 品牌区 + 底部共享状态 |
  | 中部主区 main | ui-conversation（会话主体） | 成员列表/用量表格/操作面板（按导航切换） |
  | 右侧栏 details | ui-conversation 的 DetailsPanel | 选中成员的详情（机器名/IP/在线时长/用量明细） |
- **技术栈对齐 DSH**：React 18 + CSS Modules（.module.css，组件级样式）+ Vite；产物嵌入二进制（include_bytes 或 Tauri assets）
- **组件化对齐 DSH**（每组件独立 .tsx + .module.css）：
  - `AppFrame.tsx`（三栏框架，对应 DSH AppFrame）+ `AppFrame.module.css`
  - `SidebarRoot.tsx`（导航+品牌+共享状态，对应 DSH ui-sidebar）+ `SidebarRoot.module.css`
  - `MemberList.tsx` / `UsageTable.tsx` / `ControlsPanel.tsx`（主区，对应 DSH 会话区组件）
  - `DetailsPanel.tsx`（右栏成员详情，对应 DSH DetailsPanel）
- **状态与交互模式对齐 DSH**：
  - 轮询/事件刷新成员与用量（DSH 会话列表同款刷新语义）
  - 组件间经轻量 store（zustand 或 React context，对应 DSH 的 stores.ts 模式）
  - 中英双语文案（对应 DSH locale 模式：locales.ts 字典 + 语言上下文，0.1.0 必做——见 D6.2）
- **克制简化**：不做 DSH 的 slots 注册系统/模块表/多插件包——0.1.0 单 Vite 应用内联组件，保留 DSH 的**视觉与组件结构**（三栏/模块化/CSS Modules），插槽机制 1.x 网页扩展时引入
- 管理 API：GET /api/members、GET /api/usage、POST /api/control（与 lan-share-server 同端口）

### D6.4：配置管理——单一配置库 + 角色分区（组长/组员两套配置统一管理）【补充：用户提问】

**问题**：组长端（服务端）与组员端（消费端）是两套配置（端口/密码/成员 vs 组长列表/token/显示名），同一二进制如何统一管理。

**方案：单一 SQLite 配置库 + 角色分区表 + schema 驱动**

1. **单一数据目录 + 单库**（对应参考实现 store.DB + cc-switch SQLite 存储）：
   `~/.aipowerlink/aipowerlink.db`（跨平台用户数据目录：Win %APPDATA% / Linux ~/.config / macOS ~/Library/Application Support）
2. **表结构（角色分区）**：
   | 表 | 角色 | 内容 |
   |----|------|------|
   | settings | 全局 | 语言偏好、主题、数据目录、默认角色 |
   | node_identity | 全局 | 本机节点身份（机器名/显示名/节点 ID）——两角色共享 |
   | server_config | 服务端 | 端口、共享开关、密码哈希、执行后端配置 |
   | members | 服务端 | 成员（机器名/IP/显示名/在线/token 指纹） |
   | usage | 服务端 | 成员用量累计（SQLite 持久化） |
   | client_config | 消费端 | 已保存的组长列表（服务名/IP/端口/指纹） |
   | client_credentials | 消费端 | 各组长密码/token（**Vault 加密存储**） |
3. **schema 驱动**（DSH settings 模式）：
   - 每配置项 schema 声明：类型/默认/角色（server|client|global）/敏感度（secret 字段）
   - 敏感字段（密码/token）存 Vault 加密（参考实现 crypto.Vault），读取时脱敏（DSH redact 模式）
4. **统一访问接口**：
   - `ConfigService`（模块注入）：按角色只暴露本角色配置视图（server 模块看不到 client 表，反之亦然）
   - CLI：`aipowerlink config get/set <key>`（按当前角色）；`--role` 切换视图
   - 管理网页：组长端设置页读写 server 配置；托盘菜单快捷操作
5. **角色切换**：
   - 启动时 `--role server|client` 指定（或自动检测：有广播接收→client，被配置为共享→server）
   - 同一数据目录可双角色并存（1.x 一台设备既组长又组员），互不覆盖
6. **导入导出**（cc-switch 学习点，0.1.x）：
   - `aipowerlink config export/import`（JSON，secret 字段仅导出占位，需重新输入）
7. **配置变更生效**：
   - 静态配置（端口/语言）重启生效；动态（共享开关/踢人）运行时生效（经事件总线）

**取舍**：0.1.0 实现单库 + 角色分区 + schema + Vault 加密 + CLI 读写；导入导出/自动角色 0.1.x。

### D7.1：DSH 其余可学习点（完整借鉴清单）【补充：用户询问】

已吸收：微内核/模块契约（D2）、管理网页三栏（D7）、i18n locale 模式（D6.2）。其余 DSH 成熟设计，按价值分批引入：

| # | DSH 能力 | 我们的借鉴 | 落地 |
|---|----------|-----------|------|
| 1 | **Schema 驱动配置 + 敏感值脱敏**（settings/redact：role('secret') 字段过线前移除，UI 只渲染 write-only 输入） | 组长端配置（密码/token）schema 声明 secret 角色，管理 API/网页永不回传明文，只回传"已设置/未设置" | **0.1.0 必做**（密码/token 安全基线） |
| 2 | **插件清单/健康（plugin-inventory）**：Loader 树 + fiber 状态（loading/active/failed） | 管理网页展示模块装配状态（每个 lan-* 模块 active/failed），异常一目了然 | 0.1.x |
| 3 | **Web 路由注册表 + 注入表**（webserver：exact/prefix 路由 + index-inject 事件收集注入） | 组长端 HTTP 服务用路由注册表（/api/* exact/prefix 分级），页面注入表 1.x | 0.1.0 部分（axum 路由天然支持） |
| 4 | **事件总线跨模块协作**（ctx.on/emit + declare module 事件类型） | 模块间经事件总线通信（成员变更/用量更新/踢人事件），而非直接耦合 | **0.1.0 必做**（微内核核心） |
| 5 | **插件生命周期管理**（fiber：loading→active→failed→unloading，依赖拓扑装配） | 模块装配报告状态，Optional 失败降级继续 | **0.1.0 必做**（D2 已含，明确状态化） |
| 6 | **后台任务（jobs）**：长任务状态（running/completed/killed） | 组长端长任务（模型推理）状态跟踪（running/completed/failed），网页展示 | 0.1.x（0.1.0 同步请求为主） |
| 7 | **会话/上下文管理（session）**：会话持久化、chunk 恢复 | 成员接入会话（token/连接）可恢复；1.x 断线重连 | 1.x |
| 8 | **API 代理（apiproxy）**：统一代理层 | 组长端执行后端代理（请求转发到 llama.cpp/云端），统一鉴权/计量 | 0.1.x |
| 9 | **模块表（seed.ts）**：平台单例共享 | 网页依赖（react/cordis）单实例共享——0.1.0 单应用天然满足；1.x 拆插件包时引入 | 1.x |
| 10 | **Skill/Goal/Workflow 体系** | 组长端"技能"（预设任务模板）分享；1.x 远期 | 1.x |
| 11 | **日志体系**（logger exporter + 分级） | 模块分级日志（debug/info/error）+ 可导出 | 0.1.0 基础（tracing） |
| 12 | **状态机/修复（session repair）**：会话损坏自动修复 | 配置/用量存储损坏自动重建（备份+校验） | 0.1.x |

**取舍原则**（克制）：0.1.0 纳入与安全/核心闭环强相关的（secret 脱敏、事件总线、生命周期状态）；其余按价值排入 0.1.x/1.x。

### D8：项目结构（Rust workspace）
```
aipowergateway/
  src-tauri/          # Tauri 壳（托盘/窗口/WebView 宿主）
  crates/
    runtime/          # 微内核（Module trait + Host + 事件总线）
    lan-share/        # 服务端共享模块（HTTP API/鉴权/成员/用量/广播）
    lan-client/       # 消费端模块（发现/接入/身份/用量）
    lan-tray/         # 托盘命令与菜单定义
  web/                # 管理网页（React 18 + CSS Modules + Vite，AppFrame 三栏参考 DSH，产物嵌入）
```
- 参考 DSH monorepo 分层：内核（runtime）与业务（lan-*）分离，契约（OpenAI 兼容/Module trait）稳定

## Risks / Trade-offs

- [GNU host 与 Tauri 打包 MSVC 冲突] → 0.1.0 开发/调试用 GNU 即可；1.x 发布评估 MSVC 工具链或便携分发
- [三平台构建成本] → 0.1.0 主 Windows + Linux 验证；macOS 产物 1.0 前在 CI 构建（避免本地缺 Apple 环境的阻塞）
- [翻译质量/一致性] → 0.1.0 核心文案人工校对；1.x 引入 i18n 审核流程（DSH README.i18n.yaml 模式可参考）
- [语言偏好与系统跟随] → 0.1.0 默认跟随系统 locale + 手动覆盖；1.x 精细化
- [平台差异蔓延进业务层] → 平台差异只在 src-tauri 壳层；业务模块（crates/lan-*）保持平台无关，契约/计量跨平台一致
- [Rust 从零建设周期] → 0.1.0 只做必要模块（share/auth/registry/usage/discovery/console + client 侧 4 模块），砍非 P0
- [参考实现 Go 22k 行不迁移] → 用户已确认切 Rust；Go 参考保留作闭源参考，架构思路（lan 包设计）对照复用
- [Bearer token 明文走 HTTP] → 局域网可信边界；1.x HTTPS/QUIC（TLS 内建）
- [UDP 广播跨 VLAN] → 0.1.0 同广播域；1.x mDNS
- [双协议维护成本] → 0.1.0 只实现核心面（chat/completions + messages 非流式/SSE）；工具调用等边缘 1.x；对照参考实现 anthropic 包复用翻译逻辑
- [WASM 插件 1.x 引入成本] → 0.1.0 不引入（仅内置 Rust 模块）；契约先以 trait Module 稳定，WASM 侧对齐同契约即可
- [Anthropic SSE 协议细节（tool_use 分块/usage 事件）] → 参考实现 sse.go 已有完整状态机可对照；0.1.0 以文本流为主，tool 流 1.x
- [Rust 网页资产嵌入体积] → 0.1.0 单页小体积；1.x 按需懒加载

## Migration Plan

- 全新工程：cargo init workspace + tauri init；无迁移
- 回滚：不适用（新建）

## Open Questions

- 管理面板打开方式：系统浏览器 vs Tauri WebView 窗口？（0.1.0 倾向系统浏览器——复用 D2 单端口，WebView 1.x 评估）
- 执行后端：0.1.0 接本地 mock / llama.cpp？（建议 mock 验证链路，1.x 接真实推理）