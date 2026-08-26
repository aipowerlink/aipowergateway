# 网关间链路压缩 + 加密设计（直连 / P2P 分层）

日期：2026-08-26
状态：已与用户逐项确认收敛，待实施（M1/M2/M3）
关联：openspec/changes/lan-share-0-1-0/design.md D3.1（HTTP/3 QUIC 演进）

---

## 1. 背景与目标

局域网/跨网共享算力时，"成员本地 gateway → 组长 gateway" 一段承载：

- 请求：OpenAI/Anthropic 兼容 JSON（多轮 messages 历史，长上下文可达数百 KB）
- 响应：SSE token 流 / 完整 JSON

两个问题：
1. **带宽**：JSON/SSE 全为文本，未压缩传输，跨网（公网）带宽利用率低
2. **保密**：跨网直连场景下（Deep Link / 公网 IP 直连），Bearer token 与对话内容明文穿越公网

目标：压缩降低带宽；按连接方式分层提供合理保密；对客户端工具（Claude Code、WorkBuddy 等）完全透明。

## 2. 现状链路（已核实）

实测于 39091 实例（PID 8280，v0.3.0-dev，--no-tray --backend mock --data-dir %TEMP%\aipg-ak-e2e）：

```
本地工具 → 成员本地 gateway(127.0.0.1:port)  ── http://{leader}:{link_base} ──> 组长 share_router(39092)
     cli/src/main.rs h_chat/h_messages          lan-client MemberGateway::proxy
组长 → 上游 LLM API (https://api.deepseek.com 等)
```

关键事实：
- 成员侧 `MemberGateway::proxy`（crates/lan-client/src/gateway.rs L71-95）整包读取响应（resp.bytes()）
- 组长侧 SSE 为**整体缓冲**后返回（openai_sse_stream 一次性拼字符串，api.rs L151）
  → **两端均非真流式**，gzip 与 AES-GCM 可整包处理，无需流式编码器
- `link_base()`：share_port 优先，回落 api_port（discovery.rs L36-39）；http 明文
- 深链注入：`set_static_leader`（cli/main.rs L428）——static_leader 存在 = 跨网直连
- 协调注册 ResolveResult 仅含 public_ip/api_port/fingerprint，无 P2P 字段
- aes-gcm 依赖已在 workspace（crates/config/src/vault.rs 用于本地令牌加密，可复用机制）

## 3. 方案总览：按连接方式分层

连接方式分为两族，组长端按**监听器**天然区分（无需猜测来源 IP）：

| 连接方式 | 传输 | 组长端边界 | 压缩 | 加密 |
|---|---|---|---|---|
| 直连·局域网 | TCP HTTP/1.1 | 39092 listener | gzip（可选） | 默认 off（信任网络） |
| 直连·跨网 | TCP HTTP/1.1 | 同上（成员声明区分） | gzip 必须 | AES-GCM 必须 |
| P2P（QUIC，1.x） | UDP HTTP/3 | quinn listener | gzip | QUIC TLS 1.3 内建，无配置 |

- **P2P 族**：QUIC 强制 TLS 1.3 → 加密零配置零成本，仅需 gzip；不暴露"是否加密"配置（避免降级后门）
- **直连族**：是否加密/加密方式可配置（见 §6）

## 4. 压缩设计（M1）

- 挂载：组长端 `CompressionLayer::new()`（tower-http compression-gzip）挂 share_router；客户端 reqwest 开 `gzip` feature（自动 Accept-Encoding + 自动解压）
- 成员请求侧压缩（可选）：gzip body + `Content-Encoding: gzip`，组长挂 DecompressionLayer
- 收益：JSON/SSE 文本 70–90% 体积削减
- **注意**：AES-GCM 加密路径下，body 为密文（高熵），**gzip 必须发生在加密之前**（先压后密），传输层压缩仅对明文有意义

## 5. 加密协议（M2，x-aipg-enc v1）

### 5.1 覆盖范围
- 加密端点：/v1/chat/completions、/v1/messages、/v1/models（**直连跨网时**）
- **排除**：/auth/token（换取 token 时尚无密钥可言）、/auth/rename
- 局域网直连：明文（不加密，兼容旧成员）

### 5.2 字节格式（raw，不用 base64）

