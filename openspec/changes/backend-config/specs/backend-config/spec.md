# backend-config：模型配置

## Purpose

组长可在管理面板配置大模型提供方（参考 DeepSeek Harness 的 settings 模型页）：配置文件持久化、保存即热生效、密钥可直填或环境变量引用，无需重启服务。

## ADDED Requirements

### Requirement: 配置文件持久化

系统 SHALL 将提供方配置持久化到 data_dir/backends.yaml（providers 列表），服务启动时自动加载并参与模型目录与路由。

#### Scenario: 冷启动加载配置
- **WHEN** 组长已通过面板保存提供方配置后重启服务
- **THEN** /api/backends 返回相同提供方列表，/v1/models 列出其模型

### Requirement: 管理 API

系统 SHALL 提供 /api/backends：GET 返回提供方列表（密钥仅掩码展示，不回明文）；POST 新增或更新（未提供密钥字段时保留原密钥）；DELETE 移除。

#### Scenario: 面板添加提供方
- **WHEN** 组长在「模型」页保存提供方（含 API 密钥或环境变量引用）
- **THEN** 返回 ok 与该提供方 id，backends.yaml 写入对应 providers 条目

#### Scenario: 编辑保留密钥
- **WHEN** 组长编辑提供方仅修改模型名（不携带密钥字段）
- **THEN** 原密钥保持生效，模型目录更新为新模型

#### Scenario: 删除提供方
- **WHEN** 组长删除提供方
- **THEN** 该提供方从 /api/backends、backends.yaml 与 /v1/models 中移除

### Requirement: 保存即热生效（无需重启）

系统 SHALL 在配置保存后热替换后端注册表：新增/修改/删除提供方无需重启服务即对模型目录与路由生效。

#### Scenario: 保存后模型立即可用
- **WHEN** 组长添加提供方并保存（未重启服务）
- **THEN** /v1/models 立即包含该提供方模型，对应模型请求可按新配置路由

### Requirement: 密钥形态与掩码

系统 SHALL 支持直填密钥（写入 backends.yaml，面板以掩码 sk-***尾4 展示）或环境变量引用（api_key_env，掩码展示 env:NAME，密钥不落盘）。

#### Scenario: 环境变量引用不落盘
- **WHEN** 组长以环境变量名保存密钥
- **THEN** backends.yaml 仅含 api_key_env 名称不含明文，环境变量存在时 keySource 为 env

### Requirement: 自定义 OpenAI 兼容端点

系统 SHALL 允许添加自定义提供方（任意 OpenAI 兼容 base_url + model），未提供 base_url 或 model 时拒绝保存。

#### Scenario: 自定义提供方校验
- **WHEN** 组长保存自定义提供方且 base_url 或 model 为空
- **THEN** 返回 400 与明确错误信息，不写入配置

### Requirement: 多模型与标准模型预设（参考 cc-switch 添加模型）

系统 SHALL 支持一个提供方配置多个模型（models 数组），并为内置提供方提供标准模型清单（DeepSeek / Kimi / Zhipu 官方模型），面板选择内置提供方时自动带入，也可手动增删。兼容仅传单个 model 的旧客户端。

#### Scenario: 内置提供方标准配置
- **WHEN** 组长选择内置提供方（如 DeepSeek）并在面板保存
- **THEN** 该提供方自动携带官方 base_url 与标准模型清单（如 deepseek-chat / deepseek-reasoner），/v1/models 列出全部模型

#### Scenario: 一个提供方多个模型
- **WHEN** 组长为一个提供方保存多个模型（models 数组）
- **THEN** 所有模型进入模型目录并可路由，backends.yaml 的 providers 条目以 models 列表落盘

#### Scenario: 单模型兼容
- **WHEN** 客户端仅提交单个 model 字段
- **THEN** 后端按单模型列表处理，功能等价

### Requirement: 连接测试（cc-switch 式）

系统 SHALL 提供 POST /api/backends/test：对给定后端配置（表单值或已保存条目，未带密钥时继承已保存密钥）向 {base_url}/models 发起 GET（5 秒超时）验证端点与密钥，失败时返回可读原因（HTTP 状态、401/403/429 鉴权提示、连接失败详情），mock 后端本地直通不走网络。测试不落盘、不影响配置。

#### Scenario: 配置后可一键测试
- **WHEN** 组长在表单或卡片点击「测试」
- **THEN** 返回 {ok:true, latencyMs}（成功）或 {ok:false, error}（失败），前端展示 ✓ 连接成功或 ✗ 具体原因

#### Scenario: 测试使用当前表单值
- **WHEN** 表单测试且 key/base_url 未被保存
- **THEN** 以表单填写值发起测试，测试结果不写入 backends.yaml

#### Scenario: 自定义提供方测试校验
- **WHEN** 自定义提供方缺 base_url 或缺密钥时点击测试
- **THEN** 返回 400 与明确错误信息

### Requirement: 自动连接测试与状态指示（DeepSeek Harness 式）

系统 SHALL 在保存后端后自动对其端点发起连接测试，并在面板打开时自动重测已配置后端；GET /api/backends 返回每条后端的 testStatus（ok/fail/untested），前端以状态点呈现：配置正确显示绿色图标（悬停含延迟），失败显示红色（悬停含原因），未测试为灰色。

#### Scenario: 保存后自动测试
- **WHEN** 组长保存提供方（或打开「模型」页）
- **THEN** 后端自动探活 {base_url}/models 并记录结果，卡片即时显示绿/红状态点

#### Scenario: 状态指示与详情
- **WHEN** 后端连接测试成功或失败
- **THEN** 绿色图标（悬停显示延迟）/ 红色图标（悬停显示具体原因），未测试为灰色

#### Scenario: 状态随测试刷新
- **WHEN** 组长点击「测试」或删除后端
- **THEN** testStatus 相应更新，删除的后端状态被清除

### Requirement: 自动获取提供方模型列表（cc-switch「获取模型」式）

表单提供「获取模型」按钮，用当前填写的 base_url 与密钥请求 OpenAI 兼容的 GET {base_url}/models，将返回的 data[].id 列表去重后填充为模型 chips；保存提供方时若未显式配置模型列表，系统自动拉取真实模型列表写入配置并落盘。

#### Scenario: 表单获取模型
- **WHEN** 组长填写提供方与密钥后点击「获取模型」
- **THEN** 探测成功时模型 chips 被替换为服务器返回的真实模型列表（去重），失败显示具体原因

#### Scenario: 保存后自动获取
- **WHEN** 保存的提供方模型列表为空且自动探活成功
- **THEN** 网关将服务器返回的真实模型列表写入配置并落盘，面板与 /v1/models 即刻生效

#### Scenario: 已配置模型不被覆盖
- **WHEN** 提供方已显式配置模型列表
- **THEN** 保存后的自动探活仅刷新状态点，不覆盖既有模型配置
