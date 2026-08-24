---
id: backend-config
title: 模型配置（backends.yaml + 面板热更新）
created: 2026-08-24
---

## Why

此前大模型后端只能通过命令行（--backend + 环境变量）配置：不了解命令行的组长无法配置模型，改模型必须改环境变量并重启。需求为「无法配置大模型」，并明确参考 DeepSeek Harness 的配置方式：配置文件 + 设置面板 + 保存即热生效（无需重启）。

## What

- data_dir/backends.yaml 保存提供方列表（providers），冷启动自动加载；启动参数（--backend/环境变量）仅起初始补齐作用，面板保存后固化到文件。
- Web 新增「模型」页（对齐 DSH）。模型设置：提供方卡片（密钥已配置徽标 + 编辑/删除），「添加提供方」「添加自定义提供方」，API 密钥直填或环境变量引用，保存后热更新注册表。
- /api/backends：GET（掩码展示）、POST（新增/更新，未提供密钥字段时保留原密钥）、DELETE（移除）；变更即写盘并原子替换注册表，无需重启服务。
- 自定义提供方任意 OpenAI 兼容端点（base_url + model）。

## Capabilities

- backend-config | 模型配置 | 组长可在面板配置/修改/删除大模型提供方（含自定义 OpenAI 兼容端点），密钥落盘或环境变量引用，保存即热生效

## Impact

- crates/lan-share：backend.rs（BackendEntry/Provider::Custom）、registry.rs（RwLock 热替换）、backend_store.rs（新）、api.rs（/api/backends）、server.rs
- crates/cli：main.rs（--backend 解析为条目）
- web：BackendsPanel + types/i18n + Sidebar/AppFrame
- README 同步；OpenSpec change 校验
