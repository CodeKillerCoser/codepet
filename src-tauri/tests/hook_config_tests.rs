use code_pet_lib::agents::{agent_specs, AgentId};
use code_pet_lib::hooks::{disable_agent_hook, enable_agent_hook, is_agent_hook_enabled};
use serde_json::json;

#[test]
fn enable_json_agent_hook_is_idempotent_and_keeps_existing_hooks() {
    let temp = tempfile::tempdir().unwrap();
    let script_path = temp.path().join("code-pet-hook.mjs");
    let settings_path = temp.path().join("settings.json");
    let original = json!({
        "hooks": {
            "PreToolUse": [
                {
                    "matcher": "Bash",
                    "hooks": [
                        { "type": "command", "command": "bash /existing/hook.sh", "timeout": 15 }
                    ]
                }
            ]
        }
    });
    std::fs::write(&settings_path, serde_json::to_string_pretty(&original).unwrap()).unwrap();

    let spec = agent_specs()
        .into_iter()
        .find(|agent| agent.id == AgentId::Claude)
        .unwrap();

    enable_agent_hook(&spec, &settings_path, &script_path).unwrap();
    enable_agent_hook(&spec, &settings_path, &script_path).unwrap();

    let updated: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&settings_path).unwrap()).unwrap();
    let pre_tool_use = updated["hooks"]["PreToolUse"].as_array().unwrap();

    assert!(pre_tool_use.iter().any(|entry| {
        entry["hooks"][0]["command"] == "bash /existing/hook.sh"
    }));
    assert_eq!(
        pre_tool_use
            .iter()
            .filter(|entry| entry["hooks"][0]["command"]
                .as_str()
                .is_some_and(|command| command.contains("code-pet-hook.mjs")))
            .count(),
        1
    );
    assert!(is_agent_hook_enabled(&spec, &settings_path, &script_path).unwrap());
}

#[test]
fn disable_json_agent_hook_removes_only_managed_entries() {
    let temp = tempfile::tempdir().unwrap();
    let script_path = temp.path().join("code-pet-hook.mjs");
    let settings_path = temp.path().join("settings.json");
    let spec = agent_specs()
        .into_iter()
        .find(|agent| agent.id == AgentId::Qoder)
        .unwrap();

    enable_agent_hook(&spec, &settings_path, &script_path).unwrap();
    disable_agent_hook(&spec, &settings_path, &script_path).unwrap();

    let updated: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&settings_path).unwrap()).unwrap();
    assert!(updated["hooks"]["PreToolUse"]
        .as_array()
        .is_none_or(|entries| entries.is_empty()));
    assert!(!is_agent_hook_enabled(&spec, &settings_path, &script_path).unwrap());
}

#[test]
fn disabling_missing_agent_config_does_not_create_file() {
    let temp = tempfile::tempdir().unwrap();
    let script_path = temp.path().join("code-pet-hook.mjs");
    let settings_path = temp.path().join("missing-settings.json");
    let spec = agent_specs()
        .into_iter()
        .find(|agent| agent.id == AgentId::Claude)
        .unwrap();

    disable_agent_hook(&spec, &settings_path, &script_path).unwrap();

    assert!(!settings_path.exists());
}

#[test]
fn json_agent_hook_is_not_enabled_when_only_one_event_is_managed() {
    let temp = tempfile::tempdir().unwrap();
    let script_path = temp.path().join("code-pet-hook.mjs");
    let settings_path = temp.path().join("settings.json");
    std::fs::write(
        &settings_path,
        serde_json::to_string_pretty(&json!({
            "hooks": {
                "PreToolUse": [
                    {
                        "matcher": "*",
                        "hooks": [
                            {
                                "type": "command",
                                "command": "CODE_PET_MANAGED=1 CODE_PET_AGENT='claude' node '/tmp/code-pet-hook.mjs' --event 'PreToolUse'",
                                "timeout": 5
                            }
                        ]
                    }
                ]
            }
        }))
        .unwrap(),
    )
    .unwrap();

    let spec = agent_specs()
        .into_iter()
        .find(|agent| agent.id == AgentId::Claude)
        .unwrap();

    assert!(!is_agent_hook_enabled(&spec, &settings_path, &script_path).unwrap());
}

#[test]
fn enable_codex_json_hooks_preserves_existing_config_and_is_idempotent() {
    let temp = tempfile::tempdir().unwrap();
    let script_path = temp.path().join("code-pet-hook.mjs");
    let config_path = temp.path().join("hooks.json");
    std::fs::write(
        &config_path,
        serde_json::to_string_pretty(&json!({
            "hooks": {
                "PreToolUse": [
                    {
                        "matcher": "Bash",
                        "hooks": [
                            { "type": "command", "command": "~/.codex/hooks/guard-tool.sh", "timeout_ms": 10000 }
                        ]
                    }
                ]
            }
        }))
        .unwrap(),
    )
    .unwrap();

    let spec = agent_specs()
        .into_iter()
        .find(|agent| agent.id == AgentId::Codex)
        .unwrap();

    enable_agent_hook(&spec, &config_path, &script_path).unwrap();
    enable_agent_hook(&spec, &config_path, &script_path).unwrap();

    let updated: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&config_path).unwrap()).unwrap();
    assert!(updated["hooks"]["PreToolUse"]
        .as_array()
        .unwrap()
        .iter()
        .any(|entry| entry["hooks"][0]["command"] == "~/.codex/hooks/guard-tool.sh"));
    assert_eq!(
        serde_json::to_string(&updated)
            .unwrap()
            .matches("code-pet-hook.mjs")
            .count(),
        spec.hook_events.len()
    );
    assert!(is_agent_hook_enabled(&spec, &config_path, &script_path).unwrap());
}

