# aipowergateway

> AIPowerLink 的**实际开源网关项目**（AGPLv3）——局域网算力共享网关（Rust + Tauri 托盘）。

## 定位

- **开源形态**：对外发布的开源网关（AGPLv3），面向开发者/团队/合作方
- **与 aitokengateway 的关系**：`aitokengateway` 为闭源参考实现（内部参考），本仓库承担实际开源发布
- **能力**：局域网算力共享 · 双协议 API（OpenAI/Anthropic）· 官方大模型（DeepSeek/Kimi/智谱）· 多后端路由 · 系统托盘 · 管理网页 · 自定义角色 · 多语言

## 技术栈

| 层 | 技术 |
|----|------|
| 语言 | Rust 1.94+ |
| 运行形态 | 系统托盘（tauri-apps tray-icon）+ CLI，参考 cc-switch |
| 代码/网页结构 | 参考 DeepSeek Harness（微内核 + 模块化 + 薄壳网页 + React 组件） |
| HTTP | axum + hyper |
| 存储 | SQLite（rusqlite）+ Vault（AES-GCM 加密敏感值） |
| 网页 | React 18 + CSS Modules + Vite |
| 日志 | tracing |

## 快速开始

### 构建

```bash
# 依赖：Rust 1.94+（MSVC 工具链）+ Node 18+（网页）
cargo build --release -p aipg-cli

# 管理网页（可选，需 Node）
cd web && npm install && npm run build
```

### 组长端（服务端角色）：开启算力共享

```bash
# 本地 mock 后端（验证链路）
aipowerlink --role server

# 共享官方大模型（DeepSeek）
AIPOWERLINK_DEEPSEEK_API_KEY=sk-xxx aipowerlink --backend deepseek

# 同时共享多家（DeepSeek + Kimi + 智谱）
AIPOWERLINK_DEEPSEEK_API_KEY=sk-ds AIPOWERLINK_KIMI_API_KEY=sk-kimi aipowerlink --backend deepseek,kimi,zhipu

# 设置共享密码（默认 aipowerlink）
AIPOWERLINK_PASSWORD=mysecret aipowerlink --role server
```

启动后：
- 管理面板：浏览器打开 http://127.0.0.1:39091/
- 组员自动发现：UDP 广播（端口 39090）
- 指纹：密码哈希前 8 位（组员可预校验）

### 组员端（消费端角色）

```bash
aipowerlink --role client
# 自动发现组长 → 输密码接入 → 调用模型
```

## 支持的协议（组员二选一）

| 协议 | 端点 | 客户端示例 |
|------|------|-----------|
| **OpenAI 兼容** | `POST /v1/chat/completions` | 任意 OpenAI 兼容工具（curl/Cursor/Open WebUI） |
| **Anthropic 兼容** | `POST /v1/messages`（含 SSE 流式） | Claude Code（ANTHROPIC_BASE_URL 指向本网关） |

### Claude Code 接入

```bash
export ANTHROPIC_BASE_URL=http://<组长IP>:39091
export ANTHROPIC_AUTH_TOKEN=<组员token>
export ANTHROPIC_MODEL=deepseek-chat  # 或 kimi-2.7-code 等
```

### 模型目录（组员可查）

```bash
curl http://<组长IP>:39091/v1/models
# 返回组长共享的全部模型（deepseek-chat / kimi-2.7-code / glm-4-flash ...）
```

## 官方大模型支持

| 提供商 | 环境变量 | 默认模型 |
|--------|---------|---------|
| DeepSeek | `AIPOWERLINK_DEEPSEEK_API_KEY` | deepseek-chat |
| Kimi（月之暗面） | `AIPOWERLINK_KIMI_API_KEY` | moonshot-v1-8k |
| 智谱 GLM | `AIPOWERLINK_ZHIPU_API_KEY` | glm-4-flash |
| 自定义 | `AIPOWERLINK_BASE_URL` + `AIPOWERLINK_MODEL` | — |

模型名前缀路由：`deepseek-*`→DeepSeek、`kimi-*`→Kimi、`glm-*`→智谱。

## 配置管理

```bash
# 读写配置（敏感值自动加密 + 脱敏显示）
aipowerlink config set port 39091
aipowerlink config set password mysecret   # 自动识别 secret
aipowerlink config list                    # 敏感值显示 [set]
aipowerlink config get password
```

## 自定义角色

```bash
# 内置角色只读，复制定制
aipowerlink role clone server my-leader
aipowerlink role list    # server(system) client(system) my-leader(user)
aipowerlink --role my-leader   # 以自定义角色启动
```

## 系统托盘（参考 cc-switch）

- 组长：打开管理面板 / 开启共享 / 暂停共享 / 修改密码 / 退出
- 组员：组长列表 / 接入状态 / 改名 / 用量 / 退出
- `--no-tray`：纯命令行模式

## 架构

```
组员（OpenAI 或 Anthropic 接口）
    ↓ 传模型名 deepseek-chat / kimi-2.7-code
组长网关 aipowerlink（鉴权 + 计量 + 广播 + 管理网页）
    ├─ deepseek-* → DeepSeek 官方
    ├─ kimi-*     → Kimi 官方
    ├─ glm-*      → 智谱官方
    └─ mock-*     → 本地 mock
```

### 模块化

| crate | 职责 |
|-------|------|
| `aipg-runtime` | 微内核：Module trait + Host + 事件总线 + 角色 + i18n + 数据目录 |
| `aipg-lan-share` | 组长端：双协议 API / 鉴权 / 成员 / 用量 / 广播 / 多后端路由 / 网页托管 |
| `aipg-lan-client` | 组员端：发现 / 接入 / 双协议调用 / 身份 / 用量 |
| `aipg-config` | 配置库：SQLite + 角色分区 + Vault 加密 + 脱敏 |
| `aipg-lan-tray` | 系统托盘（tray-icon） |
| `aipg-cli` | 命令行入口（aipowerlink） |

## 跨平台

- Windows / Linux / macOS 同等支持（Tauri tray-icon 跨平台）
- 管理面板经系统浏览器打开（平台 open）

## 协议演进

- **0.1.0**：HTTP/1.1 over TCP + UDP 广播发现
- **1.x**：HTTP/3（QUIC）over UDP 高速通道——跨网打洞直连、中继兜底；应用层（OpenAI/Anthropic）不变

## 许可

AGPL-3.0-or-later