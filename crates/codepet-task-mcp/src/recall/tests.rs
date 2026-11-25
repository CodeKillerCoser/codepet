use super::*;
use codepet_model_gateway::settings::{Profile, Settings};

#[tokio::test]
async fn busy_preview_does_not_restart_or_interrupt_ingestion() {
    use axum::{
        http::StatusCode,
        routing::{get, post},
        Json, Router,
    };
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!("http://{}", listener.local_addr().unwrap());
    let task = tokio::spawn(async move {
        axum::serve(
            listener,
            Router::new()
                .route("/graph", post(|| async { StatusCode::CONFLICT }))
                .route("/health", get(|| async { Json(json!({"protocol":1})) })),
        )
        .await
        .unwrap();
    });
    let temp = tempfile::tempdir().unwrap();
    let ledger = Ledger::new(temp.path());
    let recall = Recall::shared(&ledger);
    let active = Arc::new(process::Process::test_client(
        endpoint,
        process::fingerprint(&Settings::default()),
    ));
    *recall.process.lock().await = Some(active.clone());
    assert!(recall.graph().await.unwrap_err().contains("409"));
    assert!(recall
        .process
        .lock()
        .await
        .as_ref()
        .is_some_and(|p| Arc::ptr_eq(p, &active)));
    task.abort();
}

#[tokio::test]
#[ignore = "requires an already processed native Cognee dataset"]
async fn real_semantic_preview_export() {
    let output = PathBuf::from(std::env::var("CODEPET_RECALL_PROBE_OUTPUT").unwrap());
    let endpoint = "http://127.0.0.1:11234/v1".to_owned();
    let config = Settings {
        enabled: true,
        plain_json: true,
        llm: Profile {
            endpoint: endpoint.clone(),
            model: "ddalcu/Qwen3.8-27B-MLX-Serve-4bit".into(),
            ..Default::default()
        },
        embedding: Profile {
            endpoint,
            model: "Qwen3-Embedding-0.6B-8bit".into(),
            dimensions: 1024,
            ..Default::default()
        },
        llm_extra_body: json!({"chat_template_kwargs":{"enable_thinking":false}})
            .as_object()
            .unwrap()
            .clone(),
    };
    let before = codepet_model_gateway::usage::summary(&output).unwrap();
    let process = process::Process::start(&output, config).await.unwrap();
    let graph = process
        .request("/graph", Some(json!({"semantic":true})))
        .await
        .unwrap();
    assert!(!graph["nodes"].as_array().unwrap().is_empty());
    assert!(
        graph["semantic_positions"]
            .as_object()
            .is_some_and(|p| !p.is_empty()),
        "native semantic coordinates missing"
    );
    assert_eq!(
        before,
        codepet_model_gateway::usage::summary(&output).unwrap(),
        "preview invoked a model"
    );
    std::fs::write(
        output.parent().unwrap().join("preview-graph.json"),
        serde_json::to_vec_pretty(&graph).unwrap(),
    )
    .unwrap();
}

#[test]
fn finish_snapshot_keeps_actual_evidence_and_excludes_git_observation_noise() {
    use codepet_task_lineage::tasks::{Actor, CreateTaskInput, FinishTaskRunInput};
    let temp = tempfile::tempdir().unwrap();
    let ledger = Ledger::new(temp.path());
    let input:CreateTaskInput=serde_json::from_value(json!({"title":"抽屉预览","agentWorkspace":{"path":temp.path()},
        "initialRun":{"agentConversation":["00000000-0000-4000-8000-000000000001"],"goal":"恢复宽度",
            "validationPlan":{"items":[{"title":"双击复位","steps":"双击分隔线","expected":"恢复默认宽度"}]}}})).unwrap();
    let receipt = ledger
        .create_task(input, Actor::agent("codex").unwrap())
        .unwrap();
    assert_eq!(snapshot::documents(&ledger).unwrap()[0]["eligible"], false);
    let finish:FinishTaskRunInput=serde_json::from_value(json!({"taskId":receipt.task_id,"runId":receipt.run_id,"expectedRevision":receipt.revision,
        "conclusion":"已实现复位","changedFiles":[],"agentConversation":["00000000-0000-4000-8000-000000000001"],
        "verification":{"planRevision":receipt.plan_revision,"stage":"local","target":"working tree","environment":"isolated test",
            "results":[{"criterionId":receipt.criterion_ids[0],"result":"passed","evidence":"实际执行双击复位，宽度恢复"}]}})).unwrap();
    ledger
        .finish_task_run(finish, Actor::agent("codex").unwrap())
        .unwrap();
    let docs = snapshot::documents(&ledger).unwrap();
    assert_eq!(docs[0]["eligible"], true);
    let text = docs[0]["text"].as_str().unwrap();
    assert!(text.contains("实际执行双击复位，宽度恢复"));
    assert!(text.contains("verificationRecords"));
    assert!(!text.contains("observedAt"));
}

