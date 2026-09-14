# Proposal: load-whitelist（负载红线拦截——挖矿 / 深伪）

## Why

合规红线要求（06-平台资质与合规清单 四红线 / 32-国内需求端闭环清单）：**排除挖矿 / 深度伪造负载**，且 **cloud 不碰内容、拦截在网关侧**。网关是境内需求端共享场景中唯一能读到明文请求的本地实体（组长机/成员机内存判定），必须承担"内容安全注意义务"：对发往上游的请求做 **挖矿/深伪红线判定**，命中即拒绝——不留内容、不过云、不落盘（零知识红线不破坏）。

## What Changes

- **lan-policy（新模块）**：`crates/lan-share/src/policy.rs`，`LoadPolicy`——内置红线规则集（挖矿 / 深伪两类，中英关键词 + 典型指令组合），对请求体文本做内存判定（`scan_request`）；命中返回类别，**不记录内容原文**（tracing 仅记 category + 命中计数）
- **双协议入口拦截**：`/v1/chat/completions` 与 `/v1/messages` 在鉴权之后、路由后端之前扫描请求文本（OpenAI messages content / system / tools 描述；Anthropic 转 OpenAI 后的等价文本），命中红线返回 **403** `blocked by load whitelist: mining|deepfake`
- **策略开关**：默认开启；`POST /api/control load-policy {enabled}` 可切换；状态持久化 `data_dir/load-policy.json`（文件优先，同 link-encrypt.json 模式）；`/api/info` 返回 `loadPolicy` + 命中计数（进程内存）
- **Web 控制台**：ControlsPanel 增加「负载红线拦截（挖矿/深伪）」开关，展示命中计数
- **文档**：README 红线说明；04-ops/06、32 号闭环清单 ↔ 本实现的对账行（实现状态）

## Capabilities

### New Capabilities

- `lan-load-whitelist`: 网关侧红线拦截——挖矿/深伪请求 403 拒绝，零知识判定（不落盘、不过云、不记录内容）

### Modified Capabilities

- `lan-share-api`: 双协议入口在鉴权后增加红线检查
- `lan-web-console`: ControlsPanel 红线开关 + 命中展示

## Impact

- 代码：lan-share 新增 policy.rs；api.rs / server.rs / lib.rs 修改；web/src ControlsPanel(.tsx/.module.css) + types.ts
- 数据：新增 load-policy.json（data_dir）；命中计数仅进程内存、重启清零（不记内容）
- 协议：无 BREAKING；新增 403 红线拦截语义（OpenAI/Anthropic 兼容 error 结构，message 指明类别）
- 文档：README（中英）红线段落 + 06/32 需求对账
- 回退：撤销 policy.rs + api.rs 两处检查 + web 开关（load-policy.json 保留无害）