//! Production frontend assets are loaded from a separate webcontent directory.
//! A validated snapshot keeps the two windows on the same resource generation.
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::{
    borrow::Cow,
    collections::BTreeMap,
    fs,
    path::{Component, Path, PathBuf},
};
use tauri::{
    utils::assets::{AssetKey, AssetsIter, CspHash},
    Assets, Runtime,
};

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Contract {
    app_id: String,
    schema_version: u32,
    backend_api_version: u32,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Manifest {
    app_id: String,
    schema_version: u32,
    backend_api_version: u32,
    version: String,
    files: BTreeMap<String, String>,
}

#[derive(Default)]
pub struct WebContentAssets {
    files: BTreeMap<String, Vec<u8>>,
    version: String,
    error_page: Option<Vec<u8>>,
}

/// Only fall back to packaged resources when there are no version directories.
/// Do not silently skip a corrupt/incompatible newest version.
pub fn resolve_directory(
    data_directory: &Path,
    packaged_directory: impl FnOnce() -> Result<PathBuf, String>,
) -> Result<PathBuf, String> {
    let versions = data_directory.join("webcontent");
    let entries = match fs::read_dir(&versions) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return packaged_directory(),
        Err(error) => return Err(format!("cannot read {}: {error}", versions.display())),
    };
    let mut newest: Option<(u64, PathBuf)> = None;
    for entry in entries {
        let entry = entry.map_err(|error| error.to_string())?;
        let name = entry.file_name();
        let Some(number) = name.to_str().and_then(version_number) else {
            continue;
        };
        let kind = entry.file_type().map_err(|error| error.to_string())?;
        if kind.is_symlink() {
            return Err(format!(
                "webcontent version must not be a symlink: {}",
                entry.path().display()
            ));
        }
        if kind.is_dir()
            && newest
                .as_ref()
                .map_or(true, |(current, _)| number > *current)
        {
            newest = Some((number, entry.path()));
        }
    }
    match newest {
        Some((_, directory)) => Ok(directory),
        None => packaged_directory(),
    }
}

fn version_number(name: &str) -> Option<u64> {
    let number = name.strip_prefix('v')?;
    if number.starts_with('0')
        || number.is_empty()
        || !number.bytes().all(|byte| byte.is_ascii_digit())
    {
        return None;
    }
    number.parse().ok()
}

impl WebContentAssets {
    pub fn load(directory: &Path) -> Result<Self, String> {
        Self::read(directory).map_err(|error| format!("{}: {error}", directory.display()))
    }

    fn read(directory: &Path) -> Result<Self, String> {
        let root = directory
            .canonicalize()
            .map_err(|e| format!("webcontent directory unavailable: {e}"))?;
        let contract: Contract =
            serde_json::from_str(include_str!("../../../webcontent-contract.json"))
                .map_err(|e| format!("invalid built-in webcontent contract: {e}"))?;
        let manifest: Manifest = serde_json::from_slice(
            &fs::read(root.join("manifest.json"))
                .map_err(|e| format!("cannot read manifest.json: {e}"))?,
        )
        .map_err(|e| format!("invalid manifest.json: {e}"))?;
        if manifest.app_id != contract.app_id
            || manifest.schema_version != contract.schema_version
            || manifest.backend_api_version != contract.backend_api_version
        {
            return Err(format!(
                "incompatible webcontent; expected app {}, schema {}, backend API {}",
                contract.app_id, contract.schema_version, contract.backend_api_version
            ));
        }
        if manifest.version.trim().is_empty() {
            return Err("missing webcontent version".into());
        }
        for entry in ["index.html", "pet.html"] {
            if !manifest.files.contains_key(entry) {
                return Err(format!("missing entry: {entry}"));
            }
        }
        let mut files = BTreeMap::new();
        for (name, expected_hash) in manifest.files {
            if !valid_asset_name(&name) {
                return Err(format!("invalid asset path: {name}"));
            }
            let path = root
                .join(&name)
                .canonicalize()
                .map_err(|e| format!("missing asset {name}: {e}"))?;
            if !path.starts_with(&root) {
                return Err(format!("asset escapes webcontent: {name}"));
            }
            let bytes = fs::read(path).map_err(|e| format!("cannot read asset {name}: {e}"))?;
            if format!("{:x}", Sha256::digest(&bytes)) != expected_hash {
                return Err(format!("asset checksum mismatch: {name}"));
            }
            files.insert(format!("/{name}"), bytes);
        }
        Ok(Self {
            files,
            version: manifest.version,
            error_page: None,
        })
    }

