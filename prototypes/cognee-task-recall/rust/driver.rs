//! Experiment-only JSON-lines driver. All processing uses cognee-rs public APIs.
use cognee::add::AddPipeline;
use cognee::api::{remember, update};
use cognee::core::RayonThreadPool;
use cognee::database::{DeleteDb, IngestDb};
use cognee::delete::{DeleteMode, DeleteRequest, DeleteScope, DeleteService};
use cognee::models::DataInput;
use cognee::search::{
    LexicalRetriever, SearchBuilder, SearchParams, SearchRequest, SearchRetriever,
};
use cognee::{ComponentManager, ConfigManager, PipelineContext, Settings};
use serde_json::{Value, json};
use std::{
    error::Error,
    io::{self, BufRead, Write},
    sync::Arc,
    time::Instant,
};
use uuid::Uuid;

type Result<T> = std::result::Result<T, Box<dyn Error>>;

fn data_input(doc: &Value) -> DataInput {
    let metadata = [
        "runId",
        "taskId",
        "workspace",
        "sourceRevision",
        "contentHash",
        "updatedAt",
    ]
    .into_iter()
    .filter_map(|key| doc.get(key).map(|value| (key.to_owned(), value.clone())))
    .collect::<serde_json::Map<_, _>>();
    DataInput::DataItem {
        data: Box::new(DataInput::Text(
            doc["text"].as_str().unwrap_or_default().to_owned(),
        )),
        label: doc["taskTitle"].as_str().map(str::to_owned),
        external_metadata: Some(Value::Object(metadata).to_string()),
        system_metadata: None,
    }
}

