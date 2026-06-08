use crate::app_log;
use crate::events::{PetEvent, PetEventKind};
use crate::settings::{
    DingTalkRobotAuthMode, DingTalkRobotChannel, DingTalkRobotTargetType, FeishuRobotChannel,
    RobotNotificationChannel, RobotNotificationSettings,
};
use base64::Engine;
use chrono::Utc;
use hmac::{Hmac, Mac};
use reqwest::Client;
use serde_json::{json, Value};
use sha2::Sha256;
use std::future::Future;
use std::pin::Pin;
use url::Url;

type StrategyFuture<'a> = Pin<Box<dyn Future<Output = Result<(), String>> + Send + 'a>>;
type HmacSha256 = Hmac<Sha256>;

#[derive(Clone, Debug)]
struct RobotNotificationMessage {
    text: String,
}

trait RobotNotificationStrategy: Send + Sync {
    fn label(&self) -> String;
    fn send<'a>(
        &'a self,
        client: &'a Client,
        message: &'a RobotNotificationMessage,
    ) -> StrategyFuture<'a>;
}

#[derive(Clone)]
struct DingTalkWebhookStrategy {
    channel: DingTalkRobotChannel,
}

#[derive(Clone)]
struct DingTalkEnterpriseStrategy {
    channel: DingTalkRobotChannel,
    endpoints: DingTalkEnterpriseEndpoints,
}

#[derive(Clone)]
struct FeishuWebhookStrategy {
    channel: FeishuRobotChannel,
}

#[derive(Clone)]
struct DingTalkEnterpriseEndpoints {
    access_token_url: String,
    oto_message_url: String,
    group_message_url: String,
}

impl Default for DingTalkEnterpriseEndpoints {
    fn default() -> Self {
        Self {
            access_token_url: "https://api.dingtalk.com/v1.0/oauth2/accessToken".to_string(),
            oto_message_url: "https://api.dingtalk.com/v1.0/robot/oToMessages/batchSend"
                .to_string(),
            group_message_url: "https://api.dingtalk.com/v1.0/robot/groupMessages/send".to_string(),
        }
    }
}

pub fn notify_event(event: PetEvent) {
    tauri::async_runtime::spawn(async move {
        if let Err(error) = send_event_notification(&event).await {
            app_log::error(
                "notifications",
                &format!(
                    "failed to send robot notification error={}",
                    redact_error(&error)
                ),
            );
        }
    });
}

pub async fn send_event_notification(event: &PetEvent) -> Result<(), String> {
    let settings = crate::settings::load_app_settings().map_err(|error| error.to_string())?;
    let robot = &settings.notifications.robot;
    if !robot.enabled || !event_matches_triggers(robot, event) {
        return Ok(());
    }

    let message = message_from_event(event);
    send_robot_notification(robot, &message, None, false)
        .await
        .map(|_| ())
}

pub async fn send_test_notification(channel_id: Option<String>) -> Result<String, String> {
    let settings = crate::settings::load_app_settings().map_err(|error| error.to_string())?;
    let message = RobotNotificationMessage {
        text: format!(
            "Code Pet 测试通知\n状态：测试\n时间：{}",
            Utc::now().format("%Y-%m-%d %H:%M:%S UTC")
        ),
    };
    let sent = send_robot_notification(
        &settings.notifications.robot,
        &message,
        channel_id.as_deref(),
        true,
    )
    .await?;
    Ok(format!("已发送 {sent} 个机器人通知"))
}

fn event_matches_triggers(robot: &RobotNotificationSettings, event: &PetEvent) -> bool {
    match event.kind {
        PetEventKind::PermissionRequested => robot.triggers.waiting_approval,
        PetEventKind::TaskFailed => robot.triggers.task_failed,
        PetEventKind::TaskCompleted => robot.triggers.task_done,
        _ => false,
    }
}

