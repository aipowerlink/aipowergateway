# Spec: lan-usage-export

## ADDED Requirements

### Requirement: 账单 CSV 导出

系统 SHALL 提供 `GET /api/usage/export` 返回用量账单 CSV 附件（`text/csv` + `Content-Disposition: attachment`），列：`member_id,prompt_tokens,completion_tokens,total_tokens,calls`，按总量降序排列；Web 控制台 SHALL 提供一键导出入口。

#### Scenario: 导出成功
- **WHEN** 组长点击「导出账单 CSV」
- **THEN** 下载 usage.csv，内容含所有成员用量且按总量降序

#### Scenario: 无用量数据
- **WHEN** 尚无任何调用记录时导出
- **THEN** CSV 仅含表头

### Requirement: 按模型维度计量

用量统计 SHALL 记录每个成员的模型维度消耗（model -> 累计 tokens），经 `/api/members` 返回 `usage.modelTokens`，并在成员详情页展示模型分布。既有 usage.json（无模型字段）SHALL 兼容读入。

#### Scenario: 多模型累计
- **WHEN** 某成员先后使用 deepseek-chat 与 kimi 各若干次
- **THEN** modelTokens 分别累计两个模型的 token，详情页可见

### Requirement: 插件开发指南

仓库 SHALL 提供 `docs/PLUGINS.md` 插件开发指南，覆盖三种插件槽位（Backend Provider / ApiState 服务组件 / API 路由）的开发步骤、契约与测试要求，并以本 change 的 quota（服务组件）与 export（API 路由）为示例。

#### Scenario: 文档可查
- **WHEN** 开发者查阅 docs/PLUGINS.md
- **THEN** 文档覆盖三种插件槽位的开发步骤、契约与测试要求