#[test]
#[ignore = "requires the native compressed sidecar artifact"]
fn installs_real_archive() {
    let source = PathBuf::from(std::env::var("CODEPET_RECALL_ARCHIVE_SOURCE").unwrap());
    let workspace = PathBuf::from(std::env::var("CODEPET_RECALL_ARCHIVE_WORKSPACE").unwrap());
    let installed = bundle::install(&source, &workspace).unwrap().unwrap();
    assert_eq!(
        installed,
        bundle::install(&source, &workspace).unwrap().unwrap()
    );
    assert!(installed.join("main.py").is_file());
    std::fs::write(
        workspace.join("installed-path.txt"),
        installed.to_string_lossy().as_bytes(),
    )
    .unwrap();
}

#[test]
fn fusion_rejects_stale_unknown_and_duplicate_sources() {
    let docs = vec![
        json!({"runId":"a","contentHash":"new","workspace":"other","updatedAt":1,"summary":{"runId":"a"}}),
        json!({"runId":"b","contentHash":"b","workspace":"current","updatedAt":2,"summary":{"runId":"b"}}),
    ];
    let result = merge(
        json!({"hits":[{"runId":"a","contentHash":"old","score":10},
        {"runId":"unknown","contentHash":"x","score":10},{"runId":"b","contentHash":"b","score":0.1},
        {"runId":"a","contentHash":"new","score":0.1},{"runId":"a","contentHash":"new","score":0.1}]}),
        &docs,
        Some("current"),
    );
    assert_eq!(result["total"], 2);
    assert_eq!(result["runs"][0]["runId"], "b");
    assert!(result["runs"]
        .as_array()
        .unwrap()
        .iter()
        .any(|r| r["runId"] == "a"));
}

