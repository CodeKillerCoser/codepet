//! Application-owned configuration, installed skills, and isolated workspaces.
use crate::{
    domain::{Message, Task},
    extraction::{ClaudeExtractor, ExtractionResult, TaskExtractor},
    stable_id,
    store::Store,
    Result,
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
};

const SKILLS: &[(&str, &str)] = &[
    (
        "extract-tasks",
        include_str!("../skills/extract-tasks/SKILL.md"),
    ),
    (
        "reconcile-tasks",
        include_str!("../skills/reconcile-tasks/SKILL.md"),
    ),
];

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default, deny_unknown_fields)]
pub struct ExtractionSettings {
    pub revision: u64,
    pub harness: String,
    pub harness_instance_id: Option<String>,
    pub model: String,
    pub skill: String,
    pub prompt: String,
    pub budget_usd: f64,
    pub timeout_seconds: u64,
    pub interval_seconds: u64,
    pub debounce_seconds: u64,
}
impl Default for ExtractionSettings {
    fn default() -> Self {
        Self {
            revision: 0,
            harness: "claude".into(),
            harness_instance_id: None,
            model: "haiku".into(),
            skill: "extract-tasks".into(),
            prompt: String::new(),
            budget_usd: 0.25,
            timeout_seconds: 90,
            interval_seconds: 60,
            debounce_seconds: 20,
        }
    }
}
pub fn settings(store: &Store) -> Result<ExtractionSettings> {
    Ok(store.read("extraction-settings.json")?.unwrap_or_default())
}
pub fn save_settings(
    store: &Store,
    mut value: ExtractionSettings,
    layout: &Layout,
) -> Result<ExtractionSettings> {
    if value.revision != settings(store)?.revision {
        return Err("抽取配置已变更，请刷新后重试".into());
    }
    if value.harness != "claude"
        || value.model.trim().is_empty()
        || value.model.len() > 120
        || !value.budget_usd.is_finite()
        || !(0.01..=5.0).contains(&value.budget_usd)
        || !(15..=300).contains(&value.timeout_seconds)
        || !(15..=86400).contains(&value.interval_seconds)
        || value.debounce_seconds > 3600
        || value.prompt.chars().count() > 8000
    {
        return Err("抽取模型、预算或定时配置无效".into());
    }
    layout.read_skill(&value.skill)?;
    value.revision += 1;
    store.write("extraction-settings.json", &value)?;
    Ok(value)
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Layout {
    pub root: PathBuf,
    pub skills: PathBuf,
    pub extraction_workspaces: PathBuf,
    pub task_workspaces: PathBuf,
}
fn safe_component(value: &str) -> Result<()> {
    if value.is_empty()
        || value.len() > 64
        || !value
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
    {
        return Err("Invalid skill name".into());
    }
    Ok(())
}
fn bounded(root: &Path, path: &Path) -> Result<()> {
    if !root.is_absolute() || !path.starts_with(root) {
        return Err("工作目录必须位于配置的数据目录内".into());
    }
    let mut current = root.to_owned();
    if fs::symlink_metadata(root).is_ok_and(|m| m.file_type().is_symlink()) {
        return Err("数据目录不允许符号链接".into());
    }
    for component in path
        .strip_prefix(root)
        .map_err(|e| e.to_string())?
        .components()
    {
        if !matches!(component, std::path::Component::Normal(_)) {
            return Err("非法工作目录".into());
        }
        current.push(component);
        if fs::symlink_metadata(&current).is_ok_and(|m| m.file_type().is_symlink()) {
            return Err("工作目录不允许符号链接".into());
        }
    }
    Ok(())
}
pub fn write_atomic(path: &Path, value: &[u8]) -> Result<()> {
    let parent = path.parent().ok_or("Missing parent")?;
    fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    let mut file = tempfile::NamedTempFile::new_in(parent).map_err(|e| e.to_string())?;
    file.write_all(value).map_err(|e| e.to_string())?;
    file.as_file().sync_all().map_err(|e| e.to_string())?;
    file.persist(path).map_err(|e| e.to_string())?;
    Ok(())
}
impl Layout {
    pub fn initialize(root: &Path) -> Result<Self> {
        let layout = Self {
            root: root.into(),
            skills: root.join("skills"),
            extraction_workspaces: root.join("workspaces/task-extraction"),
            task_workspaces: root.join("workspaces/tasks"),
        };
        for path in [
            &layout.skills,
            &layout.extraction_workspaces,
            &layout.task_workspaces,
        ] {
            bounded(root, path)?;
            fs::create_dir_all(path).map_err(|e| e.to_string())?;
        }
        for (name, body) in SKILLS {
            let path = layout.skills.join(name).join("SKILL.md");
            bounded(root, &path)?;
            fs::create_dir_all(path.parent().unwrap()).map_err(|e| e.to_string())?;
            match fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&path)
            {
                Ok(mut file) => file.write_all(body.as_bytes()).map_err(|e| e.to_string())?,
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
                Err(error) => return Err(error.to_string()),
            }
        }
        Ok(layout)
    }
    pub fn read_skill(&self, name: &str) -> Result<String> {
        safe_component(name)?;
        let path = self.skills.join(name).join("SKILL.md");
        bounded(&self.root, &path)?;
        if fs::metadata(&path).map_err(|e| e.to_string())?.len() > 65536 {
            return Err("Skill exceeds 64 KiB".into());
        }
        let text = fs::read_to_string(path).map_err(|e| e.to_string())?;
        if !text.starts_with("---") || !text.contains("description:") {
            return Err("Skill requires frontmatter and description".into());
        }
        Ok(text)
    }
    pub fn list_skills(&self) -> Result<Vec<String>> {
        let mut names = vec![];
        for entry in fs::read_dir(&self.skills).map_err(|e| e.to_string())? {
            let entry = entry.map_err(|e| e.to_string())?;
            let name = entry.file_name().to_string_lossy().into_owned();
            if self.read_skill(&name).is_ok() {
                names.push(name);
            }
        }
        names.sort();
        Ok(names)
    }
    pub fn allocate_task(&self, provider_id: &str, task: &Task) -> Result<PathBuf> {
        let path = self
            .task_workspaces
            .join(stable_id(format!("{provider_id}:{}", task.id)));
        bounded(&self.root, &path)?;
        fs::create_dir_all(&path).map_err(|e| e.to_string())?;
        write_atomic(
            &path.join("task.json"),
            &serde_json::to_vec_pretty(task).map_err(|e| e.to_string())?,
        )?;
        write_atomic(&path.join("workspace.json"), &serde_json::to_vec_pretty(&json!({"providerId":provider_id,"taskId":task.id,"skillsDirectory":self.skills,"kind":"task","schemaVersion":1})).map_err(|e|e.to_string())?)?;
        Ok(path)
    }
}

