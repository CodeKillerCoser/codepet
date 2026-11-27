use super::*;
use crate::directory::{
    read_page_with_project_support, repair_time, DbEvidence, DbQuery, ProjectDbSource,
};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use codepet_provider_data::conversation_state::SharedConversationStateStore;
use codepet_provider_sdk::{ConversationListQuery, ConversationProjectFilterProject};
use ring::{
    digest, hmac,
    rand::{SecureRandom, SystemRandom},
};
use serde::Serialize;
use std::{collections::BTreeMap, path::Component, sync::OnceLock};

#[derive(Debug, Default, Deserialize, Serialize)]
enum Position {
    #[default]
    Start,
    Db(u64, String),
    Native(String),
    Done,
}

#[derive(Default, Deserialize, Serialize)]
struct ProjectCursor {
    binding: String,
    native: Position,
    desktop: Position,
    desktop_first: bool,
}

struct DesktopProject {
    id: String,
    codex_worktrees: PathBuf,
    roots: Vec<PathBuf>,
    assignments: CodexDesktopProjectAssignments,
    worktree_roots: Mutex<HashMap<PathBuf, Option<PathBuf>>>,
}

impl DesktopProject {
    fn includes(&self, id: &str, cwd: Option<&str>) -> bool {
        if let Some(explicit) = self.assignments.by_thread.get(id) {
            return explicit.as_deref() == Some(self.id.as_str());
        }
        if self.assignments.projectless.contains(id)
            || self.assignments.other_host_threads.contains(id)
        {
            return false;
        }
        if self
            .assignments
            .root_hints
            .get(id)
            .is_some_and(|hint| self.matches_root(Path::new(hint)))
        {
            return true;
        }
        let Some(cwd) = cwd
            .map(Path::new)
            .filter(|path| path.is_absolute())
            .map(normalized_path)
        else {
            return false;
        };
        if self.matches_root(&cwd) {
            return true;
        }
        let Ok(relative) = cwd.strip_prefix(&self.codex_worktrees) else {
            return false;
        };
        let mut components = relative.components();
        let (Some(Component::Normal(slot)), Some(Component::Normal(checkout))) =
            (components.next(), components.next())
        else {
            return false;
        };
        // Only follow Git's registered link for a Codex-managed worktree.
        let path = self.codex_worktrees.join(slot).join(checkout);
        let mut roots = lock(&self.worktree_roots);
        roots
            .entry(path.clone())
            .or_insert_with(|| registered_workspace(&path))
            .as_deref()
            .is_some_and(|root| self.matches_root(root))
    }

    fn matches_root(&self, path: &Path) -> bool {
        let path = normalized_path(path);
        self.roots.iter().any(|root| path.starts_with(root))
    }
}

fn normalized_path(path: &Path) -> PathBuf {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::ParentDir => {
                normalized.pop();
            }
            Component::CurDir => {}
            other => normalized.push(other),
        }
    }
    #[cfg(windows)]
    return PathBuf::from(normalized.to_string_lossy().to_lowercase());
    #[cfg(not(windows))]
    normalized
}

fn registered_workspace(worktree: &Path) -> Option<PathBuf> {
    let pointer = std::fs::read_to_string(worktree.join(".git")).ok()?;
    let git_dir = worktree.join(pointer.trim().strip_prefix("gitdir: ")?);
    // Check that the registration points back to this checkout.
    let backlink = std::fs::read_to_string(git_dir.join("gitdir")).ok()?;
    if std::fs::canonicalize(git_dir.join(backlink.trim())).ok()?
        != std::fs::canonicalize(worktree.join(".git")).ok()?
    {
        return None;
    }
    let common = std::fs::read_to_string(git_dir.join("commondir")).ok()?;
    let common = std::fs::canonicalize(git_dir.join(common.trim())).ok()?;
    if common.file_name()? != ".git" {
        return None;
    }
    Some(common.parent()?.to_path_buf())
}

