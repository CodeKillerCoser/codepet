use codepet_task_mcp::http::HttpRuntime;
use reqwest::{Client, RequestBuilder, StatusCode};
use serde_json::{json, Value};
use std::{
    path::{Path, PathBuf},
    process::Command,
};

struct Endpoint {
    runtime: HttpRuntime,
    url: String,
    authorization: String,
    client: Client,
}
impl Endpoint {
    async fn start(data: &Path, port: u16) -> Self {
        let runtime = HttpRuntime::start(data.to_owned(), port);
        let config = runtime.configuration().await.unwrap();
        assert!(config["config"]
            .as_str()
            .unwrap()
            .contains("url = \"http://127.0.0.1:"));
        assert!(!config["config"].as_str().unwrap().contains("command ="));
        let token = std::fs::read_to_string(data.join("tasks/mcp-http.token")).unwrap();
        let authorization = format!("Bearer {token}");
        assert!(config["config"].as_str().unwrap().contains(&authorization));
        Self {
            runtime,
            url: config["url"].as_str().unwrap().into(),
            authorization,
            client: Client::builder().no_proxy().build().unwrap(),
        }
    }
    fn post(&self, method: &str, params: Value) -> RequestBuilder {
        self.client
            .post(&self.url)
            .header("accept", "application/json, text/event-stream")
            .header("authorization", &self.authorization)
            .header("X-CodePet-Agent", "codex")
            .header("MCP-Protocol-Version", "2025-06-18")
            .json(&json!({"jsonrpc":"2.0","id":1,"method":method,"params":params}))
    }
    async fn rpc(&self, method: &str, params: Value) -> Value {
        let response = self.post(method, params).send().await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert!(response.headers().get("Mcp-Session-Id").is_none());
        let value: Value = response.json().await.unwrap();
        assert!(
            value["error"].is_null(),
            "unexpected protocol error: {}",
            value["error"]
        );
        value["result"].clone()
    }
    async fn call(&self, name: &str, args: Value) -> Value {
        let result = self
            .rpc("tools/call", json!({"name":name,"arguments":args}))
            .await;
        assert_ne!(
            result["isError"], true,
            "tool failed: {}",
            result["content"]
        );
        serde_json::from_str(result["content"][0]["text"].as_str().unwrap()).unwrap()
    }
}
fn init() -> Value {
    json!({"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"http-acceptance","version":"1"}})
}
fn repository(root: &Path) -> PathBuf {
    let path = root.join("repository");
    std::fs::create_dir_all(&path).unwrap();
    for args in [
        vec!["init", "-b", "main"],
        vec!["config", "user.email", "test@example.invalid"],
        vec!["config", "user.name", "HTTP test"],
    ] {
        assert!(Command::new("git")
            .arg("-C")
            .arg(&path)
            .args(args)
            .output()
            .unwrap()
            .status
            .success());
    }
    std::fs::write(path.join("file"), "test").unwrap();
    assert!(Command::new("git")
        .arg("-C")
        .arg(&path)
        .args(["add", "file"])
        .output()
        .unwrap()
        .status
        .success());
    assert!(Command::new("git")
        .arg("-C")
        .arg(&path)
        .args(["commit", "-m", "initial"])
        .output()
        .unwrap()
        .status
        .success());
    path
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn http_writes_share_the_app_ledger_and_survive_restart_without_sessions() {
    let root = tempfile::tempdir().unwrap();
    let workspace = repository(root.path());
    let data = root.path().join("data");
    let endpoint = Endpoint::start(&data, 0).await;
    let initialized = endpoint.rpc("initialize", init()).await;
    assert_eq!(initialized["serverInfo"]["name"], "codepet-tasks");
    let tools = endpoint.rpc("tools/list", json!({})).await;
    assert_eq!(tools["tools"].as_array().unwrap().len(), 6);
    assert!(serde_json::to_vec(&tools).unwrap().len() < 24000);
    let input = json!({"agentWorkspace":{"path":workspace},"title":"HTTP 任务","description":"通过本机 HTTP 登记目标","initialRun":{"agentConversation":["00000000-0000-4000-8000-000000000003"]}});
    let (first, retry) = tokio::join!(
        endpoint.call("create-task", input.clone()),
        endpoint.call("create-task", input)
    );
    assert_eq!(first["taskId"], retry["taskId"]);
    assert_eq!(first["revision"], 1);
    let task_id = first["taskId"].as_str().unwrap();
    assert_eq!(
        endpoint.runtime.ledger().get(task_id).unwrap().title,
        "HTTP 任务"
    );
    let unauthorized_actor = endpoint.post("tools/call", json!({"name":"create-task","arguments":{"taskId":task_id,"expectedRevision":1,"actor":"human"}})).send().await.unwrap();
    let value: Value = unauthorized_actor.json().await.unwrap();
    assert!(value.get("error").is_some() || value["result"]["isError"] == true);
    let port = reqwest::Url::parse(&endpoint.url).unwrap().port().unwrap();
    endpoint.runtime.shutdown_once().await;
    endpoint.runtime.shutdown_once().await;
    assert!(endpoint.runtime.shutdown_completed());
    assert!(endpoint.runtime.configuration().await.is_err());
    let reopened = Endpoint::start(&data, port).await;
    assert_eq!(reopened.authorization, endpoint.authorization);
    assert_eq!(reopened.url, endpoint.url);
    let task = reopened
        .call("find-task-runs", json!({"taskId":task_id}))
        .await;
    assert_eq!(task["revision"], 1);
    reopened.runtime.shutdown_once().await;
}

#[tokio::test]
async fn rejects_unauthorized_and_cross_origin_requests_before_tool_dispatch() {
    let data = tempfile::tempdir().unwrap();
    let endpoint = Endpoint::start(data.path(), 0).await;
    let request = || {
        endpoint
            .client
            .post(&endpoint.url)
            .header("accept", "application/json, text/event-stream")
            .json(&json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":init()}))
    };
    assert_eq!(
        request().send().await.unwrap().status(),
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        request()
            .bearer_auth("wrong")
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        request()
            .header("origin", "https://example.invalid")
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        endpoint
            .post("initialize", init())
            .header("origin", "null")
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        endpoint
            .post("initialize", init())
            .header("host", "example.invalid")
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        endpoint
            .post("initialize", init())
            .header("X-CodePet-Agent", "another")
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::BAD_REQUEST
    );
    let origin = endpoint.url.trim_end_matches("/mcp");
    assert_eq!(
        endpoint
            .post("initialize", init())
            .header("origin", origin)
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::OK
    );
    endpoint.runtime.shutdown_once().await;
}

#[tokio::test]
async fn occupied_port_is_reported_without_advertising_another_service_or_disabling_the_ledger() {
    let data = tempfile::tempdir().unwrap();
    let occupied = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let port = occupied.local_addr().unwrap().port();
    let runtime = HttpRuntime::start(data.path().to_owned(), port);
    let error = runtime.configuration().await.unwrap_err();
    assert!(error.contains(&port.to_string()));
    assert!(runtime.ledger().list().unwrap().is_empty());
    runtime.shutdown_once().await;
    assert_eq!(occupied.local_addr().unwrap().port(), port);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn official_sdk_client_negotiates_and_calls_the_http_tools() {
    use rmcp::{
        model::{CallToolRequestParams, ClientConfig},
        transport::{
            streamable_http_client::StreamableHttpClientTransportConfig,
            StreamableHttpClientTransport,
        },
        ServiceExt,
    };
    let data = tempfile::tempdir().unwrap();
    let endpoint = Endpoint::start(data.path(), 0).await;
    let mut headers = reqwest_mcp::header::HeaderMap::new();
    headers.insert(
        "x-codepet-agent",
        reqwest_mcp::header::HeaderValue::from_static("codex-sdk"),
    );
    let http = reqwest_mcp::Client::builder()
        .no_proxy()
        .default_headers(headers)
        .build()
        .unwrap();
    let transport = StreamableHttpClientTransport::with_client(
        http,
        StreamableHttpClientTransportConfig::with_uri(endpoint.url.clone())
            .auth_header(endpoint.authorization.trim_start_matches("Bearer ")),
    );
    let client = ClientConfig::default().serve(transport).await.unwrap();
    assert_eq!(client.list_all_tools().await.unwrap().len(), 6);
    let result = client
        .call_tool(
            CallToolRequestParams::new("find-task-runs")
                .with_arguments(serde_json::from_value(json!({"query":"SDK client"})).unwrap()),
        )
        .await
        .unwrap();
    assert_ne!(result.is_error, Some(true));
    client.cancel().await.unwrap();
    endpoint.runtime.shutdown_once().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn request_header_is_required_and_each_writer_has_its_own_identity() {
    let data = tempfile::tempdir().unwrap();
    let workspace = repository(data.path());
    let endpoint = Endpoint::start(&data.path().join("data"), 0).await;
    let rpc = |agent: Option<&str>, name: &str, input: Value| {
        let mut request = endpoint
            .client
            .post(&endpoint.url)
            .header("Authorization", &endpoint.authorization)
            .header("accept", "application/json, text/event-stream")
            .header("MCP-Protocol-Version", "2025-06-18");
        if let Some(agent) = agent {
            request = request.header("X-CodePet-Agent", agent);
        }
        request.json(&json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":name,"arguments":input}}))
    };
    let input = json!({"title":"来源测试","description":"任务背景","initialRun":{"agentConversation":["conversation-test"],"kind":"bugfix","title":"首次实现","goal":"目标","solution":"方案"},"agentWorkspace":{"path":workspace}});
    for agent in [None, Some("human"), Some(""), Some("bad source")] {
        assert_eq!(
            rpc(agent, "create-task", input.clone())
                .send()
                .await
                .unwrap()
                .status(),
            StatusCode::BAD_REQUEST
        );
    }
    let task_a = rpc(Some("codex-a"), "create-task", input.clone())
        .send()
        .await
        .unwrap()
        .json::<Value>()
        .await
        .unwrap();
    let receipt: Value =
        serde_json::from_str(task_a["result"]["content"][0]["text"].as_str().unwrap()).unwrap();
    let update = json!({"taskId":receipt["taskId"],"runId":receipt["runId"],"expectedRevision":1,"agentConversation":["conversation-test"],"description":"第二个 Agent 确认方案","status":"published","delivery":"Release 1.0"});
    let result = rpc(Some("codex-b"), "update-task-run", update)
        .send()
        .await
        .unwrap()
        .json::<Value>()
        .await
        .unwrap();
    assert_ne!(result["result"]["isError"], true, "{result}");
    let task = endpoint
        .runtime
        .ledger()
        .get(receipt["taskId"].as_str().unwrap())
        .unwrap();
    assert_eq!(task.created_by, "codex-a");
    assert_eq!(task.description, "任务背景");
    assert_eq!(task.runs[0].created_by, "codex-a");
    assert_eq!(task.runs[0].timeline.last().unwrap().actor, "codex-b");
    let next = json!({"taskId":task.id,"expectedRevision":task.revision,"title":"发布后复发","kind":"bugfix","agentConversation":["conversation-test"]});
    let result = rpc(Some("codex-c"), "create-task-run", next)
        .send()
        .await
        .unwrap()
        .json::<Value>()
        .await
        .unwrap();
    assert_ne!(result["result"]["isError"], true, "{result}");
    let task = endpoint.runtime.ledger().get(&task.id).unwrap();
    assert_eq!(task.runs[1].created_by, "codex-c");
    assert_eq!(task.runs[0].created_by, "codex-a");
    let comment = json!({"taskId":task.id,"runId":task.runs[0].id,"comment":"跨 Agent 复核历史执行","agentConversation":["agent-b-chat"],"operationId":"http-comment-one"});
    let response = rpc(Some("codex-b"), "comment-task-run", comment.clone())
        .send()
        .await
        .unwrap()
        .json::<Value>()
        .await
        .unwrap();
    assert_ne!(response["result"]["isError"], true, "{response}");
    let posted: Value =
        serde_json::from_str(response["result"]["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(posted["runId"], task.runs[0].id);
    let reply = json!({"taskId":task.id,"runId":task.runs[0].id,"comment":"已复核，补充上下文","replyTo":posted["activityId"],"agentConversation":["agent-c-chat"]});
    let response = rpc(Some("codex-c"), "comment-task-run", reply)
        .send()
        .await
        .unwrap()
        .json::<Value>()
        .await
        .unwrap();
    assert_ne!(response["result"]["isError"], true, "{response}");
    let latest = endpoint.runtime.ledger().get(&task.id).unwrap();
    let comments = latest.runs[0]
        .timeline
        .iter()
        .filter(|event| event.kind == "comment")
        .collect::<Vec<_>>();
    assert_eq!(comments.len(), 2);
    assert_eq!(comments[1].actor, "codex-c");
    assert_eq!(
        comments[1].reply_to.as_deref(),
        posted["activityId"].as_str()
    );
    assert_eq!(json!(latest.runs[1]), json!(task.runs[1]));
    let task = latest;
    let denied = json!({"taskId":task.id,"runId":task.runs[1].id,"expectedRevision":task.revision,"status":"closed"});
    let result = rpc(Some("codex-c"), "update-task-run", denied)
        .send()
        .await
        .unwrap()
        .json::<Value>()
        .await
        .unwrap();
    assert_eq!(result["result"]["isError"], true);
    let ledger = endpoint.runtime.ledger();
    let task = ledger.get(&task.id).unwrap();
    let closed = ledger
        .close(
            &task.id,
            &task.runs.last().unwrap().id,
            task.revision,
            false,
            "用户标记完成",
        )
        .unwrap();
    let closed = ledger
        .complete_task(&task.id, closed.revision, "用户完成整个任务")
        .unwrap();
    let before = json!(ledger.get(&task.id).unwrap());
    for (name, input) in [
        (
            "create-task-run",
            json!({"taskId":task.id,"expectedRevision":closed.revision,"title":"自动复发处理","agentConversation":["conversation-test"]}),
        ),
        (
            "update-task-run",
            json!({"taskId":task.id,"runId":closed.run_id,"expectedRevision":closed.revision,"status":"in_progress","agentConversation":["conversation-test"]}),
        ),
    ] {
        let result = rpc(Some("codex-c"), name, input)
            .send()
            .await
            .unwrap()
            .json::<Value>()
            .await
            .unwrap();
        assert_eq!(result["result"]["isError"], true, "{result}");
    }
    assert_eq!(json!(ledger.get(&task.id).unwrap()), before);
    endpoint.runtime.shutdown_once().await;
}

#[tokio::test]
async fn edited_instructions_reach_new_initializations_and_survive_restart() {
    let data = tempfile::tempdir().unwrap();
    let endpoint = Endpoint::start(data.path(), 0).await;
    let original = endpoint.rpc("initialize", init()).await;
    assert_eq!(
        original["instructions"],
        codepet_task_mcp::instructions::DEFAULT
    );
    let edited = "自定义项目规则：在方案确定和实现告一段落时合并登记。";
    endpoint
        .runtime
        .save_instructions(0, Some(edited.into()))
        .await
        .unwrap();
    assert_eq!(
        endpoint.runtime.configuration().await.unwrap()["instructions"],
        edited
    );
    assert_eq!(
        endpoint.rpc("initialize", init()).await["instructions"],
        edited
    );
    assert!(endpoint
        .runtime
        .save_instructions(0, Some("过期写入".into()))
        .await
        .is_err());
    endpoint.runtime.shutdown_once().await;
    let restarted = Endpoint::start(data.path(), 0).await;
    assert_eq!(
        restarted.rpc("initialize", init()).await["instructions"],
        edited
    );
    restarted.runtime.save_instructions(1, None).await.unwrap();
    assert_eq!(
        restarted.rpc("initialize", init()).await["instructions"],
        original["instructions"]
    );
    restarted.runtime.shutdown_once().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn required_links_atomic_verification_and_finish_are_enforced_over_http() {
    let root = tempfile::tempdir().unwrap();
    let workspace = repository(root.path());
    let endpoint = Endpoint::start(&root.path().join("data"), 0).await;
    endpoint.rpc("initialize", init()).await;
    let chat = "00000000-0000-4000-8000-000000000004";
    let input = json!({"title":"完成协议","agentWorkspace":{"path":workspace},"initialRun":{"agentConversation":[format!("codex://threads/{chat}")],"validationPlan":{"items":[{"title":"读写","steps":"写入后读取","expected":"一致","scope":"local"}]}}});
    for ids in [Value::Null, json!([]), json!([""]), json!([" "])] {
        let mut bad = input.clone();
        if ids.is_null() {
            bad["initialRun"]
                .as_object_mut()
                .unwrap()
                .remove("agentConversation");
        } else {
            bad["initialRun"]["agentConversation"] = ids;
        }
        let response: Value = endpoint
            .post("tools/call", json!({"name":"create-task","arguments":bad}))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        assert!(
            response.get("error").is_some() || response["result"]["isError"] == true,
            "{response}"
        );
    }
    assert!(endpoint.runtime.ledger().list().unwrap().is_empty());
    let receipt = endpoint.call("create-task", input).await;
    let finish = json!({"taskId":receipt["taskId"],"runId":receipt["runId"],"expectedRevision":1,"conclusion":"开发完成，实际验证通过","changedFiles":[],"agentConversation":[chat],"verification":{"planRevision":1,"stage":"local","target":"HTTP current worktree","environment":"macOS","results":[{"criterionId":receipt["criterionIds"][0],"result":"passed","evidence":"HTTP 实际读写一致"}]}});
    for field in ["result", "evidence"] {
        let mut bad = finish.clone();
        bad["verification"]["results"][0]
            .as_object_mut()
            .unwrap()
            .remove(field);
        let response: Value = endpoint
            .post(
                "tools/call",
                json!({"name":"finish-task-run","arguments":bad}),
            )
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        assert!(
            response.get("error").is_some() || response["result"]["isError"] == true,
            "{response}"
        );
        assert_eq!(
            endpoint
                .runtime
                .ledger()
                .get(receipt["taskId"].as_str().unwrap())
                .unwrap()
                .revision,
            1
        );
    }
    let done = endpoint.call("finish-task-run", finish.clone()).await;
    assert_eq!(done["status"], "ready_for_delivery");
    assert_eq!(
        endpoint.call("finish-task-run", finish).await["revision"],
        2
    );
    let task = endpoint
        .runtime
        .ledger()
        .get(receipt["taskId"].as_str().unwrap())
        .unwrap();
    assert!(task.runs[0].closure.is_none());
    assert_eq!(task.runs[0].validation_plan.records.len(), 1);
    assert_eq!(task.runs[0].validation_plan.records[0].actor, "agent");
    assert_eq!(task.runs[0].agent_conversation[0].conversation_id, chat);
    endpoint.runtime.shutdown_once().await;
}
