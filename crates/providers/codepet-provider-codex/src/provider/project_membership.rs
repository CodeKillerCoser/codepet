use super::*;
use rusqlite::{Connection, OpenFlags};
use std::collections::{BTreeMap, BTreeSet};

const ASSIGNMENTS_KEY: &str = "thread-project-assignments";
const PROJECT_IDS_KEY: &str = "app-server-project-id-by-legacy-project-id-by-host";
const PROJECTLESS_KEY: &str = "projectless-thread-ids";
const ROOT_HINTS_KEY: &str = "thread-workspace-root-hints";
const OWNER_ROOT_HINTS_PREFIX: &str = "electron-owner-value:thread-workspace-root-hints:";

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum CodexConversationMembership {
    Project(String),
    UnmappedProject,
    Standalone,
}

#[derive(Clone, Debug, Default)]
pub(super) struct CodexDesktopProjectAssignments {
    pub(super) by_thread: HashMap<String, Option<String>>,
    pub(super) other_host_threads: BTreeSet<String>,
    pub(super) projectless: BTreeSet<String>,
    pub(super) root_hints: BTreeMap<String, String>,
}

impl CodexDesktopProjectAssignments {
    pub(super) fn membership(
        &self,
        snapshot: &CodexConversationSnapshot,
    ) -> CodexConversationMembership {
        self.membership_for(&snapshot.thread.id, snapshot.thread.project_id.as_deref())
    }

    pub(super) fn membership_for(
        &self,
        thread_id: &str,
        native_project_id: Option<&str>,
    ) -> CodexConversationMembership {
        if let Some(project_id) = native_project_id.filter(|project_id| !project_id.is_empty()) {
            return CodexConversationMembership::Project(project_id.to_string());
        }
        match self.by_thread.get(thread_id) {
            Some(Some(project_id)) => CodexConversationMembership::Project(project_id.clone()),
            Some(None) => CodexConversationMembership::UnmappedProject,
            None => CodexConversationMembership::Standalone,
        }
    }

    pub(super) fn decorate(
        &self,
        snapshot: &mut CodexConversationSnapshot,
    ) -> CodexConversationMembership {
        let membership = self.membership(snapshot);
        if let CodexConversationMembership::Project(project_id) = &membership {
            snapshot.thread.project_id = Some(project_id.clone());
        }
        membership
    }
}

impl CodexInstanceRuntime {
    pub(super) async fn desktop_project_assignments(
        &self,
        request: &ConversationListRequest,
    ) -> Result<CodexDesktopProjectAssignments, ProtocolError> {
        // Continuation pages reuse the assignments captured by their cursor.
        if request.cursor.is_some() {
            return Ok(CodexDesktopProjectAssignments::default());
        }
        let directory = self.settings.data_directory.clone();
        tokio::task::spawn_blocking(move || {
            load_codex_desktop_project_assignments(directory.as_deref())
        })
        .await
        .map_err(provider_task_error)?
    }
}

pub(super) fn load_codex_desktop_project_assignments(
    configured: Option<&Path>,
) -> Result<CodexDesktopProjectAssignments, ProtocolError> {
    let Some(codex_home) = configured.map(Path::to_path_buf).or_else(codex_home) else {
        return Ok(CodexDesktopProjectAssignments::default());
    };
    let database = codex_home.join(".codex-global-state.sqlite");
    let state = if database
        .try_exists()
        .map_err(assignment_state_unavailable)?
    {
        // The JSON left behind by Desktop migration is a stale snapshot.
        read_sqlite_assignment_state(&database)?
    } else {
        match std::fs::read_to_string(codex_home.join(".codex-global-state.json")) {
            Ok(contents) => serde_json::from_str(&contents).map_err(assignment_state_invalid)?,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(CodexDesktopProjectAssignments::default());
            }
            Err(error) => return Err(assignment_state_unavailable(error)),
        }
    };
    if !state.is_object()
        || [ASSIGNMENTS_KEY, PROJECT_IDS_KEY]
            .iter()
            .any(|key| state.get(*key).is_some_and(|value| !value.is_object()))
    {
        return Err(assignment_state_invalid(
            "expected project assignment objects",
        ));
    }
    Ok(parse_codex_desktop_project_assignments(&state, &codex_home))
}