impl CodexInstanceRuntime {
    pub(super) async fn list_project_conversations(
        &self,
        generation: &str,
        request: &ConversationListRequest,
        filter: &ConversationProjectFilterProject,
    ) -> Result<ConversationListResponse, ProtocolError> {
        let id = &filter.project.native_resource_id;
        let home = self
            .settings
            .data_directory
            .clone()
            .or_else(codex_home)
            .ok_or_else(|| {
                protocol_error(
                    "conversation_query_incomplete",
                    "Cannot resolve Codex home".into(),
                    true,
                )
            })?;
        let server = self.ready_server()?;
        let project_id = id.clone();
        let (project, assignments) = tokio::task::spawn_blocking({
            let home = home.clone();
            move || {
                let project = server
                    .project_read(&project_id)
                    .map_err(CodexProtocolMapper::error)?;
                let assignments = load_codex_desktop_project_assignments(Some(&home))?;
                Ok::<_, ProtocolError>((project, assignments))
            }
        })
        .await
        .map_err(provider_task_error)??;
        let desktop = Arc::new(DesktopProject {
            id: id.clone(),
            codex_worktrees: normalized_path(&home.join("worktrees")),
            roots: project
                .roots
                .iter()
                .map(|root| Path::new(&root.path))
                .filter(|path| path.is_absolute())
                .map(normalized_path)
                .collect(),
            assignments,
            worktree_roots: Mutex::default(),
        });
        let binding = cursor_binding(generation, request, &desktop);
        let mut cursor = match &request.cursor {
            Some(token) => decode_cursor(token, &binding)?,
            None => ProjectCursor {
                binding,
                ..Default::default()
            },
        };
        let limit = request.limit.unwrap_or(20) as usize;
        let updated_after = match &request.query {
            Some(ConversationListQuery::ConversationUpdatedAfterQuery(query)) => {
                Some(query.updated_after)
            }
            _ => None,
        };
        let (native_limit, desktop_limit) = match limit {
            1 if matches!(cursor.native, Position::Done)
                || cursor.desktop_first && !matches!(cursor.desktop, Position::Done) =>
            {
                (0, 1)
            }
            1 => (1, 0),
            _ => (limit.div_ceil(2), limit / 2),
        };
        let (native, supplement) = tokio::try_join!(
            self.read_project_source(
                &home,
                &desktop,
                &mut cursor.native,
                native_limit,
                updated_after,
                true
            ),
            self.read_project_source(
                &home,
                &desktop,
                &mut cursor.desktop,
                desktop_limit,
                updated_after,
                false
            ),
        )?;
        let mut rows = native;
        rows.extend(supplement);
        // With one slot, allow the other source to use it when the first returns nothing.
        if limit == 1 && rows.is_empty() {
            let use_native = native_limit == 0;
            let position = if use_native {
                &mut cursor.native
            } else {
                &mut cursor.desktop
            };
            rows = self
                .read_project_source(&home, &desktop, position, 1, updated_after, use_native)
                .await?;
        }
        cursor.desktop_first = native_limit != 0;
        if generation != self.query_generation()? {
            return Err(conversation_atoms::generation_changed());
        }
        for row in &mut rows {
            self.project_conversation(row);
        }
        conversation_atoms::sort_summaries(&mut rows);
        if let Some(scope) = &request.reader_scope {
            SharedConversationStateStore::from_env()?.decorate_many(scope, &mut rows)?;
        }
        let next_cursor = if matches!(
            (&cursor.native, &cursor.desktop),
            (Position::Done, Position::Done)
        ) {
            None
        } else {
            Some(encode_cursor(&cursor)?)
        };
        Ok(ConversationListResponse {
            conversations: rows,
            page_info: PageInfo { next_cursor },
        })
    }

    async fn read_project_source(
        &self,
        home: &Path,
        project: &Arc<DesktopProject>,
        position: &mut Position,
        limit: usize,
        updated_after: Option<u64>,
        native: bool,
    ) -> Result<Vec<Conversation>, ProtocolError> {
        if limit == 0 || matches!(position, Position::Done) {
            return Ok(Vec::new());
        }
        if let Position::Native(_) = position {
            return self
                .read_legacy_project_page(project, position, limit, updated_after)
                .await;
        }
        let after = match position {
            Position::Db(time, id) => Some((*time, id.clone())),
            _ => None,
        };
        let source = if native {
            ProjectDbSource::Native(project.id.clone())
        } else {
            let project = Arc::clone(project);
            ProjectDbSource::Desktop(Arc::new(move |id, cwd| project.includes(id, cwd)))
        };
        let query = DbQuery {
            after,
            updated_after,
            project_source: Some(source),
            limit: Some(limit + 1),
            ..Default::default()
        };
        let home = home.to_path_buf();
        let (supports_native, mut entries) =
            tokio::task::spawn_blocking(move || read_page_with_project_support(&home, &query))
                .await
                .map_err(provider_task_error)??;
        if native && !supports_native {
            if matches!(position, Position::Db(..)) {
                return Err(invalid_cursor());
            }
            return self
                .read_legacy_project_page(project, position, limit, updated_after)
                .await;
        }
        let more = entries.len() > limit;
        entries.truncate(limit);
        let next = if more {
            let (id, fact) = entries.last().unwrap();
            Position::Db(fact.updated_ms.unwrap_or_default(), id.clone())
        } else {
            Position::Done
        };
        let rows = self
            .read_project_summaries(project, entries, native, supports_native)
            .await?;
        *position = next;
        Ok(rows)
    }