async fn send_robot_notification(
    robot: &RobotNotificationSettings,
    message: &RobotNotificationMessage,
    channel_id: Option<&str>,
    include_disabled: bool,
) -> Result<usize, String> {
    let strategies = strategies_for_settings(robot, channel_id, include_disabled);
    if strategies.is_empty() {
        return Err(match channel_id {
            Some(id) => format!("未找到可测试的机器人渠道：{id}"),
            None => "没有可用的机器人渠道".to_string(),
        });
    }

    let client = Client::builder()
        .timeout(std::time::Duration::from_secs(12))
        .build()
        .map_err(|error| error.to_string())?;
    let mut sent = 0usize;
    let mut failures = Vec::new();
    for strategy in strategies {
        match strategy.send(&client, message).await {
            Ok(()) => sent += 1,
            Err(error) => failures.push(format!("{}：{}", strategy.label(), error)),
        }
    }

    if sent == 0 && !failures.is_empty() {
        return Err(failures.join("；"));
    }
    if !failures.is_empty() {
        app_log::error(
            "notifications",
            &format!(
                "partial robot notification failure error={}",
                redact_error(&failures.join("；"))
            ),
        );
    }
    Ok(sent)
}

fn strategies_for_settings(
    robot: &RobotNotificationSettings,
    channel_id: Option<&str>,
    include_disabled: bool,
) -> Vec<Box<dyn RobotNotificationStrategy>> {
    robot
        .channels
        .iter()
        .filter(|channel| {
            let id = channel_id_for(channel);
            channel_id.map(|target| target == id).unwrap_or(true)
                && (include_disabled || channel_enabled(channel))
        })
        .filter_map(|channel| match channel {
            RobotNotificationChannel::DingTalk(channel) => match channel.auth_mode {
                DingTalkRobotAuthMode::EnterpriseRobot => {
                    Some(Box::new(DingTalkEnterpriseStrategy {
                        channel: channel.clone(),
                        endpoints: DingTalkEnterpriseEndpoints::default(),
                    }) as Box<dyn RobotNotificationStrategy>)
                }
                DingTalkRobotAuthMode::Webhook => Some(Box::new(DingTalkWebhookStrategy {
                    channel: channel.clone(),
                })
                    as Box<dyn RobotNotificationStrategy>),
            },
            RobotNotificationChannel::Feishu(channel) => Some(Box::new(FeishuWebhookStrategy {
                channel: channel.clone(),
            })
                as Box<dyn RobotNotificationStrategy>),
        })
        .collect()
}

fn channel_id_for(channel: &RobotNotificationChannel) -> &str {
    match channel {
        RobotNotificationChannel::DingTalk(channel) => channel.id.as_str(),
        RobotNotificationChannel::Feishu(channel) => channel.id.as_str(),
    }
}

fn channel_enabled(channel: &RobotNotificationChannel) -> bool {
    match channel {
        RobotNotificationChannel::DingTalk(channel) => channel.enabled,
        RobotNotificationChannel::Feishu(channel) => channel.enabled,
    }
}

impl RobotNotificationStrategy for DingTalkWebhookStrategy {
    fn label(&self) -> String {
        channel_label(&self.channel.name, "钉钉 webhook")
    }

    fn send<'a>(
        &'a self,
        client: &'a Client,
        message: &'a RobotNotificationMessage,
    ) -> StrategyFuture<'a> {
        Box::pin(async move {
            let webhook_url = required_field(&self.channel.webhook_url, "钉钉 webhook 地址")?;
            let mut url =
                Url::parse(webhook_url).map_err(|error| format!("webhook 地址无效：{error}"))?;
            if let Some(secret) = optional_field(&self.channel.webhook_secret) {
                let timestamp = Utc::now().timestamp_millis().to_string();
                let sign = dingtalk_sign(&timestamp, secret)?;
                url.query_pairs_mut()
                    .append_pair("timestamp", &timestamp)
                    .append_pair("sign", &sign);
            }

            let body = json!({
                "msgtype": "text",
                "text": {
                    "content": message.text,
                },
            });
            post_json(client, url.as_str(), &body).await.map(|_| ())
        })
    }
}

impl RobotNotificationStrategy for DingTalkEnterpriseStrategy {
    fn label(&self) -> String {
        channel_label(&self.channel.name, "钉钉企业机器人")
    }

