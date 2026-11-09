#!/bin/zsh
set -u
script_dir="${0:A:h}"
/usr/bin/swift "$script_dir/diagnose_notch_macos.swift" "$@"
result=$?
if [[ -t 0 ]]; then
  printf '\n诊断结束（退出码 %s）。按回车关闭窗口。' "$result"
  read -r
fi
exit "$result"
