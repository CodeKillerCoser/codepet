use axum::{routing::post, Json, Router};
use codepet_model_gateway::{
    settings::{self, Profile, Settings},
    usage, ModelGateway,
};
use serde_json::{json, Value};

#[tokio::test]
async fn authenticated_forwarding_records_real_usage_and_preserves_secrets() {
    let data = tempfile::tempdir().unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!("http://{}/v1", listener.local_addr().unwrap());
    let upstream = tokio::spawn(async move {
        axum::serve(listener,Router::new()
        .route("/v1/chat/completions",post(|headers:axum::http::HeaderMap,Json(body):Json<Value>|async move {
            assert_eq!(headers["authorization"],"Bearer secret");
            assert_eq!(body["model"],"configured-model");
            assert!(body.get("response_format").is_none());
            Json(json!({"choices":[{"message":{"content":"ok"}}],"usage":{"prompt_tokens":17,"completion_tokens":5}}))
        })).route("/v1/embeddings",post(|Json(body):Json<Value>|async move {
            assert_eq!(body["model"],"configured-model");
            let count=body["input"].as_array().unwrap().len(); assert!(count<=8);
            Json(json!({"data":(0..count).map(|i|json!({"index":i,"embedding":[1.0,2.0]})).collect::<Vec<_>>(),"usage":{"prompt_tokens":count}}))
        }))).await.unwrap();
    });
    let profile = Profile {
        endpoint,
        model: "configured-model".into(),
        api_key: "secret".into(),
        dimensions: 2,
    };
    let config = Settings {
        enabled: true,
        llm: profile.clone(),
        embedding: profile,
        plain_json: true,
        ..Default::default()
    };
    let public = settings::save(data.path(), json!(config)).unwrap();
    assert_eq!(public["llm"]["apiKey"], "");
    assert_eq!(public["llm"]["hasApiKey"], true);
    settings::save(data.path(), public).unwrap();
    assert_eq!(settings::read(data.path()).unwrap().llm.api_key, "secret");
    let mut moved = settings::public(&settings::read(data.path()).unwrap());
    moved["llm"]["endpoint"] = json!("http://127.0.0.1:9/v1");
    settings::save(data.path(), moved).unwrap();
    assert!(
        settings::read(data.path()).unwrap().llm.api_key.is_empty(),
        "saved secrets cannot follow a changed endpoint"
    );
    // Deliberately hostile proxy env: forwarding and client still reach loopback.
    std::env::set_var("HTTP_PROXY", "http://127.0.0.1:1");
    std::env::set_var("HTTPS_PROXY", "http://127.0.0.1:1");
    std::env::set_var("ALL_PROXY", "http://127.0.0.1:1");
    std::env::set_var("NO_PROXY", "");
    let gateway = ModelGateway::start(data.path(), config).await.unwrap();
    let client = reqwest::Client::builder().no_proxy().build().unwrap();
    let url = format!("{}/chat/completions", gateway.endpoint);
    let body =
        json!({"model":"attacker-model","response_format":{"type":"json_object"},"messages":[]});
    assert_eq!(
        client.post(&url).json(&body).send().await.unwrap().status(),
        401
    );
    assert_eq!(
        client
            .post(&url)
            .bearer_auth(&gateway.token)
            .header("Origin", "https://evil.invalid")
            .json(&body)
            .send()
            .await
            .unwrap()
            .status(),
        403
    );
    assert_eq!(
        client
            .post(&url)
            .bearer_auth(&gateway.token)
            .header("Host", "evil.invalid")
            .json(&body)
            .send()
            .await
            .unwrap()
            .status(),
        403
    );
    assert_eq!(
        client
            .post(&url)
            .bearer_auth(&gateway.token)
            .json(&json!({"stream":true}))
            .send()
            .await
            .unwrap()
            .status(),
        400
    );
    let response: Value = client
        .post(&url)
        .bearer_auth(&gateway.token)
        .json(&body)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(response["usage"]["prompt_tokens"], 17);
    let embeddings: Value = client
        .post(format!("{}/embeddings", gateway.endpoint))
        .bearer_auth(&gateway.token)
        .json(&json!({"input":vec!["text";9]}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(embeddings["data"].as_array().unwrap().len(), 9);
    assert_eq!(embeddings["data"][8]["index"], 8);
    let stats = usage::summary(data.path()).unwrap();
    let rows = stats["models"].as_array().unwrap();
    assert_eq!(
        rows.iter().find(|r| r["purpose"] == "llm").unwrap()["inputTokens"],
        17
    );
    assert_eq!(
        rows.iter().find(|r| r["purpose"] == "embedding").unwrap()["requests"],
        2
    );
    drop(gateway);
    upstream.abort();
}

#[test]
fn endpoint_and_advanced_options_are_validated() {
    let profile = Profile {
        endpoint: "http://cloud.invalid/v1".into(),
        model: "model".into(),
        dimensions: 1024,
        ..Default::default()
    };
    let mut settings = Settings {
        enabled: true,
        llm: profile.clone(),
        embedding: profile,
        ..Default::default()
    };
    assert!(settings::validate(&settings).is_err());
    settings.llm.endpoint = "http://127.0.0.1:11234/v1".into();
    settings.embedding.endpoint = "https://api.example.invalid/v1".into();
    assert!(settings::validate(&settings).is_ok());
    settings
        .llm_extra_body
        .insert("model".into(), json!("override"));
    assert!(settings::validate(&settings).is_err());
}
