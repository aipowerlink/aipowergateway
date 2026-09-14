# 规则执行引擎 Rust 化（数据面）

## Why

09 号文档定义了网关数据面核心语义：用户请求的 `model` 字段是**规则集的 `name`**（如
`default-cost`），网关按规则名解析出真实上游候选序列，按估算 prompt token 选最便宜且
装得下的候选；上游失败按序回退。当前 aipowergateway（Rust 重写）尚无任何规则/解析代码
（grep `model_rule|RuleSet|rule_set` 零命中），须实现 Resolver + 路由接入 + 遥测字段。

本轮范围为 09 号文档 §7「Gateway 数据面」的**本地内联**部分（config/data_dir 加载），
云端 saasync 60s 轮询（aipowergateway 无 SaaS 客户端）留后续。

## What Changes

- `crates/lan-share/src/rules.rs`：RuleResolver（byName 内存表，线程安全）——`resolve`
  （* 优先 → exact → min order；token_tier 过滤+升序+无上限置底+fallback 追加；fixed 原序）、
  `rule_names`、`list`、`upsert`、`remove`、`load`、`load_from_file`（单集或数组，JSON 缺失/损坏
  不 panic）；`estimate_tokens` 粗粒度估算（字符数/4）。
- `data_dir/model-rule-set.json` 启动加载（文件优先，与 load-policy.json 同模式）。
- 双协议入口（/v1/chat/completions + /v1/messages）在 `route(model)` 前调用 resolve：
  非流式完整候选循环 + 可重试错误推进；流式限候选[0]；Anthropic 上限候选[0]（§4.3）。
  未命中规则名 → 维持原行为（当真实模型名路由）。
- HTTP 响应头 `X-APL-Rule`（命中规则名）/ `X-APL-Upstream`（最终上游模型）（14 号文档）。
- `/v1/models`（OpenAI + Anthropic 格式）、`/api/models`、`/api/info` 合并规则名。
- `/api/rules` GET（已加载规则集 + 规则名）+ POST（保存 model-rule-set.json + 热加载）。
- `usage_records` 遥测：MemberUsage 增加 `rule_set_tokens` 聚合维度（09 号 §5 路径兼容）。
- Web Console ControlsPanel 新增「规则执行引擎」卡片：展示规则集 + 规则名 + JSON 编辑保存。

## Impact

- lan-share 测试：rules.rs 9 项 + usage.rs rule_set 维度 2 项新增。
- 不破坏既有路由：未命中规则名行为与之前完全一致；`/api/rules` 为新增端点。
- 依赖 09/08/14 号文档 schema；与 10 号配额（最外层先拦）不冲突。