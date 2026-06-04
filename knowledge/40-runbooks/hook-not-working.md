# Hook Not Working

## Symptom

An enabled Agent does not produce task cards in the pet window.

## Evidence To Collect

- `git status --short` to separate local changes from runtime state.
- Agent enabled state from the app UI or `list_agents` behavior.
- Generated Agent config file, especially the managed command.
- Whether `code-pet-hook.mjs` exists under local app data.
- Collector health at `http://127.0.0.1:47621/health` while the app is running.
- `code-pet/logs/code-pet.log` for collector or startup errors.

## Checks

1. Confirm the Agent config path from `src-tauri/src/agent/registry.rs`.
2. Confirm managed entries include all expected hook events.
3. Confirm Windows quoting or Unix quoting did not corrupt the command.
4. Trigger a small Agent task and inspect whether `/hook` receives data.
5. If app was down during the task, inspect `~/.code-pet/spool/events.jsonl`.

## Validation After Fix

- `cargo test --manifest-path src-tauri/Cargo.toml hook_config_tests hook_script_tests`
- Manual task from the affected Agent.
