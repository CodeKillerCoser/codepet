#[path = "provider_vertical/conversation_history.rs"]
mod conversation_history;
#[path = "provider_vertical/directory_metadata.rs"]
mod directory_metadata;
#[path = "provider_vertical/interaction.rs"]
mod interaction;
#[path = "provider_vertical/lifecycle_approvals.rs"]
mod lifecycle_approvals;
#[path = "provider_vertical/native_and_titles.rs"]
mod native_and_titles;
#[path = "provider_vertical/projects.rs"]
mod projects;
#[path = "provider_vertical/shared_execution.rs"]
mod shared_execution;
#[path = "provider_vertical/transport_control.rs"]
mod transport_control;

#[path = "../../test_support/mux_stdio.rs"]
mod mux_stdio;

use codepet_provider_codex::{
    CodexProvider, ExecutionLifecycleHook, ProviderEventSink, CODEX_INSTANCE_KIND, CODEX_PLUGIN_ID,
};
use codepet_provider_sdk::{
    ApprovalDecision, ApprovalResolveRequest, ContentBlock, ConversationAcquireInteractionRequest,
    ConversationCreateRequest, ConversationGetRequest, ConversationItem, ConversationListRequest,
    ConversationProjectFilter, ConversationProjectFilterAll, ConversationProjectFilterAllKind,
    ConversationProjectFilterProject, ConversationProjectFilterProjectKind,
    ConversationProjectFilterStandalone, ConversationProjectFilterStandaloneKind,
    ConversationSearchRequest, FlatModelCatalogKind, FlatModelSelection,
    InstanceCapabilitiesRequest, InstanceCreateRequest, InstanceDestroyRequest,
    InstanceStartRequest, InstanceStopRequest, JsonObject, ModelSelection, ProjectCreateRequest,
    ProjectDeleteRequest, ProjectGetRequest, ProjectListRequest, ProjectRoot, ProjectUpdateRequest,
    ProtocolEvent, ProtocolServer as ProviderProtocolServer, ProtocolVersionOffer,
    ProviderInitializeRequest, ProviderInstanceRoute, ProviderResourceId, ProviderShutdownRequest,
    ToolOutcome, TurnInput, TurnInputKind, TurnInterruptRequest, TurnSelection, TurnStartRequest,
    TurnSteerRequest, PROTOCOL_VERSION,
};
use serde_json::json;
use serde_json::Value;
use std::collections::{BTreeMap, VecDeque};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Barrier};
use std::time::{Duration, Instant};

fn provider_executable() -> PathBuf {
    std::env::var_os("CODEPET_TEST_PROVIDER_EXE")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_BIN_EXE_codepet-provider-codex")))
}

fn item_resource_id(item: &ConversationItem) -> &str {
    match item {
        ConversationItem::MessageConversationItem(value) => &value.resource.native_resource_id,
        ConversationItem::ReasoningConversationItem(value) => &value.resource.native_resource_id,
        ConversationItem::CommandConversationItem(value) => &value.resource.native_resource_id,
        ConversationItem::FileChangeConversationItem(value) => &value.resource.native_resource_id,
        ConversationItem::ToolConversationItem(value) => &value.resource.native_resource_id,
        ConversationItem::ApprovalConversationItem(value) => &value.resource.native_resource_id,
        ConversationItem::UnknownConversationItem(value) => &value.resource.native_resource_id,
        ConversationItem::Unrecognized(_) => panic!("fixture expected known item"),
    }
}

fn item_status(item: &ConversationItem) -> codepet_provider_sdk::ConversationItemStatus {
    match item {
        ConversationItem::MessageConversationItem(value) => value.status.clone(),
        ConversationItem::ReasoningConversationItem(value) => value.status.clone(),
        ConversationItem::CommandConversationItem(value) => value.status.clone(),
        ConversationItem::FileChangeConversationItem(value) => value.status.clone(),
        ConversationItem::ToolConversationItem(value) => value.status.clone(),
        ConversationItem::ApprovalConversationItem(value) => value.status.clone(),
        ConversationItem::UnknownConversationItem(value) => value.status.clone(),
        ConversationItem::Unrecognized(_) => panic!("fixture expected known item"),
    }
}

