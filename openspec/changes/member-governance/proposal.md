---
id: member-governance
title: 成员治理（可见性 + 黑名单）
created: 2026-08-24
---

## Why

免密接入（remove-password）后，信任边界由密码转为治理：组长需要看得见每个组员（来源 IP、所连网关标识），并能把骚扰者拉黑。现状：成员记录 IP 恒为空（/auth/token 未采集客户端地址）；无网关标识列；踢人（revoke）虽然禁言但仅存内存、重启即失效、且无法解禁误伤。

## What

- /auth/token 采集客户端真实 IP（axum ConnectInfo）写入成员记录并展示。
- 成员记录携带网关标识（name:port），面板展示「网关 ID」。
- 黑名单持久化：banned 成员/IP 落盘 data_dir/banned.json，跨重启生效。
- /api/control 新增 unban 动作（解禁），面板对已拉黑成员显示「解禁」。
- /api/members 增加 banned 标记与 gatewayId 字段。

## Capabilities

- member-governance | 成员可见性与黑名单 | 组长可见组员 IP/网关 ID，可持久化拉黑与解禁

## Impact

- crates/lan-share：auth.rs / member.rs / server.rs / api.rs
- crates/cli：main.rs（cfg.name）
- web：成员列表/详情/控制面板 + i18n
- README 同步；OpenSpec change 校验