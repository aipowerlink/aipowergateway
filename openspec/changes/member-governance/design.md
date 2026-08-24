# 设计：成员可见性与黑名单

## D1：客户端真实 IP

server.rs 以 into_make_service_with_connect_info::<SocketAddr>() 装配，auth_token 以 ConnectInfo<SocketAddr> 提取对端地址（IPv4-mapped 转 v4）传给 issue/upsert。

## D2：网关标识

ShareServerConfig 增加 name（cli 传广播名 aipowerlink-share）；gateway_id = {name}:{port} 随成员登记写入 Member.gateway_id，/api/members 输出 gatewayId。

## D3：黑名单持久化 + 解禁

AuthService::new(ttl, Option<PathBuf>)：banned/banned_ips 从 banned.json 载入、变更即保存（仿 quota.rs）。新增 unban(member_id, ip) 与 is_member_banned(member_id)。api_control 增 unban 动作。

## D4：面板

成员详情显示 IP（已有）与网关 ID；列表对 banned 成员显示徽标，操作按钮在「拉黑/解禁」间切换（revoke=拉黑，unban=解禁）。