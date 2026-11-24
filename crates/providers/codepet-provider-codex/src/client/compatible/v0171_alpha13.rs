use super::super::projects::project_update_params;
use super::super::*;
use crate::protocol::CodexProjectRoot;
use serde::Deserialize;
use std::collections::BTreeMap;

pub(in crate::client) struct CompatibleV0171Alpha13;

// Official 0.171.0-alpha.13 Project: numeric position was removed. recencyAt
// is not an ordering key; project/list explicitly defines returned order.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Project {
    id: String,
    name: String,
    roots: Vec<CodexProjectRoot>,
    metadata: BTreeMap<String, String>,
    created_at: i64,
    updated_at: i64,
}
impl Project {
    fn normalize(self, position: i64) -> CodexProject {
        CodexProject {
            id: self.id,
            name: self.name,
            roots: self.roots,
            metadata: self.metadata,
            position,
            created_at: self.created_at,
            updated_at: self.updated_at,
        }
    }
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ProjectPage {
    data: Vec<Project>,
    next_cursor: Option<String>,
}
#[derive(Deserialize)]
struct ProjectResponse {
    project: Project,
}

impl CompatibleV0171Alpha13 {
    pub(in crate::client) fn project_list(
        client: &CodexAppServerSession,
        cursor: Option<String>,
        limit: Option<u32>,
    ) -> Result<CodexProjectPage, CodexAppServerError> {
        let first_page = cursor.is_none();
        let response: ProjectPage =
            client.request("project/list", json!({"cursor":cursor,"limit":limit}))?;
        Ok(CodexProjectPage {
            data: Self::resolve_project_positions(client, response.data, first_page)?,
            next_cursor: response.next_cursor,
        })
    }
    pub(in crate::client) fn project_read(
        client: &CodexAppServerSession,
        project_id: &str,
    ) -> Result<CodexProject, CodexAppServerError> {
        let response: ProjectResponse =
            client.request("project/read", json!({"projectId":project_id}))?;
        Ok(Self::resolve_project_positions(client, vec![response.project], false)?.remove(0))
    }
    pub(in crate::client) fn project_create(
        client: &CodexAppServerSession,
        request: CodexProjectCreateRequest,
    ) -> Result<CodexProject, CodexAppServerError> {
        let response: ProjectResponse = client.request("project/create",json!({"idempotencyKey":request.idempotency_key,"name":request.name,"roots":request.roots,"metadata":request.metadata}))?;
        Ok(Self::resolve_project_positions(client, vec![response.project], false)?.remove(0))
    }
    pub(in crate::client) fn project_update(
        client: &CodexAppServerSession,
        request: CodexProjectUpdateRequest,
    ) -> Result<CodexProject, CodexAppServerError> {
        let response: ProjectResponse =
            client.request("project/update", project_update_params(request))?;
        Ok(Self::resolve_project_positions(client, vec![response.project], false)?.remove(0))
    }
    // Project.position is a Provider-defined ordering value. This contract uses
    // authoritative list order to derive ranks, never IDs or recency.
    fn resolve_project_positions(
        client: &CodexAppServerSession,
        projects: Vec<Project>,
        first_page: bool,
    ) -> Result<Vec<CodexProject>, CodexAppServerError> {
        if first_page || projects.is_empty() {
            return Ok(projects
                .into_iter()
                .enumerate()
                .map(|(rank, project)| project.normalize(rank as i64))
                .collect());
        }
        let mut positions = HashMap::new();
        // A continuation cursor is opaque. Resolve global ranks with a bounded scan
        // rather than resetting the rank at every page or changing native cursors.
        // Reads/create/update also need the same rank to avoid reordering the UI.
        let mut cursor: Option<String> = None;
        let mut seen = HashSet::new();
        let mut offset = 0_i64;
        let deadline = std::time::Instant::now() + REQUEST_TIMEOUT;
        for _ in 0..100 {
            let remaining = deadline
                .checked_duration_since(std::time::Instant::now())
                .ok_or_else(|| {
                    CodexAppServerError::Timeout(
                        "project ordering scan exceeded its deadline".into(),
                    )
                })?;
            let result = client.request_value_with_timeout(
                "project/list",
                json!({"cursor":cursor,"limit":100}),
                remaining,
            )?;
            let page: ProjectPage = serde_json::from_value(result).map_err(|error| {
                CodexAppServerError::Protocol(format!("invalid project/list response: {error}"))
            })?;
            for (rank, candidate) in page.data.iter().enumerate() {
                if projects.iter().any(|project| project.id == candidate.id) {
                    positions
                        .entry(candidate.id.clone())
                        .or_insert(offset + rank as i64);
                }
            }
            if projects
                .iter()
                .all(|project| positions.contains_key(&project.id))
            {
                return Ok(projects
                    .into_iter()
                    .map(|project| {
                        let rank = positions[&project.id];
                        project.normalize(rank)
                    })
                    .collect());
            }
            offset += page.data.len() as i64;
            match page.next_cursor {
                Some(next) if seen.insert(next.clone()) => cursor = Some(next),
                Some(_) => {
                    return Err(CodexAppServerError::Protocol(
                        "project/list returned a repeated cursor while resolving order".into(),
                    ))
                }
                None => return Err(CodexAppServerError::Protocol(
                    "project/list no longer contains the requested project while resolving order"
                        .into(),
                )),
            }
        }
        Err(CodexAppServerError::Protocol(
            "project/list exceeded the bounded ordering scan".into(),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn current_dto_preserves_required_fields_from_official_schema() {
        let schema: Value = serde_json::from_str(include_str!(
            "../../../tests/fixtures/contracts/0.171.0-alpha.13.json"
        ))
        .unwrap();
        let legacy: Value = serde_json::from_str(include_str!(
            "../../../tests/fixtures/contracts/0.166.0-alpha.5.1.json"
        ))
        .unwrap();
        let pointer = "/projectListResponse/definitions/Project/required";
        assert!(legacy
            .pointer(pointer)
            .unwrap()
            .as_array()
            .unwrap()
            .contains(&json!("position")));
        let required = schema.pointer(pointer).unwrap().as_array().unwrap();
        assert!(!required.contains(&json!("position")));
        let example = json!({"id":"p","name":"P","roots":[{"path":"/p"}],"metadata":{"team":"qa"},"createdAt":1,"updatedAt":2});
        let normalized = serde_json::from_value::<Project>(example.clone())
            .unwrap()
            .normalize(9);
        assert_eq!(normalized.id, "p");
        assert_eq!(normalized.position, 9);
        assert_eq!(normalized.metadata["team"], "qa");
        for field in required {
            let mut invalid = example.clone();
            invalid
                .as_object_mut()
                .unwrap()
                .remove(field.as_str().unwrap());
            assert!(
                serde_json::from_value::<Project>(invalid).is_err(),
                "{field}"
            );
        }
    }
}
