# 任务：执行体抽象（PAIR 与 aipoweredge-agent 作为上游 Provider 接入）

## P0 — 执行体预设（模式 A，manual）

- [ ] 执行体预设常量与模板（provider.rs：EXECUTOR_PRESETS：pair:// + agent://）
- [ ] /api/backends POST 支持 `preset: "pair"` / `preset: "agent"`（校验 base_url 非空）
- [ ] Web 模型页「添加执行体」入口（家庭 PAIR / 机构 agent 双卡片）+ i18n
- [ ] 保存后自动「获取模型」并落盘真实模型列表（复用 backend-config）
- [ ] 测试：预设创建 / 缺 base_url 拒绝 / 获取模型落盘（基线 63 全绿）

## P1 — 健康轮询（模式 B，auto-discovery 前置）

- [ ] HealthMonitor 轮询调度器（health.rs，tokio spawn，间隔/阈值可配置，默认 15s/3/10）
- [ ] provider_states 降权/摘除/回归状态机，接入 pool.Select 优先级
- [ ] GET /api/backends 返回 healthState + executor_kind（ok/degraded/removed/untested；pair/agent/none）
- [ ] PUT /api/backends/:id/polling 启停与调整
- [ ] Web 轮询配置表单 + 四态状态点（绿/黄/红/灰，悬停详情）
- [ ] 测试：连续失败降权→摘除→恢复回归；无轮询条目零开销

## P2 — 计量贯通与 C2C 预留

- [ ] usage_records 记录 provider=pair|agent 标记（数据面先行）
- [ ] provider_registry 预留 split_ratio / listing_id 字段（不实现分成逻辑）
- [ ] 配额/QoS 对执行体与云厂商统一执行（验证）

## 常驻 — 文档与校验

- [ ] README 同步「兼容 NVIDIA PAIR / aipoweredge-agent 执行体」
- [ ] openspec change 校验通过