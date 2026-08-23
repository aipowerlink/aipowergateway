## Purpose

组长端（服务端角色）局域网算力共享：开放 OpenAI 兼容 HTTP API、密码鉴权、成员管理、token 用量计量与展示，让组长可控地分享本机算力给局域网组员。

## ADDED Requirements

### Requirement: OpenAI 兼容 HTTP API
系统 SHALL 监听可配置 HTTP 端口（默认 39091），提供 OpenAI 兼容 API（/v1/chat/completions），返回标准 OpenAI 响应（含 usage 计量）。

#### Scenario: 共享开启后 API 可调用
- **WHEN** 组长开启共享且服务端角色运行
- **THEN** HTTP API 可被局域网内调用，返回标准 OpenAI 格式响应

#### Scenario: 端口被占用
- **WHEN** 配置端口被其他进程占用
- **THEN** 系统报错并拒绝启动共享（不静默换端口）

### Requirement: 密码接入鉴权
系统 SHALL 要求组员以访问密码换取 Bearer token，token 有效期内凭 token 调用 API。

#### Scenario: 换 token 成功
- **WHEN** 组员提供正确密码请求换 token
- **THEN** 系统签发 Bearer token，组员可调用 API

#### Scenario: 密码错误
- **WHEN** 组员提供错误密码
- **THEN** 系统拒绝并提示密码错误

### Requirement: 踢人吊销
组长 SHALL 能吊销指定组员的 token，使其立即失效且无法重新换取。

#### Scenario: 踢掉组员
- **WHEN** 组长选择踢掉某组员
- **THEN** 该组员 token 立即失效，后续调用被拒绝（401）

#### Scenario: 被踢后重试
- **WHEN** 被踢组员尝试重新换 token
- **THEN** 系统拒绝并提示已被禁止

### Requirement: 修改密码
组长 SHALL 能修改访问密码，修改后旧密码与已签发 token 全部失效。

#### Scenario: 改密生效
- **WHEN** 组长修改访问密码
- **THEN** 旧密码换 token 被拒，已签发 token 全部失效

### Requirement: 成员登记与在线状态
系统 SHALL 在组员换 token 时登记机器名、来源 IP、显示名，并维护在线状态（心跳超时标记离线）。

#### Scenario: 成员入列
- **WHEN** 组员换 token 成功
- **THEN** 组长端成员列表出现该组员（机器名/IP/显示名）

#### Scenario: 成员离线
- **WHEN** 组员心跳超时（默认 90s）
- **THEN** 该成员标记为离线

### Requirement: 显示名修改同步
组员 SHALL 能修改显示名，修改即时同步到组长端。

#### Scenario: 组员改名
- **WHEN** 组员提交新显示名
- **THEN** 组长端成员列表显示名立即更新

### Requirement: 按成员计量 token
系统 SHALL 依据 OpenAI 标准响应 usage 字段按成员累计 token 用量并持久化（重启不丢）。

#### Scenario: 调用后累计
- **WHEN** 组员 API 调用完成并返回 usage
- **THEN** 该组员累计用量增加相应 token

#### Scenario: 重启保留
- **WHEN** 服务端进程重启
- **THEN** 累计用量从持久化恢复

### Requirement: 成员列表与用量查询
组长 SHALL 能查询全部成员（含在线/离线、机器名、IP、显示名）与每人 token 用量。

#### Scenario: 组长查看
- **WHEN** 组长请求成员/用量
- **THEN** 系统返回成员登记信息与累计用量

### Requirement: 共享暂停
组长 SHALL 能暂停共享（不再接受新换 token 与调用），已连接会话保持或按配置断开。

#### Scenario: 暂停共享
- **WHEN** 组长暂停共享
- **THEN** 新接入被拒绝，页面/托盘显示共享已暂停