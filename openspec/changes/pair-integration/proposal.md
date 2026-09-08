# pair-integration

---
id: pair-integration
title: 执行体抽象：PAIR 与 aipoweredge-agent 作为上游 Provider 接入（对接而非替代）
created: 2026-09-06
---

## Why

NVIDIA 于 IFA 2026 发布开源 beta **PAIR（Personal AI Router）**（Apache-2.0 / Go / 641★），把家庭闲置 PC/Mac 聚合为单一 OpenAI/Ollama 兼容 endpoint，做设备发现（mDNS）、容量感知路由、动态腾退、6 位配对码 + mTLS。PAIR 与自研 `aipoweredge-agent`（AGPLv3，任务执行/计量/沙箱/状态机）**都是算力供给端**（供给侧执行体、场景分化）：PAIR ≈ 家庭场景的免费执行体；agent = 机构场景的自控执行体。PAIR **明确不做**令牌、计量、商业——这正是 AIPowerLink 的独占层。

**对接而非替代**：网关的 `Provider::Custom` = **执行体抽象的最小契约**（OpenAI 兼容 base_url + 模型 + 健康状态），PAIR 与自研 agent 都是该契约的实现。APL 用同一抽象面消费两者，实现**执行体无关**；PAIR 免费贡献家庭闲置算力供给，agent 守住机构场景（沙箱/计量/状态机/数据主权）。替代路线需重造 PAIR 已完成且无差异化的执行体工程。

载体已存在：`backend-config`（2026-08-24）已实现 `Provider::Custom`（任意 OpenAI 兼容 base_url + model）、`/api/backends` 热更新、自动探活 testStatus、「获取模型」——本 change 在其上增量补齐执行体接入体验与动态环境健康路由。

## What

- **执行体预设（家庭/机构双预设）**：面板添加「家庭执行体（PAIR）」与「机构执行体（aipoweredge-agent）」入口，分别预填 `pair://` 与 `agent://` 标识与引导文案（`base_url` 由分享者填执行体暴露的 OpenAI 兼容地址），一键创建 `Provider::Custom` 条目并自动「获取模型」探测真实模型列表。
- **Provider 级配置化健康轮询**：对执行体条目（默认 PAIR/agent 必开、其他自定义可配置）按可配置间隔轮询 `{base_url}/models`，连续失败 N 次标记降权（影响 `pool.Select` 优先级）/摘除（不再进入候选），恢复后自动回归；弥补执行体设备动态离线/被抢占的常态。
- **计量贯通**：执行体上游请求照常记 `usage_records`（`provider=pair|agent` 标记），配额/QoS 规则对其一视同仁；为 C2C 分成预留 `provider_registry` 标记位（远期模式 C 数据面先行）。
- **边界声明**：容量感知/腾退/mTLS（PAIR）与沙箱/状态机（agent）均为执行体职责，APL 内核只保留「执行体健康度 + 计量」契约；仅消费其 OpenAI 兼容 endpoint（失败语义由既有熔断 R1 + failover CCS-11 处理）。

## Capabilities

- pair-integration | 执行体接入 | 分享者可一键添加 NVIDIA PAIR（家庭）或 aipoweredge-agent（机构）执行体为上游 Provider；自定义 Provider 支持配置化健康轮询与自动降权/摘除；执行体上游请求纳入既有计量与配额；APL 保持零知识与 Key 控制

## Impact

- crates/lan-share：provider.rs（PAIR preset 常量）、health.rs（轮询调度器，新增）、registry.rs（降权/摘除/回归状态）、api.rs（/api/backends 扩展 preset 字段与轮询配置）
- web：BackendsPanel 增加「PAIR 家庭集群」入口 + 轮询配置表单 + ∠i18n
- backend-config：Provider::Custom 既有实现复用，无 BREAKING
- 文档：14-PAIR对接方案（03-tech/00-overview）已建立；README 同步
- OpenSpec change 校验