fn item_contents(item: &ConversationItem) -> &[ContentBlock] {
    match item {
        ConversationItem::MessageConversationItem(value) => &value.contents,
        ConversationItem::ReasoningConversationItem(value) => &value.contents,
        ConversationItem::FileChangeConversationItem(value) => &value.contents,
        ConversationItem::CommandConversationItem(value) => match &value.tool.outcome {
            Some(ToolOutcome::ToolSuccessOutcome(outcome)) => &outcome.content,
            Some(ToolOutcome::ToolFailureOutcome(outcome)) => &outcome.content,
            Some(ToolOutcome::Unrecognized(_)) | None => &[],
        },
        ConversationItem::ToolConversationItem(value) => match &value.tool.outcome {
            Some(ToolOutcome::ToolSuccessOutcome(outcome)) => &outcome.content,
            Some(ToolOutcome::ToolFailureOutcome(outcome)) => &outcome.content,
            Some(ToolOutcome::Unrecognized(_)) | None => &[],
        },
        _ => &[],
    }
}

fn content_id(content: &ContentBlock) -> &str {
    match content {
        ContentBlock::TextContentBlock(value) => &value.content_id,
        ContentBlock::ReasoningSummaryContentBlock(value) => &value.content_id,
        ContentBlock::OutputContentBlock(value) => &value.content_id,
        ContentBlock::ActivitySummaryContentBlock(value) => &value.content_id,
        ContentBlock::StructuredJsonContentBlock(value) => &value.content_id,
        ContentBlock::ImageContentBlock(value) => &value.content_id,
        ContentBlock::AudioContentBlock(value) => &value.content_id,
        ContentBlock::ResourceLinkContentBlock(value) => &value.content_id,
        ContentBlock::EmbeddedResourceContentBlock(value) => &value.content_id,
        ContentBlock::Unrecognized(_) => panic!("fixture expected known content"),
    }
}

fn app_server_executable() -> PathBuf {
    std::env::var_os("CODEPET_TEST_APP_SERVER_EXE")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_BIN_EXE_codex-app-server-fixture")))
}

fn route(device_id: &str) -> ProviderInstanceRoute {
    ProviderInstanceRoute {
        device_id: device_id.to_string(),
        provider_plugin_id: CODEX_PLUGIN_ID.to_string(),
        provider_instance_id: "codex".to_string(),
    }
}

fn all_project_filter() -> ConversationProjectFilter {
    ConversationProjectFilter::ConversationProjectFilterAll(ConversationProjectFilterAll {
        kind: ConversationProjectFilterAllKind::All,
    })
}

fn instance_settings(app_server: &Path, approval_mode: &str, marker: &Path) -> JsonObject {
    [
        (
            "dataDirectory".to_string(),
            json!(marker.parent().unwrap().join("codex-home")),
        ),
        (
            "appServerExecutable".to_string(),
            json!(app_server.to_string_lossy()),
        ),
        (
            "appServerArgs".to_string(),
            json!([
                "--approval-mode",
                approval_mode,
                "--marker",
                marker.to_string_lossy()
            ]),
        ),
    ]
    .into_iter()
    .collect()
}

async fn configured_direct_provider(
    approval_mode: &str,
    marker: &Path,
) -> (Arc<CodexProvider>, ProviderInstanceRoute, String) {
    configured_direct_provider_with_events(approval_mode, marker, Arc::new(|_| Ok(()))).await
}

async fn configured_direct_provider_with_events(
    approval_mode: &str,
    marker: &Path,
    events: Arc<dyn ProviderEventSink>,
) -> (Arc<CodexProvider>, ProviderInstanceRoute, String) {
    configured_direct_provider_with_events_and_hook(approval_mode, marker, events, None).await
}