    async fn read_project_summaries(
        &self,
        project: &DesktopProject,
        entries: Vec<(String, DbEvidence)>,
        native: bool,
        supports_native: bool,
    ) -> Result<Vec<Conversation>, ProtocolError> {
        let reader = self.directory_pages.metadata.clone();
        let generation = self.query_generation()?;
        let epoch = self.atoms.event_epoch();
        let server = self.ready_server()?;
        let mut jobs = tokio::task::JoinSet::new();
        for (id, fact) in entries {
            let reader = reader.clone();
            let generation = generation.clone();
            let server = server.clone();
            jobs.spawn(async move { (reader.read(generation, epoch, server, id).await, fact) });
        }
        let mut rows = Vec::new();
        while let Some(result) = jobs.join_next().await {
            let (snapshot, fact) = result.map_err(provider_task_error)?;
            let mut snapshot = snapshot.map_err(CodexProtocolMapper::error)?;
            if snapshot.thread.ephemeral {
                continue;
            }
            let actual = snapshot
                .thread
                .project_id
                .as_deref()
                .filter(|id| !id.is_empty());
            if native && actual != Some(project.id.as_str())
                || !native && actual.is_some() && supports_native
            {
                return Err(conversation_atoms::generation_changed());
            }
            // Old DBs without project_id can also select a native candidate. Leave it to the RPC source.
            if !native && actual.is_some() {
                continue;
            }
            project.assignments.decorate(&mut snapshot);
            let mut row = lock(&self.mapper).conversation(&snapshot);
            repair_time(&mut row, &fact);
            rows.push(row);
        }
        Ok(rows)
    }

    async fn read_legacy_project_page(
        &self,
        project: &DesktopProject,
        position: &mut Position,
        limit: usize,
        updated_after: Option<u64>,
    ) -> Result<Vec<Conversation>, ProtocolError> {
        let cursor = match position {
            Position::Native(cursor) => Some(cursor.clone()),
            _ => None,
        };
        let server = self.ready_server()?;
        let project_id = project.id.clone();
        let page = tokio::task::spawn_blocking({
            let cursor = cursor.clone();
            move || {
                server.thread_list(CodexThreadListRequest {
                    cursor,
                    limit: Some(limit as u32),
                    project_id: Some(Some(project_id)),
                    workspace_root: None,
                    search_term: None,
                })
            }
        })
        .await
        .map_err(provider_task_error)?
        .map_err(CodexProtocolMapper::error)?;
        if page.next_cursor.is_some() && page.next_cursor == cursor {
            return Err(protocol_error(
                "conversation_query_incomplete",
                "Native list cursor repeated".into(),
                true,
            ));
        }
        *position = page.next_cursor.map_or(Position::Done, Position::Native);
        let mapper = lock(&self.mapper);
        Ok(page
            .data
            .iter()
            .filter(|snapshot| {
                !snapshot.thread.ephemeral
                    && snapshot.thread.project_id.as_deref() == Some(project.id.as_str())
            })
            .map(|snapshot| mapper.conversation(snapshot))
            .filter(|row| {
                updated_after.is_none_or(|time| row.updated_at.is_some_and(|at| at >= time))
            })
            .collect())
    }
}

fn cursor_binding(
    generation: &str,
    request: &ConversationListRequest,
    project: &DesktopProject,
) -> String {
    let assignments = &project.assignments;
    let assigned = assignments.by_thread.iter().collect::<BTreeMap<_, _>>();
    let fields = json!([
        generation,
        request.route,
        request.project_filter,
        request.query,
        request.reader_scope,
        project.roots,
        project.codex_worktrees,
        assigned,
        assignments.projectless,
        assignments.other_host_threads,
        assignments.root_hints
    ]);
    URL_SAFE_NO_PAD.encode(digest::digest(
        &digest::SHA256,
        fields.to_string().as_bytes(),
    ))
}

