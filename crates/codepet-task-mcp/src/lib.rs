//! Rust MCP adapters sharing the same task tools and ledger with the desktop app.
pub mod http;
use codepet_task_lineage::tasks::{Actor, FindInput, Ledger, SaveInput};
use rmcp::{
    handler::server::wrapper::Parameters,
    model::{CallToolResult, ContentBlock, Implementation, ServerCapabilities, ServerConfig},
    tool, tool_handler, tool_router, ServerHandler, ServiceExt,
};

const INSTRUCTIONS: &str = "Track independently verifiable goals, not every message or implementation step. Usually write twice: save the agreed plan/checklist when starting implementation, then save progress/tests/explicit commits at a milestone. Combine fields in one tasks_save call. Tasks are in_progress, ready_for_delivery, published, or closed. Implementation done: phase=ready_for_delivery. Actual publication: phase=published plus delivery version/reference. Commits do not imply publication. Only the user closes. Keep the returned taskId/revision; do not poll, list before every write, or resend unchanged plans. Search only to resume or identify an existing task. Reopen a closed task with a reason for recurrence. Missing chat IDs stay unlinked. Git facts are collected locally; human acceptance happens in Code Pet. A failed ledger call must not interrupt coding: report it briefly and retry only at the next milestone.";

#[derive(Clone)]
pub struct TaskServer {
    ledger: Ledger,
}
impl TaskServer {
    pub fn new(ledger: Ledger) -> Self {
        Self { ledger }
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
        description = "Search compact task summaries to resume work; taskId retrieves the current round/checklist. No routine polling. History is opt-in."
    )]
    async fn tasks_find(&self, Parameters(input): Parameters<FindInput>) -> CallToolResult {
        let ledger = self.ledger.clone();
        response(
            tokio::task::spawn_blocking(move || ledger.find(input))
                .await
                .unwrap_or_else(|e| Err(e.to_string())),
        )
    }
    #[tool(
        description = "Save one task milestone, combining goal, plan/checklist, progress, tests and explicit commit SHAs as needed. Omit unchanged fields. New: workspace/title/objective. Update: taskId/expectedRevision. Returns a small receipt. Reopen only after closure. Never call per message/file/tool; usually plan + implementation milestone suffice. No human acceptance through MCP."
    )]
    async fn tasks_save(&self, Parameters(input): Parameters<SaveInput>) -> CallToolResult {
        let ledger = self.ledger.clone();
        response(
            tokio::task::spawn_blocking(move || {
                ledger
                    .save(input, Actor::Agent)
                    .and_then(|r| serde_json::to_value(r).map_err(|e| e.to_string()))
            })
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
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?
        .block_on(async {
            let service = TaskServer::new(ledger)
                .serve(rmcp::transport::stdio())
                .await?;
            service.waiting().await?;
            Ok(())
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn small_tool_surface_and_instructions_budget() {
        let tools = TaskServer::tool_router().list_all();
        assert_eq!(tools.len(), 2);
        let bytes = serde_json::to_vec(&tools).unwrap().len();
        assert!(bytes < 14000, "tool definitions grew to {bytes} bytes");
        assert!(INSTRUCTIONS.len() < 1200);
    }
}