async fn configured_direct_provider_with_events_and_hook(
    approval_mode: &str,
    marker: &Path,
    events: Arc<dyn ProviderEventSink>,
    lifecycle_hook: Option<Arc<dyn ExecutionLifecycleHook>>,
) -> (Arc<CodexProvider>, ProviderInstanceRoute, String) {
    let provider = Arc::new(match lifecycle_hook {
        Some(lifecycle_hook) => {
            CodexProvider::new_with_execution_lifecycle_hook(events, lifecycle_hook)
        }
        None => CodexProvider::new(events),
    });
    let route = route("device-provider-direct");
    ProviderProtocolServer::provider_initialize(
        provider.as_ref(),
        ProviderInitializeRequest {
            directories: None,
            host_client_id: "client-provider-direct".to_string(),
            host_device_id: route.device_id.clone(),
            host_version: "test".to_string(),
            protocol: ProtocolVersionOffer {
                min_version: PROTOCOL_VERSION,
                version: PROTOCOL_VERSION,
            },
        },
    )
    .await
    .unwrap();
    ProviderProtocolServer::instance_create(
        provider.as_ref(),
        InstanceCreateRequest {
            route: route.clone(),
            instance_kind: CODEX_INSTANCE_KIND.to_string(),
            display_name: "Codex Direct Fixture".to_string(),
            settings: instance_settings(&app_server_executable(), approval_mode, marker),
        },
    )
    .await
    .unwrap();
    ProviderProtocolServer::instance_start(
        provider.as_ref(),
        InstanceStartRequest {
            route: route.clone(),
        },
    )
    .await
    .unwrap();
    if !approval_mode.starts_with("metadata-") {
        wait_for_capabilities(provider.as_ref(), &route).await;
    }
    let revision = ProviderProtocolServer::instance_capabilities(
        provider.as_ref(),
        codepet_provider_sdk::InstanceCapabilitiesRequest {
            route: route.clone(),
        },
    )
    .await
    .unwrap()
    .capabilities
    .revision;
    (provider, route, revision)
}

struct BlockHandleOnceHook {
    conversation_id: String,
    operation: &'static str,
    armed: AtomicBool,
    entered: Arc<Barrier>,
    release: Arc<Barrier>,
}

impl ExecutionLifecycleHook for BlockHandleOnceHook {
    fn after_handle_acquired(&self, conversation_id: &str, operation: &str) {
        if conversation_id == self.conversation_id
            && operation == self.operation
            && self.armed.swap(false, Ordering::SeqCst)
        {
            self.entered.wait();
            self.release.wait();
        }
    }
}

struct BlockResumeUntilCancelledHook {
    conversation_id: String,
    resume_entered: Arc<Barrier>,
    resume_release: Arc<Barrier>,
    cancelled_entered: Arc<Barrier>,
    cancelled_release: Arc<Barrier>,
}

impl ExecutionLifecycleHook for BlockResumeUntilCancelledHook {
    fn before_resume_linearization(&self, conversation_id: &str) {
        if conversation_id == self.conversation_id {
            self.resume_entered.wait();
            self.resume_release.wait();
        }
    }

    fn after_execution_cancelled(&self, conversation_id: &str) {
        if conversation_id == self.conversation_id {
            self.cancelled_entered.wait();
            self.cancelled_release.wait();
        }
    }
}

