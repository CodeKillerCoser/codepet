use crate::agents::{agent_specs, resolve_agent_config_path, AgentId, AgentSpec, AgentView};
use crate::hooks::{
    disable_agent_hook, enable_agent_hook_events, install_hook_script,
    is_agent_hook_enabled, is_agent_hook_enabled_for_events,
};
use crate::settings::{load_app_settings, save_app_settings, AgentPreferenceSettings, AppSettings};
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
    let settings = load_app_settings()?;
    agent_specs()
        .into_iter()
        .map(|spec| {
            let config_path = resolve_agent_config_path(spec.id);
            let selected_hook_events = selected_hook_events_for_spec(&settings, &spec);
            let selected_event_refs = hook_event_refs(&selected_hook_events);
            let enabled = is_agent_hook_enabled_for_events(
                &spec,
                &config_path,
                &script_path,
                &selected_event_refs,
            )?;
            Ok(AgentView::from_spec(
                spec,
                config_path,
                enabled,
                selected_hook_events,
            ))
        })
        .collect()
}

pub fn set_agent_enabled(
    agent_id: AgentId,
    enabled: bool,
) -> Result<Vec<AgentView>, Box<dyn std::error::Error>> {
    let script_path = install_hook_script()?;
    let settings = load_app_settings()?;
    let spec = agent_specs()
        .into_iter()
        .find(|agent| agent.id == agent_id)
        .ok_or("unknown agent id")?;
    let config_path = resolve_agent_config_path(agent_id);
    let selected_hook_events = selected_hook_events_for_spec(&settings, &spec);
    set_agent_enabled_events_at_path(
        &spec,
        &config_path,
        &script_path,
        &selected_hook_events,
        enabled,
    )?;
    list_agent_views()
}

pub fn set_agent_hook_events(
    agent_id: AgentId,
    hook_events: Vec<String>,
) -> Result<Vec<AgentView>, Box<dyn std::error::Error>> {
    let script_path = install_hook_script()?;
    let spec = agent_specs()
        .into_iter()
        .find(|agent| agent.id == agent_id)
        .ok_or("unknown agent id")?;
    let config_path = resolve_agent_config_path(agent_id);
    let mut settings = load_app_settings()?;
    let previous_hook_events = selected_hook_events_for_spec(&settings, &spec);
    let previous_event_refs = hook_event_refs(&previous_hook_events);
    let was_enabled = is_agent_hook_enabled_for_events(
        &spec,
        &config_path,
        &script_path,
        &previous_event_refs,
    )?;
    let selected_hook_events = normalize_hook_events_for_spec(&spec, &hook_events);

    settings
        .agents
        .by_agent
        .entry(agent_id)
        .or_insert_with(AgentPreferenceSettings::default)
        .hook_events = selected_hook_events.clone();
    save_app_settings(&settings)?;

    if was_enabled {
        set_agent_enabled_events_at_path(
            &spec,
            &config_path,
            &script_path,
            &selected_hook_events,
            true,
        )?;
    }

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
    let hook_events = spec
        .hook_events
        .iter()
        .map(|event| event.to_string())
        .collect::<Vec<_>>();
    set_agent_enabled_events_at_path(spec, config_path, script_path, &hook_events, enabled)
}

pub fn set_agent_enabled_events_at_path(
    spec: &AgentSpec,
    config_path: &Path,
    script_path: &Path,
    hook_events: &[String],
    enabled: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    if enabled {
        enable_agent_hook_events(spec, config_path, script_path, hook_events)
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

fn selected_hook_events_for_spec(settings: &AppSettings, spec: &AgentSpec) -> Vec<String> {
    settings
        .agents
        .by_agent
        .get(&spec.id)
        .map(|preference| normalize_hook_events_for_spec(spec, &preference.hook_events))
        .unwrap_or_else(|| spec.hook_events.iter().map(|event| event.to_string()).collect())
}

fn normalize_hook_events_for_spec(spec: &AgentSpec, hook_events: &[String]) -> Vec<String> {
    if hook_events.is_empty() {
        return spec.hook_events.iter().map(|event| event.to_string()).collect();
    }

    let selected = spec
        .hook_events
        .iter()
        .copied()
        .filter(|event| hook_events.iter().any(|selected| selected == *event))
        .map(|event| event.to_string())
        .collect::<Vec<_>>();
    if selected.is_empty() {
        spec.hook_events.iter().map(|event| event.to_string()).collect()
    } else {
        selected
    }
}

fn hook_event_refs(hook_events: &[String]) -> Vec<&str> {
    hook_events.iter().map(String::as_str).collect()
}