/// Opt-in real model verification; writes only an isolated artifacts directory.
#[tokio::test]
#[ignore = "requires the user's local MLX models and an independently packaged sidecar"]
async fn real_packaged_cognee() {
    let source =
        std::env::var("CODEPET_RECALL_PROBE_DOCUMENTS").expect("set fixed Run document fixture");
    let output = PathBuf::from(
        std::env::var("CODEPET_RECALL_PROBE_OUTPUT").expect("set isolated output directory"),
    );
    std::fs::create_dir_all(&output).unwrap();
    let documents: Vec<Value> = serde_json::from_slice(&std::fs::read(source).unwrap()).unwrap();
    let documents = documents
        .into_iter()
        .map(|mut doc| {
            doc["eligible"] = json!(true);
            doc
        })
        .collect::<Vec<_>>();
    let endpoint = "http://127.0.0.1:11234/v1".to_owned();
    let config = Settings {
        enabled: true,
        plain_json: true,
        llm: Profile {
            endpoint: endpoint.clone(),
            model: "ddalcu/Qwen3.8-27B-MLX-Serve-4bit".into(),
            ..Default::default()
        },
        embedding: Profile {
            endpoint,
            model: "Qwen3-Embedding-0.6B-8bit".into(),
            dimensions: 1024,
            ..Default::default()
        },
        llm_extra_body: json!({"chat_template_kwargs":{"enable_thinking":false}})
            .as_object()
            .unwrap()
            .clone(),
    };
    let process = process::Process::start(&output, config.clone())
        .await
        .unwrap();
    process
        .request("/sync", Some(json!({"documents":documents})))
        .await
        .unwrap();
    // A second independent host must attach to the same sidecar and Ladybug writer.
    let attached = process::Process::start(&output, config).await.unwrap();
    for attempt in 0..180 {
        let status = attached.request("/health", None).await.unwrap();
        eprintln!("Probe status {attempt}: {status}");
        std::fs::write(
            output.join("status.json"),
            serde_json::to_vec_pretty(&status).unwrap(),
        )
        .unwrap();
        if status["ready"].as_u64() == Some(documents.len() as u64) {
            break;
        }
        assert!(attempt < 179, "Cognee processing did not finish");
        tokio::time::sleep(Duration::from_secs(10)).await;
    }
    let query = attached
        .request(
            "/query",
            Some(json!({"query":"抽屉默认占半个窗口，拖宽之后双击恢复默认，预览文件别被挡住"})),
        )
        .await
        .unwrap();
    std::fs::write(
        output.join("query.json"),
        serde_json::to_vec_pretty(&query).unwrap(),
    )
    .unwrap();
    assert!(
        query["recall"]["routes"]
            .as_array()
            .unwrap()
            .contains(&json!("semantic")),
        "{query}"
    );
    assert!(
        query["recall"]["routes"]
            .as_array()
            .unwrap()
            .contains(&json!("graph")),
        "{query}"
    );
    assert!(!query["hits"].as_array().unwrap().is_empty());
    for route in ["semantic", "graph", "fulltext"] {
        assert!(
            query["hits"]
                .as_array()
                .unwrap()
                .iter()
                .any(|hit| hit["evidence"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|e| e["route"] == route)),
            "{route} supplied no source-backed hits: {query}"
        );
    }
    let graph = attached.request("/graph", Some(json!({}))).await.unwrap();
    std::fs::write(
        output.join("graph.json"),
        serde_json::to_vec_pretty(&graph).unwrap(),
    )
    .unwrap();
    assert!(!graph["nodes"].as_array().unwrap().is_empty());
    let before = codepet_model_gateway::usage::summary(&output).unwrap();
    attached
        .request("/sync", Some(json!({"documents":documents})))
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_secs(2)).await;
    assert_eq!(
        before,
        codepet_model_gateway::usage::summary(&output).unwrap(),
        "unchanged sync invoked a model"
    );
    std::fs::write(
        output.join("usage.json"),
        serde_json::to_vec_pretty(&before).unwrap(),
    )
    .unwrap();
    let mut updated = documents.clone();
    let target = updated
        .iter_mut()
        .min_by_key(|d| d["text"].as_str().unwrap().len())
        .unwrap();
    let run_id = target["runId"].clone();
    let replacement = "CodePet 独立 sidecar 更新验证：任务召回 uses SidecarUpdatedLibrary。此句是隔离测试探针，不是产品事实。";
    use sha2::{Digest, Sha256};
    target["text"] = json!(replacement);
    target["contentHash"] = json!(format!("{:x}", Sha256::digest(replacement.as_bytes())));
    let hash = target["contentHash"].clone();
    attached
        .request("/sync", Some(json!({"documents":updated})))
        .await
        .unwrap();
    for attempt in 0..90 {
        let status = attached.request("/health", None).await.unwrap();
        if status["ready"].as_u64() == Some(updated.len() as u64) {
            break;
        }
        assert!(attempt < 89, "Updated document not indexed: {status}");
        tokio::time::sleep(Duration::from_secs(5)).await;
    }
    let replacement_graph = attached.request("/graph", Some(json!({}))).await.unwrap();
    std::fs::write(
        output.join("updated-graph.json"),
        serde_json::to_vec_pretty(&replacement_graph).unwrap(),
    )
    .unwrap();
    let metadata = |node: &Value| -> Value {
        if let Some(text) = node["external_metadata"].as_str() {
            serde_json::from_str(text).unwrap_or(Value::Null)
        } else {
            node["external_metadata"].clone()
        }
    };
    let old_source = graph["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|n| n["type"] == "TextDocument" && metadata(n)["runId"] == run_id)
        .expect("original document provenance");
    let new_source = replacement_graph["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|n| n["type"] == "TextDocument" && metadata(n)["runId"] == run_id)
        .expect("updated document provenance");
    assert_eq!(
        old_source["id"], new_source["id"],
        "Cognee update must retain data_id"
    );
    let chunks = replacement_graph["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|n| n["type"] == "DocumentChunk" && metadata(n)["runId"] == run_id)
        .collect::<Vec<_>>();
    assert!(!chunks.is_empty());
    assert!(
        chunks.iter().all(|n| metadata(n)["contentHash"] == hash),
        "old source chunks survived update"
    );
    updated.retain(|d| d["runId"] != run_id);
    attached
        .request("/sync", Some(json!({"documents":updated})))
        .await
        .unwrap();
    let mut deleted = false;
    for _ in 0..30 {
        tokio::time::sleep(Duration::from_secs(2)).await;
        let current = attached.request("/graph", Some(json!({}))).await;
        if let Ok(current) = current {
            if !current["nodes"]
                .as_array()
                .unwrap()
                .iter()
                .any(|n| metadata(n)["runId"] == run_id)
            {
                deleted = true;
                break;
            }
        }
    }
    assert!(deleted, "deleted Run still exists in native graph");
    std::fs::write(output.join("update-report.json"),serde_json::to_vec_pretty(&json!({"runId":run_id,"dataIdPreserved":true,"oldChunksRemoved":true,"deletionRemovedGraph":true})).unwrap()).unwrap();
    drop(process);
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert!(
        attached.request("/health", None).await.is_err(),
        "owner drop must stop the child"
    );
}
