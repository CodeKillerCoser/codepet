use super::compatible::{
    v0166_alpha5_1::CompatibleV0166Alpha5_1, v0171_alpha13::CompatibleV0171Alpha13,
};
use super::*;

// User-selected legacy upper bound, not a claim that all earlier binaries were
// tested. Official schema snapshots: 0.166.0-alpha.5.1 / 0.171.0-alpha.13.
impl CodexAppServerSession {
    pub fn project_list(
        &self,
        cursor: Option<String>,
        limit: Option<u32>,
    ) -> Result<CodexProjectPage, CodexAppServerError> {
        if self.version_at_most("0.166.0-alpha.5.1")? {
            CompatibleV0166Alpha5_1::project_list(self, cursor, limit)
        } else {
            // Latest implementation also serves newer versions by default.
            CompatibleV0171Alpha13::project_list(self, cursor, limit)
        }
    }
    pub fn project_read(&self, project_id: &str) -> Result<CodexProject, CodexAppServerError> {
        if self.version_at_most("0.166.0-alpha.5.1")? {
            CompatibleV0166Alpha5_1::project_read(self, project_id)
        } else {
            // Latest implementation also serves newer versions by default.
            CompatibleV0171Alpha13::project_read(self, project_id)
        }
    }
    pub fn project_create(
        &self,
        request: CodexProjectCreateRequest,
    ) -> Result<CodexProject, CodexAppServerError> {
        if self.version_at_most("0.166.0-alpha.5.1")? {
            CompatibleV0166Alpha5_1::project_create(self, request)
        } else {
            // Latest implementation also serves newer versions by default.
            CompatibleV0171Alpha13::project_create(self, request)
        }
    }
    pub fn project_update(
        &self,
        request: CodexProjectUpdateRequest,
    ) -> Result<CodexProject, CodexAppServerError> {
        if self.version_at_most("0.166.0-alpha.5.1")? {
            CompatibleV0166Alpha5_1::project_update(self, request)
        } else {
            // Latest implementation also serves newer versions by default.
            CompatibleV0171Alpha13::project_update(self, request)
        }
    }
    pub fn project_delete(&self, project_id: &str) -> Result<(), CodexAppServerError> {
        let _: serde_json::Map<String, Value> =
            self.request("project/delete", json!({"projectId":project_id}))?;
        Ok(())
    }
}

pub(super) fn project_update_params(request: CodexProjectUpdateRequest) -> Value {
    let mut params = serde_json::Map::new();
    params.insert("projectId".to_string(), json!(request.project_id));
    if let Some(name) = request.name {
        params.insert("name".to_string(), json!(name));
    }
    if let Some(roots) = request.roots {
        params.insert("roots".to_string(), json!(roots));
    }
    if let Some(metadata) = request.metadata {
        params.insert("metadata".to_string(), json!(metadata));
    }
    Value::Object(params)
}
