# logrotate 将备份识别为重复配置

## 现象

VPS 的 logrotate 任务失败，coturn 日志无法按正常计划轮转。

## 证据

2026-11-06 排查发现 `/etc/logrotate.d/` 同时存在 `coturn` 和两份旧备份：
`coturn.before-diagnostics-20260909T174612Z`、`coturn.before-diagnostics-20260909T174951Z`。
全局 logrotate 校验报告 `/var/log/coturn/turnserver.log` 重复配置。
仓库 `services/signaling/deploy_diagnostics.py` 将备份放在原配置旁，且只校验单个新文件，未检查 include 后的配置全集。

## 根因

备份文件名未被 logrotate 忽略，include 目录将备份当成生效配置再次解析。单文件校验不会发现这一冲突。

## 引入历史

备份文件时间为 2026-09-09。具体引入提交未确认。

## 修复方案

两份备份移至 `/opt/codepet-signal/backups/logrotate-fix-20261105T171227Z/`，保留恢复能力。源脚本改为备份至 `/opt/codepet-signal/backups/diagnostics-<UTC timestamp>/`，保留原绝对路径对应的目录结构；同名部署目录已存在时拒绝覆盖。
部署后执行 `logrotate --debug /etc/logrotate.conf` 检查全部配置。

## 涉及模块

- VPS `/etc/logrotate.d/`：移出重复配置，恢复服务轮转。
- `deploy_diagnostics.py`：修复再次部署后复发的来源。
- `test_deploy_diagnostics.py`：验证备份不进入 include 目录、内容保留以及已有备份不覆盖。

## 验证结果

全量 dry run 成功；`systemctl start logrotate.service` 后 Result=success、ExecMainStatus=0。
修复轮转未重启信令、coturn、nginx、xray，四者均 active。备份脚本测试本机与 VPS 均通过。修复版部署脚本已同步到 VPS。

## 回归防线

备份必须放在服务 include 目录外。变更配置后校验服务的完整入口，不能只校验被修改文件。恢复前先检查备份路径和对应目标，禁止直接将整个备份目录拷贝进 include。

## 规约候选

将此约束用于以后信令、coturn、nginx 和轮转脚本的 review；当前回归测试和本 runbook 已覆盖，不新增中心化索引。

## 未知项

尚未等待次日定时触发；已执行同一 systemd 任务验证当前配置。
