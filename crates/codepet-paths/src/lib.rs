//! Code Pet installation, application data and managed execution paths.
//! Settings and path resolution share the same bootstrap settings.json.
use serde::{Deserialize, Serialize};
use std::{fs, io, path::{Path, PathBuf}};

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DataSettings {
    #[serde(default)]
    pub data_directory: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PathManager {
    pub install: PathBuf,
    pub data: PathBuf,
    pub workspace: PathBuf,
    pub resources: PathBuf,
    pub settings_file: PathBuf,
}

impl PathManager {
    pub fn discover() -> io::Result<Self> {
        let executable = std::env::current_exe()?;
        let home = dirs::home_dir().ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "user home directory unavailable"))?;
        let platform_data = dirs::data_local_dir().ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "platform application data directory unavailable"))?;
        Self::from_roots(&executable, &home, &platform_data, cfg!(target_os = "macos"))?.load_settings()
    }

    pub fn load_settings(mut self) -> io::Result<Self> {
        #[derive(Default, Deserialize)]
        struct Document { #[serde(default)] data: DataSettings }
        let document: Document = match fs::read(&self.settings_file) {
            Ok(bytes) => serde_json::from_slice(&bytes).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?,
            Err(e) if e.kind() == io::ErrorKind::NotFound => Document::default(),
            Err(e) => return Err(e),
        };
        self.data = data_directory(self.settings_file.parent().and_then(Path::parent).expect("bootstrap data root"), &document.data);
        if !self.data.is_absolute() { return Err(io::Error::new(io::ErrorKind::InvalidData, "data directory must be absolute")); }
        Ok(self)
    }

    pub fn from_roots(executable: &Path, home: &Path, platform_data: &Path, macos: bool) -> io::Result<Self> {
        if !executable.is_absolute() || !home.is_absolute() || !platform_data.is_absolute() {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, "path roots must be absolute"));
        }
        let binary_dir = executable.parent().ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "executable has no parent"))?;
        let bundle = binary_dir.parent().and_then(Path::parent).filter(|p| {
            macos && binary_dir.file_name().is_some_and(|n| n == "MacOS")
                && binary_dir.parent().and_then(Path::file_name).is_some_and(|n| n == "Contents")
                && p.extension().is_some_and(|e| e == "app")
        });
        let install = bundle.unwrap_or(binary_dir).to_path_buf();
        let resources = bundle.map(|p| p.join("Contents/Resources")).unwrap_or_else(|| binary_dir.to_path_buf());
        let data = platform_data.join("code-pet");
        Ok(Self { install, resources, workspace: workspace_for_home(home), settings_file: data.join("config/settings.json"), data })
    }
    pub fn webcontent(&self) -> PathBuf { self.data.join("versions/webcontent") }
    pub fn packaged_webcontent(&self) -> PathBuf { self.resources.join("webcontent") }
    pub fn logs(&self) -> PathBuf { self.data.join("logs") }
    pub fn pets(&self) -> PathBuf { self.data.join("pets") }
    pub fn hooks(&self) -> PathBuf { self.data.join("config/hooks") }
    pub fn provider_plugins(&self) -> PathBuf { self.resources.join("provider-plugins") }
}

