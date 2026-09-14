# 规则执行引擎 Rust 化（数据面）— Tasks

## Done

- [x] `crates/lan-share/src/rules.rs`：RuleSet/Rule/Candidate/Resolution + RuleResolver
      （resolve / rule_names / list / upsert / remove / load / load_from_file）+ estimate_tokens
      + 9 项单测（token_tier 选模、fixed 原序、未命中回退原行为、规则选择优先级、排序、
      upsert/remove、文件加载单集与数组、token 估算比例）
- [x] lib.rs 导出 `pub mod rules` + `pub use rules::{Resolution, Rule, RuleResolver, RuleSet}`
- [x] server.rs：assemble 加载 model-rule-set.json → ApiState.rules / rules_file；/api/rules 路由
- [x] api.rs ApiState 新增 rules / rules_file 字段
- [x] chat_completions：resolve → 候选循环（route 失败或 chat 可重试错误 → 下一候选；
      全部失败 502 规则命中 / 500 未命中）；流式取候选[0]；X-APL-Rule / X-APL-Upstream 头
- [x] messages（Anthropic）：resolve → 候选[0]；响应头 + 遥测
- [x] usage.rs：MemberUsage.rule_set_tokens + record_rule（不重复计总量）+ 2 项单测
- [x] api.rs /api/rules GET+POST（保存落盘 + 热加载）、api_models / api_info 合并规则名
- [x] models_openai / models_anthropic 合并规则名
- [x] Web ControlsPanel 规则引擎卡片 + i18n（zh/en）+ jsonArea CSS

## TODO

- [x] Web `npm run build` 通过（47 modules）
- [x] `cargo build` 全量通过；`cargo test -p aipg-lan-share` 85 通过
- [x] 端到端验证（mock 后端 + 内联规则集）：
      - model=default-cost 小提示词 → token_tier 选 ghost-8k → 连接失败回退 mock-7b，
        200 + X-APL-Rule=default-cost / X-APL-Upstream=mock-7b ✅
      - 超大提示词（est>6000）跳过 ghost-8k 直选 mock-7b ✅
      - 未命中规则名（mock-7b）原行为 200；不存在模型 400 原行为 ✅
      - 全部候选失败 → 502（always-fail 规则，ghost-8k 连接拒绝）✅
      - POST /api/rules 保存 + 热加载即刻生效，model-rule-set.json 落盘 ✅
      - /v1/models 列出规则名（owned_by=aipowerlink-rule）；/api/info rules/ruleSetCount ✅
      - /api/members usage 含 ruleSetTokens 聚合（default-cost:13100，总量不重复计）✅
      - Anthropic /v1/messages 规则命中（候选[0]，成功 200 + 响应头；候选失败无回退 500）✅
      - OpenAI 流式请求截取候选[0] + SSE + X-APL 响应头 ✅
- [x] Web 面板手工抽查（GET/POST /api/rules 全链路）
- [x] openspec validate --changes 通过（8/8）
- [x] README 规则引擎小节
- [x] apl_docs：09 号 §7 数据面标注（本地内联 ✅）、用户使用手册 §11 一致性修正
- [x] commit + push origin/main