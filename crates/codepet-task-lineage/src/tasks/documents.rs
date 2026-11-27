//! References are interpreted only in known storage fields and only from the ledger root.
use super::Task;
use crate::{stable_id, store::Store, Result};
use serde_json::{json, Value};
use std::{
    collections::{HashMap, HashSet},
    fs::File,
    io::Read,
    path::Path,
};

const REFERENCED_SCHEMA: u64 = 7;
const TEXT_THRESHOLD: usize = 1024;
const LIST_THRESHOLD: usize = 4096;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Kind {
    Markdown,
    List,
    Diff,
}

impl Kind {
    fn extension(self) -> &'static str {
        match self {
            Self::Markdown => ".md",
            Self::List => ".json",
            Self::Diff => ".diff",
        }
    }
}

pub(super) fn key(kind: Kind, bytes: &[u8]) -> String {
    format!("documents/{}{}", stable_id(bytes), kind.extension())
}

fn digest(kind: Kind, reference: &str) -> Result<&str> {
    reference
        .strip_prefix("documents/")
        .and_then(|name| name.strip_suffix(kind.extension()))
        .filter(|hash| {
            hash.len() == 64
                && hash
                    .bytes()
                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        })
        .ok_or_else(|| format!("无效的任务文档相对引用：{reference}"))
}

pub(super) fn open(root: &Path, kind: Kind, reference: &str) -> Result<File> {
    digest(kind, reference)?;
    Store::open_document(root, reference)
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Resolve {
    Full,
    TextOnly,
}

pub(super) fn encode(task: &Task, store: &Store) -> Result<Value> {
    let mut stored = serde_json::to_value(task).map_err(|error| error.to_string())?;
    let mut docs = HashSet::new();
    visit_payloads(&mut stored, &mut |field, kind| {
        let bytes = match kind {
            Kind::Markdown => match field.as_str() {
                Some(text) if text.len() >= TEXT_THRESHOLD => text.as_bytes().to_vec(),
                _ => return Ok(()),
            },
            Kind::List => {
                if !field.is_array() {
                    return Ok(());
                }
                let bytes = serde_json::to_vec(field).map_err(|error| error.to_string())?;
                if bytes.len() < LIST_THRESHOLD {
                    return Ok(());
                }
                bytes
            }
            Kind::Diff => unreachable!("patches use ranged references"),
        };
        let reference = key(kind, &bytes);
        if docs.insert(reference.clone()) {
            store.publish_document(&reference, &bytes)?;
        }
        *field = json!({"ref": reference});
        Ok(())
    })?;
    if !docs.is_empty() {
        stored["schemaVersion"] = json!(REFERENCED_SCHEMA);
    }
    Ok(stored)
}

pub(super) fn decode(mut stored: Value, root: &Path, mode: Resolve) -> Result<Task> {
    match stored["schemaVersion"].as_u64() {
        Some(4..=6) => {}
        Some(REFERENCED_SCHEMA) => {
            let mut cache = HashMap::<String, Value>::new();
            visit_payloads(&mut stored, &mut |field, kind| {
                let Some(reference) = field.get("ref").and_then(Value::as_str) else {
                    return Ok(());
                };
                let hash = digest(kind, reference)?;
                if kind == Kind::List && mode == Resolve::TextOnly {
                    *field = json!([]);
                    return Ok(());
                }
                if let Some(content) = cache.get(reference) {
                    *field = content.clone();
                    return Ok(());
                }
                let mut bytes = vec![];
                open(root, kind, reference)?
                    .read_to_end(&mut bytes)
                    .map_err(|error| error.to_string())?;
                if stable_id(&bytes) != hash {
                    return Err(format!("任务文档摘要不匹配：{reference}"));
                }
                let content = match kind {
                    Kind::Markdown => {
                        Value::String(String::from_utf8(bytes).map_err(|error| error.to_string())?)
                    }
                    Kind::List => Value::Array(
                        serde_json::from_slice(&bytes).map_err(|error| error.to_string())?,
                    ),
                    Kind::Diff => unreachable!("patches use ranged references"),
                };
                cache.insert(reference.to_owned(), content.clone());
                *field = content;
                Ok(())
            })?;
        }
        _ => return Err("任务格式版本或执行历史无效".into()),
    }
    let mut task: Task = serde_json::from_value(stored).map_err(|error| error.to_string())?;
    if task.runs.is_empty() {
        return Err("任务格式版本或执行历史无效".into());
    }
    task.normalize_completion();
    Ok(task)
}

type Visit<'a> = dyn FnMut(&mut Value, Kind) -> Result<()> + 'a;

fn field(parent: &mut Value, name: &str, kind: Kind, visit: &mut Visit<'_>) -> Result<()> {
    if let Some(value) = parent.get_mut(name) {
        visit(value, kind)?;
    }
    Ok(())
}

fn entries<'a>(parent: &'a mut Value, name: &str) -> impl Iterator<Item = &'a mut Value> {
    parent
        .get_mut(name)
        .and_then(Value::as_array_mut)
        .into_iter()
        .flatten()
}