pub struct ManagedExtractor {
    pub inner: ClaudeExtractor,
    pub layout: Layout,
    pub config: ExtractionSettings,
}
impl TaskExtractor for ManagedExtractor {
    fn version(&self) -> String {
        format!(
            "{}:settings-{}:{}",
            self.inner.version(),
            self.config.revision,
            self.layout
                .read_skill(&self.config.skill)
                .map(stable_id)
                .unwrap_or_default()
        )
    }
    fn extract(&self, messages: &[Message], candidates: &[Task]) -> Result<ExtractionResult> {
        let now = chrono::Utc::now().timestamp_millis();
        if messages.iter().any(|m| {
            !matches!(m.role.as_str(), "user" | "assistant")
                || !crate::extraction::recent_message(m, now)
        }) {
            return Err("Only recent user/assistant text is eligible".into());
        }
        let skill = self.layout.read_skill(&self.config.skill)?;
        let workspace = tempfile::Builder::new()
            .prefix("run-")
            .tempdir_in(&self.layout.extraction_workspaces)
            .map_err(|e| e.to_string())?
            .keep();
        let prompt = format!("{skill}\n\n## 用户配置的抽取要求\n{}", self.config.prompt);
        write_atomic(&workspace.join("SKILL.md"), skill.as_bytes())?;
        write_atomic(&workspace.join("prompt.md"), prompt.as_bytes())?;
        write_atomic(
            &workspace.join("config.json"),
            &serde_json::to_vec_pretty(&self.config).map_err(|e| e.to_string())?,
        )?;
        write_atomic(
            &workspace.join("input.json"),
            &serde_json::to_vec(&json!({"messages":messages,"existingTasks":candidates}))
                .map_err(|e| e.to_string())?,
        )?;
        let outcome = self
            .inner
            .extract_in(messages, candidates, Some((&workspace, &prompt)));
        let receipt = match &outcome {
            Ok(result) => json!({"state":"completed","result":result}),
            Err(error) => json!({"state":"failed","error":error}),
        };
        write_atomic(
            &workspace.join("result.json"),
            &serde_json::to_vec(&receipt).map_err(|e| e.to_string())?,
        )?;
        outcome
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Job {
    pub id: String,
    pub request_id: String,
    pub thread_id: Option<String>,
    pub state: String,
    pub created_at: i64,
    pub finished_at: Option<i64>,
    pub error: Option<String>,
}
pub fn jobs(store: &Store) -> Result<Vec<Job>> {
    let mut values: Vec<Job> = store.list("jobs")?;
    values.sort_by_key(|j| std::cmp::Reverse(j.created_at));
    values.truncate(30);
    Ok(values)
}
pub fn job_key(id: &str) -> String {
    format!("jobs/{}.json", stable_id(id))
}
pub fn enqueue(store: &Store, request_id: &str, thread_id: Option<String>) -> Result<(Job, bool)> {
    if request_id.is_empty() || request_id.len() > 128 {
        return Err("Invalid request ID".into());
    }
    if let Some(job) = store.read::<Job>(&job_key(request_id))? {
        if job.thread_id != thread_id {
            return Err("同一请求 ID 不可用于不同抽取范围".into());
        }
        return Ok((job, false));
    }
    // Keep the lease only for admission: an active inference must never be labelled interrupted.
    let _lease = store.extraction_lease()?;
    for job in store.list::<Job>("jobs")? {
        if matches!(job.state.as_str(), "queued" | "running") {
            return Err("已有抽取任务等待或运行中".into());
        }
    }
    let job = Job {
        id: request_id.into(),
        request_id: request_id.into(),
        thread_id,
        state: "queued".into(),
        created_at: chrono::Utc::now().timestamp_millis(),
        finished_at: None,
        error: None,
    };
    store.write(&job_key(&job.id), &job)?;
    Ok((job, true))
}
pub fn recover_jobs(store: &Store) -> Result<()> {
    let _lease = store.extraction_lease()?;
    for mut job in store.list::<Job>("jobs")? {
        if matches!(job.state.as_str(), "queued" | "running") {
            job.state = "interrupted".into();
            job.error = Some("应用重启，未完成的抽取可重新触发".into());
            job.finished_at = Some(chrono::Utc::now().timestamp_millis());
            store.write(&job_key(&job.id), &job)?;
        }
    }
    Ok(())
}

pub fn capabilities() -> serde_json::Value {
    json!({"namespace":"codepet.tasks","version":1,"harnesses":["claude"],"storage":"json-journal","lookbackHours":48,"methods":["tasks.capabilities","tasks.list","tasks.messages","tasks.dirty","tasks.extract","tasks.jobs","tasks.settings.get","tasks.settings.set","tasks.skills","tasks.skill.get","tasks.workspace.request"]})
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn installed_skills_preserve_edits_and_settings_reject_stale_writes() {
        let dir = tempfile::tempdir().unwrap();
        let layout = Layout::initialize(dir.path()).unwrap();
        let path = layout.skills.join("extract-tasks/SKILL.md");
        let original = fs::read_to_string(&path).unwrap();
        fs::write(&path, format!("{original}\n用户自定义规则")).unwrap();
        Layout::initialize(dir.path()).unwrap();
        assert!(layout
            .read_skill("extract-tasks")
            .unwrap()
            .contains("用户自定义规则"));
        assert!(layout.read_skill("../extract-tasks").is_err());
        let store = Store::open(&dir.path().join("store")).unwrap();
        let config = save_settings(&store, ExtractionSettings::default(), &layout).unwrap();
        assert_eq!(config.revision, 1);
        assert!(save_settings(&store, ExtractionSettings::default(), &layout).is_err());
        assert!(save_settings(
            &store,
            ExtractionSettings {
                skill: "absent".into(),
                ..config.clone()
            },
            &layout
        )
        .is_err());
        assert!(save_settings(
            &store,
            ExtractionSettings {
                interval_seconds: 0,
                ..config.clone()
            },
            &layout
        )
        .is_err());
        assert_eq!(settings(&store).unwrap().revision, 1);
    }
    #[test]
    fn job_admission_is_idempotent_and_restart_requires_explicit_retry() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path()).unwrap();
        assert!(enqueue(&store, "request", Some("a".into())).unwrap().1);
        assert!(!enqueue(&store, "request", Some("a".into())).unwrap().1);
        assert!(enqueue(&store, "request", Some("b".into())).is_err());
        assert!(enqueue(&store, "second", None).is_err());
        let lease = store.extraction_lease().unwrap();
        assert!(recover_jobs(&store).is_err());
        drop(lease);
        recover_jobs(&store).unwrap();
        assert_eq!(jobs(&store).unwrap()[0].state, "interrupted");
        assert!(!enqueue(&store, "request", Some("a".into())).unwrap().1);
        assert!(enqueue(&store, "retry", Some("a".into())).unwrap().1);
    }
}
