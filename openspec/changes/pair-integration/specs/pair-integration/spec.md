# pair-integration：执行体抽象（PAIR 与 aipoweredge-agent 作为上游 Provider 接入）

## Purpose

把 NVIDIA PAIR（家庭）与自研 aipoweredge-agent（机构）作为 aipowergateway 的**上游执行体 Provider**（Provider::Custom）接入：一键预设、配置化健康轮询（应对设备动态离线/被抢占）、计量贯通。二者是**同类执行体（≈）、场景分化**，网关用同一 Provider 抽象面消费实现**执行体无关**。APL 保持零知识与 Key 控制——执行体设备主拿不到租用者的 Key 与请求内容。容量感知/腾退/mTLS（PAIR）与沙箱/状态机（agent）为执行体职责，不进入 APL 内核。

## ADDED Requirements

### Requirement: 执行体预设（家庭 PAIR / 机构 agent）

系统 SHALL 提供「添加执行体」入口：支持家庭执行体（`preset: "pair"`，预填 `pair://` 标识）与机构执行体（`preset: "agent"`，预填 `agent://` 标识）两种模板；分享者填写执行体暴露的 OpenAI 兼容 `base_url` 后一键创建 `Provider::Custom` 条目；未提供 `base_url` 时拒绝保存（400）。

#### Scenario: 一键添加家庭执行体
- **WHEN** 分享者在「模型」页选择「添加执行体 → 家庭（PAIR）」并填写 PAIR endpoint 地址
- **THEN** 生成 `provider=pair` 的自定义 Provider 条目，保存后自动探测模型列表并落盘，`/v1/models` 立即列出 PAIR 模型

#### Scenario: 一键添加机构执行体
- **WHEN** 分享者选择「添加执行体 → 机构（aipoweredge-agent）」并填写 agent endpoint 地址
- **THEN** 生成 `provider=agent` 的自定义 Provider 条目，保存后自动探测模型列表并落盘，`/v1/models` 立即列出 agent 模型

#### Scenario: 缺 base_url 拒绝
- **WHEN** 分享者以执行体预设保存但 `base_url` 为空
- **THEN** 返回 400 与明确错误信息，不写入配置

### Requirement: Provider 级配置化健康轮询

系统 SHALL 允许对执行体 Provider（pair/agent）启用健康轮询：按可配置间隔请求 `{base_url}/models`，连续失败达到降权阈值标记为 degraded（路由优先级下调），达到摘除阈值标记为 removed（退出候选序列），任一次成功自动恢复为 ok。

#### Scenario: 执行体下线自动降权
- **WHEN** PAIR 集群整体离线且连续探测失败 3 次
- **THEN** 该 Provider 标记 degraded，`pool.Select` 路由优先级下调，请求优先落其余可用 Provider

#### Scenario: 长期失败自动摘除
- **WHEN** 执行体 Provider 连续探测失败达到摘除阈值（默认 10 次）
- **THEN** 该 Provider 退出候选序列，不再接收新请求；`/api/backends` 返回 `healthState=removed`

#### Scenario: 恢复自动回归
- **WHEN** 已 degraded/removed 的 Provider 任一次健康探测成功
- **THEN** 状态恢复为 ok，重新进入候选序列参与路由

#### Scenario: 轮询配置可调
- **WHEN** 分享者通过 `PUT /api/backends/:id/polling` 调整间隔/阈值或停用轮询
- **THEN** HealthMonitor 按新配置执行；停用轮询的条目 `healthState=untested` 且不 spawn 轮询任务

### Requirement: 计量与配额贯通

系统 SHALL 对执行体上游请求与云厂商无差别计量：`usage_records` 记录 `provider=pair|agent`、模型、token 用量与 `access_key_id`；RPM/TPM/每日配额在路由层对全部 Provider 统一执行。

#### Scenario: 执行体请求计量
- **WHEN** 使用端经网关调用 PAIR 上的模型（如 `pair-node-3:qwen`）
- **THEN** 按 Access Key 记录 `usage_records`（provider=pair），成员用量视图与配额扣减与云厂商一致

### Requirement: 零知识与边界

系统 SHALL 保持零知识信任模型不变：执行体仅作为上游 Provider 消费其 OpenAI 兼容 endpoint；APL 持有全部上游 Key 与零知识信封，不向执行体暴露租用者 Key 或请求内容；容量感知/腾退/mTLS（PAIR）与沙箱/状态机（agent）不引入 APL 内核。

#### Scenario: 密钥不泄露
- **WHEN** 分享者经网关转发请求到 PAIR 集群或 agent 节点
- **THEN** 网关侧仅以执行体可访问凭证通信，租用者的 Access Key 与云厂商 Key 不出 APL 进程边界，执行体设备主不可见