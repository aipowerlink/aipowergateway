# 设计：执行体抽象（PAIR 与 aipoweredge-agent 作为上游 Provider 接入）

## D1：执行体预设条目

`EXECUTOR_PRESETS = [ { id: "pair", label: "家庭执行体（PAIR）", scheme: "pair://" }, { id: "agent", label: "机构执行体（aipoweredge-agent）", scheme: "agent://" } ]`。面板「添加执行体」时生成 `BackendEntry { provider: "pair"|"agent", model: "", base_url: "" }`，引导分享者填入执行体暴露的 OpenAI 兼容地址与可访问凭证；保存后触发「获取模型」（复用 backend-config D1/后端 `GET {base_url}/models`）拉取真实模型列表落盘。`backend_id()` = `pair-home-1` / `agent-store-3` 等用户命名。

## D2：Provider 级健康轮询

`HealthMonitor`（tokio spawn）：对启用轮询的条目（默认 pair/agent 必开、其他自定义可配置）按 `poll_interval_secs`（默认 15s）周期性 `GET {base_url}/models`（复用 probe 实现，超时 5s）。状态机：`ok` →（连续失败 ≥ `fail_threshold`，默认 3）→ `degraded`（路由优先级 -1）→（连续失败 ≥ `remove_threshold`，默认 10）→ `removed`（退出候选序列）。恢复：任一次成功即回 `ok`（degraded/removed 均回归）。状态写入 `RegistryInner.provider_states: HashMap<backend_id, ProviderState>`，`pool.Select` 读取降权；`GET /api/backends` 返回 `healthState`（ok/degraded/removed/untested）供面板状态点展示。

## D3：计量贯通

执行体上游请求与云厂商无差别：`usage_records` 记录 `provider=pair|agent`、`model`、`token`（input/output/total）、`access_key_id`；配额（RPM/TPM/每日）在路由层对全部 Provider 统一执行，不区分来源。`provider_registry` 远期扩展 `split_ratio`（C2C 分成比例，模式 C）与 `listing_id`（算力市场挂牌 ID）——本 change 仅落数据面标记字段，不实现分成逻辑。

## D4：管理 API 与面板扩展

- `GET /api/backends`：响应条目追加 `executor_kind: pair|agent|none`、`poll_interval_secs`、`healthState`；
- `POST /api/backends`：`preset: "pair"` / `"agent"` 时走执行体模板（校验 base_url 非空），常规自定义走既有路径；
- `PUT /api/backends/:id/polling`：启停/调整轮询间隔与阈值（未启用轮询的条目 healthState=untested）；
- Web 模型页：卡片区「添加执行体」入口卡片（家庭 PAIR 带 pair 标识、机构 agent 带 agent 标识，一步到「获取模型」），编辑面板含轮询配置节，状态点区分 ok（绿，悬停延迟）/ degraded（黄）/ removed（红，悬停原因）/ untested（灰）。

## D5：边界与回退

不引入执行体特有的能力到 APL 内核：容量感知/腾退/mTLS（PAIR）与沙箱/任务状态机（agent）均为执行体职责；APL 只消费 OpenAI 兼容 endpoint + 健康度 + 计量。回退：删除执行体条目即恢复普通云厂商配置，`HealthMonitor` 对无轮询条目零开销（不 spawn 任务）；执行体协议变动仅需更新预设/文档，Provider 抽象层天然隔离。