fn cursor_key() -> Result<hmac::Key, ProtocolError> {
    static KEY: OnceLock<Option<[u8; 32]>> = OnceLock::new();
    let key = KEY.get_or_init(|| {
        let mut key = [0u8; 32];
        SystemRandom::new().fill(&mut key).ok().map(|_| key)
    });
    key.as_ref()
        .map(|key| hmac::Key::new(hmac::HMAC_SHA256, key))
        .ok_or_else(|| {
            protocol_error(
                "internal_error",
                "Cannot generate project cursor key".into(),
                true,
            )
        })
}

fn invalid_cursor() -> ProtocolError {
    protocol_error(
        "invalid_cursor",
        "Project cursor or its filter has changed".into(),
        false,
    )
}

fn encode_cursor(cursor: &ProjectCursor) -> Result<String, ProtocolError> {
    let payload = URL_SAFE_NO_PAD.encode(serde_json::to_vec(cursor).map_err(|_| invalid_cursor())?);
    let signature = URL_SAFE_NO_PAD.encode(hmac::sign(&cursor_key()?, payload.as_bytes()));
    Ok(format!("project-v1.{payload}.{signature}"))
}

fn decode_cursor(token: &str, binding: &str) -> Result<ProjectCursor, ProtocolError> {
    let (payload, signature) = token
        .strip_prefix("project-v1.")
        .and_then(|token| token.split_once('.'))
        .ok_or_else(invalid_cursor)?;
    let signature = URL_SAFE_NO_PAD
        .decode(signature)
        .map_err(|_| invalid_cursor())?;
    hmac::verify(&cursor_key()?, payload.as_bytes(), &signature).map_err(|_| invalid_cursor())?;
    let cursor: ProjectCursor = serde_json::from_slice(
        &URL_SAFE_NO_PAD
            .decode(payload)
            .map_err(|_| invalid_cursor())?,
    )
    .map_err(|_| invalid_cursor())?;
    if cursor.binding != binding {
        return Err(invalid_cursor());
    }
    Ok(cursor)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn project_display_accepts_only_matching_registered_worktrees_and_explicit_hints() {
        let fixture = tempfile::tempdir().unwrap();
        let root = fixture.path().join("repo");
        let worktrees = fixture.path().join("codex").join("worktrees");
        let checkout = worktrees.join("id").join("checkout");
        let gitdir = root.join(".git").join("worktrees").join("checkout");
        std::fs::create_dir_all(&gitdir).unwrap();
        std::fs::create_dir_all(&checkout).unwrap();
        std::fs::write(
            checkout.join(".git"),
            format!("gitdir: {}", gitdir.display()),
        )
        .unwrap();
        std::fs::write(
            gitdir.join("gitdir"),
            checkout.join(".git").to_str().unwrap(),
        )
        .unwrap();
        std::fs::write(gitdir.join("commondir"), "../..").unwrap();
        let root = root.canonicalize().unwrap();
        let project = DesktopProject {
            id: "project".into(),
            roots: vec![normalized_path(&root)],
            codex_worktrees: normalized_path(&worktrees),
            worktree_roots: Mutex::default(),
            assignments: CodexDesktopProjectAssignments {
                projectless: ["standalone".to_string()].into(),
                root_hints: [("hint".to_string(), root.to_string_lossy().into_owned())].into(),
                ..Default::default()
            },
        };
        assert!(project.includes("worktree", checkout.join("subdir").to_str()));
        assert!(project.includes("hint", None));
        assert!(!project.includes("standalone", checkout.to_str()));
        assert!(!project.includes("unregistered", worktrees.join("unknown/repo").to_str()));
        assert!(!project.includes("neighbor", root.with_file_name("repo-other").to_str()));
    }

    #[test]
    fn stateless_cursor_round_trips_both_positions_and_rejects_forgery() {
        let cursor = ProjectCursor {
            binding: "scope".into(),
            native: Position::Db(42, "id".into()),
            desktop: Position::Done,
            desktop_first: true,
        };
        let token = encode_cursor(&cursor).unwrap();
        let decoded = decode_cursor(&token, "scope").unwrap();
        assert!(matches!(decoded.native, Position::Db(42, ref id) if id == "id"));
        assert!(matches!(decoded.desktop, Position::Done));
        assert!(decode_cursor(&token, "another").is_err());
        let forged = token.replacen("project-v1.", "project-v1.X", 1);
        assert!(decode_cursor(&forged, "scope").is_err());
    }
}