    fn send<'a>(
        &'a self,
        client: &'a Client,
        message: &'a RobotNotificationMessage,
    ) -> StrategyFuture<'a> {
        Box::pin(async move {
            let robot_code = required_field(&self.channel.robot_code, "钉钉 robotCode")?;
            let client_id = required_field(&self.channel.client_id, "钉钉 clientId")?;
            let client_secret = required_field(&self.channel.client_secret, "钉钉 clientSecret")?;
            let access_token =
                dingtalk_access_token(client, client_id, client_secret, &self.endpoints).await?;
            let msg_param = serde_json::to_string(&json!({ "content": message.text }))
                .map_err(|error| error.to_string())?;

            let (url, body) = match self.channel.target_type {
                DingTalkRobotTargetType::UserIds => {
                    let user_ids = normalized_list(&self.channel.user_ids);
                    if user_ids.is_empty() {
                        return Err("请填写钉钉接收人 userId".to_string());
                    }
                    (
                        self.endpoints.oto_message_url.as_str(),
                        json!({
                            "robotCode": robot_code,
                            "userIds": user_ids,
                            "msgKey": "sampleText",
                            "msgParam": msg_param,
                        }),
                    )
                }
                DingTalkRobotTargetType::OpenConversationId => {
                    let open_conversation_id = required_field(
                        &self.channel.open_conversation_id,
                        "钉钉 openConversationId",
                    )?;
                    (
                        self.endpoints.group_message_url.as_str(),
                        json!({
                            "robotCode": robot_code,
                            "openConversationId": open_conversation_id,
                            "msgKey": "sampleText",
                            "msgParam": msg_param,
                        }),
                    )
                }
            };

            post_json_with_token(client, url, &body, &access_token)
                .await
                .map(|_| ())
        })
    }
}

impl RobotNotificationStrategy for FeishuWebhookStrategy {
    fn label(&self) -> String {
        channel_label(&self.channel.name, "飞书机器人")
    }

    fn send<'a>(
        &'a self,
        client: &'a Client,
        message: &'a RobotNotificationMessage,
    ) -> StrategyFuture<'a> {
        Box::pin(async move {
            let webhook_url = required_field(&self.channel.webhook_url, "飞书 webhook 地址")?;
            let mut body = json!({
                "msg_type": "text",
                "content": {
                    "text": message.text,
                },
            });
            if let Some(secret) = optional_field(&self.channel.webhook_secret) {
                let timestamp = Utc::now().timestamp().to_string();
                body["timestamp"] = Value::String(timestamp.clone());
                body["sign"] = Value::String(feishu_sign(&timestamp, secret)?);
            }
            post_json(client, webhook_url, &body).await.map(|_| ())
        })
    }
}

async fn dingtalk_access_token(
    client: &Client,
    client_id: &str,
    client_secret: &str,
    endpoints: &DingTalkEnterpriseEndpoints,
) -> Result<String, String> {
    let body = json!({
        "appKey": client_id,
        "appSecret": client_secret,
    });
    let value = post_json(client, &endpoints.access_token_url, &body).await?;
    value
        .get("accessToken")
        .or_else(|| value.get("access_token"))
        .and_then(Value::as_str)
        .map(ToString::to_string)
        .ok_or_else(|| "钉钉 accessToken 响应缺少 accessToken".to_string())
}

async fn post_json(client: &Client, url: &str, body: &Value) -> Result<Value, String> {
    let response = client
        .post(url)
        .json(body)
        .send()
        .await
        .map_err(|error| error.without_url().to_string())?;
    response_value(response).await
}

async fn post_json_with_token(
    client: &Client,
    url: &str,
    body: &Value,
    token: &str,
) -> Result<Value, String> {
    let response = client
        .post(url)
        .header("x-acs-dingtalk-access-token", token)
        .json(body)
        .send()
        .await
        .map_err(|error| error.without_url().to_string())?;
    response_value(response).await
}

async fn response_value(response: reqwest::Response) -> Result<Value, String> {
    let status = response.status();
    let text = response
        .text()
        .await
        .map_err(|error| error.without_url().to_string())?;
    if !status.is_success() {
        return Err(format!(
            "HTTP {} {}",
            status.as_u16(),
            compact_response_text(&text)
        ));
    }
    let value = if text.trim().is_empty() {
        Value::Null
    } else {
        serde_json::from_str::<Value>(&text).unwrap_or(Value::String(text.clone()))
    };
    ensure_provider_success(&value)?;
    Ok(value)
}

