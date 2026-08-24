# Design: quota-and-export

## D1：配额检查位置

`QuotaService.check(member_id, used)` 放在两个协议 handler 的 token 校验之后、后端路由之前——原因是配额针对**成员**（按累计用量判超限），与模型无关；放在入口统一拦截最干净。用量从 `UsageService.get(member_id).total()` 读取（服务间单向依赖，避免状态双写）。

## D2：429 语义

对齐 LiteLLM/AgentGateway：HTTP 429 + `{"error":{"message":"quota exceeded: limit N tokens","type":"insufficient_quota","code":"quota_exceeded","quota_limit":N}}`。Claude Code 等客户端对 429 的默认退避逻辑即生效。

## D3：持久化模式

仿 UsageService：`Arc<RwLock<HashMap<..>>>` 内存态 + 每次变更全量写 JSON（quota.json）。单组长规模（几十成员）下无性能问题，避免引入数据库。

## D4：模型维度计量

`record()` 增加 `model: &str` 参数，`MemberUsage.model_tokens[model] += pt + ct`。旧 usage.json 缺字段读入自动为空（`#[serde(default)]`），无迁移。

## D5：CSV 格式

`member_id,prompt_tokens,completion_tokens,total_tokens,calls`，按总量降序排列（组长最关心谁消耗最多）。
