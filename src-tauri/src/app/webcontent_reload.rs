//! Shared resource generations. Backend services survive a frontend reload.
use super::webcontent::{resolve_directory, WebContentAssets};
use serde::Serialize;
use std::{
    borrow::Cow,
    collections::BTreeMap,
    sync::{Arc, Mutex, RwLock},
};
use tauri::{
    utils::assets::{AssetKey, AssetsIter, CspHash},
    Assets, Manager, Runtime,
};

#[derive(Clone, Serialize, PartialEq, Debug)]
#[serde(rename_all = "camelCase")]
pub struct WebcontentInfo {
    version: String,
    built_at: u64,
    can_reload: bool,
}

struct Generation {
    active: WebContentAssets,
    // Old pages may still request lazy assets while the two windows navigate.
    // Retain verified assets for this process lifetime; entry HTML always comes
    // from the active generation. A process restart releases this cache.
    previous_files: BTreeMap<String, Vec<u8>>,
}

#[derive(Clone)]
pub struct ReloadableAssets {
    generation: Arc<RwLock<Generation>>,
    reloading: Arc<Mutex<()>>,
}

impl ReloadableAssets {
    pub fn new(active: WebContentAssets) -> Self {
        Self {
            generation: Arc::new(RwLock::new(Generation {
                active,
                previous_files: BTreeMap::new(),
            })),
            reloading: Arc::new(Mutex::new(())),
        }
    }

    fn info(&self) -> Result<WebcontentInfo, String> {
        let current = self.generation.read().map_err(|e| e.to_string())?;
        Ok(WebcontentInfo {
            version: current.active.version().into(),
            built_at: current.active.built_at(),
            can_reload: !tauri::is_dev(),
        })
    }

    fn replace(&self, next: WebContentAssets) -> Result<WebContentAssets, String> {
        let mut current = self.generation.write().map_err(|e| e.to_string())?;
        let old = std::mem::replace(&mut current.active, next);
        for (name, bytes) in &old.files {
            if name != "/index.html" && name != "/pet.html" {
                current
                    .previous_files
                    .entry(name.clone())
                    .or_insert_with(|| bytes.clone());
            }
        }
        Ok(old)
    }

    fn content(&self, key: &str) -> Option<Vec<u8>> {
        let current = self.generation.read().ok()?;
        current
            .active
            .content(key)
            .map(|bytes| bytes.into_owned())
            .or_else(|| current.previous_files.get(key).cloned())
    }

    fn activate(
        &self,
        next: WebContentAssets,
        reload_pet: impl FnOnce(&WebcontentInfo) -> Result<(), String>,
    ) -> Result<WebcontentInfo, String> {
        let candidate = WebcontentInfo {
            version: next.version().into(),
            built_at: next.built_at(),
            can_reload: true,
        };
        let current = self.info()?;
        if !current.version.is_empty() && rank(&candidate)? <= rank(&current)? {
            return Ok(current);
        }
        let old = self.replace(next)?;
        if let Err(error) = reload_pet(&candidate) {
            self.replace(old)?;
            return Err(format!("桌宠窗口重新加载失败，已恢复当前资源：{error}"));
        }
        Ok(candidate)
    }
}

impl<R: Runtime> Assets<R> for ReloadableAssets {
    fn get(&self, key: &AssetKey) -> Option<Cow<'_, [u8]>> {
        self.content(key.as_ref()).map(Cow::Owned)
    }
    fn iter(&self) -> Box<AssetsIter<'_>> {
        let files = self
            .generation
            .read()
            .map(|current| current.active.files.clone())
            .unwrap_or_default();
        Box::new(
            files
                .into_iter()
                .map(|(name, bytes)| (Cow::Owned(name), Cow::Owned(bytes))),
        )
    }
    fn csp_hashes(&self, _: &AssetKey) -> Box<dyn Iterator<Item = CspHash<'_>> + '_> {
        Box::new(std::iter::empty())
    }
}

#[tauri::command]
pub fn webcontent_info(
    state: tauri::State<'_, ReloadableAssets>,
) -> Result<WebcontentInfo, String> {
    state.info()
}

fn rank(info: &WebcontentInfo) -> Result<(semver::Version, u64), String> {
    let mut version = semver::Version::parse(&info.version).map_err(|e| e.to_string())?;
    version.build = semver::BuildMetadata::EMPTY;
    Ok((version, info.built_at))
}