fn ensure_provider_success(value: &Value) -> Result<(), String> {
    if let Some(code) = response_code(value, "errcode") {
        if code != 0 {
            return Err(provider_error("errcode", code, value));
        }
    }
    if let Some(code) = response_code(value, "code") {
        if code != 0 {
            return Err(provider_error("code", code, value));
        }
    }
    if let Some(code) = response_code(value, "StatusCode") {
        if code != 0 {
            return Err(provider_error("StatusCode", code, value));
        }
    }
    Ok(())
}

fn response_code(value: &Value, field: &str) -> Option<i64> {
    value
        .get(field)
        .and_then(|code| code.as_i64().or_else(|| code.as_str()?.parse::<i64>().ok()))
}

fn provider_error(field: &str, code: i64, value: &Value) -> String {
    let message = value
        .get("errmsg")
        .or_else(|| value.get("msg"))
        .or_else(|| value.get("message"))
        .and_then(Value::as_str)
        .unwrap_or("接口返回失败");
    format!("{field}={code} {message}")
}

fn message_from_event(event: &PetEvent) -> RobotNotificationMessage {
    let status = match event.kind {
        PetEventKind::PermissionRequested => "等待授权",
        PetEventKind::TaskFailed => "任务失败",
        PetEventKind::TaskCompleted => "任务完成",
        _ => "任务更新",
    };
    let mut lines = vec![
        "Code Pet 通知".to_string(),
        format!("状态：{status}"),
        format!("Agent：{}", event.provider.as_str()),
        format!("任务：{}", compact_line(&event.title, 120)),
    ];
    if !event.message.trim().is_empty() {
        lines.push(format!("内容：{}", compact_line(&event.message, 500)));
    }
    if let Some(cwd) = event
        .cwd
        .as_deref()
        .filter(|value| !value.trim().is_empty())
    {
        lines.push(format!("目录：{}", compact_line(cwd, 180)));
    }
    if let Some(tool_name) = event
        .tool_name
        .as_deref()
        .filter(|value| !value.trim().is_empty())
    {
        lines.push(format!("工具：{}", compact_line(tool_name, 80)));
    }
    lines.push(format!(
        "时间：{}",
        event
            .created_at
            .with_timezone(&Utc)
            .format("%Y-%m-%d %H:%M:%S UTC")
    ));
    RobotNotificationMessage {
        text: lines.join("\n"),
    }
}

fn required_field<'a>(value: &'a str, label: &str) -> Result<&'a str, String> {
    optional_field(value).ok_or_else(|| format!("{label}不能为空"))
}

fn optional_field(value: &str) -> Option<&str> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed)
    }
}

fn normalized_list(values: &[String]) -> Vec<String> {
    values
        .iter()
        .filter_map(|value| optional_field(value).map(ToString::to_string))
        .collect()
}

fn channel_label(name: &str, fallback: &str) -> String {
    optional_field(name).unwrap_or(fallback).to_string()
}

fn dingtalk_sign(timestamp: &str, secret: &str) -> Result<String, String> {
    hmac_sha256_base64(
        secret.as_bytes(),
        format!("{timestamp}\n{secret}").as_bytes(),
    )
}

fn feishu_sign(timestamp: &str, secret: &str) -> Result<String, String> {
    hmac_sha256_base64(format!("{timestamp}\n{secret}").as_bytes(), b"")
}

fn hmac_sha256_base64(key: &[u8], payload: &[u8]) -> Result<String, String> {
    let mut mac = HmacSha256::new_from_slice(key).map_err(|error| error.to_string())?;
    mac.update(payload);
    Ok(base64::engine::general_purpose::STANDARD.encode(mac.finalize().into_bytes()))
}

fn compact_line(value: &str, max_chars: usize) -> String {
    let compact = value.split_whitespace().collect::<Vec<_>>().join(" ");
    if compact.chars().count() <= max_chars {
        return compact;
    }
    let mut next = compact
        .chars()
        .take(max_chars.saturating_sub(1))
        .collect::<String>();
    next.push('…');
    next
}

fn compact_response_text(value: &str) -> String {
    compact_line(value, 240)
}