async fn execute(cm: &Arc<ComponentManager>, cmd: &Value) -> Result<Value> {
    let owner = Uuid::parse_str(&cm.settings().default_user_id)?;
    let dataset_name = "codepet_task_runs";
    let storage = cm.storage().await?;
    let db = cm.database().await?;
    let graph = cm.graph_db().await?;
    let vector = cm.vector_db().await?;
    let operation = cmd["op"].as_str().ok_or("Missing op")?;
    if operation == "search" && cmd["type"] == "CHUNKS_LEXICAL" {
        // The public orchestrator strips lexical scores. Like the Python probe,
        // call the framework's native BM25 retriever with scores enabled so
        // zero-score top-k padding is not counted as a keyword match.
        let retriever = LexicalRetriever::bm25(graph, Some(8), true, None);
        let items = retriever
            .get_context(
                cmd["query"].as_str().ok_or("Missing query")?,
                &SearchParams {
                    top_k: Some(8),
                    ..Default::default()
                },
            )
            .await?;
        return Ok(
            json!({"search_type":"CHUNKS_LEXICAL", "result":{"kind":"Items","data":items},
            "context":{},"evidence":[],"nativeRetrieverWithScores":true}),
        );
    }

    if operation == "snapshot" {
        let (nodes, edges) = graph.get_graph_data().await?;
        let raw = json!({"nodes": nodes, "edges": edges});
        let payload =
            cognee::visualization::build_visualization_payload(nodes, edges, None, vec![])?;
        if let Some(path) = cmd["html"].as_str() {
            cognee::visualize(graph.as_ref(), Some(std::path::Path::new(path))).await?;
        }
        return Ok(json!({"raw": raw, "payload": payload}));
    }
    if operation == "vectors" {
        let ids = cmd["ids"]
            .as_array()
            .ok_or("Missing ids")?
            .iter()
            .map(|x| Uuid::parse_str(x.as_str().unwrap_or_default()))
            .collect::<std::result::Result<Vec<_>, _>>()?;
        let rows = vector.retrieve("DocumentChunk", "text", &ids).await?;
        return Ok(json!(
            rows.into_iter()
                .map(|row| json!({
                    "id":row.id,"score":row.score,"metadata":row.metadata,
                }))
                .collect::<Vec<_>>()
        ));
    }
    if operation == "collections" {
        let mut rows = vec![];
        for (kind, field) in vector.list_collections().await? {
            rows.push(json!({"type":kind, "field":field, "size":vector.collection_size(&kind, &field).await?}));
        }
        return Ok(json!(rows));
    }

    let dataset =
        IngestDb::get_dataset_by_name_in_tenant(db.as_ref(), dataset_name, owner, None).await?;
    if operation == "rows" {
        return Ok(match dataset {
            Some(ds) => {
                json!({"datasetId":ds.id,"rows":DeleteDb::get_dataset_data(db.as_ref(), ds.id).await?})
            }
            None => json!({"rows":[]}),
        });
    }

    let llm = cm.llm().await?;
    let embedding = cm.embedding_engine().await?;
    if operation == "search" {
        let tuning = cm.settings().search_tuning();
        let orchestrator = SearchBuilder::new(vector, embedding, graph, llm, db.clone())
            .with_tuning(tuning)
            .with_dataset_resolver(db as Arc<dyn IngestDb>)
            .with_search_history_logging(false)
            .build();
        let request: SearchRequest = serde_json::from_value(json!({
            "query_text":cmd["query"], "search_type":cmd["type"], "top_k":8,
            "datasets":[dataset_name], "user_id":owner, "only_context":true,
            "verbose":true, "include_references":true,
        }))?;
        return Ok(serde_json::to_value(orchestrator.search(&request).await?)?);
    }

    let mut cfg = cm.settings().cognify_config();
    // Same 4096-token budget, custom prompt and single extraction concurrency as the Python probe.
    cfg = cfg
        .with_chunk_size(4096)
        .with_chunks_per_batch(1)
        .with_max_parallel_extractions(1);
    let ontology = cm.ontology_resolver(None).await?;
    let pool = Arc::new(RayonThreadPool::with_default_threads()?);
    let add = Arc::new(
        AddPipeline::new(storage.clone(), db.clone() as Arc<dyn IngestDb>)
            .with_thread_pool(pool)
            .with_graph_db(graph.clone())
            .with_vector_db(vector.clone())
            .with_database(db.clone())
            .with_loader_registry(cm.add_loader_registry().await),
    );
    if operation == "ingest" {
        let docs = cmd["documents"].as_array().ok_or("Missing documents")?;
        let personalization = cm.settings().preference_config();
        let result = remember(
            docs.iter().map(data_input).collect(),
            dataset_name,
            None,
            false,
            owner,
            None,
            add,
            llm,
            storage,
            graph,
            vector,
            embedding,
            Some(db),
            None,
            None,
            ontology,
            Arc::new(cfg),
            None,
            personalization,
            false,
        )
        .await?;
        let mut output = serde_json::to_value(&result)?;
        if let Some(c) = &result.cognify_result {
            output["processing"] = json!({"chunks":c.chunks.len(), "entities":c.entities.len(),
                "edges":c.edges.len(), "summaries":c.summaries.len(),
                "embeddings":c.embedding_count, "failures":c.failures,
                "alreadyCompleted":c.already_completed});
        }
        return Ok(output);
    }

    let dataset = dataset.ok_or("Dataset missing")?;
    let service = DeleteService::new(storage.clone(), db.clone() as Arc<dyn DeleteDb>)
        .with_graph_db(graph.clone())
        .with_vector_db(vector.clone())
        .with_pipeline_run_repo(db.clone().pipeline_runs());
    let id = Uuid::parse_str(cmd["dataId"].as_str().ok_or("Missing dataId")?)?;
    if operation == "delete" {
        let request = DeleteRequest {
            scope: DeleteScope::Data {
                owner_id: owner,
                data_id: id,
                dataset_name: Some(dataset_name.to_owned()),
                delete_dataset_if_empty: false,
                tenant_id: None,
            },
            mode: DeleteMode::Soft,
            memory_only: false,
        };
        let result = service.execute(&request).await?;
        return Ok(json!({"result":format!("{result:?}")}));
    }
    if operation == "update" {
        let result = update(
            id,
            vec![data_input(&cmd["document"])],
            dataset.id,
            dataset_name,
            owner,
            None,
            None,
            None,
            true,
            None,
            &service,
            add.as_ref(),
            llm,
            storage,
            graph,
            vector,
            embedding,
            Some(db),
            ontology,
            &cfg,
        )
        .await?;
        return Ok(json!({"dataId":result.data_id,
            "delete":format!("{:?}",result.delete_result),
            "processing":result.cognify_result.as_ref().map(|c| json!({
                "chunks":c.chunks.len(),"entities":c.entities.len(),"edges":c.edges.len(),
                "failures":c.failures,"alreadyCompleted":c.already_completed}))}));
    }
    Err(format!("Unsupported op {operation}").into())
}

#[tokio::main]
async fn main() -> Result<()> {
    cognee::telemetry::set_enabled(false);
    let cm = Arc::new(ComponentManager::new(ConfigManager::new(
        Settings::load_from_env(),
    )));
    eprintln!("CodePet cognee-rs driver ready; external models only");
    for line in io::stdin().lock().lines() {
        let line = line?;
        let started = Instant::now();
        let result = match serde_json::from_str::<Value>(&line) {
            Ok(cmd) => execute(&cm, &cmd).await,
            Err(error) => Err(error.into()),
        };
        let output = match result {
            Ok(value) => {
                json!({"ok":true,"result":value,"seconds":started.elapsed().as_secs_f64()})
            }
            Err(error) => {
                json!({"ok":false,"error":error.to_string(),"seconds":started.elapsed().as_secs_f64()})
            }
        };
        println!("{output}");
        io::stdout().flush()?;
    }
    cm.release().await;
    Ok(())
}
