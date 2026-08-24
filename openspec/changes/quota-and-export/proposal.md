# Proposal: quota-and-export（组员 token 配额 + 用量账单导出）

## Why

0.1.0 已有按成员计量（lan-usage：usage.json 持久化 + 网页用量表），但组长**无法限制每个组员的消耗**——共享算力没有预算概念时，单个组员可耗尽全部配额。借鉴 LiteLLM virtual key quota 与 NVIDIA AgentGateway shared budget：为每个组员设 token 配额，超出返回 **429**（OpenAI/Anthropic 通用语义）。

同时组长需要一个可存档/可带走/可导入表格的**账单**——CSV 导出（对齐 LiteLLM spend logs）。顺带把用量计量补上**模型维度**（AgentGateway per-model metrics 的轻量版）：组长能看见每个模型各自消耗了多少。

## What Changes

- **lan-quota（新模块）**：`crates/lan-share/src/quota.rs`，`QuotaService`——按成员配额上限，JSON 持久化（`data_dir/quota.json`，与 usage.json 同模式），0/未设置 = 不限
- **双协议入口配额检查**：`/v1/chat/completions` 与 `/v1/messages` 在鉴权之后、路由后端之前检查，超限返回 429 `insufficient_quota`
- **lan-usage 扩展**：`record()` 增加模型维度（`model_tokens: HashMap<model, tokens>`，serde default 向后兼容）；`export_csv()` 生成账单
- **新 API**：`GET /api/usage/export`（CSV 附件）、`GET /api/quota`（列表）、`POST /api/quota`（设置，quota=0 解除）；`/api/members` 返回 `modelTokens`
- **Web 控制台**：UsageTable 增加配额列（行内编辑、超额标红）与「导出账单 CSV」按钮；DetailsPanel 展示该成员模型分布
- **文档**：`docs/PLUGINS.md` 插件开发指南——以 quota（服务组件形态）与 export（API 路由形态）为示例

## Capabilities

### New Capabilities

- `lan-quota`: 按成员 token 配额——组长可设每人上限，双协议入口超限 429，持久化 quota.json
- `lan-usage-export`: 账单 CSV 导出——`GET /api/usage/export` 按总量降序生成 CSV，web 一键下载

### Modified Capabilities

- `lan-usage`: 用量计量增加模型维度（model -> 累计 tokens），JSON 结构向后兼容
- `lan-web-console`: UsageTable 配额列/导出按钮，DetailsPanel 模型分布

## Impact

- 代码：lan-share 新增 quota.rs；api.rs / server.rs / usage.rs 修改；web/src 四个文件
- 数据：新增 quota.json（data_dir）；usage.json 结构向后兼容（旧文件缺 model_tokens 字段，serde default 空 map）
- 协议：无 BREAKING；新增 429 配额语义（OpenAI error.code=quota_exceeded / Anthropic 同状态码）
- 文档：docs/PLUGINS.md（插件开发指南）
- 回退：撤销 quota.rs + api.rs 配额检查 + web 配额 UI（usage 的 model_tokens 保留无害）
