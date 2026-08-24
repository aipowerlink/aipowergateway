# 任务：模型配置

- [x] BackendEntry + Provider::Custom + 密钥解析/掩码（backend.rs）
- [x] BackendRegistry RwLock 热替换 + backend_from_entry/registry_from_entries（registry.rs）
- [x] BackendStore 文件持久化（backend_store.rs）
- [x] /api/backends GET/POST/DELETE + 密钥保留（api.rs）
- [x] server/cli 装配（with_entries，--backend 解析为条目）
- [x] Web 模型设置面板 + i18n + 导航
- [x] 测试：registry 4 + store 3（基线 56 → 63 全绿）
- [x] E2E：添加/编辑/删除/重启持久化/密钥掩码/env 引用