async fn verify_unsupported_project(mode: &str) {
    let marker = tempfile::NamedTempFile::new().unwrap();
    let (provider, route, _) = configured_direct_provider(mode, marker.path()).await;
    wait_for_capabilities(&provider, &route).await;
    let capabilities = ProviderProtocolServer::instance_capabilities(
        provider.as_ref(),
        InstanceCapabilitiesRequest {
            route: route.clone(),
        },
    )
    .await
    .unwrap();
    assert!(!capabilities
        .capabilities
        .methods
        .contains(&codepet_provider_sdk::ProviderCapability::ProjectList));

    assert!(capabilities.capabilities.turn_send.is_some());
    assert!(capabilities
        .capabilities
        .methods
        .contains(&codepet_provider_sdk::ProviderCapability::TurnStart));
    let listed = ProviderProtocolServer::conversation_list(
        provider.as_ref(),
        ConversationListRequest {
            query: None,
            reader_scope: None,
            route: route.clone(),
            cursor: None,
            limit: Some(20),
            project_filter: all_project_filter(),
        },
    )
    .await
    .unwrap();
    assert!(!listed.conversations.is_empty());

    let project = ProviderResourceId {
        device_id: route.device_id.clone(),
        provider_plugin_id: route.provider_plugin_id.clone(),
        provider_instance_id: route.provider_instance_id.clone(),
        native_resource_id: "project-unsupported".to_string(),
    };
    let error = ProviderProtocolServer::conversation_create(
        provider.as_ref(),
        ConversationCreateRequest {
            route: route.clone(),
            project: Some(project),
            title: None,
            permission_level: "workspace-write".to_string(),
            model: None,
            reasoning_effort: None,
            workspace_root: None,
            workspace_mode: None,
            extension: None,
        },
    )
    .await
    .unwrap_err();
    assert_eq!(error.code, "capability_unsupported");
    ProviderProtocolServer::instance_stop(provider.as_ref(), InstanceStopRequest { route })
        .await
        .unwrap();
}

fn assert_provider_reads_full_turns(mode: &str) {
    let directory = tempfile::tempdir().unwrap();
    let marker = directory.path().join("item-paginated-history.txt");
    let request_log = directory.path().join("item-paginated-history-requests.txt");
    let mut provider = ProviderBinary::spawn();
    provider.configure_with_request_log(mode, &marker, Some(&request_log));
    std::fs::write(&request_log, "").unwrap();

    let response = provider.request(
        "item-paginated-history",
        "conversation.get",
        json!({
            "conversation": conversation_resource_value("thread-paginated"),
            "limit": 1
        }),
    );

    assert!(response.get("error").is_none(), "{response}");
    assert_eq!(
        response
            .pointer("/result/items/0/resource/nativeResourceId")
            .and_then(Value::as_str),
        Some("agent-page-two")
    );
    assert_eq!(
        response
            .pointer("/result/pageInfo/nextCursor")
            .and_then(Value::as_str),
        Some("page-two")
    );
    assert_eq!(
        response.pointer("/result/conversation/project"),
        Some(&Value::Null)
    );
    assert_eq!(
        std::fs::read_to_string(&request_log)
            .unwrap()
            .lines()
            .collect::<Vec<_>>(),
        vec![
            "thread/read\tthread-paginated",
            "thread/turns/list\tthread-paginated"
        ]
    );

    provider.request(
        "item-paginated-stop",
        "instance.stop",
        json!({ "route": route_value() }),
    );
    provider.request("item-paginated-shutdown", "provider.shutdown", json!({}));
}

struct MockDesktopShutdown {
    marker: PathBuf,
    calls: std::sync::atomic::AtomicUsize,
    fail: bool,
    activate_after_close: bool,
}

impl ExecutionLifecycleHook for MockDesktopShutdown {
    fn close_desktop_for_takeover(
        &self,
        check: &mut dyn FnMut() -> Result<(), codepet_provider_sdk::ProtocolError>,
    ) -> Result<(), codepet_provider_sdk::ProtocolError> {
        check()?;
        self.calls.fetch_add(1, Ordering::SeqCst);
        if self.fail {
            return Err(codepet_provider_sdk::ProtocolError {
                code: "force_takeover_failed".into(),
                message: "mock permission denied".into(),
                retryable: true,
                details: None,
            });
        }
        std::fs::write(self.marker.with_extension("desktop-closed"), "closed").unwrap();
        if self.activate_after_close {
            std::fs::write(
                self.marker
                    .with_file_name("thread-active-writer.turn-state"),
                "inProgress",
            )
            .unwrap();
        }
        Ok(())
    }
}

struct ProviderBinary {
    child: Child,
    stdin: Option<mux_stdio::Writer>,
    stdout: mux_stdio::Reader,
    buffered: VecDeque<Value>,
    data_directory: tempfile::TempDir,
}

