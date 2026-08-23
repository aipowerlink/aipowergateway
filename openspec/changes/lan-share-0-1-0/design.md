# Design: lan-share-0-1-0（aipowergateway Rust 架构）

## Context

现状（见 proposal.md — Why）：aipowergateway 为全新 Rust 工程（目前仅 README + openspec）。本 change 从零建设 0.1.0 局域网算力共享。参考：06 文档（角色运行时选择 §十、模块契约 §11.4）、DSH（代码/网页结构）、cc-switch（Tauri 托盘运行形态）。

## Goals / Non-Goals

**Goals:**
- Rust 实现 gateway 双角色（服务端/消费端）局域网共享，模块化装配
- Tauri 托盘常驻（参考 cc-switch）；网页薄壳 + React（参考 DSH web）
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

### D7：网页——薄壳 + React（参考 DSH web，克制版）
- 薄壳 index.html + main.tsx 挂 #root；组件：MemberList / UsageTable / ControlsPanel / AppFrame（三栏）
- 技术栈：React 18 + CSS Modules + Vite（与 DSH web 一致）；产物打包进二进制（include_bytes 或 Tauri assets）
- 不做 DSH 的 slots/模块表（0.1.0 单应用内联；1.x 网页扩展再引入）
- 管理 API：GET /api/members、GET /api/usage、POST /api/control（与 lan-web-console 同端口）

### D8：项目结构（Rust workspace）
```
aipowergateway/
  src-tauri/          # Tauri 壳（托盘/窗口/WebView 宿主）
  crates/
    runtime/          # 微内核（Module trait + Host + 事件总线）
    lan-share/        # 服务端共享模块（HTTP API/鉴权/成员/用量/广播）
    lan-client/       # 消费端模块（发现/接入/身份/用量）
    lan-tray/         # 托盘命令与菜单定义
  web/                # 管理网页（React + Vite，产物嵌入）
```
- 参考 DSH monorepo 分层：内核（runtime）与业务（lan-*）分离，契约（OpenAI 兼容/Module trait）稳定

## Risks / Trade-offs

- [GNU host 与 Tauri 打包 MSVC 冲突] → 0.1.0 开发/调试用 GNU 即可；1.x 发布评估 MSVC 工具链或便携分发
- [Rust 从零建设周期] → 0.1.0 只做必要模块（share/auth/registry/usage/discovery/console + client 侧 4 模块），砍非 P0
- [参考实现 Go 22k 行不迁移] → 用户已确认切 Rust；Go 参考保留作闭源参考，架构思路（lan 包设计）对照复用
- [Bearer token 明文走 HTTP] → 局域网可信边界；1.x HTTPS/QUIC（TLS 内建）
- [UDP 广播跨 VLAN] → 0.1.0 同广播域；1.x mDNS
- [双协议维护成本] → 0.1.0 只实现核心面（chat/completions + messages 非流式/SSE）；工具调用等边缘 1.x；对照参考实现 anthropic 包复用翻译逻辑
- [Anthropic SSE 协议细节（tool_use 分块/usage 事件）] → 参考实现 sse.go 已有完整状态机可对照；0.1.0 以文本流为主，tool 流 1.x
- [Rust 网页资产嵌入体积] → 0.1.0 单页小体积；1.x 按需懒加载

## Migration Plan

- 全新工程：cargo init workspace + tauri init；无迁移
- 回滚：不适用（新建）

## Open Questions

- 管理面板打开方式：系统浏览器 vs Tauri WebView 窗口？（0.1.0 倾向系统浏览器——复用 D2 单端口，WebView 1.x 评估）
- 执行后端：0.1.0 接本地 mock / llama.cpp？（建议 mock 验证链路，1.x 接真实推理）