    pub fn version(&self) -> &str {
        &self.version
    }

    // Only a diagnostic page is built into Rust, never a fallback copy of the UI.
    pub fn unavailable(error: &str) -> Self {
        let escaped = error
            .replace('&', "&amp;")
            .replace('<', "&lt;")
            .replace('>', "&gt;")
            .replace('"', "&quot;");
        let html = format!("<!doctype html><html lang=\"zh-CN\"><meta charset=\"utf-8\"><title>Code Pet — webcontent 加载失败</title><body><h1>无法加载前端资源</h1><p>请退出 App，安装与当前程序兼容的完整 webcontent 目录，然后重新启动。</p><pre style=\"white-space:pre-wrap;overflow-wrap:anywhere\">{escaped}</pre></body></html>");
        Self {
            error_page: Some(html.into_bytes()),
            ..Self::default()
        }
    }

    fn content(&self, key: &str) -> Option<Cow<'_, [u8]>> {
        if let Some(page) = &self.error_page {
            return matches!(key, "/index.html" | "/pet.html")
                .then(|| Cow::Borrowed(page.as_slice()));
        }
        self.files
            .get(key)
            .map(|bytes| Cow::Borrowed(bytes.as_slice()))
    }
}

fn valid_asset_name(name: &str) -> bool {
    !name.is_empty()
        && !name.contains(['\\', ':', '\0'])
        && !name
            .split('/')
            .any(|part| part.is_empty() || part == "." || part == "..")
        && Path::new(name)
            .components()
            .all(|part| matches!(part, Component::Normal(_)))
}

