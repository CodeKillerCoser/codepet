//! Update delivery follows the published artifacts: a manual DMG on macOS,
//! signed Tauri installers on the other desktop platforms.
use std::time::Duration;
use tauri::AppHandle;
use tauri_plugin_updater::Update;

#[derive(Clone)]
pub enum PendingUpdate {
    Automatic(Update),
    Manual(ManualUpdate),
}

#[derive(Clone, Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppUpdateMetadata {
    pub version: String,
    pub current_version: String,
    pub manual_download: bool,
}

#[derive(Clone, Debug)]
pub struct ManualUpdate {
    pub metadata: AppUpdateMetadata,
    download_url: String,
}

impl PendingUpdate {
    pub fn metadata(&self) -> AppUpdateMetadata {
        match self {
            Self::Automatic(update) => AppUpdateMetadata {
                version: update.version.clone(),
                current_version: update.current_version.clone(),
                manual_download: false,
            },
            Self::Manual(update) => update.metadata.clone(),
        }
    }
}

pub async fn install(app: &AppHandle, update: &PendingUpdate) -> Result<(), String> {
    match update {
        PendingUpdate::Automatic(update) => {
            update
                .download_and_install(|_, _| {}, || {})
                .await
                .map_err(|error| error.to_string())?;
            app.restart()
        }
        PendingUpdate::Manual(update) => {
            use tauri_plugin_opener::OpenerExt;
            app.opener()
                .open_url(&update.download_url, None::<&str>)
                .map_err(|error| format!("无法打开安装包下载地址：{error}"))
        }
    }
}

#[cfg(not(target_os = "macos"))]
pub async fn check(app: &AppHandle) -> Result<Option<PendingUpdate>, String> {
    use tauri_plugin_updater::UpdaterExt;
    app.updater_builder()
        .timeout(Duration::from_secs(20))
        .build()
        .map_err(|error| error.to_string())?
        .check()
        .await
        .map(|update| update.map(PendingUpdate::Automatic))
        .map_err(|error| error.to_string())
}

#[cfg(target_os = "macos")]
pub async fn check(app: &AppHandle) -> Result<Option<PendingUpdate>, String> {
    check_mac_release(&app.package_info().version, std::env::consts::ARCH)
        .await
        .map(|update| update.map(PendingUpdate::Manual))
}

#[cfg(any(target_os = "macos", test))]
async fn check_mac_release(
    current: &semver::Version,
    arch: &str,
) -> Result<Option<ManualUpdate>, String> {
    let release = reqwest::Client::builder()
        .timeout(Duration::from_secs(20))
        .user_agent(format!("CodePet/{}", current))
        .build()
        .map_err(|error| error.to_string())?
        .get("https://api.github.com/repos/CodeKillerCoser/codepet/releases/latest")
        .header(reqwest::header::ACCEPT, "application/vnd.github+json")
        .send()
        .await
        .and_then(reqwest::Response::error_for_status)
        .map_err(|error| format!("无法获取最新发行版：{error}"))?
        .json::<Release>()
        .await
        .map_err(|error| format!("发行版信息无效：{error}"))?;
    mac_update(&release, current, arch)
}

#[cfg(any(target_os = "macos", test))]
#[derive(serde::Deserialize)]
struct Release {
    tag_name: String,
    draft: bool,
    assets: Vec<Asset>,
}

#[cfg(any(target_os = "macos", test))]
#[derive(serde::Deserialize)]
struct Asset {
    name: String,
    state: String,
    size: u64,
}

