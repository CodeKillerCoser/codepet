use std::fs;
use std::io;
use std::path::{Path, PathBuf};

const LAUNCH_AGENT_LABEL: &str = "com.codepet.desktop";

pub fn launch_agent_plist(bundle_path: &Path) -> String {
    let bundle_path = escape_xml(&bundle_path.display().to_string());
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key>
  <string>{LAUNCH_AGENT_LABEL}</string>
  <key>ProgramArguments</key>
  <array>
    <string>/usr/bin/open</string>
    <string>-a</string>
    <string>{bundle_path}</string>
  </array>
  <key>RunAtLoad</key>
  <true/>
</dict>
</plist>
"#
    )
}

pub fn launch_agent_enabled() -> bool {
    default_launch_agent_path().is_some_and(|path| path.exists())
}

pub fn set_launch_agent_enabled(enabled: bool) -> io::Result<bool> {
    let plist_path = default_launch_agent_path().ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "home directory not found"))?;
    let bundle_path = current_app_bundle_path().unwrap_or_else(|| PathBuf::from("/Applications/Code Pet.app"));
    set_launch_agent_enabled_at(&plist_path, &bundle_path, enabled)
}

pub fn set_launch_agent_enabled_at(plist_path: &Path, bundle_path: &Path, enabled: bool) -> io::Result<bool> {
    if enabled {
        if let Some(parent) = plist_path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(plist_path, launch_agent_plist(bundle_path))?;
        return Ok(true);
    }

    if plist_path.exists() {
        fs::remove_file(plist_path)?;
    }
    Ok(false)
}

fn default_launch_agent_path() -> Option<PathBuf> {
    Some(dirs::home_dir()?.join("Library").join("LaunchAgents").join(format!("{LAUNCH_AGENT_LABEL}.plist")))
}

fn current_app_bundle_path() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let macos_dir = exe.parent()?;
    let contents_dir = macos_dir.parent()?;
    let app_dir = contents_dir.parent()?;
    (app_dir.extension().and_then(|extension| extension.to_str()) == Some("app")).then(|| app_dir.to_path_buf())
}

fn escape_xml(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}
