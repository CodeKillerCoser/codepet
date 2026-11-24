//! User-editable initialization instructions; authorship and permissions remain Rust rules.
use codepet_task_lineage::{store::Store, tasks::Ledger};
use serde_json::{json, Value};

pub const DEFAULT: &str = include_str!("../instructions.md");

fn store(ledger: &Ledger) -> Result<Store, String> {
    Store::open(&ledger.data_directory().join("tasks/mcp-settings"))
}

fn view(saved: Option<Value>) -> Value {
    let saved = saved.unwrap_or_else(|| json!({"revision":0,"text":null}));
    let text = saved["text"].as_str();
    json!({
        "instructions":text.unwrap_or(DEFAULT),
        "defaultInstructions":DEFAULT,
        "instructionsRevision":saved["revision"].as_u64().unwrap_or(0),
        "instructionsCustomized":text.is_some()
    })
}

pub fn read(ledger: &Ledger) -> Result<Value, String> {
    let saved: Option<Value> = store(ledger)?.read("instructions.json")?;
    if let Some(value) = &saved {
        if !value["revision"].is_u64() || !(value["text"].is_null() || value["text"].is_string()) {
            return Err("Agent 提示词配置格式无效".into());
        }
    }
    Ok(view(saved))
}

/// None restores the bundled default, so future default improvements still apply.
pub fn save(ledger: &Ledger, expected_revision: u64, text: Option<&str>) -> Result<Value, String> {
    if let Some(text) = text {
        if text.trim().is_empty() || text.len() > 64 * 1024 || text.chars().count() > 16000 {
            return Err("Agent 提示词不能为空，且不能超过 16000 字或 64 KiB".into());
        }
    }
    let store = store(ledger)?;
    let previous: Option<Value> = store.read("instructions.json")?;
    let current = view(previous);
    let revision = current["instructionsRevision"].as_u64().unwrap();
    if revision != expected_revision {
        return Err(format!(
            "Agent 提示词已更新：当前 revision={revision}；请重新打开后合并"
        ));
    }
    let saved = json!({"revision":revision+1,"text":text});
    store.write("instructions.json", &saved)?;
    Ok(view(Some(saved)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn edits_persist_conflicts_do_not_overwrite_and_reset_tracks_default() {
        let data = tempfile::tempdir().unwrap();
        let ledger = Ledger::new(data.path());
        assert_eq!(read(&ledger).unwrap()["instructions"], DEFAULT);
        assert!(save(&ledger, 0, Some("   ")).is_err());
        let edited = save(&ledger, 0, Some("只在关键节点登记任务。\n保留回执。")).unwrap();
        assert_eq!(edited["instructionsRevision"], 1);
        assert_eq!(read(&Ledger::new(data.path())).unwrap(), edited);
        assert!(save(&ledger, 0, Some("过期草稿")).is_err());
        assert_eq!(read(&ledger).unwrap(), edited);
        let reset = save(&ledger, 1, None).unwrap();
        assert_eq!(reset["instructions"], DEFAULT);
        assert_eq!(reset["instructionsCustomized"], false);
    }
}