#[cfg(any(target_os = "macos", test))]
fn mac_update(
    release: &Release,
    current: &semver::Version,
    arch: &str,
) -> Result<Option<ManualUpdate>, String> {
    let version = semver::Version::parse(
        release
            .tag_name
            .strip_prefix('v')
            .unwrap_or(&release.tag_name),
    )
    .map_err(|error| format!("发行版版本号无效：{error}"))?;
    // Build metadata identifies a build; it does not change SemVer precedence.
    if release.draft || !version.cmp_precedence(current).is_gt() {
        return Ok(None);
    }
    let architecture = match arch {
        "aarch64" => "aarch64",
        "x86_64" => "x64",
        _ => return Err("当前 Mac 架构暂不支持此更新".into()),
    };
    let asset = [architecture, "universal"]
        .into_iter()
        .find_map(|suffix| {
            let platform = match suffix {
                "aarch64" => "arm64",
                "x64" => "x86_64",
                _ => suffix,
            };
            let mut base_version = version.clone();
            base_version.build = semver::BuildMetadata::EMPTY;
            let normalized_name = format!(
                "codepet-macos-{platform}_{base_version}_{}.dmg",
                version.build
            );
            release.assets.iter().find(|asset| {
                asset.state == "uploaded"
                    && asset.size > 0
                    && [
                        format!("Code.Pet_{version}_{suffix}.dmg"),
                        format!("Code Pet_{version}_{suffix}.dmg"),
                        normalized_name.clone(),
                    ]
                    .contains(&asset.name)
            })
        })
        .ok_or("新版本暂未提供适用于当前 Mac 的 DMG 安装包，请稍后重试")?;
    // Build the URL from our repository and validated asset name, never an arbitrary response URL.
    let mut url =
        url::Url::parse("https://github.com/CodeKillerCoser/codepet/releases/download/").unwrap();
    url.path_segments_mut()
        .unwrap()
        .pop_if_empty()
        .push(&release.tag_name)
        .push(&asset.name);
    Ok(Some(ManualUpdate {
        metadata: AppUpdateMetadata {
            version: version.to_string(),
            current_version: current.to_string(),
            manual_download: true,
        },
        download_url: url.to_string(),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn release() -> Release {
        serde_json::from_value(serde_json::json!({"tag_name":"v0.3.10-beta+a68ec0c", "draft":false,
            "assets":[{"name":"Code.Pet_0.3.10-beta+a68ec0c_aarch64.dmg", "state":"uploaded", "size":68877049}]})).unwrap()
    }
    fn version(value: &str) -> semver::Version {
        value.parse().unwrap()
    }
    #[test]
    fn dmg_only_release_is_a_manual_update_without_a_universal_updater() {
        let update = mac_update(
            &release(),
            &version("0.3.9-beta+menubar.20261109.9"),
            "aarch64",
        )
        .unwrap()
        .unwrap();
        assert!(update.metadata.manual_download);
        assert_eq!(update.metadata.version, "0.3.10-beta+a68ec0c");
        assert_eq!(update.download_url, "https://github.com/CodeKillerCoser/codepet/releases/download/v0.3.10-beta+a68ec0c/Code.Pet_0.3.10-beta+a68ec0c_aarch64.dmg");
    }
    #[test]
    fn same_version_different_build_and_older_releases_are_not_updates() {
        for current in ["0.3.10-beta+another-build", "0.3.10", "0.3.11-beta"] {
            assert!(mac_update(&release(), &version(current), "aarch64")
                .unwrap()
                .is_none());
        }
    }
    #[test]
    fn missing_or_incomplete_dmg_is_not_reported_as_up_to_date() {
        let mut release = release();
        release.assets[0].state = "new".into();
        assert!(mac_update(&release, &version("0.3.9-beta"), "aarch64").is_err());
        release.assets.clear();
        assert!(mac_update(&release, &version("0.3.9-beta"), "aarch64").is_err());
    }
    #[test]
    fn intel_mac_must_not_receive_an_arm_dmg() {
        let mut release = release();
        assert!(mac_update(&release, &version("0.3.9-beta"), "x86_64").is_err());
        release.assets[0].name = "Code.Pet_0.3.10-beta+a68ec0c_universal.dmg".into();
        assert!(mac_update(&release, &version("0.3.9-beta"), "x86_64")
            .unwrap()
            .is_some());
    }
    #[test]
    fn malformed_versions_and_wrong_version_assets_are_rejected() {
        let mut release = release();
        release.assets[0].name = "Code.Pet_0.3.9-beta_aarch64.dmg".into();
        assert!(mac_update(&release, &version("0.3.9-beta"), "aarch64").is_err());
        release.tag_name = "not-a-version".into();
        assert!(mac_update(&release, &version("0.3.9-beta"), "aarch64").is_err());
    }
    #[test]
    fn normalized_build_asset_matches_platform_version_and_commit() {
        let mut release = release();
        release.assets[0].name = "codepet-macos-arm64_0.3.10-beta_a68ec0c.dmg".into();
        let update = mac_update(&release, &version("0.3.9-beta"), "aarch64")
            .unwrap()
            .unwrap();
        assert!(update
            .download_url
            .ends_with("/codepet-macos-arm64_0.3.10-beta_a68ec0c.dmg"));
        assert!(mac_update(&release, &version("0.3.9-beta"), "x86_64").is_err());
        release.assets[0].name = "codepet-macos-arm64_0.3.10-beta_aaaaaaa.dmg".into();
        assert!(mac_update(&release, &version("0.3.9-beta"), "aarch64").is_err());
    }
    #[tokio::test]
    #[ignore = "fetches the public GitHub release; run explicitly for release smoke checks"]
    async fn live_dmg_check() {
        let update = check_mac_release(&version("0.3.9-beta+menubar.20261109.9"), "aarch64")
            .await
            .unwrap()
            .unwrap();
        println!(
            "Live update: {} -> {} ({})",
            update.metadata.current_version, update.metadata.version, update.download_url
        );
        assert!(update.metadata.manual_download);
        assert!(
            check_mac_release(&version(&update.metadata.version), "aarch64")
                .await
                .unwrap()
                .is_none()
        );
    }
    #[test]
    fn published_manifest_reproduces_the_old_target_error() {
        let release: tauri_plugin_updater::RemoteRelease = serde_json::from_value(serde_json::json!({
            "version":"0.3.10-beta+a68ec0c", "platforms":{"windows-x86_64":{"url":"https://github.com/example/setup.exe", "signature":"signed"}}
        })).unwrap();
        assert_eq!(
            release
                .download_url("macos-universal")
                .unwrap_err()
                .to_string(),
            "the platform `macos-universal` was not found in the response `platforms` object"
        );
    }
}
