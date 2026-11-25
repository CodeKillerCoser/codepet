use rusqlite::{params, Connection};
use serde_json::{json, Value};
use std::{
    path::Path,
    time::{SystemTime, UNIX_EPOCH},
};

fn connect(data: &Path) -> Result<Connection, String> {
    std::fs::create_dir_all(data.join("model-gateway")).map_err(|e| e.to_string())?;
    let db =
        Connection::open(data.join("model-gateway/usage.sqlite")).map_err(|e| e.to_string())?;
    db.busy_timeout(std::time::Duration::from_secs(5))
        .map_err(|e| e.to_string())?;
    db.execute_batch("PRAGMA journal_mode=WAL; CREATE TABLE IF NOT EXISTS usage(
        at INTEGER NOT NULL, purpose TEXT NOT NULL, model TEXT NOT NULL, ok INTEGER NOT NULL,
        input INTEGER NOT NULL, output INTEGER NOT NULL, reported INTEGER NOT NULL, milliseconds INTEGER NOT NULL);
        CREATE TABLE IF NOT EXISTS processing_usage(usage_row INTEGER PRIMARY KEY, job_id TEXT NOT NULL);
        CREATE INDEX IF NOT EXISTS processing_usage_job ON processing_usage(job_id);").map_err(|e| e.to_string())?;
    Ok(db)
}
pub(crate) fn record(
    data: &Path,
    purpose: &str,
    model: &str,
    ok: bool,
    response: &Value,
    ms: u128,
    job: Option<&str>,
) -> Result<(), String> {
    let usage = &response["usage"];
    let input = usage["prompt_tokens"]
        .as_u64()
        .or(usage["input_tokens"].as_u64());
    let output = usage["completion_tokens"]
        .as_u64()
        .or(usage["output_tokens"].as_u64());
    let mut db = connect(data)?;
    let transaction = db.transaction().map_err(|e| e.to_string())?;
    transaction
        .execute(
            "INSERT INTO usage VALUES(?,?,?,?,?,?,?,?)",
            params![
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs() as i64,
                purpose,
                model,
                ok,
                input.unwrap_or(0),
                output.unwrap_or(0),
                input.is_some() && (purpose == "embedding" || output.is_some()),
                ms.min(i64::MAX as u128) as i64
            ],
        )
        .map_err(|e| e.to_string())?;
    if let Some(id) = job {
        transaction
            .execute(
                "INSERT INTO processing_usage VALUES(?,?)",
                params![transaction.last_insert_rowid(), id],
            )
            .map_err(|e| e.to_string())?;
    }
    transaction.commit().map_err(|e| e.to_string())?;
    Ok(())
}

/// Exact request attribution; never infer an attempt's spend from global counter deltas.
pub fn for_jobs(data: &Path, jobs: &[String]) -> Result<Value, String> {
    let db = connect(data)?;
    let mut result = serde_json::Map::new();
    let mut stmt = db.prepare("SELECT purpose,model,count(*),sum(ok),sum(input),sum(output),sum(reported) FROM usage JOIN processing_usage ON usage.rowid=processing_usage.usage_row WHERE job_id=? GROUP BY purpose,model ORDER BY purpose,model").map_err(|e| e.to_string())?;
    for job in jobs {
        let rows = stmt.query_map([job], |r| Ok(json!({"purpose":r.get::<_,String>(0)?,"model":r.get::<_,String>(1)?,
            "requests":r.get::<_,i64>(2)?,"succeeded":r.get::<_,i64>(3)?,"inputTokens":r.get::<_,i64>(4)?,
            "outputTokens":r.get::<_,i64>(5)?,"reportedRequests":r.get::<_,i64>(6)?}))).map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>,_>>().map_err(|e| e.to_string())?;
        let sum = |key: &str| {
            rows.iter()
                .map(|r| r[key].as_i64().unwrap_or(0))
                .sum::<i64>()
        };
        result.insert(job.clone(), json!({"models":rows,"requests":sum("requests"),"reportedRequests":sum("reportedRequests"),
            "inputTokens":sum("inputTokens"),"outputTokens":sum("outputTokens"),"knownTokens":sum("inputTokens")+sum("outputTokens"),
            "complete":sum("requests")==sum("reportedRequests")}));
    }
    Ok(Value::Object(result))
}
pub fn summary(data: &Path) -> Result<Value, String> {
    let db = connect(data)?;
    let mut stmt = db.prepare("SELECT purpose,model,count(*),sum(ok),sum(input),sum(output),sum(reported),sum(milliseconds) FROM usage GROUP BY purpose,model ORDER BY purpose,model").map_err(|e| e.to_string())?;
    let rows = stmt.query_map([], |r| Ok(json!({"purpose":r.get::<_,String>(0)?,"model":r.get::<_,String>(1)?,
        "requests":r.get::<_,i64>(2)?,"succeeded":r.get::<_,i64>(3)?,"inputTokens":r.get::<_,i64>(4)?,
        "outputTokens":r.get::<_,i64>(5)?,"reportedRequests":r.get::<_,i64>(6)?,"milliseconds":r.get::<_,i64>(7)?}))).map_err(|e| e.to_string())?;
    Ok(json!({"models":rows.collect::<Result<Vec<_>,_>>().map_err(|e| e.to_string())?}))
}