fn read_sqlite_assignment_state(path: &Path) -> Result<Value, ProtocolError> {
    let connection = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(assignment_state_unavailable)?;
    connection
        .busy_timeout(Duration::from_millis(500))
        .map_err(assignment_state_unavailable)?;
    let mut statement = connection
        .prepare(
            "SELECT key, value FROM settings WHERE scope = 'global'
             AND (key IN (?1, ?2, ?3, ?4) OR substr(key, 1, length(?5)) = ?5)",
        )
        .map_err(assignment_state_unavailable)?;
    let rows = statement
        .query_map(
            [
                ASSIGNMENTS_KEY,
                PROJECT_IDS_KEY,
                PROJECTLESS_KEY,
                ROOT_HINTS_KEY,
                OWNER_ROOT_HINTS_PREFIX,
            ],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
        )
        .map_err(assignment_state_unavailable)?;
    let mut state = serde_json::Map::new();
    for row in rows {
        let (key, contents) = row.map_err(assignment_state_unavailable)?;
        state.insert(
            key,
            serde_json::from_str(&contents).map_err(assignment_state_invalid)?,
        );
    }
    Ok(Value::Object(state))
}

fn assignment_state_unavailable(error: impl std::fmt::Display) -> ProtocolError {
    protocol_error(
        "project_assignment_state_unavailable",
        format!("failed to read Codex Desktop project assignments: {error}"),
        true,
    )
}

fn assignment_state_invalid(error: impl std::fmt::Display) -> ProtocolError {
    protocol_error(
        "project_assignment_state_invalid",
        format!("failed to parse Codex Desktop project assignments: {error}"),
        false,
    )
}

fn parse_codex_desktop_project_assignments(
    state: &Value,
    codex_home: &Path,
) -> CodexDesktopProjectAssignments {
    let host_key = format!("local:{}", codex_home.to_string_lossy());
    let native_projects = state
        .get(PROJECT_IDS_KEY)
        .and_then(Value::as_object)
        .and_then(|hosts| hosts.get(&host_key))
        .and_then(Value::as_object);
    let mut result = CodexDesktopProjectAssignments::default();
    if let Some(assignments) = state.get(ASSIGNMENTS_KEY).and_then(Value::as_object) {
        for (thread_id, assignment) in assignments {
            if assignment.get("projectKind").and_then(Value::as_str) != Some("local") {
                result.other_host_threads.insert(thread_id.clone());
                continue;
            }
            let Some(legacy_project_id) = assignment.get("projectId").and_then(Value::as_str)
            else {
                continue;
            };
            let native_project_id = native_projects
                .and_then(|projects| projects.get(legacy_project_id))
                .and_then(Value::as_str)
                .map(str::to_string);
            result
                .by_thread
                .insert(thread_id.clone(), native_project_id);
        }
    }
    if let Some(projectless) = state.get(PROJECTLESS_KEY).and_then(Value::as_array) {
        result.projectless.extend(
            projectless
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_owned),
        );
    }
    let mut hints = BTreeMap::<String, Option<String>>::new();
    for (key, value) in state.as_object().into_iter().flatten() {
        if key != ROOT_HINTS_KEY && !key.starts_with(OWNER_ROOT_HINTS_PREFIX) {
            continue;
        }
        for (thread_id, root) in value.as_object().into_iter().flatten() {
            if let Some(root) = root.as_str().filter(|root| Path::new(root).is_absolute()) {
                hints
                    .entry(thread_id.clone())
                    .and_modify(|known| {
                        if known.as_deref() != Some(root) {
                            *known = None;
                        }
                    })
                    .or_insert_with(|| Some(root.to_owned()));
            }
        }
    }
    result.root_hints.extend(
        hints
            .into_iter()
            .filter_map(|(id, root)| root.map(|root| (id, root))),
    );
    result
}

