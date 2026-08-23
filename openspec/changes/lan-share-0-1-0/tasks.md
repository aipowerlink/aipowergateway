# Tasks: lan-share-0-1-0（局域网算力共享 0.1.0 实现任务）

> 依据：待实现功能清单（apl_docs 21）+ design D1-D8 + specs。实现顺序按依赖分 7 阶段。

## 1. 骨架（workspace + 微内核 + CLI + 数据目录）

- [x] 1.1 cargo workspace 初始化（src-tauri + crates/runtime + crates/lan-share + crates/lan-client + crates/lan-tray + web/），验证 `cargo build` 通过
- [x] 1.2 实现微内核 crates/runtime：trait Module（name/requires/optional/apply）+ Host（provide/get + emit/subscribe + 依赖拓扑装配 + Boot/Stop 逆序回收），验证单元测试覆盖依赖序与生命周期
- [x] 1.3 实现 Runtime::boot(role) 角色装配：内置 server/client 角色模块集，验证不同 role 装配不同模块集
- [x] 1.4 实现 CLI 入口（clap）：--role / --no-tray / config / role 子命令解析，验证 `--help` 输出齐全
- [x] 1.5 实现跨平台数据目录解析（Win %APPDATA% / Linux ~/.config / macOS ~/Library/Application Support）+ 最小侵入（不写系统全局），验证三平台路径返回正确
- [x] 1.6 接入 tracing 分级日志（debug/info/error + 文件导出），验证日志分级输出

## 2. 服务端链路（API + 鉴权 + 计量）

- [x] 2.1 实现 lan-share-server：axum HTTP 服务监听（默认 39091），API + 静态网页单端口路由，验证端口监听与路由注册
- [x] 2.2 实现 OpenAI 兼容 /v1/chat/completions handler（标准请求/响应 + usage），验证 curl 调用返回标准格式
- [x] 2.3 实现 Anthropic 兼容 /v1/messages handler（非流式），验证标准 Anthropic 格式响应
- [x] 2.4 实现 /v1/messages SSE 流式（StreamTranslator 状态机：message_start/content_block_delta/message_delta/message_stop），验证 SSE 事件序列正确
- [x] 2.5 实现 mock 执行后端（按 spec 返回标准响应 + usage 计量），验证双协议调用均返回带 usage 的结果
- [x] 2.6 实现 lan-auth：POST /auth/token（password+machineName+displayName → Bearer token），验证正确/错误密码
- [x] 2.7 实现 lan-usage：消费 API 响应 usage 按 member_id 累计 + SQLite 持久化，验证累计与重启不丢
- [x] 2.8 实现共享开关（开启/暂停/恢复，暂停拒绝新接入），验证暂停后新换 token 被拒

## 3. 成员与广播

- [x] 3.1 实现 lan-member-registry：换 token 登记成员（机器名/IP/显示名），验证成员入库
- [x] 3.2 实现在线状态：心跳维护（90s 超时离线），验证心跳刷新与超时离线
- [x] 3.3 实现改名接口 POST /auth/rename + registry 同步，验证组长端改名即时更新
- [x] 3.4 实现踢人吊销（token 失效 + 禁止名单 IP/指纹双维度），验证被踢后调用 401 且无法重换 token
- [x] 3.5 实现修改密码（旧密码/旧 token 全失效），验证改密后旧 token 调用被拒
- [x] 3.6 实现密码指纹派生（哈希前 N 位），验证指纹用于广播
- [x] 3.7 实现 lan-discovery-broadcast：UDP 周期广播 {name, apiPort, fingerprint}（10s，关闭即停），验证广播内容与启停

## 4. 消费端链路

- [x] 4.1 实现 lan-discovery-client：UDP 监听广播 + 主动扫描，组长列表（去重刷新/离线移除），验证发现与去重
- [x] 4.2 实现 lan-share-client：选择组长 + 密码 → 换 Bearer token，验证接入成功/失败
- [x] 4.3 实现 OpenAI 兼容调用（带 token），验证标准响应接收
- [x] 4.4 实现 Anthropic 兼容调用（含 SSE 流式消费），验证流式响应接收
- [x] 4.5 实现被踢/改密即时失效（401 拒绝 + 明确提示），验证失效提示
- [x] 4.6 实现 lan-identity：换 token 上报机器名/显示名（默认=机器名）+ 改名同步，验证上报与改名
- [x] 4.7 实现 lan-usage-view：响应 usage 累计 + 个人用量查看，验证累计与展示
- [x] 4.8 实现组长离线不卡死（请求超时 → 标记离线 + 保留配置），验证超时行为
- [x] 4.9 实现组长列表持久化（client_config 表），验证重启后列表保留

## 5. 管理网页（参考 DSH，三栏）

