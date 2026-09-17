# 远程设备撤销与删除

## 规则

撤销访问保留记录，删除设备记录必须先确认该客户端的全部凭据已撤销并关闭残留连接。删除以稳定 clientId 为范围，不按名称匹配，也不能只删除列表代表的单条 credential。

## 适用场景

Host 连接接入页设备管理、Tauri 设备命令、credential store，以及 Remote 忘记设备流程。

## 反例

只隐藏设备行会在重启后恢复；只删代表 credential 会留下旧授权；检查撤销状态后无锁删除可能误删并发配对的新凭据。Remote 等待不可达 Host 撤销后才更新界面，会让本地忘记操作看似无效。

## 推荐做法

- `frontend/lib/RemoteDeviceList.svelte` 对已撤销设备提供删除入口，操作失败保留记录并展示错误。
- `src-tauri/src/runtime_gateway/remote_access.rs` 校验整组撤销并等待残留连接关闭，超时则保留记录以便重试。
- `crates/codepet-host/src/remote/access.rs` 在同一存储锁内复核授权状态、构造副本、持久化后再替换内存。重新配对的有效记录使删除失败。
- 手机端忘记删除本地配对信息和安全凭据，停止并释放会话，立即刷新连接页；远端撤销尽力执行。Host 不可达时不承诺远端授权已撤销，更不承诺删除 Host 的历史记录。

## 来源

2026-09-16 用户反馈设备列表持续增长、手机忘记无反馈。旧 Host 只有 revoke 命令，设置页持有设备快照且等待远端请求，源码可复现这些行为；具体引入提交未确认。功能链路见 `knowledge/10-architecture/remote-access-tauri-runtime.md`。

## 验证方式

- Host `delete_revoked_client_removes_all_records_and_preserves_other_clients`：覆盖有效授权拒绝删除、同 client 多记录、并发重新配对状态、其他客户端隔离、持久化重载和旧 bearer 失效。
- Tauri `pairing_json_is_active_only_and_remote_views_stay_secret_safe`：覆盖命令层拒绝未撤销设备、撤销后删除及列表清空。
- Remote `pair_device_screen_test.dart`、`codepet_remote_app_test.dart`：覆盖取消、失败提示、成功返回、列表刷新及注册表清理。
- 人工补验 Windows Host 与 Android 真机：断网忘记、Host 撤销后删除、重启不恢复、键盘焦点和窄屏布局。浏览器预览工具本次超时，不能把编译或源码测试当成视觉验收。
