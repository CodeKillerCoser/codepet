use code_pet_lib::autostart::{launch_agent_plist, set_launch_agent_enabled_at};

#[test]
fn launch_agent_plist_opens_the_app_bundle_at_login() {
    let plist = launch_agent_plist(std::path::Path::new("/Applications/Code Pet.app"));

    assert!(plist.contains("<key>Label</key>"));
    assert!(plist.contains("<string>com.codepet.desktop</string>"));
    assert!(plist.contains("<string>/usr/bin/open</string>"));
    assert!(plist.contains("<string>/Applications/Code Pet.app</string>"));
    assert!(plist.contains("<key>RunAtLoad</key>"));
    assert!(plist.contains("<true/>"));
}

#[test]
fn set_launch_agent_enabled_writes_and_removes_the_plist() {
    let temp = tempfile::tempdir().unwrap();
    let plist_path = temp.path().join("com.codepet.desktop.plist");
    let bundle_path = temp.path().join("Code Pet.app");

    assert!(set_launch_agent_enabled_at(&plist_path, &bundle_path, true).unwrap());
    let text = std::fs::read_to_string(&plist_path).unwrap();
    assert!(text.contains(&bundle_path.display().to_string()));

    assert!(!set_launch_agent_enabled_at(&plist_path, &bundle_path, false).unwrap());
    assert!(!plist_path.exists());
}
