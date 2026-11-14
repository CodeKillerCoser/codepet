# Provider Registry 与完整卸载

## 规则

插件的安装、查询和卸载由 `providers/manager/registry.rs` 统一协调，PluginManager 的活动插件表是唯一来源；UI、IPC、Gateway 列表和心跳不得维护另一份独立注册状态。实例持久化表只保留稳定身份和历史关系。

## 适用场景

受管自定义 Provider 安装、热加载、卸载、重新安装、通知订阅，以及桌面和远程客户端的 Provider 清单同步。

## 反例

直接把安装目录移走、保留运行中注册，造成列表仍显示插件，后续移除却因目录不存在失败。只删卡片而保留进程或只停止进程而保留下一轮心跳中的条目，也不算卸载完成。

## 推荐做法

Registry 验证包归属，在该插件生命周期锁内等待进程退出和 RPC 关闭、关闭所有本地订阅，删除安装包、注销活动实例和插件、发布变更。订阅与卸载共用该插件 gate，阻止迟到订阅写回。文件删除移交 blocking worker，不持有全局活动表锁。错误必须返回，允许文件缺失的受管项继续注销；不得关闭其他 Provider 或清空历史库。下一次心跳从当前活动表重新生成 providers。

## 来源

[移除与重启队列问题](../40-runbooks/provider-removal-and-restart-queue-occupancy.md)及用户于 2026-11-13 确认的完整卸载要求。

## 验证方式

`cargo test --manifest-path crates/Cargo.toml -p codepet-host --test manager_gateway`。覆盖包目录不存在、PID 退出、订阅 receiver 关闭、下一轮心跳不含目标项、其他 Provider generation 和客户端连接保持、重新安装、优雅退出超时的强制清理。
