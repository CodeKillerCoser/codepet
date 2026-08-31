# Remote 客户端身份与 credential 投影规约

## 规则

Remote 设备列表必须以协议 `clientId` 作为逻辑设备身份。credential 是授权记录，不得直接一条 credential 渲染成一台设备；同一 client 的在线数和撤销范围都必须覆盖其全部 credential。

## 适用场景

适用于 `RemoteCredentialStore` 到 Tauri `list_remote_clients` 的投影、连接页设备状态，以及本机发起的设备访问撤销。Gateway pairing、TLS/WSS 鉴权仍可按 credential 工作。

## 反例

同一手机重新配对后会留下多个有效 credential。若逐 credential 展示，UI 会同时出现同名“旧记录离线”和“新记录在线”；若只撤销代表 credential，旧 bearer 仍可能继续授权。按 descriptor 名称合并又会误伤同名不同客户端。

## 推荐做法

- 按完整 `clientId` 分组，不按名称、系统或 credential id 猜设备身份。
- 选择最新 active credential 作为动作代表；`createdAt` 取首次配对，`lastSeenAt` 取真实连接记录最大值，在线 session 数求和。
- 设备撤销先按 client 原子撤销全部 active credential，再逐 credential 断开残留 session。
- 不为此修改 pairing/WSS wire、bearer 校验或消息 Schema。

## 来源

2026-08-31 Host 现场发现同一 Remote 安装身份存在两条 active credential：一条旧记录显示离线，另一条承载当前 WSS session。三方 clientId 与 TLS identity 均一致，证明问题位于逻辑设备投影而非握手绑定。

## 验证方式

Tauri 定向测试必须覆盖同 client 多 credential 聚合和 online count 求和、同名不同 client 不合并、client 级撤销使该 client 全部 bearer 失效且不影响其他 client。
