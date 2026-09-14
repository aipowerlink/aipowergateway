# aipowergateway

> 局域网算力共享网关——在局域网内与团队分享你的模型访问（DeepSeek、Kimi、智谱 GLM、CodeBuddy）。Rust + 系统托盘。

## 简介

AIPowerLink 网关让一个人（**组长**）在同一局域网内与其他人（**组员**）分享自己的 LLM API 访问：

- 组员安装客户端，自动发现组长即可调用模型——**免密、零配置**
- 一个二进制、双角色：`--role server`（组长）或 `--role client`（组员）
- **双协议**：OpenAI 兼容 + Anthropic 兼容（可直接接 Claude Code）
- **多后端**：同时分享 DeepSeek、Kimi、智谱 GLM、CodeBuddy——按模型名路由
- 组长可查看每个组员的 token 用量、来源 IP 与网关 ID，随时拉黑/解禁（持久化）
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

# 分享 CodeBuddy（腾讯 Copilot key；也兼容 CODEBUDDY_API_KEY 变量名）
AIPOWERLINK_CODEBUDDY_API_KEY=ck-xxx aipowergateway --backend codebuddy

# 同时分享多个后端
AIPOWERLINK_DEEPSEEK_API_KEY=sk-ds AIPOWERLINK_KIMI_API_KEY=sk-kimi aipowergateway --backend deepseek,kimi,zhipu

# 免密：组员无需密码即可接入（0.2.0+）
```

### 模型设置（面板，对齐 DeepSeek Harness）

打开管理面板 →「模型」页：

- **添加提供方**：选择 DeepSeek / Kimi / Zhipu / CodeBuddy，或「添加自定义提供方」指向任意 OpenAI 兼容端点（base_url + 模型）；API 密钥可直接填入，或按环境变量名引用。
  - **标准配置预设**（参考 cc-switch 添加模型）：选择内置提供方即自动带入官方 API 地址与标准模型清单（如 `deepseek-chat`、`deepseek-reasoner`），模型以标签形式增删，或点「使用标准模型」一键恢复。
- **添加执行体**（`pair://` / `agent://` 预设）：把**供给侧执行体**作为上游 Provider 接入——家庭 PAIR 集群（NVIDIA PAIR，聚合家庭闲置算力）或 aipoweredge-agent 节点。点对应卡片，只需填入执行体暴露的 OpenAI 兼容 `base_url`（其 `GET {base_url}/models` 端点；本地执行体通常无需密钥），保存后网关自动检测真实模型列表并落盘，路由即刻生效；卡片在列表中带 `pair://` / `agent://` 徽标。
- **编辑 / 删除**：一个提供方可服务多个模型；只改模型/地址时原密钥自动保留；变更写入 `data_dir/backends.yaml` 并**无需重启**即热生效（模型目录与路由立即更新）。
- **测试**（连接测试，cc-switch 式）：表单内「测试」按当前填写的内容探测（不落盘），卡片「测试」用已保存的密钥探测——网关对 `{base_url}/models` 发起 GET（5 秒超时），成功返回延迟，失败返回具体原因（HTTP 状态 + 鉴权提示 401/403/429，或连接失败详情）；CodeBuddy 无 `/models` 端点（腾讯返回 404）且仅支持流式，因此用最小流式 chat 探活；mock 后端本地直通、不走网络。
- **自动连接与状态点**（参考 DeepSeek Harness）：保存提供方后立即自动探活；打开面板对每个已配置提供方后台自动重测。卡片名称前的状态点：**绿色 = 配置正确**（悬停显示延迟），**红色 = 上次测试失败**（悬停显示具体原因），**灰色 = 尚未测试**。
- **健康轮询**（供给侧执行体）：执行体（`pair://` / `agent://`）保存后由网关**持续**轮询其端点（`GET {base_url}/models`，暂停/恢复与间隔、阈值均可在卡片上调整，默认 15s / 3 / 10）。卡片叠加**四态状态点**（优先于连接测试三态）：**绿 = 健康**，**黄 = 降级**（连续失败，仍参与路由），**红 = 已摘除**（连续失败过多，退出路由并从未 `GET /v1/models` 模型目录剔除），**灰 = 未轮询/待探测**（悬停显示最近失败原因与连续失败次数）。每条目可在卡片上启停轮询、调整间隔与降级/摘除阈值（API：`PUT /api/backends/:id/polling`）。任一次轮询成功即恢复健康并清零失败计数；云厂商默认不轮询，且无任何启用条目时**零开销**（不拉起轮询任务）。
- **计量贯通与 C2C 预留字段**：用量记录携带 **provider 维度**（执行体流量记 `provider=pair|agent`，云厂商记官方名），按成员持久化，面板/成员 API 以 `providerTokens` 返回；`/api/usage/export` 账单保持成员级 CSV。后端条目可携带**预留的 C2C 字段** `splitRatio`（分成比例）与 `listingId`（算力市场挂牌 ID）——写入 `backends.yaml` 并经 `GET /api/backends` 原样回传，但**尚未实现任何分成/结算逻辑**（模式 C 预留）。配额（RPM/TPM/每日）在**路由层对全部 Provider 统一执行**——执行体与云厂商同等受配额约束，超限一律 429 `quota_exceeded`，无豁免。
- **自动获取具体模型列表**（参考 cc-switch「获取模型」）：表单「获取模型」按钮按当前填写的内容探测端点（OpenAI 兼容 `GET {base_url}/models` → `data[].id`，自动去重），把模型服务器返回的**真实模型清单**填入模型 chips；CodeBuddy 无法列出模型（`/models` 为 404），其探测返回官方目录（`hy4-preview`、`deepseek-v4-flash`）；保存提供方时若未显式配置模型（models 为空），网关自动拉取该提供方的真实模型列表并落盘，`/v1/models` 即刻生效——显式配置的模型列表不会被覆盖。**填写 API 密钥后自动获取最新模型**（参考 DeepSeek Harness 模型添加）：停止输入约 1 秒后自动用服务器最新模型列表替换模型 chips，无需点按钮（mock 不走网络；自定义提供方需先填 base_url）。

