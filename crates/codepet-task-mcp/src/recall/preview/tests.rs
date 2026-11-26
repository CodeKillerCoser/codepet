use super::*;
use crate::recall::Recall;
use codepet_task_lineage::tasks::Ledger;
use std::{
    fs,
    time::{Duration, Instant},
};

fn fixture(data: &Path) -> std::path::PathBuf {
    let hash = process::fingerprint(&settings::Settings::default());
    let root = data.join("tasks/recall/indexes").join(hash);
    fs::create_dir_all(root.join("published")).unwrap();
    fs::write(root.join("dataset.json"), r#"{"id":"dataset"}"#).unwrap();
    fs::write(
        root.join("published/preview-graph.json"),
        serde_json::to_vec(&json!({
            "version":VERSION,"dataset":"dataset",
            "graph":{"nodes":[{"id":"completed"}],"links":[],"previewVersion":"one"}
        }))
        .unwrap(),
    )
    .unwrap();
    root
}

#[test]
fn published_preview_is_bounded_and_checks_profile_dataset_and_format() {
    let temp = tempfile::tempdir().unwrap();
    let root = fixture(temp.path());
    let graph = published(temp.path(), None).unwrap();
    assert_eq!(graph["nodes"][0]["id"], "completed");
    assert_eq!(graph["previewStatus"], json!({"preparing":true}));
    let same = published(temp.path(), Some("one")).unwrap();
    assert_eq!(same["unchanged"], true);
    assert!(same.get("nodes").is_none());
    for value in [
        json!({"version":"old"}),
        json!({"version":VERSION,"dataset":"other"}),
    ] {
        fs::write(root.join("published/preview-graph.json"), value.to_string()).unwrap();
        assert!(published(temp.path(), None).is_none());
    }
    fixture(temp.path());
    fs::write(root.join("dataset.json"), r#"{"id":"other"}"#).unwrap();
    assert!(published(temp.path(), None).is_none());
    fixture(temp.path());
    File::options()
        .write(true)
        .open(root.join("published/preview-graph.json"))
        .unwrap()
        .set_len(MAX_BYTES + 1)
        .unwrap();
    assert!(published(temp.path(), None).is_none());
    fixture(temp.path());
    fs::create_dir_all(temp.path().join("model-gateway")).unwrap();
    fs::write(
        temp.path().join("model-gateway/config.json"),
        r#"{"llm":{"model":"different"}}"#,
    )
    .unwrap();
    assert!(published(temp.path(), None).is_none());
}

#[tokio::test]
async fn first_preview_bypasses_install_and_startup_then_hands_off_to_live_service() {
    use axum::{routing::post, Json, Router};
    let temp = tempfile::tempdir().unwrap();
    fixture(temp.path());
    let recall = Recall::shared(&Ledger::new(temp.path()));
    let (done, waiting) = tokio::sync::oneshot::channel();
    recall.prepare(&tokio::runtime::Handle::current(), async {
        waiting.await.unwrap()
    });
    let startup = recall.process.lock().await;
    let graph = tokio::time::timeout(Duration::from_millis(500), recall.graph_preview(false))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(graph["previewSource"], "published");
    assert!(startup.is_none());
    let same = recall
        .graph_preview_since(false, Some("one"))
        .await
        .unwrap();
    assert_eq!(same["unchanged"], true);
    assert!(recall
        .graph_preview_since(true, Some("one"))
        .await
        .unwrap_err()
        .contains("(409)"));
    drop(startup);
    let app = Router::new().route(
        "/graph",
        post(|| async {
            Json(
                json!({"unchanged":true,"previewVersion":"one","previewStatus":{"updating":false}}),
            )
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!("http://{}", listener.local_addr().unwrap());
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    *recall.process.lock().await = Some(std::sync::Arc::new(process::Process::test_client(
        endpoint,
        process::fingerprint(&settings::Settings::default()),
    )));
    done.send(Ok(())).unwrap();
    while recall
        .preparation
        .lock()
        .unwrap()
        .as_ref()
        .unwrap()
        .pending()
    {
        tokio::task::yield_now().await;
    }
    let live = recall
        .graph_preview_since(false, Some("one"))
        .await
        .unwrap();
    assert_eq!(live["previewStatus"]["updating"], false);
    assert!(live.get("previewSource").is_none());
    server.abort();
}

#[tokio::test]
async fn missing_preview_during_install_returns_preparing_without_waiting() {
    let temp = tempfile::tempdir().unwrap();
    let recall = Recall::shared(&Ledger::new(temp.path()));
    recall.prepare(&tokio::runtime::Handle::current(), std::future::pending());
    let error = tokio::time::timeout(Duration::from_millis(500), recall.graph_preview(false))
        .await
        .unwrap()
        .unwrap_err();
    assert!(error.contains("(409)"));
}

#[tokio::test]
#[ignore = "Read-only local real snapshot measurement; explicit data directory required"]
async fn real_published_preview_does_not_wait_for_installation() {
    let data = std::path::PathBuf::from(
        std::env::var_os("CODEPET_RECALL_PREVIEW_PROBE_DATA").expect("set probe data directory"),
    );
    let recall = Recall::shared(&Ledger::new(&data));
    recall.prepare(&tokio::runtime::Handle::current(), std::future::pending());
    let started = Instant::now();
    let graph = recall.graph_preview(false).await.unwrap();
    println!(
        "PUBLISHED_PREVIEW {}",
        json!({"milliseconds":started.elapsed().as_secs_f64()*1000.0,"nodes":graph["nodes"].as_array().unwrap().len(),"links":graph["links"].as_array().unwrap().len(),"source":graph["previewSource"]})
    );
    if let Some(output) = std::env::var_os("CODEPET_RECALL_PREVIEW_PROBE_OUTPUT") {
        fs::write(output, serde_json::to_vec(&graph).unwrap()).unwrap();
    }
}
