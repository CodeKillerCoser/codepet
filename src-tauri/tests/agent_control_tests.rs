use code_pet_lib::agent_control::{agent_statuses_for_paths, set_agent_enabled_at_path};
use code_pet_lib::agents::{agent_specs, AgentId};

#[test]
fn agent_statuses_report_all_enabled_after_installing_each_agent() {
    let temp = tempfile::tempdir().unwrap();
    let script_path = temp.path().join("code-pet-hook.mjs");

    let specs = agent_specs();
    let paths = specs
        .iter()
        .map(|spec| (spec.id, temp.path().join(format!("{}.config", spec.id.as_str()))))
        .collect::<Vec<_>>();

    for spec in &specs {
        let path = paths
            .iter()
            .find(|(id, _)| id == &spec.id)
            .map(|(_, path)| path.as_path())
            .unwrap();
        set_agent_enabled_at_path(spec, path, &script_path, true).unwrap();
    }

    let statuses = agent_statuses_for_paths(&specs, &paths, &script_path).unwrap();

    assert_eq!(statuses.len(), 4);
    assert!(statuses.iter().all(|status| status.enabled));
    assert!(statuses.iter().any(|status| status.id == AgentId::Codex));
    assert!(statuses.iter().any(|status| status.id == AgentId::Claude));
    assert!(statuses.iter().any(|status| status.id == AgentId::Qoder));
    assert!(statuses.iter().any(|status| status.id == AgentId::Cursor));
}
