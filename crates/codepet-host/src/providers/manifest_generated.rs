// Generated from protocol/v1/provider; do not edit.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all="camelCase",deny_unknown_fields)]
pub struct PluginInstanceConfig {
#[serde(default,skip_serializing_if="Option::is_none")]
pub instance_id: Option<codepet_provider_sdk::ProviderInstanceId>,
pub instance_kind: String,
pub display_name: String,
#[serde(default)]
pub settings: codepet_provider_sdk::JsonObject,
#[serde(default="super::default_true")]
pub enabled: bool,
}
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all="camelCase",deny_unknown_fields)]
pub struct PluginDescriptor {
pub plugin_id: codepet_provider_sdk::ProviderPluginId,
pub display_name: String,
#[serde(default,skip_serializing_if="Option::is_none")]
pub icon: Option<String>,
#[serde(alias="command")]
pub executable: std::path::PathBuf,
#[serde(default)]
pub args: Vec<String>,
#[serde(default)]
pub env: std::collections::BTreeMap<String,String>,
#[serde(default="super::default_true")]
pub enabled: bool,
#[serde(default)]
pub instances: Vec<PluginInstanceConfig>,
}
#[derive(Clone, Debug, serde::Deserialize)]
#[serde(rename_all="camelCase",deny_unknown_fields)]
pub(super) struct PluginManifest { pub manifest_version:u32, #[serde(rename="$schema",default)] pub _schema:Option<String>, #[serde(default)] pub transport:Option<String>, #[serde(flatten)] pub descriptor:PluginDescriptor }
pub const PLUGIN_MANIFEST_FILE_NAME: &str = "codepet-provider.json";
pub const PLUGIN_MANIFEST_VERSION: u32 = 2;
pub fn validate_v2_manifest(value: &serde_json::Value) -> Result<(), String> { { let v0 = value;
if !v0.is_object() { return Err(format!("Invalid plugin manifest: {}", "manifest")); }
if v0.get("manifestVersion").is_none() { return Err(format!("Invalid plugin manifest: {}", "manifest")); }
if v0.get("pluginId").is_none() { return Err(format!("Invalid plugin manifest: {}", "manifest")); }
if v0.get("displayName").is_none() { return Err(format!("Invalid plugin manifest: {}", "manifest")); }
if v0.get("command").is_none() { return Err(format!("Invalid plugin manifest: {}", "manifest")); }
if v0.get("transport").is_none() { return Err(format!("Invalid plugin manifest: {}", "manifest")); }
if let Some(child)=v0.get("$schema") { let v1 = child;
if !v1.is_string() { return Err(format!("Invalid plugin manifest: {}", "manifest.$schema")); }
}
if let Some(child)=v0.get("manifestVersion") { let v2 = child;
if v2 != &serde_json::json!(2) { return Err(format!("Invalid plugin manifest: {}", "manifest.manifestVersion")); }
}
if let Some(child)=v0.get("pluginId") { let v3 = child;
if !v3.is_string() { return Err(format!("Invalid plugin manifest: {}", "manifest.pluginId")); }
{let text=v3.as_str().unwrap_or_default().as_bytes(); if text.is_empty() || !text[0].is_ascii_alphanumeric() || !text.iter().all(|c|c.is_ascii_alphanumeric() || b"._-".contains(c)) { return Err(format!("Invalid plugin manifest: {}", "manifest.pluginId")); }}
}
if let Some(child)=v0.get("displayName") { let v4 = child;
if !v4.is_string() { return Err(format!("Invalid plugin manifest: {}", "manifest.displayName")); }
if v4.as_str().unwrap_or_default().chars().count()<1 { return Err(format!("Invalid plugin manifest: {}", "manifest.displayName")); }
}
if let Some(child)=v0.get("icon") { let v5 = child;
if !v5.is_string() { return Err(format!("Invalid plugin manifest: {}", "manifest.icon")); }
if v5.as_str().unwrap_or_default().chars().count()<1 { return Err(format!("Invalid plugin manifest: {}", "manifest.icon")); }
}
if let Some(child)=v0.get("command") { let v6 = child;
if !v6.is_string() { return Err(format!("Invalid plugin manifest: {}", "manifest.command")); }
if v6.as_str().unwrap_or_default().chars().count()<1 { return Err(format!("Invalid plugin manifest: {}", "manifest.command")); }
}
if let Some(child)=v0.get("args") { let v7 = child;
if !v7.is_array() { return Err(format!("Invalid plugin manifest: {}", "manifest.args")); }
for child in v7.as_array().unwrap() { let v8 = child;
if !v8.is_string() { return Err(format!("Invalid plugin manifest: {}", "manifest.args[]")); }
}
}
if let Some(child)=v0.get("env") { let v9 = child;
if !v9.is_object() { return Err(format!("Invalid plugin manifest: {}", "manifest.env")); }
for child in v9.as_object().unwrap().values() { let v10 = child;
if !v10.is_string() { return Err(format!("Invalid plugin manifest: {}", "manifest.env.*")); }
}
}
if let Some(child)=v0.get("transport") { let v11 = child;
if !v11.is_string() { return Err(format!("Invalid plugin manifest: {}", "manifest.transport")); }
if !["stdio-jsonrpc-v1","stdio-codepet-zstd-v1","stdio-codepet-mux-v1"].contains(&v11.as_str().unwrap_or_default()) { return Err(format!("Invalid plugin manifest: {}", "manifest.transport")); }
}
if let Some(child)=v0.get("enabled") { let v12 = child;
if !v12.is_boolean() { return Err(format!("Invalid plugin manifest: {}", "manifest.enabled")); }
}
if let Some(child)=v0.get("instances") { let v13 = child;
if !v13.is_array() { return Err(format!("Invalid plugin manifest: {}", "manifest.instances")); }
for child in v13.as_array().unwrap() { let v14 = child;
if !v14.is_object() { return Err(format!("Invalid plugin manifest: {}", "manifest.instances[]")); }
if v14.get("instanceKind").is_none() { return Err(format!("Invalid plugin manifest: {}", "manifest.instances[]")); }
if v14.get("displayName").is_none() { return Err(format!("Invalid plugin manifest: {}", "manifest.instances[]")); }
if let Some(child)=v14.get("instanceId") { let v15 = child;
if !v15.is_string() { return Err(format!("Invalid plugin manifest: {}", "manifest.instances[].instanceId")); }
if v15.as_str().unwrap_or_default().chars().count()<1 { return Err(format!("Invalid plugin manifest: {}", "manifest.instances[].instanceId")); }
}
if let Some(child)=v14.get("instanceKind") { let v16 = child;
if !v16.is_string() { return Err(format!("Invalid plugin manifest: {}", "manifest.instances[].instanceKind")); }
if v16.as_str().unwrap_or_default().chars().count()<1 { return Err(format!("Invalid plugin manifest: {}", "manifest.instances[].instanceKind")); }
}
if let Some(child)=v14.get("displayName") { let v17 = child;
if !v17.is_string() { return Err(format!("Invalid plugin manifest: {}", "manifest.instances[].displayName")); }
if v17.as_str().unwrap_or_default().chars().count()<1 { return Err(format!("Invalid plugin manifest: {}", "manifest.instances[].displayName")); }
}
if let Some(child)=v14.get("settings") { let v18 = child;
if !v18.is_object() { return Err(format!("Invalid plugin manifest: {}", "manifest.instances[].settings")); }
}
if let Some(child)=v14.get("enabled") { let v19 = child;
if !v19.is_boolean() { return Err(format!("Invalid plugin manifest: {}", "manifest.instances[].enabled")); }
}
if v14.as_object().unwrap().keys().any(|key| !["instanceId","instanceKind","displayName","settings","enabled"].contains(&key.as_str())) { return Err(format!("Invalid plugin manifest: {}", "manifest.instances[]")); }
}
}
if v0.as_object().unwrap().keys().any(|key| !["$schema","manifestVersion","pluginId","displayName","icon","command","args","env","transport","enabled","instances"].contains(&key.as_str())) { return Err(format!("Invalid plugin manifest: {}", "manifest")); }
} Ok(()) }