fn redact_error(error: &str) -> String {
    error
        .split_whitespace()
        .map(|part| {
            if part.contains("access_token=")
                || part.contains("appSecret")
                || part.contains("clientSecret")
                || part.contains("sign=")
            {
                "[redacted]"
            } else {
                part
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agents::AgentId;
    use crate::events::{PetEvent, PetEventKind, TaskStatus};
    use crate::settings::RobotNotificationTriggers;
    use axum::extract::{OriginalUri, RawQuery, State};
    use axum::http::HeaderMap;
    use axum::routing::post;
    use axum::{Json, Router};
    use std::sync::{Arc, Mutex};

    #[derive(Clone, Debug)]
    struct CapturedRequest {
        path: String,
        query: String,
        headers: Vec<(String, String)>,
        body: Value,
    }

    type CapturedRequests = Arc<Mutex<Vec<CapturedRequest>>>;

    fn event(kind: PetEventKind) -> PetEvent {
        PetEvent {
            id: "event-1".to_string(),
            provider: AgentId::Codex,
            kind,
            status: TaskStatus::Done,
            title: "生成报告".to_string(),
            message: "任务已经完成".to_string(),
            session_id: Some("session-1".to_string()),
            cwd: Some("/workspace/codepet".to_string()),
            tool_name: None,
            should_ring: true,
            created_at: Utc::now(),
            raw: Value::Null,
            source: None,
        }
    }

    async fn start_capture_server() -> (String, CapturedRequests) {
        let captured = Arc::new(Mutex::new(Vec::new()));
        let app = Router::new()
            .route("/*path", post(capture_robot_request))
            .with_state(captured.clone());
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        (format!("http://{addr}/"), captured)
    }

    async fn capture_robot_request(
        State(captured): State<CapturedRequests>,
        OriginalUri(uri): OriginalUri,
        RawQuery(query): RawQuery,
        headers: HeaderMap,
        Json(body): Json<Value>,
    ) -> Json<Value> {
        let headers = headers
            .iter()
            .filter_map(|(key, value)| {
                Some((key.as_str().to_string(), value.to_str().ok()?.to_string()))
            })
            .collect::<Vec<_>>();
        captured.lock().unwrap().push(CapturedRequest {
            path: uri.path().to_string(),
            query: query.unwrap_or_default(),
            headers,
            body,
        });

        if uri.path().contains("oauth") {
            Json(json!({ "accessToken": "token-1" }))
        } else {
            Json(json!({ "errcode": 0, "code": 0, "StatusCode": 0 }))
        }
    }

    fn dingtalk_channel(webhook_url: String, webhook_secret: &str) -> RobotNotificationChannel {
        RobotNotificationChannel::DingTalk(DingTalkRobotChannel {
            id: "ding-webhook".to_string(),
            name: "钉钉 webhook".to_string(),
            enabled: true,
            auth_mode: DingTalkRobotAuthMode::Webhook,
            target_type: DingTalkRobotTargetType::UserIds,
            robot_code: String::new(),
            client_id: String::new(),
            client_secret: String::new(),
            user_ids: Vec::new(),
            open_conversation_id: String::new(),
            webhook_url,
            webhook_secret: webhook_secret.to_string(),
        })
    }

    fn feishu_channel(webhook_url: String, webhook_secret: &str) -> RobotNotificationChannel {
        RobotNotificationChannel::Feishu(FeishuRobotChannel {
            id: "feishu-webhook".to_string(),
            name: "飞书 webhook".to_string(),
            enabled: true,
            webhook_url,
            webhook_secret: webhook_secret.to_string(),
        })
    }

    fn robot_with_channel(channel: RobotNotificationChannel) -> RobotNotificationSettings {
        RobotNotificationSettings {
            enabled: true,
            triggers: RobotNotificationTriggers::default(),
            channels: vec![channel],
        }
    }

    #[test]
    fn robot_triggers_match_task_outcomes_and_approval() {
        let mut robot = RobotNotificationSettings::default();
        robot.triggers.task_done = false;

        assert!(event_matches_triggers(
            &robot,
            &event(PetEventKind::PermissionRequested)
        ));
        assert!(event_matches_triggers(
            &robot,
            &event(PetEventKind::TaskFailed)
        ));
        assert!(!event_matches_triggers(
            &robot,
            &event(PetEventKind::TaskCompleted)
        ));
        assert!(!event_matches_triggers(
            &robot,
            &event(PetEventKind::TaskUpdated)
        ));
    }

    #[test]
    fn event_message_keeps_code_pet_keyword_and_context() {
        let message = message_from_event(&event(PetEventKind::TaskCompleted));

        assert!(message.text.contains("Code Pet 通知"));
        assert!(message.text.contains("状态：任务完成"));
        assert!(message.text.contains("Agent：codex"));
        assert!(message.text.contains("任务：生成报告"));
        assert!(message.text.contains("目录：/workspace/codepet"));
    }

    #[tokio::test]
    async fn dingtalk_webhook_posts_text_and_signed_query_to_local_server() {
        let (base_url, captured) = start_capture_server().await;
        let robot = robot_with_channel(dingtalk_channel(format!("{base_url}ding"), "secret"));
        let message = message_from_event(&event(PetEventKind::TaskCompleted));

        let sent = send_robot_notification(&robot, &message, None, false)
            .await
            .unwrap();

        let requests = captured.lock().unwrap();
        assert_eq!(sent, 1);
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].path, "/ding");
        assert!(requests[0].query.contains("timestamp="));
        assert!(requests[0].query.contains("sign="));
        assert_eq!(requests[0].body["msgtype"], "text");
        assert!(requests[0].body["text"]["content"]
            .as_str()
            .unwrap()
            .contains("状态：任务完成"));
    }

    #[tokio::test]
    async fn feishu_webhook_posts_text_and_signature_to_local_server() {
        let (base_url, captured) = start_capture_server().await;
        let robot = robot_with_channel(feishu_channel(format!("{base_url}feishu"), "secret"));
        let message = message_from_event(&event(PetEventKind::PermissionRequested));

        let sent = send_robot_notification(&robot, &message, None, false)
            .await
            .unwrap();

        let requests = captured.lock().unwrap();
        assert_eq!(sent, 1);
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].path, "/feishu");
        assert_eq!(requests[0].body["msg_type"], "text");
        assert!(requests[0].body["timestamp"].as_str().is_some());
        assert!(requests[0].body["sign"].as_str().is_some());
        assert!(requests[0].body["content"]["text"]
            .as_str()
            .unwrap()
            .contains("状态：等待授权"));
    }

    #[tokio::test]
    async fn dingtalk_enterprise_robot_gets_token_then_posts_message_to_local_server() {
        let (base_url, captured) = start_capture_server().await;
        let channel = DingTalkRobotChannel {
            id: "ding-enterprise".to_string(),
            name: "钉钉企业机器人".to_string(),
            enabled: true,
            auth_mode: DingTalkRobotAuthMode::EnterpriseRobot,
            target_type: DingTalkRobotTargetType::UserIds,
            robot_code: "robot-code".to_string(),
            client_id: "client-id".to_string(),
            client_secret: "client-secret".to_string(),
            user_ids: vec!["user-a".to_string()],
            open_conversation_id: String::new(),
            webhook_url: String::new(),
            webhook_secret: String::new(),
        };
        let strategy = DingTalkEnterpriseStrategy {
            channel,
            endpoints: DingTalkEnterpriseEndpoints {
                access_token_url: format!("{base_url}oauth"),
                oto_message_url: format!("{base_url}oto"),
                group_message_url: format!("{base_url}group"),
            },
        };
        let client = Client::builder()
            .timeout(std::time::Duration::from_secs(3))
            .build()
            .unwrap();
        let message = message_from_event(&event(PetEventKind::TaskFailed));

        strategy.send(&client, &message).await.unwrap();

        let requests = captured.lock().unwrap();
        let oauth = requests
            .iter()
            .find(|request| request.path == "/oauth")
            .unwrap();
        let oto = requests
            .iter()
            .find(|request| request.path == "/oto")
            .unwrap();
        assert_eq!(oauth.body["appKey"], "client-id");
        assert_eq!(oauth.body["appSecret"], "client-secret");
        assert_eq!(oto.body["robotCode"], "robot-code");
        assert_eq!(oto.body["userIds"][0], "user-a");
        assert_eq!(oto.body["msgKey"], "sampleText");
        assert!(oto
            .headers
            .iter()
            .any(|(key, value)| { key == "x-acs-dingtalk-access-token" && value == "token-1" }));
        let msg_param: Value =
            serde_json::from_str(oto.body["msgParam"].as_str().unwrap()).unwrap();
        assert!(msg_param["content"]
            .as_str()
            .unwrap()
            .contains("状态：任务失败"));
    }
}
