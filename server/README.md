# Code Pet Server

独立的 Python 信令服务项目，使用 Python 3.12、aiohttp 和 SQLite，负责设备登记、扫码配对信箱、WebRTC 信令和临时 TURN 凭据。手机/PC 名称、系统、Provider、对话与文件通过已建立的业务通道交换。

## 项目结构

```text
server/
  server.py                 # HTTP 服务入口与信令接口
  registration.py           # 安装身份登记、签名与授权检查
  policy.py                 # 配额、内存预算与过期清理
  config_source.py           # 配置读取与模板渲染
  deploy.py                  # 首次安装
  deploy_management.py       # 现有 VPS 更新、备份、失败恢复
  probe_turn_*.py            # 有界 TURN 分配与隔离流量验证
  requirements.txt
  Makefile
  config/                    # 所有服务配置平铺在这里
  tests/                     # 本项目测试
```

保留现有 Python 模块边界；仅配置和测试使用目录，不按 nginx/coturn/systemd 再建子目录。旧 `deploy_registration.py`、`deploy_turn_capacity.py`、`deploy_diagnostics.py` 是调用统一维护部署的兼容入口。

## 配置

以下文件全部平铺在 `config/`：

| 文件 | 用途 | 线上位置 |
| --- | --- | --- |
| `deployment.json` | 公开 IP、端口、限流、TURN 配额和出口预算 | 生成服务配置及防火墙端口 |
| `signaling.json` | 登记、可信代理、信箱容量与请求限额 | `/etc/codepet-signal/config.json` 非敏感部分 |
| `nginx.conf.tpl` | HTTPS、反向代理、来源限流 | `/etc/nginx/conf.d/codepet-signal.conf` |
| `turnserver.conf.tpl` | TURN 认证、转发、私网保护与日志 | `/etc/coturn/turnserver.conf` |
| `turn-budget.nft.tpl` | coturn 实际出口总量保护 | `/etc/codepet-signal/turn-budget.nft` |
| `codepet-signal.service` | 信令服务启动 | `/etc/systemd/system/` |
| `codepet-turn-budget.service`、`coturn-budget.conf` | 出口保护及 coturn 启动依赖 | `/etc/systemd/system/` 及 `coturn.service.d/` |
| `codepet-cert-renew.service`、`codepet-cert-renew.timer` | 定期证书续期 | `/etc/systemd/system/` |
| `codepet-reload.sh.tpl` | 续期后的证书复制与 reload | `/etc/letsencrypt/renewal-hooks/deploy/codepet-reload.sh` |
| `coturn.logrotate` | 日志轮转 | `/etc/logrotate.d/coturn` |

部署直接读取这些文件。模板只替换 `${UPPERCASE_NAME}`，保留 nginx 的 `$host`、`$remote_addr`。真实 TURN 密钥首次在服务器生成，后续复用；证书私钥、设备 token、SQLite、备份和含密钥的渲染结果不入库。
已有安装的数据库路径、`publicIp`、`turnSecret` 及未管理字段保留，其他管理参数由仓库配置应用。换地址/数据库路径需要显式迁移。
增量更新保留现有下载 location；新安装预留 `/etc/nginx/codepet-downloads/*.conf`。独立 VPN/xray 不由本项目管理。

## 本地安装与测试

在本目录运行，或从仓库根使用 `make -C server …`：

```sh
make setup
make test
```

使用已有环境时执行 `make test VENV_PYTHON=/path/to/python`。等价命令是在本目录运行 `python -m unittest discover -s tests -p 'test_*.py' -q`。
运行服务时通过 `CODEPET_SIGNAL_CONFIG=/path/to/private/config.json python server.py` 指定准备好的私有配置；默认读取 `/etc/codepet-signal/config.json`。

测试覆盖登记幂等与权限隔离、反代来源伪造、独立限流、Retry-After、480 个并存信箱、总字节预算、取消/过期回收、重复同步不写数据库，以及配置模板和密钥保留。480 信箱是准入回归，不是 480 条真实中继吞吐压测。

## VPS 部署

复制整个项目目录到 staging，保留当前结构。已有 VPS 以 root 运行：

```sh
/opt/codepet-venv/bin/python /path/to/staging/server/deploy_management.py /path/to/staging/server
```

部署先运行测试，备份代码、配置和 SQLite，检查 nginx/nftables/logrotate 后应用；短暂重启信令和 TURN，nginx reload。设备身份与配对授权保留；失败恢复代码/配置，不覆盖实时数据库，避免丢失部署期间的登记。备份在 `/opt/codepet-signal/backups/management-<UTC>/`。
首次安装使用 `deploy.py`，需要先准备 Python 虚拟环境及依赖、nginx、coturn、firewalld 和有效证书。两个入口共用配置。线上代码仍在 `/opt/codepet-signal/`，SQLite 仍在 `/var/lib/codepet-signal/signal.db`。

`probe_turn_budget.py` 在独立 Linux 网络命名空间验证真实限流和 UID 隔离；`probe_turn_capacity.py --allocations 48 --per-identity 16` 申请有限的真实 TURN 分配，并确认 lifetime=0 释放。均须在授权 VPS 上运行。
出口保护是超额丢包保护，不是平滑整形或账号带宽保证。1,024 总分配只是配置门槛，不能据此推算用户上限。手机切网、VPN、后台挂起及原生关闭仍需 APP 端验证。

## 登记与扫码

两台 PC、三部手机成功登记，通常有五条安装身份记录；重复登记幂等，登记不授予 PC 访问权。二维码绑定目标 PC ID、信令地址、临时邀请和身份校验信息。手机通过 LAN / VPS 并行提交同一份申请，PC 确认后签发访问凭据，后续按这份授权交换信令。三部手机都连接两台 PC，需要六组配对授权。
落盘表为 `device_registrations`、`hosts`、`clients`，临时握手和邀请在内存。原始 schema 在 `registration.py` / `server.py`；排查证据在仓库 `knowledge/40-runbooks/vps-multiuser-turn-capacity-audit.md`。