#[tauri::command]
pub async fn load_latest_webcontent(
    app: tauri::AppHandle,
    window: tauri::WebviewWindow,
    state: tauri::State<'_, ReloadableAssets>,
) -> Result<WebcontentInfo, String> {
    if window.label() != "main" {
        return Err("Only the main window can reload frontend resources".into());
    }
    if tauri::is_dev() {
        return Err("开发模式使用 Vite 热更新".into());
    }
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let _guard = state
            .reloading
            .try_lock()
            .map_err(|_| "前端资源正在加载，请稍后重试".to_string())?;
        let paths = super::paths::for_package(app.package_info()).map_err(|e| e.to_string())?;
        let directory = resolve_directory(&paths.data, || Ok(paths.packaged_webcontent()))?;
        let next = WebContentAssets::load(&directory)?;
        // Validate and obtain the pet URL before publishing anything.
        let pet = app.get_webview_window("pet");
        let pet_url = pet
            .as_ref()
            .map(|pet| pet.url().map_err(|e| e.to_string()))
            .transpose()?;
        let candidate = state.activate(next, |candidate| {
            if let (Some(pet), Some(mut url)) = (pet, pet_url) {
                url.set_query(Some(&format!("webcontent={}", candidate.built_at)));
                pet.navigate(url).map_err(|e| e.to_string())?;
            }
            Ok(())
        })?;
        super::log::info(
            "webcontent",
            &format!(
                "hot loaded {} version={} builtAt={}",
                directory.display(),
                candidate.version,
                candidate.built_at
            ),
        );
        // The caller navigates only after receiving this response, so the IPC
        // response isn't destroyed by reloading its own webview too early.
        Ok(candidate)
    })
    .await
    .map_err(|e| e.to_string())?
}

#[cfg(test)]
mod tests {
    use super::*;
    fn package(version: &str, built_at: u64) -> WebContentAssets {
        use sha2::{Digest, Sha256};
        let directory = tempfile::tempdir().unwrap();
        let mut manifest: serde_json::Value =
            serde_json::from_str(include_str!("../../../webcontent-contract.json")).unwrap();
        manifest["version"] = serde_json::json!(version);
        manifest["builtAt"] = serde_json::json!(built_at);
        let content = format!("{version}/{built_at}");
        let digest = format!("{:x}", Sha256::digest(content.as_bytes()));
        manifest["files"] = serde_json::json!({"index.html": digest, "pet.html": digest});
        for name in ["index.html", "pet.html"] {
            std::fs::write(directory.path().join(name), &content).unwrap();
        }
        std::fs::write(directory.path().join("manifest.json"), manifest.to_string()).unwrap();
        WebContentAssets::load(directory.path()).unwrap()
    }
    #[test]
    fn failed_window_navigation_restores_old_snapshot_and_can_retry() {
        let state = ReloadableAssets::new(package("1.0.0", 1));
        assert!(state
            .activate(package("1.0.0", 2), |_| Err("window unavailable".into()))
            .is_err());
        assert_eq!(state.info().unwrap().built_at, 1);
        assert_eq!(state.content("/index.html").unwrap(), b"1.0.0/1");
        let info = state.activate(package("1.0.0", 2), |_| Ok(())).unwrap();
        assert_eq!(info.built_at, 2);
        assert_eq!(state.content("/pet.html").unwrap(), b"1.0.0/2");
    }
    #[test]
    fn same_or_older_packages_never_reload_or_downgrade_a_running_page() {
        let state = ReloadableAssets::new(package("2.0.0", 10));
        for candidate in [
            package("2.0.0", 10),
            package("2.0.0", 9),
            package("1.9.0", 99),
        ] {
            let info = state
                .activate(candidate, |_| {
                    panic!("unchanged resources must not navigate")
                })
                .unwrap();
            assert_eq!(info.version, "2.0.0");
            assert_eq!(info.built_at, 10);
        }
    }
    #[test]
    fn swap_is_shared_and_keeps_old_lazy_assets_but_not_old_entry_html() {
        let mut old = WebContentAssets::default();
        old.files.insert("/index.html".into(), b"old".to_vec());
        old.files.insert("/assets/old.js".into(), b"lazy".to_vec());
        let state = ReloadableAssets::new(old);
        let window_assets = state.clone();
        let mut next = WebContentAssets::default();
        next.files.insert("/index.html".into(), b"new".to_vec());
        let previous = state.replace(next).unwrap();
        assert_eq!(window_assets.content("/index.html").unwrap(), b"new");
        assert_eq!(window_assets.content("/assets/old.js").unwrap(), b"lazy");
        state.replace(previous).unwrap();
        assert_eq!(window_assets.content("/index.html").unwrap(), b"old");
    }
    #[test]
    fn upgrades_use_semver_then_build_time_and_ignore_build_metadata() {
        let info = |version: &str, built_at| WebcontentInfo {
            version: version.into(),
            built_at,
            can_reload: true,
        };
        assert!(rank(&info("1.1.0", 1)).unwrap() > rank(&info("1.0.0", 999)).unwrap());
        assert!(rank(&info("1.0.0", 2)).unwrap() > rank(&info("1.0.0", 1)).unwrap());
        assert_eq!(rank(&info("1.0.0+a", 1)), rank(&info("1.0.0+b", 1)));
    }
}
