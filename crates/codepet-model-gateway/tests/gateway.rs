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

#[tokio::test]
async fn attempt_usage_is_exact_across_batches_queries_and_missing_reports() {
    let data = tempfile::tempdir().unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!("http://{}/v1", listener.local_addr().unwrap());
    let upstream = tokio::spawn(async move {
        axum::serve(listener, Router::new()
            .route("/v1/chat/completions", post(|headers: axum::http::HeaderMap, Json(body): Json<Value>| async move {
                assert!(!headers.contains_key("x-codepet-processing-job"), "private attempt identity must not reach the provider");
                if body["missing"] == true { Json(json!({"choices":[]})) }
                else { Json(json!({"choices":[],"usage":{"prompt_tokens":10,"completion_tokens":3}})) }
            }))
            .route("/v1/embeddings", post(|Json(body): Json<Value>| async move {
                let count = body["input"].as_array().unwrap().len();
                Json(json!({"data":(0..count).map(|i|json!({"index":i,"embedding":[1.0,2.0]})).collect::<Vec<_>>(),"usage":{"prompt_tokens":count}}))
            }))).await.unwrap();
    });
    let profile = Profile {
        endpoint,
        model: "test".into(),
        dimensions: 2,
        ..Default::default()
    };
    let gateway = ModelGateway::start(
        data.path(),
        Settings {
            enabled: true,
            llm: profile.clone(),
            embedding: profile,
            ..Default::default()
        },
    )
    .await
    .unwrap();
    let client = reqwest::Client::builder().no_proxy().build().unwrap();
    let first = uuid::Uuid::new_v4().to_string();
    let second = uuid::Uuid::new_v4().to_string();
    for (job, missing) in [(Some(&first), false), (Some(&second), true), (None, false)] {
        let mut request = client
            .post(format!("{}/chat/completions", gateway.endpoint))
            .bearer_auth(&gateway.token)
            .json(&json!({"messages":[],"missing":missing}));
        if let Some(job) = job {
            request = request.header("x-codepet-processing-job", job);
        }
        assert!(request.send().await.unwrap().status().is_success());
    }
    assert!(client
        .post(format!("{}/embeddings", gateway.endpoint))
        .bearer_auth(&gateway.token)
        .header("x-codepet-processing-job", &first)
        .json(&json!({"input":vec!["text";9]}))
        .send()
        .await
        .unwrap()
        .status()
        .is_success());
    let usage = usage::for_jobs(data.path(), &[first.clone(), second.clone()]).unwrap();
    assert_eq!(usage[&first]["requests"], 3);
    assert_eq!(usage[&first]["knownTokens"], 22);
    assert_eq!(usage[&first]["complete"], true);
    assert_eq!(usage[&second]["requests"], 1);
    assert_eq!(usage[&second]["complete"], false);
    assert_eq!(usage[&second]["reportedRequests"], 0);
    let bad = client
        .post(format!("{}/chat/completions", gateway.endpoint))
        .bearer_auth(&gateway.token)
        .header("x-codepet-processing-job", "invalid")
        .json(&json!({}))
        .send()
        .await
        .unwrap();
    assert_eq!(bad.status(), 400);
    upstream.abort();
}
