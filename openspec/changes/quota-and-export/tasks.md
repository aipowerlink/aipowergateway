# Tasks: quota-and-export（组员 token 配额 + 用量账单导出）

## 1. QuotaService（新模块）

- [x] 1.1 `crates/lan-share/src/quota.rs`：`QuotaService`（set/get/all/check + quota.json 持久化，仿 UsageService）
- [x] 1.2 配额测试：`set_get_and_check` / `persists_across_reload` / `all_lists_sorted`
- [x] 1.3 `lib.rs` 导出 `pub mod quota` + 类型 re-export

## 2. 双协议配额检查

- [x] 2.1 `api.rs`：`ApiState` 增 `quota` 字段；`quota_exceeded(limit)` → 429 `insufficient_quota`
- [x] 2.2 `/v1/chat/completions` 鉴权后插入配额检查
- [x] 2.3 `/v1/messages`（Anthropic）同理
- [x] 2.4 `server.rs`：`ShareServer::new` 装配 `QuotaService`（`data_dir/quota.json`）

## 3. 用量模型维度 + CSV

- [x] 3.1 `usage.rs`：`record(member_id, model, prompt, completion)`，`MemberUsage.model_tokens`（serde default 向后兼容）
- [x] 3.2 `usage.rs`：`export_csv()` 按总量降序；测试 `model_dimension_tracked` / `export_csv_format`

## 4. API 路由

- [x] 4.1 `GET /api/usage/export`（CSV 附件）
- [x] 4.2 `GET /api/quota` + `POST /api/quota`（设置/列表）
- [x] 4.3 `/api/members` 返回 `usage.modelTokens`

## 5. Web 控制台

- [x] 5.1 `UsageTable.tsx`：配额列（行内输入编辑、超额标红）、「导出账单 CSV」按钮
- [x] 5.2 `AppFrame.tsx`：拉取 `/api/quota`、`setQuota` 提交
- [x] 5.3 `DetailsPanel.tsx`：模型分布展示；types.ts 文案
- [x] 5.4 `npm run build` 通过

## 6. 插件文档

- [x] 6.1 `docs/PLUGINS.md`：三种插件槽位 + 开发契约 + 测试/发布说明

## 7. 验证

- [x] 7.1 `cargo test --workspace` 全绿（lan-share 23 项）
- [x] 7.2 `openspec validate quota-and-export` 通过
