//! Five milestone tools; Rust owns the ledger and request-scoped Agent identity.
pub mod clients;
pub mod http;
pub mod instructions;
pub mod managed;
pub mod recall;
use codepet_task_lineage::tasks::{
    Actor, CreateTaskInput, CreateTaskRunInput, FindTaskRunsInput, FinishTaskRunInput, Ledger,
    UpdateTaskRunInput,
};
use rmcp::{
    handler::server::wrapper::Parameters,
    model::{
        CallToolResult, ContentBlock, Implementation, InitializeRequestParams, InitializeResult,
        ServerCapabilities, ServerConfig,
    },
    service::RequestContext,
    tool, tool_handler, tool_router, RoleServer, ServerHandler, ServiceExt,
};
const INSTRUCTIONS: &str = include_str!("../instructions.md");
#[derive(Clone)]
pub struct TaskServer {
    ledger: Ledger,
    recall: std::sync::Arc<recall::Recall>,
    stdio_actor: Actor,
}
impl TaskServer {
    pub fn new(ledger: Ledger) -> Self {
        Self {
            recall: recall::Recall::shared(&ledger),
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
        description = "Register a new independently verifiable goal with its first run. Recall related task memory first and resume matching work rather than duplicating it. Combine task context, initial run and checklist. initialRun.agentConversation requires the actual current conversation ID/link. Author comes from the connection. Keep taskId/runId/revision; never call per message/file."
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
        description = "Start another handling attempt after publication, with its reason. A closed/completed task is locked: only the user can manually change it to development in the App; Agents cannot reopen it or create a replacement run. After that human action, use update-task-run on the new current run. An omitted checklist reuses prior items with new IDs and no results. agentConversation must contain the actual current conversation ID/link. Keep the returned runId and task revision."
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
        description = "Save one current-run milestone: findings/solution, checklist, tests, explicit commits or release reference. Omit unchanged fields. expectedRevision is the task revision. agentConversation is required and nonempty on every write. Each verification item requires result and nonempty evidence together. Use finish-task-run when development is done. Only the user accepts or closes. Git is observed by Rust; never supply invented state or file lists. No polling."
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
        name = "finish-task-run",
        description = "Finish development with required changedFiles (repository-relative file paths; [] for no code changes), nonempty conclusion, actual conversation link and item verification results (status plus evidence). Required local checks must pass for ready_for_delivery; failed/incomplete checks keep in_progress. This never accepts or closes for the user. Atomically saves tests, Git snapshot and conclusion. Retain the receipt; no polling."
    )]
    async fn finish_task_run(
        &self,
        Parameters(input): Parameters<FinishTaskRunInput>,
        context: RequestContext<RoleServer>,
    ) -> CallToolResult {
        let actor = match self.actor(&context) {
            Ok(a) => a,
            Err(e) => return response(Err(e)),
        };
        let ledger = self.ledger.clone();
        let result = tokio::task::spawn_blocking(move || {
            ledger
                .finish_task_run(input, actor)
                .and_then(|r| serde_json::to_value(r).map_err(|e| e.to_string()))
        })
        .await
        .unwrap_or_else(|e| Err(e.to_string()));
        if result.is_ok() {
            self.recall.changed();
        }
        response(result)
    }
    #[tool(
        name = "find-task-runs",
        description = "Recall task memory before starting related work or resuming: find specific runs, then read relevant goals, solutions, conclusions and verification by runId. Results retain the matched historical run and isCurrentRun. workspace only ranks, never filters. runId reads an exact run without taskId; taskId alone reads its current run/checklist. History is opt-in. Keep receipts; do not query before every write."
    )]
    async fn find_task_runs(
        &self,
        Parameters(input): Parameters<FindTaskRunsInput>,
    ) -> CallToolResult {
        response(self.recall.find(input).await)
    }
}
#[tool_handler]
impl ServerHandler for TaskServer {
    async fn initialize(
        &self,
        request: InitializeRequestParams,
        context: RequestContext<RoleServer>,
    ) -> Result<InitializeResult, rmcp::ErrorData> {
        context.peer.set_peer_info(request.clone());
        let mut result = self.negotiate_initialize(&request)?;
        let ledger = self.ledger.clone();
        let settings = tokio::task::spawn_blocking(move || instructions::read(&ledger))
            .await
            .map_err(|e| rmcp::ErrorData::internal_error(e.to_string(), None))?
            .map_err(|e| rmcp::ErrorData::internal_error(e, None))?;
        result.instructions = Some(settings["instructions"].as_str().unwrap().to_owned());
        Ok(result)
    }

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
    run_with_ledger(ledger)
}
pub fn run_with_ledger(ledger: Ledger) -> Result<(), Box<dyn std::error::Error>> {
    let mut server = TaskServer::new(ledger);
    if let Ok(source) = std::env::var("CODEPET_AGENT") {
        server.stdio_actor = Actor::agent(&source).map_err(std::io::Error::other)?;
    }
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?
        .block_on(async {
            let recall = server.recall.clone();
            let watch = recall.watch(&tokio::runtime::Handle::current());
            let service = server.serve(rmcp::transport::stdio()).await?;
            service.waiting().await?;
            watch.abort();
            recall.stop().await;
            Ok(())
        })
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn five_tools_and_bounded_instructions() {
        let tools = TaskServer::tool_router().list_all();
        let mut names = tools.iter().map(|t| t.name.as_ref()).collect::<Vec<_>>();
        names.sort();
        assert_eq!(
            names,
            [
                "create-task",
                "create-task-run",
                "find-task-runs",
                "finish-task-run",
                "update-task-run"
            ]
        );
        let bytes = serde_json::to_vec(&tools).unwrap().len();
        assert!(bytes < 24000, "definitions: {bytes}");
        assert!(INSTRUCTIONS.len() < 2000);
        let leading: String = INSTRUCTIONS.chars().take(512).collect();
        for milestone in [
            "create-task BEFORE",
            "update-task-run",
            "finish-task-run BEFORE",
        ] {
            assert!(leading.contains(milestone));
        }
    }
}
