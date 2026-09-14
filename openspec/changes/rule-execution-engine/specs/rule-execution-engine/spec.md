# rule-execution-engine：规则执行引擎 Rust 化（数据面）
## Purpose

网关数据面把客户端请求的 `model` 字段解析为「规则集 name」→ 真实上游模型候选序列，
按估算 prompt token 选最便宜且装得下的候选；上游失败按序回退。规则集本地内联加载
（`data_dir/model-rule-set.json`），未命中规则名时维持原行为（当真实模型名路由）。
对齐 09 号文档 §4/§5 与 14 号文档 Ubiquitous 响应头。

## ADDED Requirements

### Requirement: 规则解析与选模

系统 SHALL 在双协议请求入口（OpenAI /v1/chat/completions 与 Anthropic /v1/messages）
以 `model` 查规则集解析真实上游候选序列：规则选择优先 `match_model=="*"`、其次精确匹配、
再次 `order` 最小；`token_tier` 策略筛掉 `max_prompt_tokens < est` 的候选并按上限升序
（无上限者置底、末尾追加 fallback），`fixed` 策略按候选原序返回；`est` 由请求明文粗粒度
估算（字符数/4）。

#### Scenario: 小提示词落到便宜模型

- **WHEN** 用户以 `model:"default-cost"` 发送小提示词请求
- **THEN** 网关按规则集 token_tier 选择 `max_prompt_tokens` 最小但能装下估算 token 的候选
      （如 moonshot-v1-8k），请求转发该真实模型

#### Scenario: 超大提示词落到能装下的模型

- **WHEN** 同一规则名下提示词估算 token 超过小候选上限
- **THEN** 网关跳过放不下的小候选，选中能装下（或无上限兜底）的较大候选

#### Scenario: 未命中规则名维持原行为

- **WHEN** 请求 `model` 不是任何已加载规则集的 name
- **THEN** 网关按原行为把该字符串当真实模型名交给注册表路由，行为与未启用规则引擎一致

### Requirement: 失败回退与响应头

系统 SHALL 对非流式请求在候选序列上完整循环：候选不可路由或上游返回可重试错误
（429/5xx/超时/连接中断，按错误串启发式）时推进下一候选，全部失败回 502；流式与
Anthropic 入口（上游以流式驱动）仅取候选[0]。命中规则的请求响应须带
`X-APL-Rule`（规则名）与 `X-APL-Upstream`（最终上游模型）响应头。

#### Scenario: 5xx 回退下一候选

- **WHEN** 候选[0] 上游返回 5xx 而候选[1] 可用
- **THEN** 网关自动推进候选[1] 完成请求，返回 200，响应头 X-APL-Upstream 为候选[1] 模型

#### Scenario: 全部候选失败回 502

- **WHEN** 非流式请求所有候选均不可路由或返回可重试错误
- **THEN** 网关返回 502，错误体注明规则命中与该失败原因

### Requirement: 规则集加载与面板管理

系统 SHALL 启动时从 `data_dir/model-rule-set.json` 加载规则集（缺失/损坏不 panic、
按空集继续服务；文件优先于默认），并提供 `/api/rules` GET（已加载规则集 + 规则名）与
POST（校验后全量保存落盘 + 内存热加载），Web Console 控制台展示规则集与 JSON 编辑保存。

#### Scenario: 面板保存规则集即刻生效

- **WHEN** 组长在控制台编辑规则集 JSON 并保存
- **THEN** model-rule-set.json 落盘、内存热加载，后续请求按新规则集解析，无需重启

#### Scenario: 损坏文件启动不崩溃

- **WHEN** model-rule-set.json 不是合法 JSON（单个或数组）
- **THEN** 网关以空规则集启动并继续服务，`/api/rules` 返回空列表

### Requirement: 模型目录与遥测

系统 SHALL 将已加载规则集 name 合并进 `/v1/models`（OpenAI 与 Anthropic 格式）、
`/api/models` 与 `/api/info`；用量记录新增 `rule_set_tokens` 聚合维度（规则名 →
累计 token），命中的单次请求同样累计，且不重复计入成员总量。

#### Scenario: /v1/models 列出规则名

- **WHEN** 加载了 name 为 `default-cost` 的规则集
- **THEN** `/v1/models` 的 data 列表包含 `default-cost`，客户端可见可选规则

#### Scenario: 用量按规则集聚合

- **WHEN** 命中规则集的请求完成
- **THEN** usage 的该成员 rule_set_tokens 按规则名累计消耗 token，成员总量不变