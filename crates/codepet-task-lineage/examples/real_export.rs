//! Explicit real-data extraction into an isolated store; exports only recent text.
use codepet_task_lineage::{
    extraction::{recent_message, ClaudeExtractor},
    service,
    sources::codex::CodexSource,
    store::Store,
};
use serde_json::json;
use std::{path::PathBuf, time::Duration};
fn main() -> Result<(), String> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    if args.len() != 6 {
        return Err("Expected Codex home, isolated store, Claude executable, export file, root IDs comma-separated or *, per-job budget".into());
    }
    let store = Store::open(&PathBuf::from(&args[1]))?;
    let roots: Vec<&str> = args[4].split(',').collect();
    let mut view = if args[4] == "*" {
        service::scan(
            &store,
            &CodexSource {
                home: PathBuf::from(&args[0]),
            },
        )?
    } else {
        service::snapshot(&store)?
    };
    let pending = |view: &service::Snapshot| -> Result<usize, String> {
        if args[4] == "*" {
            return Ok(view.pending_messages);
        }
        roots
            .iter()
            .map(|id| service::pending_for(&store, view, Some(id)))
            .sum()
    };
    let extractor = ClaudeExtractor {
        executable: PathBuf::from(&args[2]),
        config_directory: None,
        model: "haiku".into(),
        budget_usd: args[5].parse::<f64>().map_err(|e| e.to_string())?,
        timeout: Duration::from_secs(90),
    };
    let mut cost = std::fs::read(&args[3])
        .ok()
        .and_then(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).ok())
        .and_then(|v| v["reportedCostUsd"].as_f64())
        .unwrap_or(0.0);
    println!(
        "{}",
        json!({"stage":"scanned","threads":view.threads.len(),"pending":pending(&view)?})
    );
    for batch in 0..250 {
        if pending(&view)? == 0 {
            break;
        }
        let before = pending(&view)?;
        let selected = if args[4] == "*" {
            None
        } else {
            roots
                .iter()
                .find(|id| service::pending_for(&store, &view, Some(id)).unwrap_or(0) > 0)
                .copied()
        };
        let mut last_error = None;
        for attempt in 0..3 {
            match service::extract_next(&store, &extractor, selected) {
                Ok(next) => {
                    view = next;
                    last_error = None;
                    break;
                }
                Err(error) => {
                    let budget_error = error.contains("budget_exhausted");
                    println!(
                        "{}",
                        json!({"stage":"retry","batch":batch+1,"attempt":attempt+1,"error":error})
                    );
                    last_error = Some(error);
                    if budget_error {
                        break;
                    }
                }
            }
        }
        if let Some(error) = last_error {
            return Err(error);
        }
        cost += view
            .last_extraction
            .as_ref()
            .and_then(|v| v["reportedCostUsd"].as_f64())
            .unwrap_or(0.0);
        let now = chrono::Utc::now();
        let mut messages = vec![];
        for thread in &view.threads {
            messages.extend(
                service::messages(&store, &thread.id)?
                    .into_iter()
                    .filter(|m| recent_message(m, now.timestamp_millis())),
            );
        }
        let export = json!({"snapshot":view,"messages":messages,"generatedAt":now.to_rfc3339(),"lookbackHours":48,"reportedCostUsd":cost});
        std::fs::write(
            &args[3],
            serde_json::to_vec(&export).map_err(|e| e.to_string())?,
        )
        .map_err(|e| e.to_string())?;
        println!(
            "{}",
            json!({"stage":"extracted","batch":batch+1,"tasks":view.tasks.len(),"pending":pending(&view)?,"reportedCostUsd":cost})
        );
        if pending(&view)? >= before {
            return Err("Extraction made no progress".into());
        }
    }
    if pending(&view)? != 0 {
        return Err("Batch safety limit reached; rerun to resume".into());
    }
    Ok(())
}
