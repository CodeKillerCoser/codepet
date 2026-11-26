//! Read only the published preview, never native indexes or live writer caches.
use super::process;
use codepet_model_gateway::settings;
use serde_json::{json, Value};
use std::{fs::File, io::Read, path::Path};

// Shared disk contract with codepet_recall.graph_cache.GraphCache.
const VERSION: &str = "cognee-1.6.10-preview-1";
const MAX_BYTES: u64 = 32 * 1024 * 1024;

fn read_json(path: &Path, limit: u64) -> Option<Value> {
    let file = File::open(path).ok()?;
    if file.metadata().ok()?.len() > limit {
        return None;
    }
    let mut bytes = Vec::new();
    file.take(limit + 1).read_to_end(&mut bytes).ok()?;
    (bytes.len() as u64 <= limit)
        .then(|| serde_json::from_slice(&bytes).ok())
        .flatten()
}

pub(super) fn published(data: &Path, known_version: Option<&str>) -> Option<Value> {
    let fingerprint = process::fingerprint(&settings::read(data).ok()?);
    let root = data.join("tasks/recall/indexes").join(&fingerprint);
    let dataset = read_json(&root.join("dataset.json"), 65536)?;
    let id = dataset["id"].as_str().filter(|id| !id.is_empty())?;
    let mut saved = read_json(&root.join("published/preview-graph.json"), MAX_BYTES)?;
    let graph = &saved["graph"];
    if saved["version"] != VERSION
        || saved["dataset"] != id
        || !graph["nodes"].is_array()
        || !graph["links"].is_array()
        || graph["previewVersion"].as_str().is_none_or(str::is_empty)
        || process::fingerprint(&settings::read(data).ok()?) != fingerprint
        || read_json(&root.join("dataset.json"), 65536)?["id"] != id
    {
        return None;
    }
    let mut result = if known_version.is_some_and(|v| graph["previewVersion"] == v) {
        json!({"unchanged":true,"previewVersion":graph["previewVersion"]})
    } else {
        saved["graph"].take()
    };
    // Queue state is unknown until the service responds. Don't invent counts.
    result["previewStatus"] = json!({"preparing":true});
    result["previewSource"] = json!("published");
    Some(result)
}

#[cfg(test)]
mod tests;
