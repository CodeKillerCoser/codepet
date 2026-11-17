# 原生进程诊断的实例选择

## 规则

诊断已运行 App 时，按实际可执行文件名和进程树确定实例，不以默认安装绝对路径判断是否运行。进程命令输出不得因列宽截断；采样只读取 executable，不读取可能含密钥或提示词的完整参数。

## 推荐做法

支持用户目录安装、重命名 app、macOS App Translocation 和中间 wrapper。先确定 Host，再检查 Provider 的祖先关系，原生子进程只从该 Provider 向下追踪。多个候选必须列出并支持显式选择 PID；采样开始后固定所选实例，不能在它退出后自动切到同名进程。没有发现进程时说明识别范围，不能据此判定业务接口不通。

## 证据与验证

2026-11-17 用户执行 recent 采样初版失败；初版硬编码 `/Applications/Code Pet.app/Contents/MacOS/code-pet`，本机默认位置通过未能覆盖其他安装方式。故障电脑实际路径尚未确认。

`scripts/diagnose_recent_live_test.py` 覆盖非默认位置、Translocation、显式 PID、wrapper、多个实例、无关进程和进程环；修改识别规则时必须保持这些隔离用例通过。原生验收还需确认三个角色能实际采样，不能仅以进程发现成功作为采样成功。
