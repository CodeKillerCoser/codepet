use code_pet_lib::title_resolver::{resolve_claude_title_from_projects, resolve_qoder_title_from_projects};
use std::fs;
use tempfile::tempdir;

#[test]
fn resolves_claude_title_from_sessions_index_first_prompt() {
    let dir = tempdir().unwrap();
    let project = dir.path().join("-Users-wangxin-Documents-code-pet");
    fs::create_dir_all(&project).unwrap();
    fs::write(
        project.join("sessions-index.json"),
        r#"[{"sessionId":"claude-hello","firstPrompt":"hellp","fullPath":"/Users/wangxin/Documents/code-pet"}]"#,
    )
    .unwrap();

    let title = resolve_claude_title_from_projects(dir.path(), "claude-hello");

    assert_eq!(title.as_deref(), Some("hellp"));
}

#[test]
fn resolves_qoder_title_from_session_file() {
    let dir = tempdir().unwrap();
    let project = dir.path().join("workspace").join("sessions");
    fs::create_dir_all(&project).unwrap();
    fs::write(
        project.join("qoder-weather-session.json"),
        r#"{"id":"qoder-weather","title":"无法获取实时天气信息","updated_at":1760000000000}"#,
    )
    .unwrap();

    let title = resolve_qoder_title_from_projects(dir.path(), "qoder-weather");

    assert_eq!(title.as_deref(), Some("无法获取实时天气信息"));
}

#[test]
fn ignores_qoder_placeholder_titles() {
    let dir = tempdir().unwrap();
    fs::create_dir_all(dir.path()).unwrap();
    fs::write(
        dir.path().join("qoder-placeholder-session.json"),
        r#"{"id":"qoder-placeholder","title":"New Session"}"#,
    )
    .unwrap();

    let title = resolve_qoder_title_from_projects(dir.path(), "qoder-placeholder");

    assert_eq!(title, None);
}
