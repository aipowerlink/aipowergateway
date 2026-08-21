# aipowergateway

> AIPowerLink 的**实际开源项目**（AGPLv3）——LLM API Key 共享 + 算力供给网关。

## 定位

- **开源形态**：本仓库是 AIPowerLink 对外发布的开源网关项目（AGPLv3），面向开发者/团队/合作方
- **与 aitokengateway 的关系**：`aitokengateway` 已转为**闭源参考项目**（内部参考实现，不对外开源）——本仓库（aipowergateway）承载实际开源发布与社区
- **能力**：统一管理 LLM Key · 零知识加密分发 · LAN P2P 共享 · OpenAI/Ollama 兼容端点 · 插件 SDK
- **姊妹项目**：aipoweredge（边缘算力）/ aipoweredge-agent（执行代理）——同为开源（AGPLv3）

## 技术理念：一切皆插件

与姊妹项目 aipoweredge / aipoweredge-agent（TS + Cordis 插件运行时）一致，本网关坚持**"一切皆插件"**：

- **微内核**：`internal/runtime`——服务注册表 + 事件总线 + 依赖序模块生命周期（Boot/Stop 逆序回收），**borrows Cordis 插件契约**（与 DSH 同源哲学）
- **模块契约**：`runtime.Module`（`Name / Requires / Optional / Apply`）——新能力一律以模块加入，不堆硬编码分支
- **公开 SDK**：`pkg/plugin`（`Plugin` 接口 + `Host` 能力 + `Registry`）——第三方插件开发入口
- **角色装配**：同一二进制按角色/配置装配模块（服务端/消费端运行时选择），Optional 降级

> 契约语义与 Cordis 一致（一切皆插件、依赖序装配、事件总线通信），实现语言为 Go——与 edge/agent（TS）互补。

## 状态

🚧 新建项目（从 aitokengateway 参考实现迁移开源面）——骨架就绪，开源内容整理中。

> 命名沿革：2026-08-15 前开源网关名为 `aitokengateway`；此后**实际开源项目**更名为 **`aipowergateway`**，`aitokengateway` 保留为闭源参考项目。
