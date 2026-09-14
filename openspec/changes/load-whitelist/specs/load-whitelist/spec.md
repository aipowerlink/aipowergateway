# load-whitelist：负载红线拦截（挖矿 / 深伪）
## Purpose

在网关侧（组长机）拦截挖矿 / 深度伪造两类红线负载请求：cloud 不碰内容、拦截归网关（32 号闭环清单）。网关对请求明文做**纯内存**红线判定，命中 403 拒绝；零知识红线不破坏（不落盘、不过云、不记录内容）。

## ADDED Requirements

### Requirement: 红线规则判定

系统 SHALL 对请求体还原的明文（system / messages content / tools 描述）执行挖矿与深伪两类关键词判定，命中时在路由上游之前拒绝（403）。

#### Scenario: 挖矿指令被拦截

- **WHEN** 组员发送含「编写挖矿程序 / mining pool / xmrig」等挖矿指令的对话请求
- **THEN** 网关返回 403，错误 message 注明 `blocked by load whitelist: mining`，请求不转发上游、不计 token 用量

#### Scenario: 深伪指令被拦截

- **WHEN** 组员发送含「deepfake / 换脸 / 语音克隆」等深伪指令的对话请求
- **THEN** 网关返回 403，错误 message 注明 `blocked by load whitelist: deepfake`，请求不转发上游

#### Scenario: 正常请求放行

- **WHEN** 请求明文不命中任何红线关键词
- **THEN** 请求正常路由、转发、记录用量，行为与未启用红线时一致

### Requirement: 零知识判定

系统 SHALL 只在网关进程内存中完成红线判定，不将请求内容写盘、不上云、不写入日志；命中提示仅返回类别，不回显命中词。

#### Scenario: 命中不落任何内容

- **WHEN** 一次红线命中发生
- **THEN** tracing 仅记录 category 与自增计数，日志与磁盘均不含请求原文或命中词

### Requirement: 策略开关与持久化

系统 SHALL 默认开启红线拦截，并允许组长通过 `/api/control load-policy` 关闭/开启；开关持久化到 `data_dir/load-policy.json`（文件优先）。

#### Scenario: 面板关闭红线

- **WHEN** 组长在控制台关闭「负载红线拦截」
- **THEN** load-policy.json 写入 disabled，后续红线请求放行；重启后保持关闭

#### Scenario: 重启恢复文件策略

- **WHEN** 组长曾关闭红线后重启服务
- **THEN** 服务按 load-policy.json 的 disabled 启动，不重置为默认开启