impl ProviderBinary {
    fn spawn() -> Self {
        let mut command = Command::new(provider_executable());
        Self::spawn_command(&mut command)
    }

    fn release_control(&mut self) -> Value {
        let response = self.request(
            "host-release-control",
            "instance.releaseControl",
            json!({
                "route": route_value(), "idleTimeoutSeconds": 2
            }),
        );
        assert!(response.get("error").is_none(), "{response}");
        response["result"]["outcome"].clone()
    }

    fn wait_for_idle_release(&mut self) {
        let deadline = Instant::now() + Duration::from_secs(6);
        loop {
            let outcome = self.release_control();
            if outcome == "released" {
                break;
            }
            assert!(
                matches!(outcome.as_str(), Some("notIdle" | "busy")),
                "{outcome}"
            );
            assert!(
                Instant::now() < deadline,
                "Host release did not complete: {outcome}"
            );
            self.collect_for(Duration::from_millis(50));
        }
    }

    #[cfg(unix)]
    fn spawn_ignoring_sigpipe() -> Self {
        let mut command = Command::new("/bin/sh");
        command
            .args([
                "-c",
                "trap '' PIPE; exec \"$1\"",
                "provider-without-sigpipe",
            ])
            .arg(provider_executable());
        Self::spawn_command(&mut command)
    }

    fn spawn_command(command: &mut Command) -> Self {
        let mut child = command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(
                if std::env::var_os("CODEPET_TEST_PROVIDER_STDERR").is_some() {
                    Stdio::inherit()
                } else {
                    Stdio::null()
                },
            )
            .spawn()
            .unwrap();
        let (stdin, stdout) =
            mux_stdio::connect(child.stdin.take().unwrap(), child.stdout.take().unwrap());
        Self {
            child,
            stdin: Some(stdin),
            stdout,
            buffered: VecDeque::new(),
            data_directory: tempfile::tempdir().unwrap(),
        }
    }

    fn configure(&mut self, approval_mode: &str, marker: &Path) -> (Value, String) {
        self.configure_with_request_log(approval_mode, marker, None)
    }

    fn configure_with_request_log(
        &mut self,
        approval_mode: &str,
        marker: &Path,
        request_log: Option<&Path>,
    ) -> (Value, String) {
        let mut app_server_args = vec![
            "--approval-mode".to_string(),
            approval_mode.to_string(),
            "--marker".to_string(),
            marker.to_string_lossy().into_owned(),
        ];
        if let Some(request_log) = request_log {
            app_server_args.push("--request-log".to_string());
            app_server_args.push(request_log.to_string_lossy().into_owned());
        }
        self.initialize_and_create_instance(&app_server_executable(), app_server_args);
        self.request("start", "instance.start", json!({ "route": route_value() }));
        let conversation = self.create_conversation("conversation-first");
        let capabilities = self.request(
            "catalog",
            "instance.capabilities",
            json!({ "route": route_value() }),
        );
        let capability_revision = capabilities
            .pointer("/result/capabilities/revision")
            .and_then(Value::as_str)
            .unwrap()
            .to_string();
        (conversation, capability_revision)
    }

    fn initialize_and_create_instance(&mut self, executable: &Path, app_server_args: Vec<String>) {
        self.request(
            "initialize",
            "provider.initialize",
            json!({
                "hostClientId": "provider-binary-test",
                "hostDeviceId": "device-provider-binary",
                "hostVersion": "test",
                "protocol": {"version": codepet_provider_sdk::PROTOCOL_VERSION, "minVersion": codepet_provider_sdk::MIN_PROTOCOL_VERSION}
            }),
        );
        self.request(
            "create",
            "instance.create",
            json!({
                "route": route_value(),
                "instanceKind": CODEX_INSTANCE_KIND,
                "displayName": "Codex Binary Fixture",
                "settings": {
                    "appServerExecutable": executable,
                    "dataDirectory": self.data_directory.path(),
                    "appServerArgs": app_server_args
                }
            }),
        );
    }

