# Spec: lan-quota

## ADDED Requirements

### Requirement: 按成员配额

组长 SHALL 能为每个组员设置 token 配额上限；未设置或为 0 视为不限。配额 SHALL 持久化到 `data_dir/quota.json`（与 usage.json 同模式）并在重启后恢复。

#### Scenario: 设置配额
- **WHEN** 组长通过 `POST /api/quota` 提交 `{memberId, quota}`
- **THEN** 该成员配额生效，再次 `GET /api/quota` 可查见

#### Scenario: 解除配额
- **WHEN** 组长提交 `{memberId, quota: 0}`
- **THEN** 该成员配额被移除，不再受限

#### Scenario: 配额持久化
- **WHEN** 服务重启后查询配额
- **THEN** 已设置的配额全部保留

### Requirement: 配额超限拒绝（429）

成员累计用量达到配额上限时，系统 SHALL 拒绝请求并返回 HTTP 429，错误体含 `code=quota_exceeded` 与 `quota_limit`；OpenAI（/v1/chat/completions）与 Anthropic（/v1/messages）两个入口均 SHALL 生效。

#### Scenario: OpenAI 入口超限
- **WHEN** 某成员用量已达配额后调用 /v1/chat/completions
- **THEN** 返回 429，`error.code` 为 `quota_exceeded`

#### Scenario: Anthropic 入口超限
- **WHEN** 某成员用量已达配额后调用 /v1/messages
- **THEN** 返回 429，语义同 OpenAI 入口

#### Scenario: 未超限放行
- **WHEN** 成员用量未达配额或无配额限制
- **THEN** 请求正常进入后端执行

### Requirement: 配额展示（web）

组长端 Web 控制台的用量页 SHALL 展示每个成员的配额与当前用量；超额状态 SHALL 以醒目样式标注；组长 SHALL 能在行内直接修改配额。

#### Scenario: 行内修改配额
- **WHEN** 组长在用量表配额输入框填写数字并离开焦点
- **THEN** 配额保存并刷新显示