#[test]
fn codex_json_hooks_use_timeout_ms_schema() {
    let temp = tempfile::tempdir().unwrap();
    let script_path = temp.path().join("code-pet-hook.mjs");
    let config_path = temp.path().join("hooks.json");
    let spec = agent_specs()
        .into_iter()
        .find(|agent| agent.id == AgentId::Codex)
        .unwrap();

    enable_agent_hook(&spec, &config_path, &script_path).unwrap();

    let updated: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&config_path).unwrap()).unwrap();
    let managed_hook = &updated["hooks"]["UserPromptSubmit"][0]["hooks"][0];
    assert_eq!(managed_hook["timeout_ms"], 5000);
    assert!(managed_hook.get("timeout").is_none());
    assert!(updated["hooks"]["UserPromptSubmit"][0].get("matcher").is_none());
}

#[test]
fn cursor_json_hooks_use_flat_hooks_schema() {
    let temp = tempfile::tempdir().unwrap();
    let script_path = temp.path().join("code-pet-hook.mjs");
    let config_path = temp.path().join("hooks.json");
    std::fs::write(
        &config_path,
        serde_json::to_string_pretty(&json!({
            "version": 1,
            "hooks": {
                "beforeShellExecution": [
                    { "command": "~/.cursor/hooks/guard-shell.sh", "timeout": 10 }
                ]
            }
        }))
        .unwrap(),
    )
    .unwrap();

    let spec = agent_specs()
        .into_iter()
        .find(|agent| agent.id == AgentId::Cursor)
        .unwrap();

    enable_agent_hook(&spec, &config_path, &script_path).unwrap();
    enable_agent_hook(&spec, &config_path, &script_path).unwrap();

    let updated: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&config_path).unwrap()).unwrap();
    assert_eq!(updated["version"], 1);
    assert!(updated["hooks"]["beforeShellExecution"]
        .as_array()
        .unwrap()
        .iter()
        .any(|entry| entry["command"] == "~/.cursor/hooks/guard-shell.sh"));
    assert_eq!(
        updated["hooks"]["beforeShellExecution"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|entry| entry["command"]
                .as_str()
                .is_some_and(|command| command.contains("--agent 'cursor'")))
            .count(),
        1
    );
    assert_eq!(updated["hooks"]["sessionStart"][0]["timeout"], 5);
    assert!(updated["hooks"]["sessionStart"][0].get("hooks").is_none());
    assert!(is_agent_hook_enabled(&spec, &config_path, &script_path).unwrap());
}

#[test]
fn codex_json_hooks_upgrade_existing_managed_timeout_field() {
    let temp = tempfile::tempdir().unwrap();
    let script_path = temp.path().join("code-pet-hook.mjs");
    let config_path = temp.path().join("hooks.json");
    let command = format!(
        "CODE_PET_MANAGED=1 CODE_PET_AGENT='codex' node '{}' --event 'UserPromptSubmit'",
        script_path.display()
    );
    std::fs::write(
        &config_path,
        serde_json::to_string_pretty(&json!({
            "hooks": {
                "UserPromptSubmit": [
                    {
                        "matcher": "*",
                        "hooks": [
                            { "type": "command", "command": command, "timeout": 5 }
                        ]
                    }
                ]
            }
        }))
        .unwrap(),
    )
    .unwrap();

    let spec = agent_specs()
        .into_iter()
        .find(|agent| agent.id == AgentId::Codex)
        .unwrap();

    enable_agent_hook(&spec, &config_path, &script_path).unwrap();

    let updated: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&config_path).unwrap()).unwrap();
    let entries = updated["hooks"]["UserPromptSubmit"].as_array().unwrap();
    assert_eq!(entries.len(), 1);
    let managed_hook = &entries[0]["hooks"][0];
    assert_eq!(managed_hook["timeout_ms"], 5000);
    assert!(managed_hook.get("timeout").is_none());
}

#[test]
fn permission_request_hooks_are_installed_with_long_timeout() {
    let temp = tempfile::tempdir().unwrap();
    let script_path = temp.path().join("code-pet-hook.mjs");

    for agent_id in [AgentId::Claude, AgentId::Qoder] {
        let config_path = temp.path().join(format!("{}-settings.json", agent_id.as_str()));
        let spec = agent_specs()
            .into_iter()
            .find(|agent| agent.id == agent_id)
            .unwrap();

        enable_agent_hook(&spec, &config_path, &script_path).unwrap();

        let updated: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&config_path).unwrap()).unwrap();
        let managed_hook = &updated["hooks"]["PermissionRequest"][0]["hooks"][0];
        assert_eq!(managed_hook["timeout"], 600);
        assert!(
            managed_hook["command"]
                .as_str()
                .is_some_and(|command| command.contains("--event 'PermissionRequest'"))
        );
        assert!(
            managed_hook["command"]
                .as_str()
                .is_some_and(|command| !command.contains("CODE_PET_AGENT="))
        );
    }
}