    fn create_conversation(&mut self, id: &str) -> Value {
        let deadline = Instant::now() + Duration::from_secs(3);
        loop {
            let caps = self.request(
                "wait-catalog",
                "instance.capabilities",
                json!({"route":route_value()}),
            );
            if caps.pointer("/result/capabilities/turnSend").is_some() {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "background catalog unavailable: {caps}"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
        let workspace =
            std::env::temp_dir().join(format!("codepet-codex-provider-{}-{id}", self.child.id()));
        std::fs::create_dir_all(&workspace).unwrap();
        self.request(
            id,
            "conversation.create",
            json!({
                "route": route_value(),
                "permissionLevel": "workspace-write",
                "model": "gpt-fixture",
                "reasoningEffort": "high",
                "workspaceRoot": workspace.to_string_lossy()
            }),
        )
        .pointer("/result/conversation/resource")
        .cloned()
        .unwrap()
    }

    fn request(&mut self, id: &str, method: &str, params: Value) -> Value {
        self.request_with_timeout(id, method, params, Duration::from_secs(5))
    }

    fn request_with_timeout(
        &mut self,
        id: &str,
        method: &str,
        params: Value,
        timeout: Duration,
    ) -> Value {
        self.send_request(id, method, provider_request_params(method, params));
        self.receive(timeout, |message| {
            message.get("id").and_then(Value::as_str) == Some(id)
        })
    }

    fn send_request(&mut self, id: &str, method: &str, params: Value) {
        let stdin = self.stdin.as_mut().expect("Provider stdin is closed");
        stdin.send(json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params }));
    }

    fn close_input_and_wait(&mut self, timeout: Duration) -> std::process::ExitStatus {
        self.close_input_and_wait_result(timeout)
            .expect("Provider did not exit after stdin EOF")
    }

