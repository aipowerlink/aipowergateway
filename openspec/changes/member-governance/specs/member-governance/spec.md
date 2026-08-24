# member-governance：成员可见性与黑名单

## Purpose

免密接入后由治理兜底信任：组长可见组员来源 IP 与所连网关标识，可持久化拉黑骚扰者并可解禁。

## ADDED Requirements

### Requirement: 真实 IP 采集与展示

系统 SHALL 在 /auth/token 时记录客户端来源 IP，并在 /api/members 与管理面板中展示。

#### Scenario: 接入即记录 IP
- **WHEN** 组员 POST /auth/token（携带 machineName）
- **THEN** 成员记录 ip 为该请求来源地址，/api/members 返回该字段

### Requirement: 网关标识

系统 SHALL 在成员记录中保存所连网关标识（name:port），并在 /api/members 以 gatewayId 返回。

#### Scenario: 成员带网关 ID
- **WHEN** 组长界面请求 /api/members
- **THEN** 每个成员条目含 gatewayId，值为广播名与端口（如 aipowerlink-share:39091）

### Requirement: 黑名单持久化

系统 SHALL 将被拉黑（revoke）的成员/IP 落盘（data_dir/banned.json），服务重启后继续生效：拉黑成员再接入被拒。

#### Scenario: 拉黑跨重启
- **GIVEN** 组长调用 /api/control revoke 拉黑成员 pc-1
- **WHEN** 服务重启后 pc-1 再 POST /auth/token
- **THEN** 请求被 401 拒绝（banned）

### Requirement: 解禁

系统 SHALL 支持 /api/control unban 解除拉黑，解除后该成员可重新接入。

#### Scenario: 解禁恢复接入
- **GIVEN** 成员 pc-1 处于黑名单
- **WHEN** 组长调用 /api/control unban（memberId=pc-1）
- **THEN** pc-1 再 POST /auth/token 成功，/api/members 中 banned=false
