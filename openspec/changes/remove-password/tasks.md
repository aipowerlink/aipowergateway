## 任务

- [ ] 1.0 AuthService 去密码：new/issue 签名、删 change_password/fingerprint/hash_password
- [ ] 1.1 auth.rs 测试改免密语义
- [ ] 1.2 api.rs auth_token 免密 + api_control 删 changePassword
- [ ] 1.3 server.rs 删 cfg.password 与 fingerprint()
- [ ] 1.4 cli main.rs 删 env 密码/托盘分支/指纹打印
- [ ] 1.5 tray.rs 删「修改密码」菜单与枚举变体
- [ ] 1.6 share_client.rs connect 去 password 参数与 body
- [ ] 1.7 web 删改密卡片与 i18n
- [ ] 1.8 README/docs 同步 + openspec validate
- [ ] 1.9 cargo test --workspace 全绿