fn workspace(owner: &mut Value, visit: &mut Visit<'_>) -> Result<()> {
    if let Some(snapshot) = owner.get_mut("agentWorkspace") {
        workspace_fields(snapshot, visit)?;
    }
    Ok(())
}

fn workspace_fields(snapshot: &mut Value, visit: &mut Visit<'_>) -> Result<()> {
    field(snapshot, "changedFiles", Kind::List, visit)?;
    for commit in entries(snapshot, "commitChanges") {
        field(commit, "files", Kind::List, visit)?;
    }
    Ok(())
}

fn verification(record: &mut Value, visit: &mut Visit<'_>) -> Result<()> {
    workspace(record, visit)?;
    for result in entries(record, "results") {
        field(result, "evidence", Kind::Markdown, visit)?;
    }
    Ok(())
}

fn criteria(owner: &mut Value, name: &str, visit: &mut Visit<'_>) -> Result<()> {
    for item in entries(owner, name) {
        for text in ["precondition", "steps", "expected"] {
            field(item, text, Kind::Markdown, visit)?;
        }
    }
    Ok(())
}

fn visit_payloads(task: &mut Value, visit: &mut Visit<'_>) -> Result<()> {
    field(task, "description", Kind::Markdown, visit)?;
    if let Some(completion) = task.get_mut("completion") {
        field(completion, "reason", Kind::Markdown, visit)?;
    }
    for run in entries(task, "runs") {
        for name in ["goal", "solution", "description"] {
            field(run, name, Kind::Markdown, visit)?;
        }
        workspace(run, visit)?;
        if let Some(closure) = run.get_mut("closure") {
            field(closure, "reason", Kind::Markdown, visit)?;
        }
        if let Some(plan) = run.get_mut("validationPlan") {
            criteria(plan, "items", visit)?;
            for revision in entries(plan, "revisions") {
                field(revision, "summary", Kind::Markdown, visit)?;
                criteria(revision, "criteria", visit)?;
            }
            for record in entries(plan, "records") {
                verification(record, visit)?;
            }
        }
        for event in entries(run, "timeline") {
            field(event, "summary", Kind::Markdown, visit)?;
            field(event, "reference", Kind::Markdown, visit)?;
            workspace(event, visit)?;
            for change in entries(event, "changes") {
                // Change values are arbitrary JSON. Visit only known payload types.
                match change.get("field").and_then(Value::as_str).unwrap_or("") {
                    "agentWorkspace" => {
                        for name in ["before", "after"] {
                            if let Some(snapshot) = change.get_mut(name) {
                                workspace_fields(snapshot, visit)?;
                            }
                        }
                    }
                    "verification" => {
                        for name in ["before", "after"] {
                            if let Some(record) = change.get_mut(name) {
                                verification(record, visit)?;
                            }
                        }
                    }
                    "validationPlan" => {
                        for name in ["before", "after"] {
                            if let Some(plan) = change.get_mut(name) {
                                criteria(plan, "items", visit)?;
                            }
                        }
                    }
                    "goal" | "solution" | "description" => {
                        field(change, "before", Kind::Markdown, visit)?;
                        field(change, "after", Kind::Markdown, visit)?;
                    }
                    _ => {}
                }
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests;
