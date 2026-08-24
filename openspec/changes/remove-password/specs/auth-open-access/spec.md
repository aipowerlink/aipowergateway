# auth-open-access：免密接入（去除密码功能）

## Purpose

局域网共享面向可信内网成员，去除密码维度：组员声明机器名即可换取会话 token，token 会话与按成员计量保持不变。

## ADDED Requirements

### Requirement: 免密换 token

系统 SHALL 允许组员在不提供密码的情况下，凭 machineName 换取 Bearer token；请求体中若携带 password 字段，系统 SHALL 忽略之。

#### Scenario: 免密签发
- **WHEN** 组员 POST /auth/token，body 为 {machineName: "test-pc", displayName: "TestPC"}（无 password）
- **THEN** 系统返回 200 与 {token, expiresAt}，token 可用于后续 API 调用

#### Scenario: 忽略旧客户端密码字段
- **WHEN** body 附带 password: "old-secret"
- **THEN** 系统仍正常签发 token（不校验、不拒绝）

### Requirement: 会话与计量不变

系统 SHALL 保持 Bearer token 会话机制：token 有效期内可调用双协议 API，用量与配额仍按会话成员计量。

#### Scenario: token 会话可用
- **WHEN** 组员用免密签发的 token 调用 /v1/chat/completions 与 /api/members
- **THEN** 调用成功且该成员用量被记录（modelTokens/配额展示不变）

### Requirement: 密码触点移除

系统 SHALL 不再提供任何密码入口：api_control 无 changePassword 动作、托盘菜单无「修改密码」、管理网页无改密卡片、CLI 无密码环境变量。

#### Scenario: 改密动作失效
- **WHEN** POST /api/control，body 为 {action: "changePassword", ...}
- **THEN** 系统返回 400（unknown action），且不改变任何状态

#### Scenario: 界面无密码入口
- **WHEN** 组长查看托盘菜单与管理网页控制面板
- **THEN** 均不出现「修改密码/新密码」相关入口

### Requirement: 指纹弃用

系统 SHALL NOT 再以密码派生的指纹作为身份预校验依据；广播报文可保留 fingerprint 字段但值为空。

#### Scenario: 广播指纹为空
- **WHEN** 组长启动共享并广播 AIPG_ANNOUNCE
- **THEN** 报文 fingerprint 字段为空串，消费端不将其用于预校验
