## Purpose

组员端（消费端角色）接入：自动发现局域网组长、密码换 token、OpenAI 兼容调用、改名与个人用量查看，装客户端即用的零配置体验。

## ADDED Requirements

### Requirement: 自动发现组长
系统 SHALL 监听 UDP 广播并主动扫描，发现共享服务后维护组长列表（服务名、API 端口、指纹、最近发现时间）。

#### Scenario: 发现组长
- **WHEN** 局域网内组长正在广播
- **THEN** 组长列表出现该组长

#### Scenario: 组长离线
- **WHEN** 组长停止广播超时
- **THEN** 组长标记离线/移除

### Requirement: 密码接入
系统 SHALL 支持以访问密码向组长换取 Bearer token，token 有效期内可调用 API。

#### Scenario: 接入成功
- **WHEN** 组员选择组长并输入正确密码
- **THEN** 获得 token 并可调用 API

#### Scenario: 密码错误
- **WHEN** 密码错误
- **THEN** 接入被拒并显示明确错误

### Requirement: OpenAI 兼容调用
系统 SHALL 经 HTTP 调用组长 /v1/chat/completions 并接收标准响应。

#### Scenario: 调用成功
- **WHEN** 组员发起调用且 token 有效
- **THEN** 收到标准 OpenAI 响应

#### Scenario: 调用失败
- **WHEN** 调用失败（网络/鉴权/执行）
- **THEN** 显示失败原因

### Requirement: 接入失效即时生效
组长踢人/改密后，组员 token SHALL 立即失效并收到明确提示。

#### Scenario: 被踢后调用
- **WHEN** 组员被踢后尝试调用
- **THEN** 请求被拒（401）并提示接入失效

### Requirement: 身份上报与改名
系统 SHALL 在换 token 时上报机器名与显示名（默认=机器名），组员可修改显示名并即时同步到组长端。

#### Scenario: 首次接入上报
- **WHEN** 组员换 token
- **THEN** 组长端收到机器名与显示名

#### Scenario: 修改显示名
- **WHEN** 组员修改显示名
- **THEN** 组长端显示名更新

### Requirement: 个人用量
系统 SHALL 依据 API 响应 usage 累计本机用量并本地保存，组员可查看。

#### Scenario: 查看个人用量
- **WHEN** 组员请求查看用量
- **THEN** 显示本机累计 token 用量

### Requirement: 组长离线不卡死
组长离线时，系统 SHALL 提示离线并保留接入配置，不悬挂卡死。

#### Scenario: 组长离线
- **WHEN** 组长离线且请求超时
- **THEN** 标记离线并提示，配置保留