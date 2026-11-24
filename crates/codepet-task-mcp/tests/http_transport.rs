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
    assert_eq!(tools["tools"].as_array().unwrap().len(), 2);
    assert!(serde_json::to_vec(&tools).unwrap().len() < 14000);
    let input =
        json!({"workspace":workspace,"title":"HTTP 任务","objective":"通过本机 HTTP 登记目标"});
    let (first, retry) = tokio::join!(
        endpoint.call("tasks_save", input.clone()),
        endpoint.call("tasks_save", input)
    );
    assert_eq!(first["taskId"], retry["taskId"]);
    assert_eq!(first["revision"], 1);
    let task_id = first["taskId"].as_str().unwrap();
    assert_eq!(
        endpoint.runtime.ledger().get(task_id).unwrap().title,
        "HTTP 任务"
    );
    let unauthorized_actor = endpoint.post("tools/call", json!({"name":"tasks_save","arguments":{"taskId":task_id,"expectedRevision":1,"actor":"human"}})).send().await.unwrap();
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
    let task = reopened.call("tasks_find", json!({"taskId":task_id})).await;
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
    let http = reqwest_mcp::Client::builder().no_proxy().build().unwrap();
    let transport = StreamableHttpClientTransport::with_client(
        http,
        StreamableHttpClientTransportConfig::with_uri(endpoint.url.clone())
            .auth_header(endpoint.authorization.trim_start_matches("Bearer ")),
    );
    let client = ClientConfig::default().serve(transport).await.unwrap();
    assert_eq!(client.list_all_tools().await.unwrap().len(), 2);
    let result = client
        .call_tool(
            CallToolRequestParams::new("tasks_find")
                .with_arguments(serde_json::from_value(json!({"query":"SDK client"})).unwrap()),
        )
        .await
        .unwrap();
    assert_ne!(result.is_error, Some(true));
    client.cancel().await.unwrap();
    endpoint.runtime.shutdown_once().await;
}
