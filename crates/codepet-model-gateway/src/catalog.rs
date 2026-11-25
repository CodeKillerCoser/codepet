//! Main-window model discovery. Never returns credentials or forwards them across redirects.
use crate::settings::{self, Profile};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{collections::BTreeMap, path::Path, time::Duration};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Model {
    pub id: String,
    pub kind: Option<String>,
    pub dimensions: Option<u32>,
}
pub fn profile(data: &Path, purpose: &str, mut draft: Value) -> Result<Profile, String> {
    let old = settings::read(data)?;
    let saved = match purpose {
        "llm" => old.llm,
        "embedding" => old.embedding,
        _ => return Err("未知模型用途".into()),
    };
    draft
        .as_object_mut()
        .ok_or("模型配置格式无效")?
        .remove("hasApiKey");
    let mut profile: Profile = serde_json::from_value(draft).map_err(|_| "模型配置格式无效")?;
    endpoint(&profile.endpoint)?;
    if profile.api_key.is_empty()
        && profile.endpoint.trim_end_matches('/') == saved.endpoint.trim_end_matches('/')
    {
        profile.api_key = saved.api_key;
    }
    Ok(profile)
}
pub fn endpoint(value: &str) -> Result<reqwest::Url, String> {
    let url = reqwest::Url::parse(value).map_err(|_| "请填写完整 API 地址")?;
    let local = matches!(url.host_str(), Some("localhost" | "127.0.0.1" | "[::1]"));
    if url.host_str().is_none()
        || (url.scheme() != "https" && !(url.scheme() == "http" && local))
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err("API 地址须为 HTTPS，或本机 HTTP；不能包含账号、查询参数或片段".into());
    }
    Ok(url)
}
pub async fn models(profile: Profile) -> Result<Vec<Model>, String> {
    endpoint(&profile.endpoint)?;
    let client = reqwest::Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(Duration::from_secs(5))
        .timeout(Duration::from_secs(20))
        .build()
        .map_err(|_| "无法初始化模型连接")?;
    let mut request = client.get(format!("{}/models", profile.endpoint.trim_end_matches('/')));
    if !profile.api_key.is_empty() {
        request = request.bearer_auth(&profile.api_key);
    }
    let mut response = request
        .send()
        .await
        .map_err(|_| "无法连接模型服务，请检查 API 地址和网络")?;
    let status = response.status();
    if !status.is_success() {
        return Err(match status.as_u16() {
            401 | 403 => format!("密钥无效或没有访问模型列表的权限（{}）", status.as_u16()),
            404 | 405 => "此服务不提供模型列表，可手动填写模型名称（404/405）".into(),
            300..=399 => "服务返回重定向，请填写最终 API 地址后重试".into(),
            _ => format!("模型服务暂不可用（{}），可稍后重试", status.as_u16()),
        });
    }
    let mut body = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|_| "读取模型列表失败")? {
        if body.len() + chunk.len() > 2 * 1024 * 1024 {
            return Err("模型列表响应过大".into());
        }
        body.extend_from_slice(&chunk);
    }
    parse(&serde_json::from_slice(&body).map_err(|_| "模型列表不是有效 JSON")?)
}
fn parse(value: &Value) -> Result<Vec<Model>, String> {
    let rows = value["data"]
        .as_array()
        .ok_or("服务未返回兼容的模型列表；可以手动填写模型名称")?;
    if rows.len() > 5000 {
        return Err("模型列表数量过多".into());
    }
    let mut result: BTreeMap<String, Model> = BTreeMap::new();
    for row in rows {
        let Some(id) = row["id"]
            .as_str()
            .filter(|id| !id.trim().is_empty() && id.len() <= 256)
        else {
            continue;
        };
        let mut kind = row["type"]
            .as_str()
            .filter(|kind| {
                matches!(
                    *kind,
                    "embedding" | "llm" | "language" | "image" | "audio" | "video"
                )
            })
            .map(str::to_owned);
        let has = |name: &str| {
            row["capabilities"]
                .as_array()
                .is_some_and(|caps| caps.iter().any(|cap| cap.as_str() == Some(name)))
        };
        if kind.is_none() {
            kind = if has("embeddings") || id.to_ascii_lowercase().contains("embedding") {
                Some("embedding".into())
            } else if has("chat") {
                Some("llm".into())
            } else if has("audio") || has("speech") || has("tts") || has("transcription") {
                Some("audio".into())
            } else if has("video") {
                Some("video".into())
            } else if has("image") {
                Some("image".into())
            } else {
                None
            };
        }
        let dimensions = row["embedding_dimensions"]
            .as_u64()
            .or_else(|| row["dimensions"].as_u64())
            .filter(|n| (1..=65536).contains(n))
            .map(|n| n as u32);
        let model = result.entry(id.to_owned()).or_insert_with(|| Model {
            id: id.into(),
            kind: None,
            dimensions: None,
        });
        if model.kind.is_none() {
            model.kind = kind;
        }
        if model.dimensions.is_none() {
            model.dimensions = dimensions;
        }
    }
    Ok(result.into_values().collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn drafts_only_reuse_a_secret_for_the_same_saved_endpoint() {
        let root = tempfile::tempdir().unwrap();
        settings::save(root.path(),json!({"enabled":false,"llm":{"endpoint":"https://provider.test/v1","apiKey":"secret"},"embedding":{}})).unwrap();
        let draft = json!({"endpoint":"https://provider.test/v1/","hasApiKey":true});
        assert_eq!(
            profile(root.path(), "llm", draft).unwrap().api_key,
            "secret"
        );
        assert!(profile(
            root.path(),
            "llm",
            json!({"endpoint":"https://other.test/v1"})
        )
        .unwrap()
        .api_key
        .is_empty());
        assert!(endpoint("https://secret@example.test/v1").is_err());
        assert!(endpoint("http://example.test/v1").is_err());
    }
    #[test]
    fn parses_only_bounded_ids_and_actual_metadata() {
        let models=parse(&json!({"data":[{"id":"z"},{"id":"a","type":"embedding","dimensions":1024},{"id":"a"},{"id":""}]})).unwrap();
        assert_eq!(models.len(), 2);
        assert_eq!(models[0].id, "a");
        assert_eq!(models[0].dimensions, Some(1024));
        assert!(models[1].dimensions.is_none());
    }
}

#[cfg(test)]
mod transport_tests {
    use super::*;
    use axum::{
        extract::Request,
        http::{HeaderMap, StatusCode},
        routing::get,
        Router,
    };
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    };
    async fn server(router: Router) -> (String, tokio::task::JoinHandle<()>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!("http://{}", listener.local_addr().unwrap());
        (
            endpoint,
            tokio::spawn(async move { axum::serve(listener, router).await.unwrap() }),
        )
    }
    #[tokio::test]
    async fn discovery_sends_auth_and_never_returns_secrets() {
        let router=Router::new().route("/v1/models",get(|headers:HeaderMap|async move{
            assert_eq!(headers.get("authorization").unwrap(),"Bearer test-private-key");
            axum::Json(serde_json::json!({"data":[{"id":"chat","capabilities":["chat"]},{"id":"vector","capabilities":["embeddings"],"dimensions":1024}]}))
        }));
        let (endpoint, task) = server(router).await;
        let models = models(Profile {
            endpoint: format!("{endpoint}/v1/"),
            api_key: "test-private-key".into(),
            ..Default::default()
        })
        .await
        .unwrap();
        assert_eq!(models.len(), 2);
        assert_eq!(models[1].dimensions, Some(1024));
        assert!(!serde_json::to_string(&models)
            .unwrap()
            .contains("test-private-key"));
        task.abort();
    }
    #[tokio::test]
    async fn redirects_are_not_followed_and_errors_do_not_echo_upstream() {
        let reached = Arc::new(AtomicUsize::new(0));
        let counter = reached.clone();
        let router = Router::new()
            .route(
                "/models",
                get(|| async {
                    (
                        StatusCode::FOUND,
                        [("location", "/sink/models")],
                        "echo-private-key",
                    )
                }),
            )
            .route(
                "/sink/models",
                get(move |_: Request| async move {
                    counter.fetch_add(1, Ordering::SeqCst);
                    "not reached"
                }),
            )
            .route(
                "/error/models",
                get(|| async { (StatusCode::UNAUTHORIZED, "echo-private-key") }),
            );
        let (endpoint, task) = server(router).await;
        let draft = Profile {
            endpoint: endpoint.clone(),
            api_key: "echo-private-key".into(),
            ..Default::default()
        };
        assert!(models(draft.clone()).await.unwrap_err().contains("重定向"));
        assert_eq!(reached.load(Ordering::SeqCst), 0);
        let error = models(Profile {
            endpoint: format!("{endpoint}/error"),
            ..draft
        })
        .await
        .unwrap_err();
        assert!(error.contains("401"));
        assert!(!error.contains("private-key"));
        task.abort();
    }
}

#[cfg(test)]
mod live_tests {
    use super::*;
    #[tokio::test]
    #[ignore = "requires a running local MLX API; lists models only, performs no inference"]
    async fn installed_mlx_catalog_distinguishes_language_embedding_and_media() {
        let found = models(Profile {
            endpoint: "http://127.0.0.1:11234/v1".into(),
            ..Default::default()
        })
        .await
        .unwrap();
        assert!(found
            .iter()
            .any(|model| model.kind.as_deref() == Some("llm")));
        assert!(found
            .iter()
            .any(|model| model.kind.as_deref() == Some("embedding")));
        for model in found {
            if model.id.contains("LTX") {
                assert_eq!(model.kind.as_deref(), Some("video"));
            }
        }
    }
}
