#!/bin/zsh
set -euo pipefail
task_dir="$(cd -- "$(dirname -- "$0")" && pwd)"
task_app="$task_dir/../../artifacts/window-drawer-demo/Code Pet Drawer.app"
if [[ ! -x "$task_app/Contents/MacOS/CodePetDrawer" ]]; then
    "$task_dir/build.command"
fi
open "$task_app"
