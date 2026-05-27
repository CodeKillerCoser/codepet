use crate::agents::{AgentId, AgentSpec};
use serde_json::{json, Value};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

const HOOK_SCRIPT: &str = include_str!("../hooks/code-pet-hook.mjs");
const MANAGED_MARKER: &str = "CODE_PET_MANAGED=1";
const SCRIPT_NAME: &str = "code-pet-hook.mjs";

pub fn install_hook_script() -> io::Result<PathBuf> {
    let dir = app_support_dir().join("hooks");
    fs::create_dir_all(&dir)?;
    let script_path = dir.join(SCRIPT_NAME);
    fs::write(&script_path, HOOK_SCRIPT)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut permissions = fs::metadata(&script_path)?.permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&script_path, permissions)?;
    }
    Ok(script_path)
}

pub fn enable_agent_hook(
    spec: &AgentSpec,
    config_path: &Path,
    script_path: &Path,
) -> Result<(), Box<dyn std::error::Error>> {
    enable_json_hooks(spec, config_path, script_path)
}

pub fn disable_agent_hook(
    spec: &AgentSpec,
    config_path: &Path,
    script_path: &Path,
) -> Result<(), Box<dyn std::error::Error>> {
    disable_json_hooks(spec, config_path, script_path)
}

pub fn is_agent_hook_enabled(
    spec: &AgentSpec,
    config_path: &Path,
    script_path: &Path,
) -> Result<bool, Box<dyn std::error::Error>> {
    if !config_path.exists() {
        return Ok(false);
    }

    let root = read_json_config(config_path)?;
    Ok(spec.hook_events.iter().all(|event| {
        root.get("hooks")
            .and_then(|hooks| hooks.get(*event))
            .and_then(Value::as_array)
            .is_some_and(|entries| {
                entries.iter().any(|entry| is_managed_json_entry(entry, script_path))
            })
    }))
}

fn enable_json_hooks(
    spec: &AgentSpec,
    config_path: &Path,
    script_path: &Path,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut root = read_json_config(config_path)?;
    if !root.is_object() {
        root = json!({});
    }

    if root.get("hooks").and_then(Value::as_object).is_none() {
        root["hooks"] = json!({});
    }

    for event in spec.hook_events {
        if root["hooks"].get(*event).and_then(Value::as_array).is_none() {
            root["hooks"][*event] = json!([]);
        }

        let entries = root["hooks"][*event]
            .as_array_mut()
            .ok_or("hook event entry is not an array")?;
        let managed_entry = managed_json_entry(spec.id, script_path, event);
        if let Some(entry) = entries
            .iter_mut()
            .find(|entry| is_managed_json_entry(entry, script_path))
        {
            *entry = managed_entry;
        } else {
            entries.push(managed_entry);
        }
    }

    write_json_config(config_path, &root)?;
    Ok(())
}

fn disable_json_hooks(
    spec: &AgentSpec,
    config_path: &Path,
    script_path: &Path,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut root = read_json_config(config_path)?;
    if let Some(hooks) = root.get_mut("hooks").and_then(Value::as_object_mut) {
        for event in spec.hook_events {
            if let Some(entries) = hooks.get_mut(*event).and_then(Value::as_array_mut) {
                entries.retain(|entry| !is_managed_json_entry(entry, script_path));
            }
        }
    }
    write_json_config(config_path, &root)?;
    Ok(())
}

fn read_json_config(config_path: &Path) -> Result<Value, Box<dyn std::error::Error>> {
    if !config_path.exists() {
        return Ok(json!({}));
    }
    let text = fs::read_to_string(config_path)?;
    if text.trim().is_empty() {
        return Ok(json!({}));
    }
    Ok(serde_json::from_str(&text)?)
}

fn write_json_config(config_path: &Path, root: &Value) -> io::Result<()> {
    if let Some(parent) = config_path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(config_path, serde_json::to_string_pretty(root)?)
}

fn is_managed_json_entry(entry: &Value, script_path: &Path) -> bool {
    let script_path = script_path_to_str(script_path);
    entry
        .get("hooks")
        .and_then(Value::as_array)
        .is_some_and(|hooks| {
            hooks.iter().any(|hook| {
                hook.get("command")
                    .and_then(Value::as_str)
                    .is_some_and(|command| {
                        command.contains(MANAGED_MARKER)
                            || command.contains(SCRIPT_NAME)
                            || command.contains(&script_path)
                    })
            })
        })
}

fn managed_command(
    agent_id: &str,
    script_path: &Path,
    event: Option<&str>,
    forward: Option<&[String]>,
) -> String {
    let mut command = format!(
        "{MANAGED_MARKER} CODE_PET_AGENT={} node {}",
        shell_quote(agent_id),
        shell_quote(script_path_to_str(script_path).as_ref())
    );
    if let Some(event) = event {
        command.push_str(" --event ");
        command.push_str(&shell_quote(event));
    }
    if let Some(forward) = forward {
        if let Ok(encoded) = serde_json::to_string(forward) {
            command.push_str(" --forward ");
            command.push_str(&shell_quote(&encoded));
        }
    }
    command
}

fn managed_json_entry(agent_id: AgentId, script_path: &Path, event: &str) -> Value {
    let mut hook = json!({
        "type": "command",
        "command": managed_command(agent_id.as_str(), script_path, Some(event), None)
    });
    if agent_id == AgentId::Codex {
        hook["timeout_ms"] = json!(hook_timeout_ms(event));
    } else {
        hook["timeout"] = json!(hook_timeout_seconds(event));
    }

    if agent_id == AgentId::Codex {
        json!({ "hooks": [hook] })
    } else {
        json!({
            "matcher": "*",
            "hooks": [hook]
        })
    }
}

fn hook_timeout_seconds(event: &str) -> u64 {
    if event == "PermissionRequest" {
        600
    } else {
        5
    }
}

fn hook_timeout_ms(event: &str) -> u64 {
    hook_timeout_seconds(event) * 1000
}

fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

fn script_path_to_str(path: &Path) -> String {
    path.to_string_lossy().to_string()
}

fn app_support_dir() -> PathBuf {
    dirs::data_local_dir()
        .or_else(dirs::data_dir)
        .unwrap_or_else(|| dirs::home_dir().unwrap_or_else(|| PathBuf::from(".")))
        .join("code-pet")
}
