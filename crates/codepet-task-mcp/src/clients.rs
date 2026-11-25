//! Client configuration is generated from the same installed launcher and transport.
use serde_json::{json, Value};
use std::path::Path;

pub fn stdio(launcher: &Path) -> Value {
    configurations(
        json!({"command":launcher,"args":["--managed-stdio"]}),
        false,
    )
}

pub fn http(url: &str, authorization: &str) -> Value {
    configurations(
        json!({"url":url,"headers":{"Authorization":authorization}}),
        true,
    )
}

fn configurations(base: Value, http: bool) -> Value {
    let clients = [
        ("codex", "Codex", "~/.codex/config.toml", "~/.codex/AGENTS.md"),
        ("claude", "Claude Code", ".mcp.json", "~/.claude/CLAUDE.md"),
        ("cursor", "Cursor", "~/.cursor/mcp.json", "Cursor → Customize → Rules → User Rules"),
        ("opencode", "OpenCode", "~/.config/opencode/opencode.jsonc", "~/.config/opencode/AGENTS.md"),
    ].into_iter().map(|(id, name, config_path, rule_path)| {
        let mut server = base.clone();
        if http { server["headers"]["X-CodePet-Agent"] = json!(id); }
        else { server["env"] = json!({"CODEPET_AGENT":id}); }
        let config = if id == "codex" {
            if http {
                format!("[mcp_servers.codepet_tasks]\nurl = {}\nhttp_headers = {{ Authorization = {}, \"X-CodePet-Agent\" = \"codex\" }}\n",server["url"],server["headers"]["Authorization"])
            } else {
                format!("[mcp_servers.codepet_tasks]\ncommand = {}\nargs = [\"--managed-stdio\"]\nenv = {{ CODEPET_AGENT = \"codex\" }}\n",server["command"])
            }
        } else if id == "opencode" {
            let entry = if http { json!({"type":"remote","url":server["url"],"headers":server["headers"],"oauth":false}) }
                else { json!({"type":"local","command":[server["command"],"--managed-stdio"],"environment":server["env"]}) };
            serde_json::to_string_pretty(&json!({"mcp":{"servers":{"codepet_tasks":entry}}})).unwrap()
        } else {
            if id == "claude" { server["type"] = json!(if http {"http"} else {"stdio"}); }
            serde_json::to_string_pretty(&json!({"mcpServers":{"codepet_tasks":server}})).unwrap()
        };
        json!({"id":id,"name":name,"config":config,"configPath":config_path,"rulePath":rule_path})
    }).collect::<Vec<_>>();
    json!(clients)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn clients_keep_distinct_sources_and_exact_paths() {
        let clients = stdio(Path::new("/Users/test/中文 App's/codepet-task-mcp"));
        assert_eq!(clients.as_array().unwrap().len(), 4);
        for client in clients.as_array().unwrap().iter().skip(1) {
            let parsed: Value = serde_json::from_str(client["config"].as_str().unwrap()).unwrap();
            let (command, source) = if client["id"] == "opencode" {
                (
                    &parsed["mcp"]["servers"]["codepet_tasks"]["command"][0],
                    &parsed["mcp"]["servers"]["codepet_tasks"]["environment"]["CODEPET_AGENT"],
                )
            } else {
                (
                    &parsed["mcpServers"]["codepet_tasks"]["command"],
                    &parsed["mcpServers"]["codepet_tasks"]["env"]["CODEPET_AGENT"],
                )
            };
            assert_eq!(command, "/Users/test/中文 App's/codepet-task-mcp");
            assert_eq!(source, &client["id"]);
        }
        assert!(clients[0]["config"]
            .as_str()
            .unwrap()
            .contains("[mcp_servers.codepet_tasks]"));
    }
    #[test]
    fn http_keeps_auth_and_request_source_for_each_client() {
        for client in http("http://127.0.0.1:47623/mcp", "Bearer test-only")
            .as_array()
            .unwrap()
        {
            if client["id"] == "codex" {
                continue;
            }
            let parsed: Value = serde_json::from_str(client["config"].as_str().unwrap()).unwrap();
            let server = if client["id"] == "opencode" {
                &parsed["mcp"]["servers"]["codepet_tasks"]
            } else {
                &parsed["mcpServers"]["codepet_tasks"]
            };
            assert_eq!(server["headers"]["Authorization"], "Bearer test-only");
            assert_eq!(server["headers"]["X-CodePet-Agent"], client["id"]);
            assert!(server.get("env").is_none());
        }
    }
}