#[cfg(test)]
mod project_assignment_compat_tests {
    use super::*;

    #[test]
    fn present_sqlite_state_does_not_fall_back_to_stale_json() {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path();
        std::fs::write(
            home.join(".codex-global-state.json"),
            json!({ ASSIGNMENTS_KEY: {
                "removed": { "projectKind": "local", "projectId": "legacy" }
            } })
            .to_string(),
        )
        .unwrap();
        let load = || load_codex_desktop_project_assignments(Some(home));
        assert_eq!(load().unwrap().by_thread.get("removed"), Some(&None));

        let current = Connection::open(home.join(".codex-global-state.sqlite")).unwrap();
        current
            .execute_batch("CREATE TABLE settings(scope TEXT, key TEXT, value TEXT);")
            .unwrap();
        // Another scope is irrelevant, even if it has a matching key.
        current
            .execute(
                "INSERT INTO settings VALUES ('atom', ?1, 'invalid json')",
                [ASSIGNMENTS_KEY],
            )
            .unwrap();
        assert!(load().unwrap().by_thread.is_empty());

        current
            .execute(
                "INSERT INTO settings VALUES ('global', ?1, 'invalid json')",
                [ASSIGNMENTS_KEY],
            )
            .unwrap();
        assert_eq!(load().unwrap_err().code, "project_assignment_state_invalid");

        current.execute_batch("DROP TABLE settings;").unwrap();
        assert_eq!(
            load().unwrap_err().code,
            "project_assignment_state_unavailable"
        );
    }

    #[test]
    fn parses_only_local_assignments_and_maps_legacy_projects_for_the_current_host() {
        let assignments = parse_codex_desktop_project_assignments(
            &json!({
                "app-server-project-id-by-legacy-project-id-by-host": {
                    "local:/fixture/.codex": {
                        "legacy-project": "native-project"
                    },
                    "remote-host": {
                        "legacy-project": "wrong-native-project"
                    }
                },
                "thread-project-assignments": {
                    "mapped-thread": {
                        "projectKind": "local",
                        "projectId": "legacy-project"
                    },
                    "unmapped-thread": {
                        "projectKind": "local",
                        "projectId": "missing-project"
                    },
                    "remote-thread": {
                        "projectKind": "remote",
                        "projectId": "legacy-project"
                    }
                }
            }),
            Path::new("/fixture/.codex"),
        );

        assert_eq!(
            assignments.by_thread.get("mapped-thread"),
            Some(&Some("native-project".to_string()))
        );
        assert_eq!(assignments.by_thread.get("unmapped-thread"), Some(&None));
        assert!(!assignments.by_thread.contains_key("remote-thread"));
    }

    #[test]
    fn native_membership_wins_and_unmapped_legacy_projects_are_not_standalone() {
        let assignments = CodexDesktopProjectAssignments {
            by_thread: HashMap::from([
                (
                    "mapped-thread".to_string(),
                    Some("legacy-native-project".to_string()),
                ),
                ("unmapped-thread".to_string(), None),
            ]),
            ..Default::default()
        };

        assert_eq!(
            assignments.membership_for("mapped-thread", Some("app-server-project")),
            CodexConversationMembership::Project("app-server-project".to_string())
        );
        assert_eq!(
            assignments.membership_for("mapped-thread", None),
            CodexConversationMembership::Project("legacy-native-project".to_string())
        );
        assert_eq!(
            assignments.membership_for("unmapped-thread", None),
            CodexConversationMembership::UnmappedProject
        );
        assert_eq!(
            assignments.membership_for("standalone-thread", None),
            CodexConversationMembership::Standalone
        );
    }
}
