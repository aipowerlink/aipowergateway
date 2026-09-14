# 设计：负载红线拦截（挖矿 / 深伪）

## D1：规则形态与判定

`LoadPolicy` 内置两类规则集，**每类一组阈值词（中英）**，对请求体**还原的明文**做大小写不敏感子串判定：

- **mining（挖矿）**：`挖矿`、`矿池`、`矿机`、`挖币`、`cryptomining`、`mining pool`、`xmrig`、`ethminer`、`teamtredminer`、`monero miner` 等（含"编写挖矿程序/脚本/木马"指令词组合）
- **deepfake（深伪）**：`deepfake`、`deep fake`、`换脸`、`伪造视频`、`伪造人脸`、`voice clone`、`语音克隆`、`音色克隆`、`变声冒充` 等（含"生成/伪造 + 人脸/视频/语音"指令组合）

判定为**纯内存** min-cost 子串匹配：命中 → 返回 `(category, matched_term)`；未命中 → None。**不记录内容本身**：tracing 仅 output `category` 与自增计数，绝不输出请求原文或命中词（零知识红线）。

## D2：扫描面与入口

`scan_request(body) -> Option<PolicyHit>` 提取需要检查的明文：

- OpenAI 体：`system`（string 或数组 text）、`messages[].content`（string / 多模态数组中的 text）、`tools[].description`、`tools[].function.description`
- Anthropic 体：转 OpenAI（`anthropic_to_openai`）后走同一扫描——因此 `messages` 入口与 `chat_completions` 共用同一函数

拦截点：`chat_completions` / `messages` **鉴权 + 配额之后、`backends.route(model)` 之前**。命中返回 **403**（OpenAI/Anthropic 兼容 error 结构）。

## D3：策略持久化与开关

- `data_dir/load-policy.json`：`{ "enabled": true }`——启动时读文件（文件优先生效），默认 enabled=true（红线默认开启）
- `POST /api/control load-policy {enabled}`：写文件 + 热更 RwLock
- 命中计数：进程内存 `AtomicU64`（mining/deepfake 各一），`GET /api/info` 暴露 `loadPolicy: {enabled, miningHits, deepfakeHits}`——重启清零（不落盘内容）

## D4：零知识边界

- 明文只在**网关进程内**读取并即时丢弃，不写盘、不上云、不出日志
- 命中提示仅返回类别（`mining`/`deepfake`），不回显命中词，避免把红线内容回带给调用方造成二次扩散
- 组员侧（成员 gateway）共享通道 `share_router` 复用同一 `ApiState`，因此红线检查**自动覆盖成员 gateway 的转发请求**

## D5：Web 面板

ControlsPanel 增加「负载红线拦截」开关（复用 autostart 开关样式）+ 命中计数展示（mining / deepfake 各一）。i18n 中英双语。