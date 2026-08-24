---
id: remove-password
title: 去除密码功能（免密接入）
created: 2026-08-24
---

## Why

局域网算力共享本面向可信内网成员，密码只增加了接入摩擦：组员需记忆/输入访问密码、组长需维护密码（CLI env / 托盘 / 管理网页 / 配置存储多处触点），且调试与排障时 'wrong password' 掩盖了真正的问题来源。按克制原则移除整个密码维度，接入简化为：组员声明机器名即换取会话 token。

## What

- POST /auth/token 不再校验密码：body 仅需 machineName/displayName，附带 password 字段亦被忽略。
- 移除全部密码触点：api_control changePassword 分支、托盘「修改密码」菜单项、管理网页改密卡片、CLI AIPOWERLINK_PASSWORD 环境变量、AuthService 密码哈希/改密/指纹。
- 指纹（密码哈希前 N 位）弃用：广播仍携带该字段（协议兼容），但值为空且不再用于预校验。
- 会话（Bearer token）机制保留：成员识别与按成员计量（usage/quota）不受影响。

## Capabilities

- auth-open-access | 免密接入会话 | 组员免密码换取 token，token 会话、踢人、计量不变

## Impact

- crates/lan-share：auth.rs / api.rs / server.rs
- crates/lan-tray：tray.rs（枚举 + 菜单）
- crates/lan-client：share_client.rs（connect 免密）
- crates/cli：main.rs（env 密码 / 托盘分支 / 广播指纹）
- web：ControlsPanel.tsx、types.ts
- README.md / docs/PLUGINS.md 同步更新