配置以 `providers` 列表保存在 `backends.yaml`（同 DSH 的 `providers:`）。直填密钥落盘并以掩码展示（`sk-***abcd`）；环境变量引用不落盘、展示为 `env:NAME`，密钥永不写明文。命令行（`--backend`/环境变量）仅作初始补齐，配置文件优先级更高。
启动后：
- 管理面板：浏览器打开 http://127.0.0.1:39091/
- 组员自动发现：UDP 广播（端口 39090）

### 组员端（消费端角色）

```bash
aipowergateway --role client
# 自动发现组长 → 免密接入 → 调用模型
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
export ANTHROPIC_MODEL=deepseek-chat   # 或 kimi-2.7-code / hy4-preview / deepseek-v4-flash 等
```

### 模型目录（组长分享的模型）

```bash
curl http://<组长IP>:39091/v1/models
# 例如 deepseek-chat / kimi-2.7-code / glm-4-flash / hy4-preview / deepseek-v4-flash
```

## 支持的官方大模型

| 提供商 | 环境变量 | 默认模型 |
|--------|---------|---------|
| DeepSeek | `AIPOWERLINK_DEEPSEEK_API_KEY` | deepseek-chat |
| Kimi（月之暗面） | `AIPOWERLINK_KIMI_API_KEY` | moonshot-v1-8k |
| 智谱 GLM | `AIPOWERLINK_ZHIPU_API_KEY` | glm-4-flash |
| CodeBuddy（腾讯 Copilot） | `AIPOWERLINK_CODEBUDDY_API_KEY`（或 `CODEBUDDY_API_KEY`） | deepseek-v4-flash |
| 自定义 | `AIPOWERLINK_BASE_URL` + `AIPOWERLINK_MODEL` | — |

