# AIPowerLink 插件开发指南（Plugin Development Guide）

> 目标：让开发者（含未来的你自己）在不通读全部源码的情况下，按本指南把新功能做成插件挂进网关。
> 配套参考：`crates/lan-share/src/quota.rs` 是「ApiState 服务组件」形态的完整示例；`crates/lan-share/src/backend.rs` 是「Backend Provider」形态的注册表实现。

## 1. 插件形态总览

aipowergateway 是单二进制微内核：所有插件**静态编译进主程序**（不做动态加载 .so/.dll——对局域网工具无必要，违背克制原则）。当前有三种插件槽位：

| 形态 | 位置 | 适用场景 | 典型例子 |
| --- | --- | --- | --- |
| A. Backend Provider | `crates/lan-share/src/backend.rs`（`Backend` trait + `BackendRegistry`） | 接入新的模型来源 | mock / deepseek / kimi / zhipu；将来接 Ollama / llama.cpp |
| B. ApiState 服务组件 | `crates/lan-share/src/api.rs`（`ApiState` 字段 + `ShareServer::new` 装配） | 在请求管线上新增能力 | `quota`：按成员配额，超限 429 |
| C. API 路由 | `crates/lan-share/src/api.rs`（handler）+ `server.rs`（router 挂载） | 暴露新的管理/查询接口 | `/api/usage/export`：账单 CSV 导出 |

另有 `crates/runtime` 的 `Module` trait + `Registry`（对应 DSH/Cordis 插件语义），目前是**骨架**：CLI role 命令管理模块清单，但实际运行路径（`cli/main.rs` → `run_server`）尚未走 `Runtime::boot` 装配。在新功能尚无热插拔需求前，**不要**为接入它而重构运行路径（克制）。

## 2. 形态 A：Backend Provider（接入新模型来源）

执行后端抽象：输入 OpenAI 兼容请求，返回标准响应（含 `usage`）。步骤如下：

1. 在 `backend.rs` 的 `Provider` 枚举加变体（如 `Ollama`），实现 `name()` / `base_url()` / `default_model()`：

```rust
Provider::Ollama => Some("http://127.0.0.1:11434/v1"), // OpenAI 兼容端点
Provider::Ollama => "llama3.2",
```

2. 若目标服务是 OpenAI 兼容（大多数是），**复用 `OpenAICompatBackend`**，无需新写后端；在 `cli/main.rs` 的 `build_registry` 对应分支注册即可：

```rust
"ollama" => registry.register(Arc::new(OpenAICompatBackend::new(OpenAICompatConfig {
    provider: Provider::Ollama,
    api_key: "ollama".into(), // 本地无需 key
    ..Default::default()
})) as Arc<dyn Backend>),
```

3. 完成后 `cargo test -p aipg-lan-share`，并确认 `/v1/models` 返回新模型。

## 3. 形态 B：ApiState 服务组件（示例：QuotaService）

在请求管线上新增能力（如配额/限流/审计）的标准做法，参照 `quota.rs`：

- 新建 `crates/lan-share/src/<name>.rs`：`#[derive(Clone)]` 服务，内部 `Arc<Inner>`（`RwLock<HashMap<..>>` 状态 + 持久化路径），仿 `UsageService` 的 `new(path)`/`save`/`load` 模式（JSON 文件持久化到 `data_dir`）。
- 在 `api.rs` 的 `ApiState` 加字段，在 `server.rs` `ShareServer::new` 装配（`cfg.data_dir.join("<name>.json")`）。
- 在**两个入口**插入逻辑：`chat_completions`（OpenAI）与 `messages`（Anthropic），token 校验之后、路由后端之前。
- module 导出：`lib.rs` 加 `pub mod <name>;` + 类型 re-export。

边界约定：
- 超限语义统一 **429**（对齐 LiteLLM per-key quota / AgentGateway budget）：响应 `{"error":{"message":..., "type":"insufficient_quota", "code":"quota_exceeded"}}`。
- 服务只做自己的事（QuotaService 只管上限，用量从 `UsageService` 读），避免状态双写。

## 4. 形态 C：API 路由（示例：/api/usage/export）

在 `api.rs` 加 `pub async fn` handler（`State<ApiState>` 注入），在 `server.rs` `router()` 挂载：

```rust
.route("/api/usage/export", get(api::api_usage_export))
```

约定：管理类 GET 返回 JSON；文件下载用 `Content-Disposition: attachment` + `text/csv`。

## 5. 开发契约（必须遵守）

1. **双协议**：OpenAI（/v1/chat/completions）与 Anthropic（/v1/messages）入口都要处理新逻辑。
2. **用量计量**：只信任响应里的 `usage`（`extract_openai_usage`），`record(member_id, model, prompt, completion)` 由 api.rs 调用，含模型维度（`model_tokens`）。
3. **测试**：每个新组件至少 3 个单元测试（功能/持久化/边界），跑 `cargo test --workspace` 全绿；Web 改动跑 `npm run build`（web/）验证 TS 编译。
4. **零警告**：cargo build/test 不产生任何 warning（本项目硬性要求）。
5. **脱敏**：新增配置/导出不得回传 token 等敏感值明文（见 0.1.0 spec 的 secret redaction；0.2.0 起无访问密码）。
6. **克制**：功能按最小闭环实现，不提前做动态加载/热更新/多实例协调。

## 6. 测试你的插件

- 单元：`cargo test -p aipg-lan-share`
- 端到端（本地）：`cargo run -p aipg-cli -- --backend mock --no-tray` 启动服务 → `curl` 调 `/v1/chat/completions` → 查 `/api/members` 看用量 → 设配额后再次调用验证 429。
- Web：`web/` 下 `npm run dev` 直连本地服务调试。

## 7. 发布

插件随主二进制发布（CI：`.github/workflows/build-release.yml` 三平台构建打包）。无独立插件分发机制——需要新能力时在仓库内开发合并即可。
