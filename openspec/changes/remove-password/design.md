# 设计：去除密码功能

## D1：会话机制保留，仅移除密码维度

Bearer token 会话是成员识别与按成员计量（usage/quota）的基础，完整保留：
- `AuthService::new(ttl_secs)`（去掉 password 参数），`issue(machine_name, display_name, ip)` 不再校验密码，仅保留 banned/ banned_ips 检查。
- `verify` / `revoke_member` / `auth_rename` / `/api/control revoke` 不变。

## D2：/auth/token 免密签发

body 仅要求 machineName；displayName 可选；password 字段即使存在也忽略（旧客户端向后兼容）。签发失败仅剩 banned 场景，返回 401。

## D3：移除密码触点

- api_control 删除 `changePassword` 分支（未知 action 返回 400 unknown action）。
- 托盘删「修改密码」菜单项与 `TrayAction::ChangePassword` 变体；CLI 删托盘动作分支。
- 管理网页删除改密卡片与 zh/en 文案；CLI 删除 AIPOWERLINK_PASSWORD 环境变量读取。

## D4：广播指纹弃用（协议兼容）

BroadcastConfig/广播 JSON 仍含 fingerprint 字段（避免消费端 discovery 解析破坏），服务端传空串；CLI 不再打印指纹。

## D5：测试与文档

cargo test --workspace 全绿（auth 测试改为免密语义）；README/docs 更新为免密接入描述；openspec validate 通过。