```
请求/响应 body = nonce(12B) ‖ AES-256-GCM(gzip(明文JSON)) ‖ tag(16B)
Header:                 x-aipg-enc: v1
Content-Type:           保持 application/json（解密后语义不变）
密钥派生:                SHA-256(bearer_token) → 32B AES key
```

- base64 会膨胀 33%，抵消 gzip 收益 → 用 raw 二进制
- nonce 随机生成（OsRng），随包传输；密文自带 tag（AAD = 空）

### 5.3 处理顺序（组长侧 middleware，handler 之前）

```
请求: 有 x-aipg-enc: v1 ? → AES-GCM decrypt → gunzip → 明文 JSON 进 handler（Json extractor 不变）
      无                 → 明文透传（LAN 成员 / 旧版本兼容）
响应: 请求带 x-aipg-enc ? → 明文 JSON → gzip → AES-GCM encrypt → x-aipg-enc: v1 返回
      否则                → 原样返回
```

- 解密必须在**模型路由（body.model）与计量之前**——组长需要明文才能 accountability
- 认证（token 校验）可在解密前（token 在 header），也可后置；为一致性建议后置

### 5.4 成员侧（发起方）判断接入点

```
MemberGateway::proxy（lan-client/src/gateway.rs）:
  proxy(path, auth, body):
    if path == "/auth/token" or path == "/auth/rename"  → 明文透传
    if static_leader.is_some() 且 link.encrypt != off    → 加密发送（5.2），响应按 5.3 解密
    else                                                  → 明文（LAN / P2P 通道）
```

- static_leader 存在 = 深链跨网（cli/main.rs L419-428 注入）；None = UDP 局域网发现
- 响应解密后返回明文 bytes 给客户端工具 → 工具无感知

### 5.5 密钥联动
- token 轮换（/auth/token force:true）后双方自动重派生，无需额外状态
- 弱点：token 本身在 header 明文传输（应用层加密只保护 body）。严格场景需 TLS（config: tls 模式）

## 6. 配置面（M3）

```
config key: link.encrypt = off | aes-gcm | tls   （组长端策略 + 成员端行为）
- 直连族生效；P2P 族无此项（QUIC TLS 固定）
- 缺省：off（维持现网兼容）；深链跨网建议改 aes-gcm
```

- 组长端强制时：对未声明加密的 /v1/* 请求可回 426 Upgrade Required（可配）
- 面板（ControlsPanel）加开关 + 状态展示（可选 M3 范围）

## 7. 兼容性 / 风险

| 项 | 处理 |
|---|---|
| 旧成员不带 x-aipg-enc | 组长明文透传，完全兼容（协商式，非强制） |
| 跨网加密成员 → 旧组长 | 组长按明文解析失败 → 400；要求组长升级（同生态版本一致）或加入 400 时明文重试（可选降级，默认关） |
| 重放攻击 | AES-GCM 单 nonce 防重放有限；信任模型接受；tls 模式根治 |
| VPN/CGNAT 误判 | 不依赖 IP，由成员 static_leader 声明（可靠） |
| /auth/token 明文 | 唯一无法加密的端点（无密钥前置）；泄露 machineName 而非内容，可接受 |
| SSE | 整包加密，客户端解密后见完整 SSE，语义不变 |

## 8. 实施分层

- M1 压缩：tower-http CompressionLayer + reqwest gzip feature（约 3 行 + Cargo 改 feature）
- M2 加密：新增 link_crypto 模块（derive_key/gzip_encrypt/decrypt_gunzip）；组长 from_fn middleware；成员 proxy 对称改动
- M3 配置：runtime config schema 加 link.encrypt；面板开关（可选）

## 9. 测试计划

- 单元：link_crypto roundtrip（密钥派生/gzip+AES/解密解压）、错误密文 → 明确错误
- 集成：mock 组长 + 成员直连跨网（static_leader）→ 加密 chat 200 + 明文 /auth/token 互通
- 兼容：无 x-aipg-enc 请求 → 明文 200；旧成员 ↔ 新组长互通
- 回归：现有 47(lan-share) + 18(runtime) 测试全绿；Lan E2E（chat 明文）不回归

## 10. 待办（未决项）
- [ ] M1 压缩收益实测（抓一次真实跨网请求/响应，对比 gzip 前后字节数）
- [ ] 是否实施 400→明文重试降级（默认关）
- [ ] link.encrypt 缺省值最终确认（off 保兼容 vs aes-gcm 保安全）
