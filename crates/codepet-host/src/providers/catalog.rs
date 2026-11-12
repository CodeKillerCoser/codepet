use crate::{HostError, HostResult};
use codepet_provider_sdk::{JsonObject, ProviderPluginId};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use url::Url;

#[path="manifest_generated.rs"]
mod manifest_contract;
pub use manifest_contract::PLUGIN_MANIFEST_FILE_NAME;

fn default_true() -> bool {
    true
}

pub use manifest_contract::{PluginDescriptor, PluginInstanceConfig};
use manifest_contract::PluginManifest;

#[derive(Clone, Debug, Default)]
pub struct PluginCatalogConfig {
    directories: Vec<PathBuf>,
}

impl PluginCatalogConfig {
    pub fn for_data_directory(data_directory: impl AsRef<Path>) -> Self {
        Self {
            directories: vec![data_directory.as_ref().join("versions/providers")],
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

    pub(crate) fn from_entries(entries: impl IntoIterator<Item = PluginDescriptor>) -> HostResult<Self> {
        let mut catalog = Self::default();
        for descriptor in entries {
            validate_descriptor(&descriptor)?;
            if catalog.descriptors.insert(descriptor.plugin_id.clone(), descriptor).is_some() {
                return Err(HostError::new("duplicate_plugin_id", "Provider ID is already installed"));
            }
        }
        Ok(catalog)
    }

    /// Parse and validate a package through the same path used by custom import.
    pub fn read_one(path: &Path) -> HostResult<PluginDescriptor> {
        let mut candidates = Vec::new();
        let mut diagnostics = Vec::new();
        read_manifest(path, &mut candidates, &mut diagnostics);
        if let Some(error) = diagnostics.into_iter().next() {
            return Err(HostError::new(error.code, error.message));
        }
        let descriptor = candidates
            .pop()
            .ok_or_else(|| HostError::new("invalid_plugin_manifest", "Missing Provider manifest"))?
            .1;
        validate_descriptor(&descriptor)?;
        Ok(descriptor)
    }

    pub(crate) fn descriptors(&self) -> impl Iterator<Item = &PluginDescriptor> {
        self.descriptors.values()
    }

    pub fn diagnostics(&self) -> &[CatalogDiagnostic] {
        &self.diagnostics
    }

    pub fn update_instance_settings(
        &mut self,
        plugin_id: &str,
        instance_kind: &str,
        mut update: impl FnMut(&mut JsonObject),
    ) -> HostResult<usize> {
        if plugin_id.trim().is_empty() || instance_kind.trim().is_empty() {
            return Err(HostError::new(
                "invalid_provider_instance_selector",
                "Provider plugin id and instance kind must not be empty",
            ));
        }
        let Some(descriptor) = self.descriptors.get_mut(plugin_id) else {
            return Ok(0);
        };
        let mut updated = 0;
        for instance in &mut descriptor.instances {
            if instance.instance_kind == instance_kind {
                update(&mut instance.settings);
                updated += 1;
            }
        }
        Ok(updated)
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
            let value: serde_json::Value =
                serde_json::from_slice(&payload).map_err(|error| error.to_string())?;
            if value["manifestVersion"] == manifest_contract::PLUGIN_MANIFEST_VERSION { manifest_contract::validate_v2_manifest(&value)?; }
            for key in [
                "CODEPET_PROVIDER_PACKAGE_DIRECTORY",
                "CODEPET_PROVIDER_EXTERNAL_COMMAND",
            ] {
                if value.get("env").and_then(|v| v.get(key)).is_some() {
                    return Err(format!("reserved Host environment key: {key}"));
                }
            }
            serde_json::from_value::<PluginManifest>(value).map_err(|error| error.to_string())
        });
    match result {
        Ok(mut manifest) if manifest.manifest_version == 1 || manifest.manifest_version == manifest_contract::PLUGIN_MANIFEST_VERSION => {
            {
                let profile = manifest
                    .transport
                    .unwrap_or_else(|| codepet_provider_sdk::MUX_PROFILE.into());
                let key = codepet_provider_sdk::TRANSPORT_ENV;
                if codepet_transport::Profile::parse(&profile).is_err()
                    || manifest
                        .descriptor
                        .env
                        .get(key)
                        .is_some_and(|value| value != &profile)
                {
                    diagnostics.push(CatalogDiagnostic {
                        code: "invalid_plugin_transport".into(),
                        message: format!(
                            "unsupported or conflicting Provider transport in {}",
                            path.display()
                        ),
                        path: Some(path.to_path_buf()),
                        plugin_id: Some(manifest.descriptor.plugin_id),
                    });
                    return;
                }
                manifest.descriptor.env.insert(key.into(), profile);
            }
            if let Some(parent) = path.parent() {
                let parent = parent.canonicalize().unwrap_or_else(|_| parent.to_owned());
                let command = &manifest.descriptor.executable;
                let packaged = parent.join(command);
                let external = manifest.manifest_version == manifest_contract::PLUGIN_MANIFEST_VERSION
                    && ((command.is_absolute() && !command.starts_with(&parent))
                        || (command.is_relative()
                            && command.components().count() == 1
                            && !packaged.is_file()));
                if external {
                    manifest
                        .descriptor
                        .env
                        .insert("CODEPET_PROVIDER_EXTERNAL_COMMAND".into(), "1".into());
                } else if command.is_relative() {
                    manifest.descriptor.executable = packaged;
                }
                manifest.descriptor.env.insert(
                    "CODEPET_PROVIDER_PACKAGE_DIRECTORY".into(),
                    parent.to_string_lossy().into_owned(),
                );
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
            message: format!(
                "decode Provider plugin manifest {}: {error}",
                path.display()
            ),
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
    if let Some(icon) = descriptor.icon.as_ref() {
        let valid_icon = Url::parse(icon).ok().is_some_and(|url| {
            url.scheme() == "https"
                && url.host_str().is_some()
                && url.username().is_empty()
                && url.password().is_none()
        });
        if !valid_icon {
            return Err(HostError::new(
                "invalid_plugin_descriptor",
                format!(
                    "Provider plugin {} icon must be an absolute HTTPS URL without credentials",
                    descriptor.plugin_id
                ),
            ));
        }
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

#[cfg(test)]
mod transport_tests {
    use super::*;
    #[test]
    fn manifest_selects_transport_explicitly_and_rejects_conflicting_env() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("codepet-provider.json");
        let mut value = serde_json::json!({"manifestVersion":1,"pluginId":"test.mux","displayName":"Mux","executable":"provider","transport":codepet_provider_sdk::MUX_PROFILE});
        fs::write(&path, serde_json::to_vec(&value).unwrap()).unwrap();
        let mut candidates = vec![]; let mut diagnostics = vec![];
        read_manifest(&path, &mut candidates, &mut diagnostics);
        assert!(diagnostics.is_empty());
        assert_eq!(candidates[0].1.env[codepet_provider_sdk::TRANSPORT_ENV], codepet_provider_sdk::MUX_PROFILE);
        value.as_object_mut().unwrap().remove("transport");
        fs::write(&path, serde_json::to_vec(&value).unwrap()).unwrap();
        candidates.clear();
        read_manifest(&path, &mut candidates, &mut diagnostics);
        assert!(diagnostics.is_empty());
        assert_eq!(candidates[0].1.env[codepet_provider_sdk::TRANSPORT_ENV], codepet_provider_sdk::MUX_PROFILE);
        value["env"] = serde_json::json!({codepet_provider_sdk::TRANSPORT_ENV:"stdio-codepet-frame-v1"});
        fs::write(&path, serde_json::to_vec(&value).unwrap()).unwrap();
        candidates.clear();
        read_manifest(&path, &mut candidates, &mut diagnostics);
        assert!(candidates.is_empty()); assert_eq!(diagnostics[0].code, "invalid_plugin_transport");
    }
}

#[cfg(test)]
mod manifest_v2_tests {
    use super::*;
    #[test]
    fn v2_launch_contract_keeps_argv_and_env_literal_and_requires_explicit_profile() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join(PLUGIN_MANIFEST_FILE_NAME);
        for profile in [
            "stdio-jsonrpc-v1",
            "stdio-codepet-zstd-v1",
            "stdio-codepet-mux-v1",
        ] {
            let mut value = serde_json::json!({"$schema":"plugin-manifest.schema.json","manifestVersion":2,"pluginId":"test.contract","displayName":"Contract",
                "command":"node","args":["entry with space.mjs","$literal","--value=a b"],"env":{"CUSTOM":"$NO_EXPANSION"},"transport":profile});
            fs::write(&path, serde_json::to_vec(&value).unwrap()).unwrap();
            let descriptor = PluginCatalog::read_one(&path).unwrap();
            assert_eq!(descriptor.executable, PathBuf::from("node"));
            assert_eq!(
                descriptor.args,
                ["entry with space.mjs", "$literal", "--value=a b"]
            );
            assert_eq!(descriptor.env["CUSTOM"], "$NO_EXPANSION");
            assert_eq!(descriptor.env[codepet_provider_sdk::TRANSPORT_ENV], profile);
            assert_eq!(
                PathBuf::from(&descriptor.env["CODEPET_PROVIDER_PACKAGE_DIRECTORY"]),
                root.path().canonicalize().unwrap()
            );
            value.as_object_mut().unwrap().remove("transport");
            fs::write(&path, serde_json::to_vec(&value).unwrap()).unwrap();
            assert!(PluginCatalog::read_one(&path).is_err());
        }
    }
    #[test]
    fn packaged_command_is_resolved_against_manifest_and_reserved_context_is_rejected() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join(PLUGIN_MANIFEST_FILE_NAME);
        let mut value = serde_json::json!({"manifestVersion":2,"pluginId":"test.pkg","displayName":"Package","command":"./bin/provider","transport":"stdio-jsonrpc-v1"});
        fs::write(&path, serde_json::to_vec(&value).unwrap()).unwrap();
        let descriptor = PluginCatalog::read_one(&path).unwrap();
        assert_eq!(
            descriptor.executable,
            root.path().canonicalize().unwrap().join("./bin/provider")
        );
        value["transport"] = serde_json::json!("unknown");
        fs::write(&path, serde_json::to_vec(&value).unwrap()).unwrap();
        assert!(PluginCatalog::read_one(&path).is_err());
        value["transport"] = serde_json::json!("stdio-jsonrpc-v1");
        value["env"] = serde_json::json!({"CODEPET_PROVIDER_PACKAGE_DIRECTORY":"/elsewhere"});
        fs::write(&path, serde_json::to_vec(&value).unwrap()).unwrap();
        assert!(PluginCatalog::read_one(&path).is_err());
    }
}
