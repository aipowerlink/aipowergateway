# 设计：模型配置

## D1：BackendEntry 与密钥形态

BackendEntry { provider, id, api_key, api_key_env, model, base_url }。backend_id() = id 或 provider。resolve_api_key() 顺序：直填 > api_key_env 环境变量 > AIPOWERLINK_{PROV}_API_KEY > AIPOWERLINK_API_KEY。面板只回传掩码（***尾4 / env:NAME），不回明文。

## D2：注册表热替换

BackendRegistry 内部 inner: RwLock<RegistryInner>；replace_all() 原子换入整批后端；route() 每次按需读锁（精确模型 > 最长前缀 > 单后端回退）。Provider::Custom 无前缀路由（仅精确模型），base_url+model 为必填（backend_from_entry 校验）。

## D3：配置存储

BackendStore::new(path, initial)：文件存在则读文件并覆盖 initial（文件优先）；缺失条目以 initial 补齐（不写盘）。upsert 按 backend_id；save() 写临时文件后原子改名。启动条目（cli env 解析）以 api_key_env 引用形式进入存储，密钥永不落明文。

## D4：管理 API 与面板

GET /api/backends 返回条目 + keySource/maskedKey + registered；POST 先经 backend_from_entry 预校验（400）再 upsert→save→rebuild（replace_all）；编辑时未提供任何密钥字段则继承原密钥。DELETE 移除并重建。Web 模型页对齐 DSH：卡片徽标 + 添加（官方/自定义）+ 编辑/删除 + 「已保存 xxx」状态行。