模型名前缀路由：`deepseek-*`→DeepSeek、`kimi-*`→Kimi、`glm-*`→智谱。CodeBuddy 模型名无统一前缀（`hy4-preview` / `deepseek-v4-flash` 按精确模型名路由）。

## 配置管理

```bash
# 读写配置（敏感值自动加密 + 脱敏显示）
aipowergateway config set port 39091
aipowergateway config list                    # 敏感值显示为 [set]
```

### 链路加密（组长端策略）

`link.encrypt` 控制组长侧链路加密（成员↔组长链路 AES-256-GCM，`x-aipg-enc: v1` 头协商）：

- `aes-gcm`（组长缺省）：协商式——加密请求解密处理、明文请求原样透传（旧成员完全兼容）
- `enforce`：强制——面板开关或 `POST /api/control {"action":"link-encrypt","mode":"enforce"}`
  切换后，未声明加密的 `/v1/*` 模型请求一律回 **426 Upgrade Required**（管理 `/api/*` 与
  `/auth/*` 排除端点保持明文可达，面板/换令牌不会被锁死）；加密流量照常处理
- `off`：纯明文透传

```bash
aipowergateway config set link.encrypt aes-gcm   # 组长缺省（协商式）
aipowergateway config set link.encrypt enforce   # 强制：未加密 /v1/* → 426
```

管理面板「控制」页新增三态「链路加密」开关，切换即时生效并持久化到数据目录
`link-encrypt.json`（重启后文件优先于配置）。成员端读取同一键：`off`（缺省）明文发送，
其余值跨网络深链流量加密。

### 负载红线拦截（挖矿 / 深伪）

合规四红线（挖矿 / 深伪 / 数据出境 / 二清）中，**挖矿与深度伪造的负载拦截在网关侧**：
cloud 不碰内容，网关对发往上游的请求明文做**纯内存**关键词判定（`crates/lan-share/src/policy.rs`），
命中挖矿/深伪请求回 **403** `blocked by load whitelist: mining|deepfake`。

- **零知识**：判定只在网关进程内，不落盘、不上云、不写入日志（tracing 仅记类别与计数，不回显内容）
- **默认开启**（红线第一行代码就要）；管理面板「控制」页「负载红线拦截」开关或
  `POST /api/control {"action":"load-policy","enabled":false}` 可关闭；持久化到
  `load-policy.json`（重启后文件优先）
- 成员 gateway 的共享通道复用同一策略，红线拦截自动覆盖成员转发请求

## 自定义角色

```bash
# 内置角色只读，复制后定制
aipowergateway role clone server my-leader
aipowergateway role list    # server(system) client(system) my-leader(user)
aipowergateway --role my-leader   # 以自定义角色启动
```

## 成员治理

免密接入靠「看得见 + 可拉黑」治理：

- **可见性** — 组长面板展示每个组员的机器名、显示名、**来源 IP**、**网关 ID**（`name:port`）、在线状态与 token 用量
- **拉黑** — 组长可一键拉黑组员：该成员与其来源 IP 被禁、token 全部吊销，并持久化到数据目录 `banned.json`（重启后依然生效）
- **解禁** — 解除拉黑后该成员可重新接入

## 系统托盘

- 组长：打开管理面板 / 开启共享 / 暂停共享 / 退出（0.2.0 起免密）
- 组员：组长列表 / 接入状态 / 改名 / 用量 / 退出
- `--no-tray`：纯命令行模式

## 启动方式

- 单实例：重复启动会提示 `already running` 并退出
- 开机自启：`aipowergateway autostart enable|disable|status`（Windows 注册表 / Linux XDG / macOS 登录项）

## 架构

```
组员（OpenAI 或 Anthropic 接口）
    |  发送模型名：deepseek-chat / kimi-2.7-code / hy4-preview
组长网关 aipowergateway（鉴权 + 计量 + 广播 + 管理面板）
    |-- deepseek-* -> DeepSeek
    |-- kimi-*     -> Kimi
    |-- glm-*      -> 智谱 GLM
    |-- CodeBuddy  -> 精确模型名（hy4-preview / deepseek-v4-flash，无前缀）
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