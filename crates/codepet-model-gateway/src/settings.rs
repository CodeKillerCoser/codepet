use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{fs, io::Write, path::Path};

#[derive(Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase", default, deny_unknown_fields)]
pub struct Profile {
    pub endpoint: String,
    pub model: String,
    pub api_key: String,
    pub dimensions: u32,
}
#[derive(Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase", default, deny_unknown_fields)]
pub struct Settings {
    pub enabled: bool,
    pub llm: Profile,
    pub embedding: Profile,
    pub plain_json: bool,
    pub llm_extra_body: serde_json::Map<String, Value>,
}
pub fn read(data: &Path) -> Result<Settings, String> {
    let path = data.join("model-gateway/config.json");
    match fs::read(path) {
        Ok(bytes) => serde_json::from_slice(&bytes).map_err(|_| "模型配置文件无效".into()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Settings::default()),
        Err(e) => Err(e.to_string()),
    }
}
pub fn validate(settings: &Settings) -> Result<(), String> {
    if !settings.enabled {
        return Ok(());
    }
    for profile in [&settings.llm, &settings.embedding] {
        let url = reqwest::Url::parse(&profile.endpoint)
            .map_err(|_| "模型地址必须是完整的 API base URL")?;
        let local = matches!(url.host_str(), Some("localhost" | "127.0.0.1" | "[::1]"));
        if (url.scheme() != "https" && !(url.scheme() == "http" && local))
            || !url.username().is_empty()
            || url.password().is_some()
            || url.query().is_some()
            || url.fragment().is_some()
            || profile.model.trim().is_empty()
            || profile.model.len() > 256
        {
            return Err("使用 HTTPS 模型地址，或本机 HTTP 地址，并填写模型名".into());
        }
    }
    if !(1..=65536).contains(&settings.embedding.dimensions) {
        return Err("Embedding 维度必须为 1–65536".into());
    }
    for key in settings.llm_extra_body.keys() {
        if !["chat_template_kwargs", "reasoning_effort", "thinking"].contains(&key.as_str()) {
            return Err(
                "模型高级参数仅支持 thinking、reasoning_effort 和 chat_template_kwargs".into(),
            );
        }
    }
    Ok(())
}
pub fn public(settings: &Settings) -> Value {
    let mut result = json!(settings);
    for key in ["llm", "embedding"] {
        let has_key = result[key]["apiKey"]
            .as_str()
            .is_some_and(|s| !s.is_empty());
        result[key]["apiKey"] = json!("");
        result[key]["hasApiKey"] = json!(has_key);
    }
    result
}
pub fn save(data: &Path, value: Value) -> Result<Value, String> {
    let old = read(data)?;
    let mut value = value;
    for key in ["llm", "embedding"] {
        if let Some(obj) = value[key].as_object_mut() {
            obj.remove("hasApiKey");
        }
    }
    let mut next: Settings = serde_json::from_value(value).map_err(|_| "模型配置格式无效")?;
    // Empty password input preserves the saved secret. Local endpoints may omit a key.
    if next.llm.api_key.is_empty()
        && next.llm.endpoint.trim_end_matches('/') == old.llm.endpoint.trim_end_matches('/')
    {
        next.llm.api_key = old.llm.api_key;
    }
    if next.embedding.api_key.is_empty()
        && next.embedding.endpoint.trim_end_matches('/')
            == old.embedding.endpoint.trim_end_matches('/')
    {
        next.embedding.api_key = old.embedding.api_key;
    }
    validate(&next)?;
    let root = data.join("model-gateway");
    fs::create_dir_all(&root).map_err(|e| e.to_string())?;
    let mut file = tempfile::NamedTempFile::new_in(&root).map_err(|e| e.to_string())?;
    file.write_all(&serde_json::to_vec_pretty(&next).unwrap())
        .map_err(|e| e.to_string())?;
    file.as_file().sync_all().map_err(|e| e.to_string())?;
    file.persist(root.join("config.json"))
        .map_err(|e| e.to_string())?;
    Ok(public(&next))
}
