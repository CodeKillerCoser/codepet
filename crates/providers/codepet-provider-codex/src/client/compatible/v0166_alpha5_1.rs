use super::super::projects::project_update_params;
use super::super::*;

pub(in crate::client) struct CompatibleV0166Alpha5_1;

impl CompatibleV0166Alpha5_1 {
    pub(in crate::client) fn project_list(
        client: &CodexAppServerSession,
        cursor: Option<String>,
        limit: Option<u32>,
    ) -> Result<CodexProjectPage, CodexAppServerError> {
        let response: ProjectListResponse =
            client.request("project/list", json!({"cursor":cursor,"limit":limit}))?;
        Ok(CodexProjectPage {
            data: response.data,
            next_cursor: response.next_cursor,
        })
    }
    pub(in crate::client) fn project_read(
        client: &CodexAppServerSession,
        project_id: &str,
    ) -> Result<CodexProject, CodexAppServerError> {
        let response: ProjectResponse =
            client.request("project/read", json!({"projectId":project_id}))?;
        Ok(response.project)
    }
    pub(in crate::client) fn project_create(
        client: &CodexAppServerSession,
        request: CodexProjectCreateRequest,
    ) -> Result<CodexProject, CodexAppServerError> {
        let response: ProjectResponse = client.request("project/create", json!({"idempotencyKey":request.idempotency_key,"name":request.name,"roots":request.roots,"metadata":request.metadata}))?;
        Ok(response.project)
    }
    pub(in crate::client) fn project_update(
        client: &CodexAppServerSession,
        request: CodexProjectUpdateRequest,
    ) -> Result<CodexProject, CodexAppServerError> {
        let response: ProjectResponse =
            client.request("project/update", project_update_params(request))?;
        Ok(response.project)
    }
}
