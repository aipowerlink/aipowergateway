# 任务：负载红线拦截（挖矿 / 深伪）

- [ ] 1.0 policy.rs：规则集 + scan_request + 持久化 + 单元测试（命中/未命中/大小写/多模态 content）
- [ ] 1.1 api.rs：双协议入口 403 拦截 + /api/control load-policy + /api/info 返回 loadPolicy
- [ ] 1.2 server.rs：装配（load-policy.json 文件优先）；lib.rs 导出 LoadPolicy
- [ ] 1.3 web：ControlsPanel 红线开关 + 命中计数展示 + i18n（中英）
- [ ] 1.4 README（中英）红线说明
- [ ] 1.5 OpenSpec validate + cargo test --workspace 全绿
- [ ] 1.6 E2E：明文挖矿请求 403；明文正常请求放行；关闭开关后放行；重启保持文件策略
- [ ] 1.7 提交（对齐 06 号四红线 / 32 号 [↪️ 网关侧] 对账行）