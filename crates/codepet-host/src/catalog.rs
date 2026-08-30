use crate::{HostError, HostResult};
use codepet_provider_sdk::{JsonObject, ProviderInstanceId, ProviderPluginId};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

pub const PLUGIN_MANIFEST_FILE_NAME: &str = "codepet-provider.json";

fn default_true() -> bool {
    true
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
pub struct PluginInstanceConfig {
    #[serde(default)]
    pub instance_id: Option<ProviderInstanceId>,
    pub instance_kind: String,
    pub display_name: String,
    #[serde(default)]
    pub settings: JsonObject,
    #[serde(default = "default_true")]
    pub enabled: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
pub struct PluginDescriptor {
    pub plugin_id: ProviderPluginId,
    pub display_name: String,
    pub executable: PathBuf,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub env: BTreeMap<String, String>,
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default)]
    pub instances: Vec<PluginInstanceConfig>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
struct PluginManifest {
    manifest_version: u32,
    #[serde(flatten)]
    descriptor: PluginDescriptor,
}

#[derive(Clone, Debug, Default)]
pub struct PluginCatalogConfig {
    directories: Vec<PathBuf>,
}

impl PluginCatalogConfig {
    pub fn for_data_directory(data_directory: impl AsRef<Path>) -> Self {
        Self {
            directories: vec![data_directory.as_ref().join("provider-plugins")],
        }
    }

    pub fn with_directory(mut self, directory: impl Into<PathBuf>) -> Self {
        self.directories.push(directory.into());
        self
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct CatalogDiagnostic {
    pub code: String,
    pub message: String,
    pub path: Option<PathBuf>,
    pub plugin_id: Option<ProviderPluginId>,
}

#[derive(Clone, Debug, Default)]
pub struct PluginCatalog {
    descriptors: BTreeMap<ProviderPluginId, PluginDescriptor>,
    diagnostics: Vec<CatalogDiagnostic>,
}

impl PluginCatalog {
    pub fn discover(config: PluginCatalogConfig) -> Self {
        let mut candidates = Vec::new();
        let mut diagnostics = Vec::new();

        let mut discovered_directories = BTreeSet::new();
        for directory in config.directories {
            if discovered_directories.insert(directory.clone()) {
                discover_directory(&directory, &mut candidates, &mut diagnostics);
            }
        }

        candidates.sort_by(|left, right| {
            left.1
                .plugin_id
                .cmp(&right.1.plugin_id)
                .then_with(|| left.0.cmp(&right.0))
        });
        let mut descriptors = BTreeMap::new();
        let mut duplicate_ids = BTreeSet::new();
        for (path, descriptor) in candidates {
            match validate_descriptor(&descriptor) {
                Ok(()) => {
                    if descriptors.insert(descriptor.plugin_id.clone(), descriptor.clone()).is_some() {
                        duplicate_ids.insert(descriptor.plugin_id.clone());
                        diagnostics.push(CatalogDiagnostic {
                            code: "duplicate_plugin_id".to_string(),
                            message: format!(
                                "multiple Provider manifests declare plugin id: {}",
                                descriptor.plugin_id
                            ),
                            path,
                            plugin_id: Some(descriptor.plugin_id),
                        });
                    }
                }
                Err(error) => diagnostics.push(CatalogDiagnostic {
                    code: error.code,
                    message: error.message,
                    path,
                    plugin_id: Some(descriptor.plugin_id),
                }),
            }
        }
        for duplicate_id in duplicate_ids {
            descriptors.remove(&duplicate_id);
        }

        Self {
            descriptors,
            diagnostics,
        }
    }

    pub(crate) fn descriptors(&self) -> impl Iterator<Item = &PluginDescriptor> {
        self.descriptors.values()
    }

    pub fn descriptor(&self, plugin_id: &str) -> Option<&PluginDescriptor> {
        self.descriptors.get(plugin_id)
    }

    pub fn diagnostics(&self) -> &[CatalogDiagnostic] {
        &self.diagnostics
    }
}

fn discover_directory(
    directory: &Path,
    candidates: &mut Vec<(Option<PathBuf>, PluginDescriptor)>,
    diagnostics: &mut Vec<CatalogDiagnostic>,
) {
    if !directory.exists() {
        return;
    }
    if !directory.is_dir() {
        diagnostics.push(CatalogDiagnostic {
            code: "invalid_plugin_directory".to_string(),
            message: format!("Provider plugin path is not a directory: {}", directory.display()),
            path: Some(directory.to_path_buf()),
            plugin_id: None,
        });
        return;
    }

    let direct_manifest = directory.join(PLUGIN_MANIFEST_FILE_NAME);
    if direct_manifest.is_file() {
        read_manifest(&direct_manifest, candidates, diagnostics);
    }

    let mut entries = match fs::read_dir(directory) {
        Ok(entries) => {
            let mut readable = Vec::new();
            for entry in entries {
                match entry {
                    Ok(entry) => readable.push(entry),
                    Err(error) => diagnostics.push(CatalogDiagnostic {
                        code: "plugin_directory_entry_read_failed".to_string(),
                        message: format!(
                            "read entry in Provider plugin directory {}: {error}",
                            directory.display()
                        ),
                        path: Some(directory.to_path_buf()),
                        plugin_id: None,
                    }),
                }
            }
            readable
        }
        Err(error) => {
            diagnostics.push(CatalogDiagnostic {
                code: "plugin_directory_read_failed".to_string(),
                message: format!("read Provider plugin directory {}: {error}", directory.display()),
                path: Some(directory.to_path_buf()),
                plugin_id: None,
            });
            return;
        }
    };
    entries.sort_by_key(|entry| entry.file_name());
    for entry in entries {
        let path = entry.path();
        if path.is_dir() {
            let manifest = path.join(PLUGIN_MANIFEST_FILE_NAME);
            if manifest.is_file() {
                read_manifest(&manifest, candidates, diagnostics);
            }
        } else if path
            .file_name()
            .and_then(|name| name.to_str())
            .map(|name| name.ends_with(".codepet-provider.json"))
            .unwrap_or(false)
        {
            read_manifest(&path, candidates, diagnostics);
        }
    }
}

fn read_manifest(
    path: &Path,
    candidates: &mut Vec<(Option<PathBuf>, PluginDescriptor)>,
    diagnostics: &mut Vec<CatalogDiagnostic>,
) {
    let result = fs::read(path)
        .map_err(|error| error.to_string())
        .and_then(|payload| {
            serde_json::from_slice::<PluginManifest>(&payload).map_err(|error| error.to_string())
        });
    match result {
        Ok(mut manifest) if manifest.manifest_version == 1 => {
            if manifest.descriptor.executable.is_relative() {
                if let Some(parent) = path.parent() {
                    manifest.descriptor.executable = parent.join(&manifest.descriptor.executable);
                }
            }
            candidates.push((Some(path.to_path_buf()), manifest.descriptor));
        }
        Ok(manifest) => diagnostics.push(CatalogDiagnostic {
            code: "unsupported_plugin_manifest_version".to_string(),
            message: format!(
                "unsupported Provider plugin manifest version {} in {}",
                manifest.manifest_version,
                path.display()
            ),
            path: Some(path.to_path_buf()),
            plugin_id: Some(manifest.descriptor.plugin_id),
        }),
        Err(error) => diagnostics.push(CatalogDiagnostic {
            code: "invalid_plugin_manifest".to_string(),
            message: format!("decode Provider plugin manifest {}: {error}", path.display()),
            path: Some(path.to_path_buf()),
            plugin_id: None,
        }),
    }
}

fn validate_descriptor(descriptor: &PluginDescriptor) -> HostResult<()> {
    if descriptor.plugin_id.trim().is_empty() {
        return Err(HostError::new(
            "invalid_plugin_descriptor",
            "Provider plugin id must not be empty",
        ));
    }
    if descriptor.display_name.trim().is_empty() || descriptor.executable.as_os_str().is_empty() {
        return Err(HostError::new(
            "invalid_plugin_descriptor",
            format!(
                "Provider plugin {} must declare a display name and executable",
                descriptor.plugin_id
            ),
        ));
    }
    let mut instance_ids = BTreeSet::new();
    for instance in &descriptor.instances {
        if instance.instance_kind.trim().is_empty() || instance.display_name.trim().is_empty() {
            return Err(HostError::new(
                "invalid_plugin_instance",
                format!(
                    "Provider plugin {} contains an instance without kind or display name",
                    descriptor.plugin_id
                ),
            ));
        }
        if let Some(instance_id) = instance.instance_id.as_ref() {
            if instance_id.trim().is_empty() || !instance_ids.insert(instance_id.clone()) {
                return Err(HostError::new(
                    "invalid_plugin_instance",
                    format!(
                        "Provider plugin {} contains an empty or duplicate instance id",
                        descriptor.plugin_id
                    ),
                ));
            }
        }
    }
    Ok(())
}