impl<R: Runtime> Assets<R> for WebContentAssets {
    fn get(&self, key: &AssetKey) -> Option<Cow<'_, [u8]>> {
        self.content(key.as_ref())
    }

    fn iter(&self) -> Box<AssetsIter<'_>> {
        Box::new(self.files.iter().map(|(name, bytes)| {
            (
                Cow::Borrowed(name.as_str()),
                Cow::Borrowed(bytes.as_slice()),
            )
        }))
    }

    fn csp_hashes(&self, _html_path: &AssetKey) -> Box<dyn Iterator<Item = CspHash<'_>> + '_> {
        // The current app has no CSP and the asset provider performs no HTML rewriting.
        Box::new(std::iter::empty())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{json, Value};

    fn fixture() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir(dir.path().join("assets")).unwrap();
        fs::write(dir.path().join("index.html"), "<h1>Main</h1>").unwrap();
        fs::write(dir.path().join("pet.html"), "<h1>Pet</h1>").unwrap();
        fs::write(dir.path().join("assets/ui.js"), "const ui = 1;").unwrap();
        write_manifest(dir.path(), "ui-v1");
        dir
    }

    #[test]
    fn uses_packaged_assets_only_when_data_versions_are_absent() {
        let data = tempfile::tempdir().unwrap();
        let packaged = fixture();
        let packaged_path = packaged.path().to_path_buf();
        assert_eq!(
            resolve_directory(data.path(), || Ok(packaged_path.clone())).unwrap(),
            packaged_path
        );
        fs::create_dir(data.path().join("webcontent")).unwrap();
        fs::create_dir(data.path().join("webcontent/.webcontent-stage-incomplete")).unwrap();
        assert_eq!(
            resolve_directory(data.path(), || Ok(packaged_path.clone())).unwrap(),
            packaged_path
        );
    }

    #[test]
    fn newest_data_version_wins_numerically_without_resolving_bundle() {
        let data = tempfile::tempdir().unwrap();
        for name in ["v2", "v10", "v9"] {
            fs::create_dir_all(data.path().join("webcontent").join(name)).unwrap();
        }
        let selected = resolve_directory(data.path(), || {
            panic!("must not consult bundle when data version exists")
        })
        .unwrap();
        assert_eq!(selected, data.path().join("webcontent/v10"));
        // A corrupt newest version must produce an error, never silently load older/bundled UI.
        assert!(WebContentAssets::load(&selected).is_err());
    }

    #[test]
    fn directory_names_have_unambiguous_numeric_versions() {
        assert_eq!(version_number("v10"), Some(10));
        for name in [
            "v0",
            "v01",
            "v",
            "V1",
            "v1.2",
            "v1-old",
            ".v20",
            "v18446744073709551616",
        ] {
            assert_eq!(version_number(name), None, "{name}");
        }
    }

    fn write_manifest(root: &Path, version: &str) {
        let mut manifest: Value =
            serde_json::from_str(include_str!("../../../webcontent-contract.json")).unwrap();
        let files = ["index.html", "pet.html", "assets/ui.js"]
            .into_iter()
            .map(|name| {
                (
                    name.to_string(),
                    json!(format!(
                        "{:x}",
                        Sha256::digest(fs::read(root.join(name)).unwrap())
                    )),
                )
            })
            .collect::<serde_json::Map<String, Value>>();
        manifest["version"] = json!(version);
        manifest["files"] = json!(files);
        fs::write(
            root.join("manifest.json"),
            serde_json::to_vec(&manifest).unwrap(),
        )
        .unwrap();
    }

    #[test]
    fn external_assets_use_normal_tauri_keys_and_ignore_unlisted_files() {
        let dir = fixture();
        fs::write(dir.path().join("secret.txt"), "not a UI asset").unwrap();
        let assets = WebContentAssets::load(dir.path()).unwrap();
        let provider: &dyn Assets<tauri::Wry> = &assets;
        assert_eq!(
            provider.get(&AssetKey::from("pet.html")).unwrap().as_ref(),
            b"<h1>Pet</h1>"
        );
        assert_eq!(provider.iter().count(), 3);
        assert!(provider.get(&AssetKey::from("secret.txt")).is_none());
        assert!(provider.get(&AssetKey::from("../secret.txt")).is_none());
    }

    #[test]
    fn a_new_ui_package_loads_without_changing_the_binary_contract() {
        let dir = fixture();
        let first = WebContentAssets::load(dir.path()).unwrap();
        fs::write(dir.path().join("assets/ui.js"), "const ui = 2;").unwrap();
        write_manifest(dir.path(), "ui-v2");
        let second = WebContentAssets::load(dir.path()).unwrap();
        assert_eq!(
            first.content("/assets/ui.js").unwrap().as_ref(),
            b"const ui = 1;"
        );
        assert_eq!(
            second.content("/assets/ui.js").unwrap().as_ref(),
            b"const ui = 2;"
        );
        assert_eq!(second.version(), "ui-v2");
    }

    #[test]
    fn missing_and_corrupt_packages_fail_with_diagnostics() {
        let dir = fixture();
        fs::write(dir.path().join("assets/ui.js"), "corrupt").unwrap();
        assert!(WebContentAssets::load(dir.path())
            .err()
            .unwrap()
            .contains("checksum mismatch"));
        fs::remove_file(dir.path().join("pet.html")).unwrap();
        assert!(WebContentAssets::load(dir.path()).is_err());
        let missing = dir.path().join("missing");
        assert!(WebContentAssets::load(&missing)
            .err()
            .unwrap()
            .contains("directory unavailable"));
    }

    #[test]
    fn rejects_incompatible_contract_and_path_traversal() {
        let dir = fixture();
        let manifest_path = dir.path().join("manifest.json");
        let mut manifest: Value =
            serde_json::from_slice(&fs::read(&manifest_path).unwrap()).unwrap();
        manifest["backendApiVersion"] = json!(99999);
        fs::write(&manifest_path, serde_json::to_vec(&manifest).unwrap()).unwrap();
        assert!(WebContentAssets::load(dir.path())
            .err()
            .unwrap()
            .contains("incompatible"));
        for name in [
            "../secret",
            "/absolute",
            "a/../secret",
            "a//file",
            "C:/secret",
            "a\\secret",
            "a:stream",
            "a/./file",
        ] {
            assert!(!valid_asset_name(name), "{name}");
        }
    }

    #[test]
    fn diagnostic_page_escapes_paths_and_does_not_serve_a_fallback_ui() {
        let assets = WebContentAssets::unavailable("<script>alert(1)</script>");
        let page = assets.content("/index.html").unwrap();
        let page = String::from_utf8_lossy(&page);
        assert!(!page.contains("<script>"));
        assert!(page.contains("&lt;script&gt;"));
        assert!(assets.content("/assets/ui.js").is_none());
    }

    #[cfg(unix)]
    #[test]
    fn rejects_symlinks_outside_the_package() {
        let dir = fixture();
        let outside = tempfile::tempdir().unwrap();
        fs::write(outside.path().join("ui.js"), "outside").unwrap();
        fs::remove_file(dir.path().join("assets/ui.js")).unwrap();
        std::os::unix::fs::symlink(
            outside.path().join("ui.js"),
            dir.path().join("assets/ui.js"),
        )
        .unwrap();
        write_manifest(dir.path(), "ui-v1");
        assert!(WebContentAssets::load(dir.path())
            .err()
            .unwrap()
            .contains("escapes webcontent"));
    }
}