    fn close_input_and_wait_result(
        &mut self,
        timeout: Duration,
    ) -> Option<std::process::ExitStatus> {
        drop(self.stdin.take());
        let deadline = Instant::now() + timeout;
        loop {
            if let Some(status) = self.child.try_wait().unwrap() {
                return Some(status);
            }
            if Instant::now() >= deadline {
                return None;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    fn wait_for_exit(&mut self, timeout: Duration) -> Option<std::process::ExitStatus> {
        let deadline = Instant::now() + timeout;
        loop {
            if let Some(status) = self.child.try_wait().unwrap() {
                return Some(status);
            }
            if Instant::now() >= deadline {
                return None;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    fn close_stdout_after_probe(&mut self) {
        let described = self.request("close-stdout-probe", "provider.describe", json!({}));
        assert_eq!(described["id"], "close-stdout-probe");
        self.stdout.close();
    }

    fn event(&mut self, method: &str) -> Value {
        self.receive(Duration::from_secs(5), |message| {
            message.get("method").and_then(Value::as_str) == Some(method)
        })
    }

    fn receive<F>(&mut self, timeout: Duration, predicate: F) -> Value
    where
        F: Fn(&Value) -> bool,
    {
        if let Some(index) = self.buffered.iter().position(&predicate) {
            return self.buffered.remove(index).unwrap();
        }
        let deadline = Instant::now() + timeout;
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            let message = self.stdout.messages.recv_timeout(remaining).unwrap();
            if predicate(&message) {
                return message;
            }
            self.buffered.push_back(message);
        }
    }

    fn collect_for(&mut self, duration: Duration) {
        let deadline = Instant::now() + duration;
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return;
            }
            match self.stdout.messages.recv_timeout(remaining) {
                Ok(message) => self.buffered.push_back(message),
                Err(mpsc::RecvTimeoutError::Timeout) => return,
                Err(mpsc::RecvTimeoutError::Disconnected) => return,
            }
        }
    }
}

fn provider_request_params(method: &str, mut params: Value) -> Value {
    let resource_fields: &[&str] = match method {
        "conversation.get" | "conversation.acquireInteraction" => &["conversation"],
        "turn.start" => &["conversation"],
        "turn.steer" | "turn.interrupt" => &["conversation", "turn"],
        "approval.resolve" => &["approval"],
        "project.get" | "project.update" | "project.delete" => &["project"],
        _ => &[],
    };
    for field in resource_fields {
        let Some(resource) = params.get_mut(*field) else {
            continue;
        };
        if resource.get("deviceId").is_some() {
            continue;
        }
        let native_resource_id = resource.get("nativeResourceId").cloned();
        if let Some(native_resource_id) = native_resource_id {
            *resource = json!({
                "deviceId": "device-provider-binary",
                "providerPluginId": CODEX_PLUGIN_ID,
                "providerInstanceId": "codex",
                "nativeResourceId": native_resource_id,
            });
        }
    }
    params
}

impl Drop for ProviderBinary {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn route_value() -> Value {
    json!({
        "deviceId": "device-provider-binary",
        "providerPluginId": CODEX_PLUGIN_ID,
        "providerInstanceId": "codex"
    })
}

fn fixture_args(approval_mode: &str, marker: &Path) -> Vec<String> {
    vec![
        "--approval-mode".to_string(),
        approval_mode.to_string(),
        "--marker".to_string(),
        marker.to_string_lossy().into_owned(),
    ]
}

fn conversation_resource_value(native_resource_id: &str) -> Value {
    let mut resource = route_value();
    resource["nativeResourceId"] = json!(native_resource_id);
    resource
}

fn conversation_resource(
    route: &ProviderInstanceRoute,
    native_resource_id: &str,
) -> ProviderResourceId {
    ProviderResourceId {
        device_id: route.device_id.clone(),
        provider_plugin_id: route.provider_plugin_id.clone(),
        provider_instance_id: route.provider_instance_id.clone(),
        native_resource_id: native_resource_id.to_string(),
    }
}

fn clear_session_log(marker: &Path) {
    for path in session_log_paths(marker) {
        std::fs::remove_file(path).unwrap();
    }
}

fn session_pids(marker: &Path, method: &str, thread_id: &str) -> Vec<u32> {
    let mut pids = Vec::new();
    for path in session_log_paths(marker) {
        let Ok(contents) = std::fs::read_to_string(path) else {
            continue;
        };
        for line in contents.lines() {
            let mut fields = line.splitn(3, '\t');
            let Some(pid) = fields.next().and_then(|value| value.parse::<u32>().ok()) else {
                continue;
            };
            let recorded_method = fields.next();
            let recorded_thread = fields.next();
            if recorded_method != Some(method)
                || (thread_id != "*" && recorded_thread != Some(thread_id))
            {
                continue;
            }
            if !pids.contains(&pid) {
                pids.push(pid);
            }
        }
    }
    pids
}

fn session_log_paths(marker: &Path) -> Vec<PathBuf> {
    let Some(parent) = marker.parent() else {
        return Vec::new();
    };
    let Some(stem) = marker.file_stem().and_then(|stem| stem.to_str()) else {
        return Vec::new();
    };
    let prefix = format!("{stem}.sessions.");
    let mut paths = std::fs::read_dir(parent)
        .into_iter()
        .flatten()
        .filter_map(Result::ok)
        .filter_map(|entry| {
            entry
                .file_name()
                .to_str()
                .is_some_and(|name| name.starts_with(&prefix))
                .then(|| entry.path())
        })
        .collect::<Vec<_>>();
    paths.sort();
    paths
}

fn session_method_count(marker: &Path, method: &str) -> usize {
    session_log_paths(marker)
        .into_iter()
        .filter_map(|path| std::fs::read_to_string(path).ok())
        .flat_map(|contents| contents.lines().map(str::to_string).collect::<Vec<_>>())
        .filter(|line| line.split('\t').nth(1) == Some(method))
        .count()
}

fn wait_for_file_method(marker: &Path, method: &str, thread_id: &str, timeout: Duration) {
    let deadline = Instant::now() + timeout;
    while session_pids(marker, method, thread_id).is_empty() {
        assert!(
            Instant::now() < deadline,
            "fixture did not record {method} for {thread_id}"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn wait_for_session_count(marker: &Path, method: &str, expected: usize, timeout: Duration) {
    let deadline = Instant::now() + timeout;
    while session_method_count(marker, method) < expected {
        assert!(
            Instant::now() < deadline,
            "fixture did not record {expected} {method} requests"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[cfg(unix)]
fn process_is_running(process_id: u32) -> bool {
    Command::new("/bin/kill")
        .arg("-0")
        .arg(process_id.to_string())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}

#[cfg(unix)]
fn terminate_processes(process_ids: &[u32]) {
    for process_id in process_ids {
        let _ = Command::new("/bin/kill")
            .args(["-KILL", &process_id.to_string()])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
    wait_for_processes_to_exit(process_ids, Duration::from_secs(2));
}

#[cfg(windows)]
fn process_is_running(process_id: u32) -> bool {
    Command::new("tasklist")
        .args(["/FI", &format!("PID eq {process_id}"), "/FO", "CSV", "/NH"])
        .output()
        .is_ok_and(|output| {
            String::from_utf8_lossy(&output.stdout).contains(&process_id.to_string())
        })
}

fn wait_for_processes_to_exit(process_ids: &[u32], timeout: Duration) {
    let deadline = Instant::now() + timeout;
    loop {
        let running = process_ids
            .iter()
            .copied()
            .filter(|process_id| process_is_running(*process_id))
            .collect::<Vec<_>>();
        if running.is_empty() {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "App Server processes did not exit: {running:?}"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn turn_start_params(
    conversation: Value,
    client_request_id: &str,
    text: &str,
    capability_revision: &str,
) -> Value {
    json!({
        "conversation": conversation,
        "clientRequestId": client_request_id,
        "capabilityRevision": capability_revision,
        "input": { "kind": "text", "text": text },
        "selection": {}
    })
}

fn wait_for_file_blocking(path: &Path) {
    let deadline = Instant::now() + Duration::from_secs(3);
    while !path.is_file() {
        assert!(Instant::now() < deadline, "fixture marker was not written");
        std::thread::sleep(Duration::from_millis(5));
    }
}

async fn wait_for_file(path: &Path) {
    tokio::time::timeout(Duration::from_secs(3), async {
        while !path.is_file() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
}

async fn wait_for_capabilities(provider: &CodexProvider, route: &ProviderInstanceRoute) {
    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            let caps = ProviderProtocolServer::instance_capabilities(
                provider,
                InstanceCapabilitiesRequest {
                    route: route.clone(),
                },
            )
            .await
            .unwrap()
            .capabilities;
            if caps.turn_send.is_some() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("background capabilities");
}

fn wait_for_replacement_server(marker: &std::path::Path, count: usize) {
    let deadline = Instant::now() + Duration::from_secs(3);
    while session_pids(marker, "process/start", "").len() < count {
        assert!(
            Instant::now() < deadline,
            "idle runtime did not restart automatically"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn metadata_read_count(marker: &Path) -> usize {
    session_log_paths(marker)
        .iter()
        .filter_map(|path| std::fs::read_to_string(path).ok())
        .map(|text| {
            text.lines()
                .filter(|line| line.contains("\tthread/read\t"))
                .count()
        })
        .sum()
}

async fn wait_for_metadata_reads(marker: &Path, count: usize) {
    tokio::time::timeout(Duration::from_secs(3), async {
        while metadata_read_count(marker) < count {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("metadata reads should run concurrently");
}

fn ids_list_request(route: &ProviderInstanceRoute, ids: Vec<String>) -> ConversationListRequest {
    ConversationListRequest {
        route: route.clone(),
        project_filter: all_project_filter(),
        cursor: None,
        limit: Some(100),
        query: Some(serde_json::from_value(json!({"kind":"ids","ids":ids})).unwrap()),
        reader_scope: None,
    }
}
