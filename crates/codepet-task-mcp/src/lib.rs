//! Four milestone tools; Rust owns the ledger and request-scoped Agent identity.
pub mod http;
use codepet_task_lineage::tasks::{
    Actor, CreateTaskInput, CreateTaskRunInput, FindTasksInput, Ledger, UpdateTaskRunInput,
};
use rmcp::{
    handler::server::wrapper::Parameters,
    model::{CallToolResult, ContentBlock, Implementation, ServerCapabilities, ServerConfig},
    service::RequestContext,
    tool, tool_handler, tool_router, RoleServer, ServerHandler, ServiceExt,
};
const INSTRUCTIONS: &str = include_str!("../instructions.md");
#[derive(Clone)]
pub struct TaskServer {
    ledger: Ledger,
    stdio_actor: Actor,
}
impl TaskServer {
    pub fn new(ledger: Ledger) -> Self {
        Self {
            ledger,
            stdio_actor: Actor::agent("codex").expect("built-in source"),
        }
    }
    fn actor(&self, context: &RequestContext<RoleServer>) -> Result<Actor, String> {
        if let Some(parts) = context.extensions.get::<axum::http::request::Parts>() {
            parts
                .extensions
                .get::<Actor>()
                .cloned()
                .ok_or("缺少 X-CodePet-Agent 请求上下文".into())
        } else {
            Ok(self.stdio_actor.clone())
        }
    }
}
fn response(result: Result<serde_json::Value, String>) -> CallToolResult {
    match result {
        Ok(v) => CallToolResult::success(vec![ContentBlock::text(v.to_string())]),
        Err(e) => CallToolResult::error(vec![ContentBlock::text(e)]),
    }
}
#[tool_router]
impl TaskServer {
    #[tool(
        name = "create-task",
        description = "Register one independently verifiable goal with its first run. Combine task context, initial run description and checklist in one call. Author comes from the connection, not arguments. Keep returned taskId/runId/revision. Do not call per message or file."
    )]
    async fn create_task(
        &self,
        Parameters(input): Parameters<CreateTaskInput>,
        context: RequestContext<RoleServer>,
    ) -> CallToolResult {
        let actor = match self.actor(&context) {
            Ok(a) => a,
            Err(e) => return response(Err(e)),
        };
        let ledger = self.ledger.clone();
        response(
            tokio::task::spawn_blocking(move || {
                ledger
                    .create_task(input, actor)
                    .and_then(|r| serde_json::to_value(r).map_err(|e| e.to_string()))
            })
            .await
            .unwrap_or_else(|e| Err(e.to_string())),
        )
    }
    #[tool(
        name = "create-task-run",
        description = "Start another handling attempt for a published or closed task, with its reason. Continuing the same attempt uses update-task-run. An omitted checklist reuses prior items with new IDs and no results. Keep the returned runId and task revision."
    )]
    async fn create_task_run(
        &self,
        Parameters(input): Parameters<CreateTaskRunInput>,
        context: RequestContext<RoleServer>,
    ) -> CallToolResult {
        let actor = match self.actor(&context) {
            Ok(a) => a,
            Err(e) => return response(Err(e)),
        };
        let ledger = self.ledger.clone();
        response(
            tokio::task::spawn_blocking(move || {
                ledger
                    .create_task_run(input, actor)
                    .and_then(|r| serde_json::to_value(r).map_err(|e| e.to_string()))
            })
            .await
            .unwrap_or_else(|e| Err(e.to_string())),
        )
    }
    #[tool(
        name = "update-task-run",
        description = "Save one current-run milestone: findings/solution, checklist, tests, explicit commits or release reference. Omit unchanged fields. expectedRevision is the task revision. Only the user accepts or closes. Git is observed by Rust; never supply invented state or file lists. No polling."
    )]
    async fn update_task_run(
        &self,
        Parameters(input): Parameters<UpdateTaskRunInput>,
        context: RequestContext<RoleServer>,
    ) -> CallToolResult {
        let actor = match self.actor(&context) {
            Ok(a) => a,
            Err(e) => return response(Err(e)),
        };
        let ledger = self.ledger.clone();
        response(
            tokio::task::spawn_blocking(move || {
                ledger
                    .update_task_run(input, actor)
                    .and_then(|r| serde_json::to_value(r).map_err(|e| e.to_string()))
            })
            .await
            .unwrap_or_else(|e| Err(e.to_string())),
        )
    }
    #[tool(
        name = "find-tasks",
        description = "Find compact task summaries only to identify or resume work. taskId retrieves its current run/checklist; runId selects a historical run. History is opt-in. Retain receipts rather than querying before every write."
    )]
    async fn find_tasks(&self, Parameters(input): Parameters<FindTasksInput>) -> CallToolResult {
        let ledger = self.ledger.clone();
        response(
            tokio::task::spawn_blocking(move || ledger.find(input))
                .await
                .unwrap_or_else(|e| Err(e.to_string())),
        )
    }
}
#[tool_handler]
impl ServerHandler for TaskServer {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::new(
                "codepet-tasks",
                env!("CARGO_PKG_VERSION"),
            ))
            .with_instructions(INSTRUCTIONS)
    }
}
pub fn run() -> Result<(), Box<dyn std::error::Error>> {
    let ledger = Ledger::discover().map_err(std::io::Error::other)?;
    let mut server = TaskServer::new(ledger);
    if let Ok(source) = std::env::var("CODEPET_AGENT") {
        server.stdio_actor = Actor::agent(&source).map_err(std::io::Error::other)?;
    }
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?
        .block_on(async {
            let service = server.serve(rmcp::transport::stdio()).await?;
            service.waiting().await?;
            Ok(())
        })
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn four_tools_and_bounded_instructions() {
        let tools = TaskServer::tool_router().list_all();
        let mut names = tools.iter().map(|t| t.name.as_ref()).collect::<Vec<_>>();
        names.sort();
        assert_eq!(
            names,
            [
                "create-task",
                "create-task-run",
                "find-tasks",
                "update-task-run"
            ]
        );
        let bytes = serde_json::to_vec(&tools).unwrap().len();
        assert!(bytes < 24000, "definitions: {bytes}");
        assert!(INSTRUCTIONS.len() < 1400);
    }
}
