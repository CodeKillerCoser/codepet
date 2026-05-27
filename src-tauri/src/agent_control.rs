use crate::agents::{agent_specs, resolve_agent_config_path, AgentId, AgentSpec, AgentView};
use crate::hooks::{
    disable_agent_hook, enable_agent_hook, install_hook_script, is_agent_hook_enabled,
};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentStatus {
    pub id: AgentId,
    pub name: String,
    pub enabled: bool,
    pub config_path: String,
}

pub fn list_agent_views() -> Result<Vec<AgentView>, Box<dyn std::error::Error>> {
    let script_path = install_hook_script()?;
    agent_specs()
        .into_iter()
        .map(|spec| {
            let config_path = resolve_agent_config_path(spec.id);
            let enabled = is_agent_hook_enabled(&spec, &config_path, &script_path)?;
            Ok(AgentView::from_spec(spec, config_path, enabled))
        })
        .collect()
}

pub fn set_agent_enabled(
    agent_id: AgentId,
    enabled: bool,
) -> Result<Vec<AgentView>, Box<dyn std::error::Error>> {
    let script_path = install_hook_script()?;
    let spec = agent_specs()
        .into_iter()
        .find(|agent| agent.id == agent_id)
        .ok_or("unknown agent id")?;
    let config_path = resolve_agent_config_path(agent_id);
    set_agent_enabled_at_path(&spec, &config_path, &script_path, enabled)?;
    list_agent_views()
}

pub fn set_all_agents_enabled(
    enabled: bool,
) -> Result<Vec<AgentStatus>, Box<dyn std::error::Error>> {
    let script_path = install_hook_script()?;
    let specs = agent_specs();
    for spec in &specs {
        let config_path = resolve_agent_config_path(spec.id);
        set_agent_enabled_at_path(spec, &config_path, &script_path, enabled)?;
    }
    let paths = specs
        .iter()
        .map(|spec| (spec.id, resolve_agent_config_path(spec.id)))
        .collect::<Vec<_>>();
    agent_statuses_for_paths(&specs, &paths, &script_path)
}

pub fn current_agent_statuses() -> Result<Vec<AgentStatus>, Box<dyn std::error::Error>> {
    let script_path = install_hook_script()?;
    let specs = agent_specs();
    let paths = specs
        .iter()
        .map(|spec| (spec.id, resolve_agent_config_path(spec.id)))
        .collect::<Vec<_>>();
    agent_statuses_for_paths(&specs, &paths, &script_path)
}

pub fn set_agent_enabled_at_path(
    spec: &AgentSpec,
    config_path: &Path,
    script_path: &Path,
    enabled: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    if enabled {
        enable_agent_hook(spec, config_path, script_path)
    } else {
        disable_agent_hook(spec, config_path, script_path)
    }
}

pub fn agent_statuses_for_paths(
    specs: &[AgentSpec],
    paths: &[(AgentId, PathBuf)],
    script_path: &Path,
) -> Result<Vec<AgentStatus>, Box<dyn std::error::Error>> {
    specs
        .iter()
        .map(|spec| {
            let config_path = paths
                .iter()
                .find(|(id, _)| id == &spec.id)
                .map(|(_, path)| path.clone())
                .unwrap_or_else(|| resolve_agent_config_path(spec.id));
            let enabled = is_agent_hook_enabled(spec, &config_path, script_path)?;
            Ok(AgentStatus {
                id: spec.id,
                name: spec.name.to_string(),
                enabled,
                config_path: config_path.display().to_string(),
            })
        })
        .collect()
}
