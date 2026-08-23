# Proposal: lan-share-0-1-0（aipowergateway 局域网算力共享 0.1.0，Rust + Tauri）

## Why

局域网内算力闲置而朋友/同事需要算力——现状无法低成本共享。0.1.0 最小闭环：组长（服务端角色）一键开启共享，组员（消费端角色）装客户端自动发现、输密码接入即用；组长能看到成员与 token 用量、随时踢人。全程零云端依赖（纯 LAN P2P）。

**技术路线（用户已确认）**：
- 语言：**Rust**（用户选定，替代参考实现 Go）
- 运行形态：**Tauri 托盘常驻**（参考 cc-switch，tauri-cli 2.2.7 已装）
- **跨平台**：Windows / Linux / macOS 三平台同等支持（参考 cc-switch 跨平台发布）
- **国际化（i18n）**：多语言支持（默认 中/英，托盘/网页/CLI 本地化，参考实现 i18n 包与 DSH locale 模式）
- 代码/网页结构：**参考 DeepSeek Harness**（微内核 + 模块化 + 薄壳网页 + React 组件）

## What Changes

- aipowergateway（Rust）新增局域网算力共享能力，**同一二进制按角色装配**（对应 06 文档 §十 角色运行时选择）：
  - **服务端角色（组长）**：
    - `lan-share-server`：双协议 HTTP API（OpenAI 兼容 /v1/chat/completions + Anthropic /v1/messages SSE），执行请求
    - `lan-auth`：密码 → Bearer token 签发/吊销、禁止名单、改密
    - `lan-member-registry`：成员表（机器名/IP/显示名/在线）、改名同步
    - `lan-usage`：按成员计量 token（消费 OpenAI usage）、持久化
    - `lan-discovery-broadcast`：UDP 周期广播（服务名/API 端口/指纹）
    - `lan-web-console`：管理网页（薄壳 + React 组件，参考 DSH web）
  - **消费端角色（组员）**：
    - `lan-discovery-client`：UDP 监听广播 + 扫描，维护组长列表
    - `lan-share-client`：HTTP 接入（换 token）、双协议 API 调用（OpenAI 兼容 + Anthropic）
    - `lan-identity`：机器名/显示名管理、接入上报
    - `lan-usage-view`：个人用量记录与展示
  - **双角色共享**：`lan-tray`（Tauri 托盘，参考 cc-switch）、`lan-runtime`（微内核模块装配）
- **协议（双协议支持）**：
  - **OpenAI 兼容**（/v1/chat/completions）——通用 AI 工具/客户端接入
  - **Anthropic/Claude Code 兼容**（/v1/messages + SSE 流式）——Claude Code CLI 经 LLM Gateway（ANTHROPIC_BASE_URL / ANTHROPIC_AUTH_TOKEN）接入；参考实现 aitokengateway/internal/anthropic 已有 OpenAI↔Anthropic 翻译层（sse.go/translate.go/types.go）可对照复用
  - 传输层按场景分级（0.1.0 TCP/HTTP1.1；1.x 跨网 QUIC/HTTP3 打洞直连、中继兜底）
- 无 BREAKING（全新工程从零建设）

## Capabilities

### New Capabilities

- `lan-share-server`: 服务端共享——双协议 HTTP API（OpenAI 兼容 /v1/chat/completions + Anthropic /v1/messages SSE）、鉴权、执行、用量、踢人吊销
- `lan-auth`: 接入鉴权——密码 → Bearer token、改密、禁止名单
- `lan-member-registry`: 成员管理——机器名/IP/显示名/在线、改名同步
- `lan-usage`: 用量计量——按成员 token 累计、持久化、查询
- `lan-discovery-broadcast`: 服务广播——UDP 周期广播服务信息
- `lan-web-console`: 组长端管理网页（薄壳 + React，参考 DSH web）
- `lan-discovery-client`: 消费端发现——UDP 扫描/监听广播、组长列表
- `lan-share-client`: 消费端接入——换 token、OpenAI 兼容调用、被踢即时失效
- `lan-identity`: 消费端身份——机器名/显示名、接入上报
- `lan-usage-view`: 消费端用量——个人 token 记录与展示
- `lan-tray`: 托盘常驻——Tauri 系统托盘（参考 cc-switch），组长/组员共用

### Modified Capabilities

（无——全新工程，无既有 spec）

## Impact

- 代码：aipowergateway 从零建 Rust 工程（src-tauri + 模块化 crates）
- 依赖：Tauri 2.x、axum/hyper（HTTP）、tokio（异步）、dgram（UDP，可用 socket2/tokio）、React 18 + Vite（网页）
- 工具链：Rust 1.94.1 + cargo-tauri 2.2.7（已确认可用，路径 D:\AppSpaces\GreenApp\Rust）
- 系统：1 个 HTTP 端口（API + 管理网页复用）+ 1 个 UDP 端口（广播）+ 托盘图标
- 文档：用户故事 19 已建立；06 文档 §十 角色模型为架构依据
- **回退**：撤销在 aipoweredge / aipoweredge-agent 上误建的 lan-share-server / lan-share-client change（edge 保持本地算力管理定位）