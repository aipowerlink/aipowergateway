## Purpose

组长端管理网页（AppFrame 三栏布局 + 组件化 + CSS Modules，参考 DeepSeek Harness 管理面）：展示成员列表、token 用量，支持踢人/改密/暂停共享操作，让组长在浏览器即可管理共享。

## ADDED Requirements

### Requirement: 管理网页展示（三栏布局）
管理网页 SHALL 以三栏布局展示（参考 DSH AppFrame：左侧导航栏、中部主区、右侧详情栏），成员列表（机器名/IP/显示名/在线）与每人 token 用量在主区展示，并实时刷新；选中成员在详情栏展示明细。

#### Scenario: 打开管理页
- **WHEN** 组长在浏览器打开管理网页
- **THEN** 三栏布局呈现：左栏导航，中栏成员列表与用量，右栏成员详情

#### Scenario: 状态变化
- **WHEN** 成员上线/离线或用量变化
- **THEN** 页面状态自动更新

#### Scenario: 选中成员查看详情
- **WHEN** 组长在主区点击某成员
- **THEN** 右侧详情栏展示该成员明细（机器名/IP/在线时长/用量）

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