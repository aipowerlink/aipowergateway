# aipowergateway

> 局域网算力共享网关——在局域网内与团队分享你的模型访问（DeepSeek、Kimi、智谱 GLM）。Rust + 系统托盘。

## 简介

AIPowerLink 网关让一个人（**组长**）在同一局域网内与其他人（**组员**）分享自己的 LLM API 访问：

- 组员安装客户端，自动发现组长，输入密码即可调用模型——**零配置**
- 一个二进制、双角色：`--role server`（组长）或 `--role client`（组员）
- **双协议**：OpenAI 兼容 + Anthropic 兼容（可直接接 Claude Code）
- **多后端**：同时分享 DeepSeek、Kimi、智谱 GLM——按模型名路由
- 组长可查看每个组员的 token 用量，随时踢人/封禁
- 局域网内离线可用——无云端依赖

## 快速开始

### 构建

```bash
# 依赖：Rust 1.94+（Windows 用 MSVC 工具链）+ Node 18+（管理网页）
cargo build --release -p aipg-cli

# 构建管理网页（可选）
cd web && npm install && npm run build
```

### 组长端（服务端角色）

```bash
# 本地 mock 后端（验证链路）
aipowergateway --role server

# 分享 DeepSeek
AIPOWERLINK_DEEPSEEK_API_KEY=sk-xxx aipowergateway --backend deepseek

# 同时分享多个后端
AIPOWERLINK_DEEPSEEK_API_KEY=sk-ds AIPOWERLINK_KIMI_API_KEY=sk-kimi aipowergateway --backend deepseek,kimi,zhipu

# 设置共享密码（默认：aipowergateway）
AIPOWERLINK_PASSWORD=mysecret aipowergateway --role server
```

启动后：
- 管理面板：浏览器打开 http://127.0.0.1:39091/
- 组员自动发现：UDP 广播（端口 39090）
- 指纹：密码哈希前 8 位（组员可预校验）

### 组员端（消费端角色）

```bash
aipowergateway --role client
# 自动发现组长 → 输密码接入 → 调用模型
```

## 支持的协议（二选一）

| 协议 | 端点 | 客户端示例 |
|------|------|-----------|
| **OpenAI 兼容** | `POST /v1/chat/completions` | 任意 OpenAI 兼容工具（curl、Cursor、Open WebUI） |
| **Anthropic 兼容** | `POST /v1/messages`（SSE 流式） | Claude Code（通过 `ANTHROPIC_BASE_URL`） |

### 接入 Claude Code

```bash
export ANTHROPIC_BASE_URL=http://<组长IP>:39091
export ANTHROPIC_AUTH_TOKEN=<组员token>
export ANTHROPIC_MODEL=deepseek-chat   # 或 kimi-2.7-code 等
```

### 模型目录（组长分享的模型）

```bash
curl http://<组长IP>:39091/v1/models
# 例如 deepseek-chat / kimi-2.7-code / glm-4-flash
```

## 支持的官方大模型

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
aipowergateway config set port 39091
aipowergateway config set password mysecret   # 自动识别为 secret
aipowergateway config list                    # 敏感值显示为 [set]
aipowergateway config get password
```

## 自定义角色

```bash
# 内置角色只读，复制后定制
aipowergateway role clone server my-leader
aipowergateway role list    # server(system) client(system) my-leader(user)
aipowergateway --role my-leader   # 以自定义角色启动
```

## 系统托盘

- 组长：打开管理面板 / 开启共享 / 暂停共享 / 修改密码 / 退出
- 组员：组长列表 / 接入状态 / 改名 / 用量 / 退出
- `--no-tray`：纯命令行模式

## 架构

```
组员（OpenAI 或 Anthropic 接口）
    |  发送模型名：deepseek-chat / kimi-2.7-code
组长网关 aipowergateway（鉴权 + 计量 + 广播 + 管理面板）
    |-- deepseek-* -> DeepSeek
    |-- kimi-*     -> Kimi
    |-- glm-*      -> 智谱 GLM
    `-- mock-*     -> 本地 mock
```

### 模块

| crate | 职责 |
|-------|------|
| `aipg-runtime` | 微内核：Module trait、Host、事件总线、角色、i18n、数据目录 |
| `aipg-lan-share` | 组长端：双协议 API、鉴权、成员、用量、广播、路由、管理网页 |
| `aipg-lan-client` | 组员端：发现、接入、双协议调用、身份、用量 |
| `aipg-config` | 配置库：SQLite、角色分区、Vault 加密、脱敏 |
| `aipg-lan-tray` | 系统托盘（tray-icon） |
| `aipg-cli` | 命令行入口（aipowergateway） |

## 平台

- Windows / Linux / macOS（跨平台托盘）
- 管理面板在系统浏览器中打开

## 许可

AGPL-3.0-or-later。见 [LICENSE](LICENSE)。

---

English: [README.md](README.md)