- [x] 5.1 搭建 web/ Vite + React 18 + CSS Modules 骨架（薄壳 index.html + main.tsx 挂 #root），验证构建产物生成
- [x] 5.2 实现 AppFrame 三栏布局（左导航/中主区/右详情）+ CSS Modules，验证三栏渲染
- [x] 5.3 实现 MemberList（成员列表：机器名/IP/显示名/在线），验证数据渲染
- [x] 5.4 实现 UsageTable（每人 token 用量），验证用量渲染
- [x] 5.5 实现 ControlsPanel（踢人/改密/暂停恢复按钮），验证操作调通管理 API
- [x] 5.6 实现 DetailsPanel（选中成员详情：机器名/IP/在线时长/用量明细），验证详情展示
- [x] 5.7 实现管理 API（GET /api/members、GET /api/usage、POST /api/control）+ 实时刷新（轮询/事件），验证前后端联调
- [x] 5.8 实现访问保护（本地鉴权：仅本机/局域网），验证非授权访问被拒
- [x] 5.9 实现网页中英双语（locales.ts 字典 + 语言切换），验证双语切换
- [x] 5.10 网页产物嵌入二进制（include_bytes/Tauri assets），验证打包后可访问

## 6. 配置管理（单库 + 分区 + 加密）

- [x] 6.1 实现配置库：SQLite 单库 + 角色分区表（settings/node_identity/server_config/members/usage/client_config/client_credentials），验证建表与分区读写
- [x] 6.2 实现 schema 驱动配置（类型/默认/角色/敏感度声明），验证配置校验
- [x] 6.3 实现 Vault 加密（密码/token 加密存储），验证密文落盘
- [x] 6.4 实现敏感值脱敏（读取/导出/日志不含明文，UI 只显示已设置/未设置），验证脱敏
- [x] 6.5 实现 ConfigService 角色视图隔离（服务端模块看不到 client 表，反之亦然），验证分区隔离
- [x] 6.6 实现 CLI config get/set（按角色视图），验证读写生效

## 7. 自定义角色（Role Profile）

- [x] 7.1 实现角色文件解析（~/.aipowerlink/roles/<id>/role.json：模块启用清单 + 配置覆盖），验证解析
- [x] 7.2 实现内置角色只读（role edit/rm server → 拒绝提示 clone），验证只读约束
- [x] 7.3 实现 role clone（内置→自定义副本，user trust），验证复制可编辑
- [x] 7.4 实现 role list/show/new/edit/rm CLI 命令（内置标 system、自定义标 user），验证命令输出
- [x] 7.5 实现 --role <custom-id> 装配自定义模块集，验证关闭的模块不运行
- [x] 7.6 实现角色校验（必需模块缺失 → 启动明确报错），验证错误提示
- [x] 7.7 实现角色切换回退（--role server 随时切回标准），验证切换

## 8. 托盘与跨平台（参考 cc-switch）

- [ ] 8.1 初始化 src-tauri（tauri init），托盘图标/菜单基础，验证 `cargo tauri dev` 托盘出现
- [ ] 8.2 实现服务端托盘菜单（打开管理面板/开启/暂停共享/改密/退出），验证菜单操作生效
- [ ] 8.3 实现消费端托盘菜单（组长列表点击接入/接入状态/改名/用量/退出），验证动态列表刷新
- [ ] 8.4 实现托盘-宿主通信（Tauri command/event ↔ 事件总线），验证托盘操作驱动模块
- [ ] 8.5 实现关闭不退出（最小侵入：关窗口/网页服务继续），验证托盘常驻
- [ ] 8.6 实现 --no-tray 纯 CLI 启动，验证无托盘运行
- [ ] 8.7 实现系统浏览器打开管理面板（platform open，三平台），验证浏览器打开
- [ ] 8.8 验证三平台托盘/自启（Win 注册表/Linux autostart/macOS LaunchAgent），验证平台差异在壳层

## 9. i18n 与收尾

- [ ] 9.1 实现 Rust 侧 i18n bundle（zh-CN/en JSON + 运行时切换），验证托盘/CLI 文案双语
- [ ] 9.2 实现语言偏好持久化（默认跟随系统 locale + 手动切换），验证偏好保留
- [ ] 9.3 全量接入（网页/托盘/CLI 文案本地化，协议错误保持英文），验证三端文案一致
- [ ] 9.4 端到端验证 0.1.0 闭环：组长开启共享 → 组员发现/接入/双协议调用 → 组长看成员与用量 → 踢人/改密即时生效 → 全程无云端依赖，验证用户故事 19 全部验收要点
- [ ] 9.5 运行 `openspec validate lan-share-0-1-0` + 全部模块测试通过，验证 change 校验与测试绿

## 10. 平台打包（0.1.0 收尾，1.0 矩阵）

- [ ] 10.1 Windows 打包（cargo tauri build），验证 MSI/NSIS 产物可安装运行
- [ ] 10.2 Linux 验证（Ubuntu 运行托盘 + API），验证 Linux 下功能一致
- [ ] 10.3 编写 README（安装/使用/角色/配置/双协议接入示例），验证文档可指引上手