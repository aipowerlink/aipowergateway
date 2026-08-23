## Purpose

组长端（服务端角色）局域网算力共享：开放双协议 HTTP API（OpenAI 兼容 + Anthropic/Claude Code 兼容）、密码鉴权、成员管理、token 用量计量与展示，让组长可控地分享本机算力给局域网组员。

## ADDED Requirements

### Requirement: 双协议 HTTP API
系统 SHALL 监听可配置 HTTP 端口（默认 39091），同时提供 OpenAI 兼容 API（/v1/chat/completions）与 Anthropic/Claude Code 兼容 API（/v1/messages，支持 SSE 流式），返回各自标准响应（含 token 计量）。

#### Scenario: OpenAI 兼容调用
- **WHEN** 组员调用 /v1/chat/completions
- **THEN** 返回标准 OpenAI 格式响应（含 usage 计量）

#### Scenario: Claude Code 接入
- **WHEN** Claude Code CLI 配置 ANTHROPIC_BASE_URL 指向本网关并调用 /v1/messages
- **THEN** 返回标准 Anthropic 格式响应（非流式或 SSE 流式，含 usage）

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

### Requirement: 敏感值脱敏（secret redaction）
系统 SHALL 不向管理网页/API 回传密码与 token 明文——只返回是否已设置；配置输出（导出/日志）同样脱敏。

#### Scenario: 管理页不显示明文
- **WHEN** 组长查看管理网页配置
- **THEN** 密码/token 显示为已设置/未设置，不回传明文

#### Scenario: 配置导出脱敏
- **WHEN** 配置被导出或写入日志
- **THEN** 敏感值被脱敏（不含密码/token 明文）

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

### Requirement: 配置角色分区
系统 SHALL 以单一配置库管理服务端/消费端两套配置：按角色分区存储，服务端模块仅读写服务端配置，消费端模块仅读写消费端配置。

#### Scenario: 服务端配置读写
- **WHEN** 组长端（服务端角色）读写配置
- **THEN** 仅访问服务端配置区（端口/密码/共享开关），不触碰消费端配置

#### Scenario: 消费端配置读写
- **WHEN** 组员端（消费端角色）读写配置
- **THEN** 仅访问消费端配置区（组长列表/token/显示名），不触碰服务端配置

#### Scenario: 双角色并存
- **WHEN** 同一设备配置了服务端与消费端两套配置
- **THEN** 两套配置分区共存，互不覆盖

### Requirement: 配置敏感值加密
系统 SHALL 将密码与 token 加密存储（Vault），读取时脱敏（不回传明文）。

#### Scenario: 密码加密存储
- **WHEN** 组长设置访问密码
- **THEN** 密码以加密形式存储（非明文），读取/导出时不返回明文

#### Scenario: 消费端 token 加密
- **WHEN** 组员保存组长 token
- **THEN** token 加密存储，读取/导出时不返回明文

### Requirement: 自定义角色配置
系统 SHALL 支持用户创建自定义角色：角色为命名的模块装配配置（含各模块启用开关与配置），用户可按需组合模块，不再限于内置 server/client 两角色。

#### Scenario: 创建自定义角色
- **WHEN** 用户创建角色 my-leader-light（如关闭用量计量与网页模块）
- **THEN** 角色以配置文件保存于用户数据目录，可被 --role 装配

#### Scenario: 以自定义角色启动
- **WHEN** 用户以 --role my-leader-light 启动
- **THEN** 仅装配该角色启用的模块（关闭的模块不运行）

#### Scenario: 角色校验失败
- **WHEN** 角色缺少必需模块或配置非法
- **THEN** 启动时明确报错并指出问题（不静默降级）

#### Scenario: 列出与删除角色
- **WHEN** 用户执行 role list / role rm
- **THEN** 列出全部角色（内置+自定义）或删除指定自定义角色

### Requirement: 内置角色只读、复制定制
系统 SHALL 将内置角色（server/client）设为只读：不可修改与删除；用户定制须复制为自定义角色后进行。

#### Scenario: 修改内置角色被拒
- **WHEN** 用户尝试编辑或删除内置角色 server/client
- **THEN** 操作被拒绝并提示只读（建议 clone 复制后修改）

#### Scenario: 复制内置角色
- **WHEN** 用户执行 role clone server my-server-custom
- **THEN** 生成自定义角色副本（user trust），可编辑其模块集

#### Scenario: 随时切换回内置
- **WHEN** 用户以 --role server 启动
- **THEN** 使用标准内置 server 角色装配（不受自定义角色影响）

#### Scenario: 角色列表标记信任级
- **WHEN** 用户执行 role list
- **THEN** 内置角色标记 system、自定义角色标记 user