pub fn default_data_directory() -> PathBuf {
    dirs::data_local_dir().expect("platform application data directory unavailable").join("code-pet")
}
pub fn home_directory() -> Option<PathBuf> { dirs::home_dir() }
pub fn settings_file() -> PathBuf { default_data_directory().join("config/settings.json") }
pub fn data_directory(default: &Path, settings: &DataSettings) -> PathBuf {
    settings.data_directory.as_deref().map(str::trim).filter(|p| !p.is_empty()).map(PathBuf::from).unwrap_or_else(|| default.to_path_buf())
}
pub fn workspace_for_home(home: &Path) -> PathBuf { home.join(".codepet") }
pub fn workspace_directory() -> io::Result<PathBuf> {
    dirs::home_dir().map(|home| workspace_for_home(&home)).ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "user home directory unavailable"))
}
pub fn remote_workspace_for_user(provider: &str) -> Option<String> {
    home_directory().and_then(|home| remote_workspace(&home, provider)).map(|path| path.to_string_lossy().into_owned())
}
pub fn remote_workspace(home: &Path, provider: &str) -> Option<PathBuf> {
    if !home.is_absolute() || provider.is_empty() || provider.contains(['/', '\\', ':']) || provider == "." || provider == ".." { return None; }
    Some(workspace_for_home(home).join("remote_workspace").join(provider))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn mac_bundle_resources_are_distinct_from_install_data_and_workspace() {
        let t = tempfile::tempdir().unwrap(); let r = t.path();
        let p = PathManager::from_roots(&r.join("Code Pet.app/Contents/MacOS/code-pet"), &r.join("user"), &r.join("user/Library/Application Support"), true).unwrap();
        assert_eq!(p.install, r.join("Code Pet.app"));
        assert_eq!(p.resources, r.join("Code Pet.app/Contents/Resources"));
        assert_eq!(p.data, r.join("user/Library/Application Support/code-pet"));
        assert_eq!(p.workspace, r.join("user/.codepet"));
    }
    #[test]
    fn windows_install_and_custom_data_do_not_relocate_workspace_or_bootstrap() {
        let t = tempfile::tempdir().unwrap(); let r = t.path();
        let mut p = PathManager::from_roots(&r.join("install/code-pet.exe"), &r.join("user"), &r.join("localdata"), false).unwrap();
        p.data = data_directory(&p.data, &DataSettings { data_directory: Some(r.join("custom").display().to_string()) });
        assert_eq!(p.install, r.join("install")); assert_eq!(p.resources, p.install);
        assert_eq!(p.data, r.join("custom")); assert_eq!(p.settings_file, r.join("localdata/code-pet/config/settings.json"));
        assert_eq!(p.workspace, r.join("user/.codepet"));
    }
    #[test]
    fn settings_and_paths_share_the_bootstrap_json_without_writing_it() {
        let t = tempfile::tempdir().unwrap(); let r = t.path();
        let p = PathManager::from_roots(&r.join("install/app.exe"), &r.join("home"), &r.join("local"), false).unwrap();
        fs::create_dir_all(p.settings_file.parent().unwrap()).unwrap();
        let contents = serde_json::to_vec(&serde_json::json!({"data":{"dataDirectory":r.join("custom")},"appearance":{"theme":"dark"}})).unwrap();
        fs::write(&p.settings_file, &contents).unwrap();
        let loaded = p.clone().load_settings().unwrap();
        assert_eq!(loaded.data, r.join("custom")); assert_eq!(loaded.workspace, p.workspace);
        assert_eq!(fs::read(&p.settings_file).unwrap(), contents);
        fs::write(&p.settings_file, b"invalid json").unwrap();
        assert!(p.clone().load_settings().is_err());
        fs::write(&p.settings_file, br#"{"data":{"dataDirectory":"relative"}}"#).unwrap();
        assert!(p.load_settings().is_err());
    }
    #[test]
    fn bootstrap_ignores_old_settings_and_keeps_default_data_outside_config() {
        let t = tempfile::tempdir().unwrap();
        let r = t.path();
        let p = PathManager::from_roots(&r.join("install/app.exe"), &r.join("home"), &r.join("local"), false).unwrap();
        fs::create_dir_all(&p.data).unwrap();
        fs::write(p.data.join("settings.json"), "invalid old settings").unwrap();
        assert_eq!(p.clone().load_settings().unwrap().data, p.data);
        fs::create_dir_all(p.settings_file.parent().unwrap()).unwrap();
        fs::write(&p.settings_file, b"{}").unwrap();
        assert_eq!(p.clone().load_settings().unwrap().data, p.data);
        assert_eq!(p.webcontent(), p.data.join("versions/webcontent"));
        assert_eq!(p.hooks(), p.data.join("config/hooks"));
    }
    #[test]
    fn provider_workspace_cannot_escape_managed_root() {
        let t = tempfile::tempdir().unwrap();
        assert_eq!(remote_workspace(t.path(), "codex"), Some(t.path().join(".codepet/remote_workspace/codex")));
        for value in ["../escape", "a/b", "a\\b", "", ".."] { assert!(remote_workspace(t.path(), value).is_none()); }
    }
}
