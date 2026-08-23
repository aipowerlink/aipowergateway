## Purpose

组长端管理网页（薄壳 + React，参考 DeepSeek Harness web 结构）：展示成员列表、token 用量，支持踢人/改密/暂停共享操作，让组长在浏览器即可管理共享。

## ADDED Requirements

### Requirement: 管理网页展示
管理网页 SHALL 展示成员列表（机器名/IP/显示名/在线）与每人 token 用量，并实时刷新。

#### Scenario: 打开管理页
- **WHEN** 组长在浏览器打开管理网页
- **THEN** 展示成员列表与用量

#### Scenario: 状态变化
- **WHEN** 成员上线/离线或用量变化
- **THEN** 页面状态自动更新

### Requirement: 管理操作
管理网页 SHALL 支持踢出成员、修改密码、暂停/恢复共享。

#### Scenario: 踢出成员
- **WHEN** 组长点击踢出某成员
- **THEN** 该成员 token 失效，页面状态更新

#### Scenario: 修改密码
- **WHEN** 组长提交新密码
- **THEN** 新密码生效，旧 token 失效

### Requirement: 访问保护
管理网页 SHALL 仅限组长本机/局域网访问（本地鉴权），非授权访问被拒绝。

#### Scenario: 未授权访问
- **WHEN** 非授权请求访问管理页
- **THEN** 请求被拒绝