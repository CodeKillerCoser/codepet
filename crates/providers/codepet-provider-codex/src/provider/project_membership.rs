use super::*;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum CodexConversationMembership {
    Project(String),
    UnmappedProject,
    Standalone,
}

#[derive(Clone, Debug, Default)]
pub(super) struct CodexDesktopProjectAssignments {
    pub(super) by_thread: HashMap<String, Option<String>>,
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

pub(super) fn load_codex_desktop_project_assignments(
    configured: Option<&Path>,
) -> Result<CodexDesktopProjectAssignments, ProtocolError> {
    let Some(codex_home) = configured.map(Path::to_path_buf).or_else(codex_home) else {
        return Ok(CodexDesktopProjectAssignments::default());
    };
    let path = codex_home.join(".codex-global-state.json");
    let contents = match std::fs::read_to_string(&path) {
        Ok(contents) => contents,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(CodexDesktopProjectAssignments::default());
        }
        Err(error) => {
            return Err(protocol_error(
                "project_assignment_state_unavailable",
                format!("failed to read Codex Desktop project assignments: {error}"),
                true,
            ));
        }
    };
    parse_codex_desktop_project_assignments(&contents, &codex_home).map_err(|error| {
        protocol_error(
            "project_assignment_state_invalid",
            format!("failed to parse Codex Desktop project assignments: {error}"),
            false,
        )
    })
}

pub(super) fn parse_codex_desktop_project_assignments(
    contents: &str,
    codex_home: &Path,
) -> Result<CodexDesktopProjectAssignments, serde_json::Error> {
    let state: Value = serde_json::from_str(contents)?;
    let host_key = format!("local:{}", codex_home.to_string_lossy());
    let native_projects = state
        .get("app-server-project-id-by-legacy-project-id-by-host")
        .and_then(Value::as_object)
        .and_then(|hosts| hosts.get(&host_key))
        .and_then(Value::as_object);
    let mut by_thread = HashMap::new();
    if let Some(assignments) = state
        .get("thread-project-assignments")
        .and_then(Value::as_object)
    {
        for (thread_id, assignment) in assignments {
            if assignment.get("projectKind").and_then(Value::as_str) != Some("local") {
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
            by_thread.insert(thread_id.clone(), native_project_id);
        }
    }
    Ok(CodexDesktopProjectAssignments { by_thread })
}

#[cfg(test)]
mod project_assignment_compat_tests {
    use super::*;

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
            })
            .to_string(),
            Path::new("/fixture/.codex"),
        )
        .unwrap();

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
