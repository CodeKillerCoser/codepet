// Generated from protocol/v2/provider; do not edit.
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
pub icon: String,
#[serde(rename="command")]
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
pub(super) struct PluginManifest { pub manifest_version:u32, #[serde(rename="$schema",default)] pub _schema:Option<String>, pub transport:String, #[serde(flatten)] pub descriptor:PluginDescriptor }
pub const PLUGIN_MANIFEST_FILE_NAME: &str = "codepet-provider.json";
pub const PLUGIN_MANIFEST_VERSION: u32 = 2;
