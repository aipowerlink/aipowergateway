# 规则执行引擎 Rust 化（数据面）

## Goals

- 实现 09 号文档 §4.1 Resolver 语义于 Rust：`resolve(requested_model, est_tokens)`
  → 候选序列 + matched；规则选择 = `*` 优先 → exact → min order。
- 本地内联加载（data_dir/model-rule-set.json，文件优先），面板可保存热更新。
- 双协议路由接入：非流式完整回退、流式/Anthropic 仅候选[0]；未命中维持原行为。
- 遥测：rule_set_tokens 聚合维度 + X-APL-Rule / X-APL-Upstream 响应头。

## Non-Goals

- 云端 saasync 60s 轮询拉取激活集（无 SaaS 客户端，09 §7「后续」）。
- 流式中途回退、规则命中率/节省量看板、可视化规则构建器。
- 真实 tokenizer（estimate_tokens 是粗粒度估算，09 号允许启发式）。