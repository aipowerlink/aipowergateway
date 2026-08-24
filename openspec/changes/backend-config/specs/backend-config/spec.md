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
