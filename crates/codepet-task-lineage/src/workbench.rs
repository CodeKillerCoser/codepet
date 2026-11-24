//! Read-only local history inventory and Git inspection, independent of Providers.
use crate::{stable_id, Result};
use rusqlite::{Connection, OpenFlags};
use serde_json::{json, Value};
use std::{
    collections::{HashMap, HashSet},
    fs,
    io::{BufRead, BufReader, Read},
    path::{Component, Path, PathBuf},
    process::{Command, Stdio},
};
const LIMIT: u64 = 256 * 1024;
fn normalized(path: &Path) -> PathBuf {
    path.canonicalize().unwrap_or_else(|_| path.to_path_buf())
}
fn db(home: &Path) -> Result<Connection> {
    Connection::open_with_flags(
        home.join("state_5.sqlite"),
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(|e| e.to_string())
}
pub fn projects(home: &Path) -> Result<Value> {
    let mut items = Vec::<Value>::new();
    let mut warnings = Vec::<String>::new();
    let mut project_catalog = "database";
    let read = || -> Result<Vec<Value>> {
        let conn = db(home)?;
        let mut stmt = conn.prepare("SELECT p.id,p.name,r.path FROM projects p JOIN project_roots r ON r.project_id=p.id ORDER BY p.position,r.position").map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map([], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                ))
            })
            .map_err(|e| e.to_string())?;
        let mut out = Vec::<Value>::new();
        for row in rows {
            let (id, name, path) = row.map_err(|e| e.to_string())?;
            if let Some(p) = out.iter_mut().find(|p| p["id"] == id) {
                p["roots"].as_array_mut().unwrap().push(json!(path));
            } else {
                out.push(json!({"id":id,"name":name,"roots":[path]}));
            }
        }
        Ok(out)
    };
    match read() {
        Ok(p) => items = p,
        Err(e) => warnings.push(format!("项目索引不可读，使用本地目录后备：{e}")),
    }
    if items.is_empty() {
        project_catalog = "globalState";
        if let Ok(bytes) = fs::read(home.join(".codex-global-state.json")) {
            if let Ok(v) = serde_json::from_slice::<Value>(&bytes) {
                if let Some(roots) = v["electron-saved-workspace-roots"].as_array() {
                    for root in roots.iter().filter_map(Value::as_str) {
                        let name = v["electron-workspace-root-labels"][root]
                            .as_str()
                            .unwrap_or_else(|| {
                                Path::new(root)
                                    .file_name()
                                    .and_then(|s| s.to_str())
                                    .unwrap_or(root)
                            });
                        items.push(json!({"id":stable_id(root),"name":name,"roots":[root]}));
                    }
                }
            }
        }
    }
    Ok(
        json!({"projects":items,"warnings":warnings,"sourceInfo":{"codexHome":home,"projectCatalog":project_catalog}}),
    )
}
pub(crate) fn git(root: &Path, args: &[&str]) -> Result<Vec<u8>> {
    let mut child = Command::new("git")
        .arg("-c")
        .arg("core.fsmonitor=false")
        .arg("-C")
        .arg(root)
        .args(args)
        .env("GIT_OPTIONAL_LOCKS", "0")
        .env("GIT_NO_LAZY_FETCH", "1")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| e.to_string())?;
    let stdout = child.stdout.take().unwrap();
    let stderr = child.stderr.take().unwrap();
    let output = std::thread::spawn(move || {
        let mut bytes = Vec::new();
        stdout
            .take(16 * 1024 * 1024 + 1)
            .read_to_end(&mut bytes)
            .map(|_| bytes)
    });
    let errors = std::thread::spawn(move || {
        let mut bytes = Vec::new();
        stderr
            .take(64 * 1024)
            .read_to_end(&mut bytes)
            .map(|_| bytes)
    });
    let started = std::time::Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if started.elapsed() < std::time::Duration::from_secs(30) => {
                std::thread::sleep(std::time::Duration::from_millis(10))
            }
            other => {
                let _ = child.kill();
                let _ = child.wait();
                let _ = output.join();
                let _ = errors.join();
                return Err(match other {
                    Err(e) => e.to_string(),
                    _ => "Git 检测超时（30 秒）".into(),
                });
            }
        }
    };
    let bytes = output
        .join()
        .map_err(|_| "Git 输出读取失败")?
        .map_err(|e| e.to_string())?;
    let errors = errors
        .join()
        .map_err(|_| "Git 错误读取失败")?
        .map_err(|e| e.to_string())?;
    if !status.success() {
        return Err(String::from_utf8_lossy(&errors).trim().to_string());
    }
    if bytes.len() > 16 * 1024 * 1024 {
        return Err("Git 输出超过扫描上限".into());
    }
    Ok(bytes)
}
pub(crate) fn text_git(root: &Path, args: &[&str]) -> Result<String> {
    Ok(String::from_utf8_lossy(&git(root, args)?).trim().into())
}
pub fn parse_status(bytes: &[u8]) -> Value {
    let mut branch = String::new();
    let mut head = String::new();
    let mut upstream = false;
    let mut comparable = false;
    let mut ahead = 0;
    let mut behind = 0;
    let mut changes = Vec::new();
    let mut records = bytes.split(|b| *b == 0);
    while let Some(raw) = records.next() {
        let row = String::from_utf8_lossy(raw);
        if let Some(s) = row.strip_prefix("# branch.head ") {
            branch = s.into();
        } else if let Some(s) = row.strip_prefix("# branch.oid ") {
            head = s.into();
        } else if row.starts_with("# branch.upstream ") {
            upstream = true;
        } else if let Some(s) = row.strip_prefix("# branch.ab ") {
            comparable = true;
            let mut p = s.split_whitespace();
            ahead = p
                .next()
                .unwrap_or("+0")
                .trim_start_matches('+')
                .parse::<u64>()
                .unwrap_or(0);
            behind = p
                .next()
                .unwrap_or("-0")
                .trim_start_matches('-')
                .parse::<u64>()
                .unwrap_or(0);
        } else if row.starts_with("? ") {
            let path = &row[2..];
            changes.push(json!({"id":stable_id(path),"path":path,"original":null,"status":"??","staged":false}));
        } else if matches!(row.as_bytes().first(), Some(b'1' | b'2' | b'u')) {
            let fields: Vec<_> = row
                .splitn(
                    if row.starts_with('1') {
                        9
                    } else if row.starts_with('2') {
                        10
                    } else {
                        11
                    },
                    ' ',
                )
                .collect();
            if fields.len() < 9 {
                continue;
            }
            let path = fields.last().unwrap();
            let original = if row.starts_with('2') {
                records
                    .next()
                    .map(|r| String::from_utf8_lossy(r).into_owned())
            } else {
                None
            };
            changes.push(json!({"id":stable_id(path),"path":path,"original":original,"status":fields[1],"staged":fields[1].as_bytes()[0]!=b'.',"conflict":row.starts_with('u')}));
        }
    }
    json!({"branch":branch,"head":head,"ahead":ahead,"behind":behind,"upstream":upstream && comparable,"changes":changes,"error":null})
}
pub fn inspect(root: &Path) -> Value {
    match git(
        root,
        &[
            "status",
            "--porcelain=v2",
            "--branch",
            "--ahead-behind",
            "-z",
            "--untracked-files=normal",
        ],
    ) {
        Ok(bytes) => parse_status(&bytes),
        Err(error) => json!({"error":error,"changes":[],"ahead":0,"behind":0,"upstream":false}),
    }
}
fn walk(root: &Path, out: &mut Vec<PathBuf>) {
    if let Ok(entries) = fs::read_dir(root) {
        for e in entries.flatten() {
            if e.file_type().is_ok_and(|t| t.is_dir()) {
                walk(&e.path(), out)
            } else {
                out.push(e.path())
            }
        }
    }
}
fn millis(v: &Value) -> i64 {
    let n = v.as_i64().unwrap_or(0);
    if n < 100_000_000_000 {
        n * 1000
    } else {
        n
    }
}
fn time(v: &Value) -> i64 {
    v.as_str()
        .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
        .map(|d| d.timestamp_millis())
        .unwrap_or_else(|| millis(v))
}
fn header(path: &Path) -> Result<Value> {
    let f = fs::File::open(path).map_err(|e| e.to_string())?;
    let reader: Box<dyn Read> = if path.extension().is_some_and(|e| e == "zst") {
        Box::new(zstd::stream::read::Decoder::new(f).map_err(|e| e.to_string())?)
    } else {
        Box::new(f)
    };
    for line in BufReader::new(reader.take(LIMIT))
        .lines()
        .take(32)
        .flatten()
    {
        if let Ok(v) = serde_json::from_str::<Value>(&line) {
            if v["type"] == "session_meta" {
                return Ok(v["payload"].clone());
            }
        }
    }
    Err("会话头不可读".into())
}
fn subagent(v: &Value) -> bool {
    v["source"]
        .as_str()
        .and_then(|s| serde_json::from_str::<Value>(s).ok())
        .is_some_and(|s| s.get("subagent").is_some())
        || v["source"].is_object()
        || v["source"].as_str().is_some_and(|s| s.contains("subagent"))
        || v["thread_source"] == "agent_created_thread"
        || v["thread_source"] == "subagent"
}
pub fn conversations(home: &Path, warnings: &mut Vec<String>) -> Vec<Value> {
    let mut titles = HashMap::new();
    match fs::File::open(home.join("session_index.jsonl")) {
        Ok(f) => {
            for line in BufReader::new(f).lines().map_while(std::result::Result::ok) {
                if let Ok(v) = serde_json::from_str::<Value>(&line) {
                    if let (Some(id), Some(title)) = (v["id"].as_str(), v["thread_name"].as_str()) {
                        titles.insert(id.to_owned(), (title.to_owned(), time(&v["updated_at"])));
                    }
                }
            }
        }
        Err(e) => warnings.push(format!("会话标题索引不可读：{e}")),
    }
    let mut out = HashMap::<String, Value>::new();
    let query = || -> Result<Vec<Value>> {
        let conn = db(home)?;
        let mut stmt = conn
            .prepare("SELECT * FROM threads")
            .map_err(|e| e.to_string())?;
        let names: Vec<String> = stmt.column_names().iter().map(|s| s.to_string()).collect();
        let rows = stmt
            .query_map([], |r| {
                let mut m = serde_json::Map::new();
                for (i, n) in names.iter().enumerate() {
                    use rusqlite::types::ValueRef;
                    m.insert(
                        n.clone(),
                        match r.get_ref(i)? {
                            ValueRef::Integer(i) => json!(i),
                            ValueRef::Text(t) => json!(String::from_utf8_lossy(t)),
                            _ => Value::Null,
                        },
                    );
                }
                Ok(Value::Object(m))
            })
            .map_err(|e| e.to_string())?;
        rows.map(|r| r.map_err(|e| e.to_string())).collect()
    };
    let database_available = match query() {
        Ok(rows) => {
            for v in rows {
                if subagent(&v) {
                    continue;
                }
                let id = v["id"].as_str().unwrap_or_default();
                let title = titles
                    .get(id)
                    .map(|t| t.0.as_str())
                    .unwrap_or_else(|| v["title"].as_str().unwrap_or(id));
                let updated = if v["updated_at_ms"].is_number() {
                    millis(&v["updated_at_ms"])
                } else {
                    millis(&v["updated_at"])
                };
                out.insert(id.into(),json!({"id":id,"source":"Codex","title":title,"cwd":v["cwd"],"updatedAt":updated,"archived":v["archived"].as_i64().unwrap_or(0)!=0}));
            }
            true
        }
        Err(e) => {
            warnings.push(format!("Codex 数据库不可读，使用会话头后备：{e}"));
            false
        }
    };
    // Only inspect missing indexed IDs, unless the database itself is unavailable.
    let missing: HashSet<_> = titles
        .keys()
        .filter(|id| !out.contains_key(*id))
        .cloned()
        .collect();
    if !missing.is_empty() || !database_available {
        let mut files = Vec::new();
        walk(&home.join("sessions"), &mut files);
        walk(&home.join("archived_sessions"), &mut files);
        for p in files {
            let name = p.file_name().unwrap_or_default().to_string_lossy();
            if database_available && !missing.iter().any(|id| name.contains(id)) {
                continue;
            }
            if let Ok(v) = header(&p) {
                if subagent(&v) {
                    continue;
                }
                if let (Some(id), Some(cwd)) = (v["id"].as_str(), v["cwd"].as_str()) {
                    if out.contains_key(id) {
                        continue;
                    }
                    let (title, updated) = titles
                        .get(id)
                        .cloned()
                        .unwrap_or((id.into(), time(&v["timestamp"])));
                    out.insert(id.into(),json!({"id":id,"source":"Codex","title":title,"cwd":cwd,"updatedAt":updated,"archived":p.starts_with(home.join("archived_sessions"))}));
                }
            }
        }
    }
    let mut result: Vec<Value> = out.into_values().collect();
    let mut seen = HashSet::new();
    result.retain(|v| seen.insert(format!("{}:{}", v["source"], v["id"])));
    result
}
pub fn scan(
    home: &Path,
    selected: &[String],
    mut progress: impl FnMut(usize, usize) -> bool + Send,
) -> Result<Value> {
    let catalog = projects(home)?;
    let projects: Vec<Value> = catalog["projects"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|p| selected.contains(&p["id"].as_str().unwrap_or_default().to_owned()))
        .cloned()
        .collect();
    if projects.len() != selected.len() {
        return Err("项目列表已变化，请刷新后重新选择".into());
    }
    let mut warnings: Vec<String> = catalog["warnings"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|v| v.as_str().map(str::to_owned))
        .collect();
    let chats = conversations(home, &mut warnings);
    let mut rows = Vec::new();
    let mut seen = HashSet::new();
    for p in &projects {
        for root in p["roots"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(Value::as_str)
        {
            match git(Path::new(root), &["worktree", "list", "--porcelain", "-z"]) {
                Ok(bytes) => {
                    let mut main = true;
                    for field in bytes.split(|b| *b == 0) {
                        if let Some(path) = String::from_utf8_lossy(field).strip_prefix("worktree ")
                        {
                            let path = normalized(Path::new(path));
                            let id = stable_id(format!("{}:{}", p["id"], path.display()));
                            if seen.insert(id.clone()) {
                                rows.push(json!({"id":id,"projectId":p["id"],"projectName":p["name"],"path":path,"main":main}));
                            }
                            main = false;
                        }
                    }
                }
                Err(e) => warnings.push(format!("{}：{e}", root)),
            }
        }
    }
    let mut cwd_cache = HashMap::<String, PathBuf>::new();
    for c in &chats {
        if let Some(cwd) = c["cwd"].as_str() {
            cwd_cache.entry(cwd.into()).or_insert_with(|| {
                let p = normalized(Path::new(cwd));
                if rows
                    .iter()
                    .any(|r| Path::new(r["path"].as_str().unwrap()) == p)
                {
                    p
                } else {
                    text_git(&p, &["rev-parse", "--show-toplevel"])
                        .map(|s| normalized(Path::new(&s)))
                        .unwrap_or(p)
                }
            });
        }
    }
    let total = rows.len();
    if !progress(0, total) {
        return Err("扫描已取消".into());
    }
    for row in &mut rows {
        let root = Path::new(row["path"].as_str().unwrap());
        row["conversations"] = json!(chats
            .iter()
            .filter(|c| c["cwd"]
                .as_str()
                .and_then(|s| cwd_cache.get(s))
                .is_some_and(|p| p == root))
            .cloned()
            .collect::<Vec<_>>());
    }
    let checked = std::sync::atomic::AtomicUsize::new(0);
    let canceled = std::sync::atomic::AtomicBool::new(false);
    let callback = std::sync::Mutex::new(progress);
    std::thread::scope(|scope| {
        for chunk in rows.chunks_mut(total.div_ceil(4).max(1)) {
            let checked = &checked;
            let canceled = &canceled;
            let callback = &callback;
            scope.spawn(move || {
                for row in chunk {
                    if canceled.load(std::sync::atomic::Ordering::SeqCst) {
                        break;
                    }
                    row["git"] = inspect(Path::new(row["path"].as_str().unwrap()));
                    let mut callback = callback.lock().unwrap();
                    let n = checked.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1;
                    if !callback(n, total) {
                        canceled.store(true, std::sync::atomic::Ordering::SeqCst);
                        break;
                    }
                }
            });
        }
    });
    if canceled.load(std::sync::atomic::Ordering::SeqCst) {
        return Err("扫描已取消".into());
    }
    let source_info = catalog["sourceInfo"].clone();
    Ok(
        json!({"projects":projects,"worktrees":rows,"warnings":warnings,"sourceInfo":source_info,"scannedAt":chrono::Utc::now().timestamp_millis()}),
    )
}
fn safe_path(root: &Path, relative: &str) -> Result<PathBuf> {
    let p = Path::new(relative);
    if p.is_absolute() || p.components().any(|c| !matches!(c, Component::Normal(_))) {
        return Err("非法文件路径".into());
    }
    let root = root.canonicalize().map_err(|e| e.to_string())?;
    let target = root.join(p);
    let mut existing = target.as_path();
    while !existing.exists() {
        existing = existing.parent().ok_or("文件路径不可读")?;
    }
    if !existing
        .canonicalize()
        .map_err(|e| e.to_string())?
        .starts_with(&root)
    {
        return Err("文件路径超出工作树".into());
    }
    Ok(target)
}
pub fn change_path(root: &Path, change: &Value) -> Result<PathBuf> {
    safe_path(root, change["path"].as_str().ok_or("文件条目不存在")?)
}
fn utf8(bytes: Vec<u8>) -> Result<String> {
    if bytes.len() > LIMIT as usize {
        return Err("文件超过 256 KiB 预览上限".into());
    }
    if bytes.contains(&0) {
        return Err("二进制文件无法预览".into());
    }
    String::from_utf8(bytes).map_err(|_| "仅支持 UTF-8 文本预览".into())
}
pub fn preview(root: &Path, change: &Value, head: &str, version: &str) -> Result<Value> {
    let target = change_path(root, change)?;
    let path = change["path"].as_str().unwrap();
    let original = change["original"].as_str().unwrap_or(path);
    safe_path(root, original)?;
    let read_blob = |revision: &str, p: &str| -> Result<Vec<u8>> {
        let spec = format!("{revision}:{p}");
        if text_git(root, &["cat-file", "-e", &spec]).is_err() {
            return Ok(Vec::new());
        }
        let size = text_git(root, &["cat-file", "-s", &spec])?
            .parse::<u64>()
            .map_err(|e| e.to_string())?;
        if size > LIMIT {
            return Err("文件超过 256 KiB 预览上限".into());
        }
        git(root, &["show", &spec])
    };
    let before = if head.is_empty() || head == "(initial)" {
        String::new()
    } else {
        utf8(read_blob(head, original)?)?
    };
    let after = if version == "index" {
        utf8(read_blob("", path)?)?
    } else if version == "workspace" {
        match fs::File::open(target) {
            Ok(f) => {
                let mut bytes = Vec::new();
                f.take(LIMIT + 1)
                    .read_to_end(&mut bytes)
                    .map_err(|e| e.to_string())?;
                utf8(bytes)?
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
            Err(e) => return Err(e.to_string()),
        }
    } else {
        return Err("未知预览版本".into());
    };
    Ok(json!({"before":before,"after":after}))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn nul_status_keeps_renames_newlines_and_detached_head() {
        let v=parse_status(b"# branch.oid abcdef\0# branch.head (detached)\0# branch.upstream origin/main\0# branch.ab +2 -3\02 R. N... 100644 100644 100644 a b R100 new name\0old\nname\0? untracked\nfile\0u UU N... 100644 100644 100644 100644 a b c conflict\0");
        assert_eq!(v["ahead"], 2);
        assert_eq!(v["behind"], 3);
        assert_eq!(v["branch"], "(detached)");
        assert_eq!(v["changes"][0]["path"], "new name");
        assert_eq!(v["changes"][0]["original"], "old\nname");
        assert_eq!(v["changes"][1]["path"], "untracked\nfile");
        assert_eq!(v["changes"][2]["conflict"], true);
        assert_eq!(v["changes"][2]["path"], "conflict");
        let gone = parse_status(
            b"# branch.oid abcdef\0# branch.head main\0# branch.upstream origin/main\0",
        );
        assert_eq!(
            gone["upstream"], false,
            "A configured but missing tracking ref is not a comparable upstream"
        );
    }
    pub(super) fn init(root: &Path) {
        fs::create_dir_all(root).unwrap();
        git(root, &["init"]).unwrap();
        git(root, &["config", "user.email", "qa@example.invalid"]).unwrap();
        git(root, &["config", "user.name", "QA"]).unwrap();
        fs::write(root.join("a.txt"), "base\n").unwrap();
        git(root, &["add", "."]).unwrap();
        git(root, &["commit", "-m", "initial"]).unwrap();
    }
    #[test]
    fn real_git_preview_versions_rename_delete_and_path_bounds() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("repo");
        init(&root);
        fs::write(root.join("a.txt"), "staged\n").unwrap();
        git(&root, &["add", "."]).unwrap();
        fs::write(root.join("a.txt"), "workspace\n").unwrap();
        let facts = inspect(&root);
        let c = &facts["changes"][0];
        let head = facts["head"].as_str().unwrap();
        assert_eq!(
            preview(&root, c, head, "index").unwrap()["after"],
            "staged\n"
        );
        assert_eq!(
            preview(&root, c, head, "workspace").unwrap()["after"],
            "workspace\n"
        );
        fs::remove_file(root.join("a.txt")).unwrap();
        assert_eq!(preview(&root, c, head, "workspace").unwrap()["after"], "");
        for bad in ["../secret", "/etc/passwd", "a/../../x"] {
            assert!(change_path(&root, &json!({"path":bad})).is_err());
        }
        fs::write(root.join("big"), vec![b'a'; LIMIT as usize + 1]).unwrap();
        assert!(preview(&root, &json!({"path":"big"}), head, "workspace").is_err());
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(tmp.path(), root.join("escape")).unwrap();
            assert!(change_path(&root, &json!({"path":"escape/secret"})).is_err());
        }
        git(&root, &["reset", "--hard", "HEAD"]).unwrap();
        git(&root, &["mv", "a.txt", "renamed.txt"]).unwrap();
        let renamed = inspect(&root);
        assert_eq!(
            preview(&root, &renamed["changes"][0], head, "index").unwrap()["before"],
            "base\n"
        );
    }
    #[test]
    fn wal_projects_titles_archive_subagent_and_missing_rollout() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path().join("codex");
        fs::create_dir_all(&home).unwrap();
        let root = tmp.path().join("repo");
        init(&root);
        let conn = Connection::open(home.join("state_5.sqlite")).unwrap();
        conn.execute_batch("PRAGMA journal_mode=WAL; CREATE TABLE projects(id TEXT,name TEXT,position INTEGER);CREATE TABLE project_roots(project_id TEXT,position INTEGER,path TEXT);CREATE TABLE threads(id TEXT,title TEXT,cwd TEXT,updated_at INTEGER,archived INTEGER,source TEXT,thread_source TEXT);").unwrap();
        conn.execute("INSERT INTO projects VALUES ('p','Named project',0)", [])
            .unwrap();
        conn.execute(
            "INSERT INTO project_roots VALUES ('p',0,?)",
            [root.to_str().unwrap()],
        )
        .unwrap();
        for (id, source, archived) in [
            ("main", "cli", 0),
            ("archive", "cli", 1),
            ("child", "subagent", 0),
        ] {
            conn.execute(
                "INSERT INTO threads VALUES (?,'db title',?,1700000000,?,?,NULL)",
                rusqlite::params![id, root.to_str().unwrap(), archived, source],
            )
            .unwrap();
        }
        fs::write(home.join("session_index.jsonl"),"{\"id\":\"main\",\"thread_name\":\"Index title\"}\n{\"id\":\"missing\",\"thread_name\":\"Recovered title\",\"updated_at\":\"2026-11-23T12:00:00Z\"}\n").unwrap();
        fs::create_dir(home.join("sessions")).unwrap();
        let header =
            json!({"type":"session_meta","payload":{"id":"missing","cwd":root,"source":"cli"}})
                .to_string()
                + "\n";
        fs::write(
            home.join("sessions/rollout-missing.jsonl.zst"),
            zstd::stream::encode_all(header.as_bytes(), 1).unwrap(),
        )
        .unwrap();
        let v = scan(&home, &["p".into()], |_, _| true).unwrap();
        assert_eq!(v["projects"][0]["name"], "Named project");
        let chats = v["worktrees"][0]["conversations"].as_array().unwrap();
        assert_eq!(chats.len(), 3);
        assert!(chats.iter().any(|c| c["title"] == "Index title"));
        assert!(chats.iter().any(|c| c["title"] == "Recovered title"));
        assert!(chats.iter().any(|c| c["archived"] == true));
    }
    #[test]
    fn unavailable_database_recovers_all_codex_rollouts() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path().join("codex");
        fs::create_dir_all(home.join("sessions")).unwrap();
        // No database or title index: recovering the first file must not disable the fallback.
        for id in ["one", "two", "three"] {
            fs::write(
                home.join(format!("sessions/rollout-{id}.jsonl")),
                json!({"type":"session_meta","payload":{"id":id,"cwd":"/repo","source":"cli"}})
                    .to_string()
                    + "\n",
            )
            .unwrap();
        }
        let mut warnings = Vec::new();
        let chats = conversations(&home, &mut warnings);
        assert_eq!(chats.iter().filter(|c| c["source"] == "Codex").count(), 3);

        assert!(warnings.iter().any(|w| w.contains("数据库不可读")));
    }
    #[test]
    fn missing_registered_worktree_is_reported_and_codex_subdirectory_is_related() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("repo");
        init(&root);
        let tree = tmp.path().join("tree");
        git(
            &root,
            &["worktree", "add", "-b", "test", tree.to_str().unwrap()],
        )
        .unwrap();
        fs::remove_dir_all(&tree).unwrap();
        let home = tmp.path().join("codex");
        fs::create_dir_all(&home).unwrap();
        fs::write(
            home.join(".codex-global-state.json"),
            json!({"electron-saved-workspace-roots":[root]}).to_string(),
        )
        .unwrap();
        fs::create_dir(root.join("sub")).unwrap();
        fs::create_dir_all(home.join("sessions")).unwrap();
        fs::write(home.join("sessions/subdirectory.jsonl"),json!({"type":"session_meta","payload":{"id":"codex-subdir","cwd":root.join("sub"),"source":"cli","timestamp":"2026-11-23T12:00:00Z"}}).to_string()+"\n").unwrap();
        let id = projects(&home).unwrap()["projects"][0]["id"]
            .as_str()
            .unwrap()
            .to_owned();
        let v = scan(&home, &[id], |_, _| true).unwrap();
        let rows = v["worktrees"].as_array().unwrap();
        assert_eq!(rows.len(), 2);
        assert!(rows.iter().any(|r| r["git"]["error"].is_string()));
        assert!(rows
            .iter()
            .any(|r| r["conversations"].as_array().unwrap().len() == 1));
    }
}

#[cfg(test)]
mod synchronization_tests {
    use super::*;
    #[test]
    fn ahead_behind_uses_local_tracking_refs_and_clean_without_upstream_stays_distinct() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("repo");
        super::tests::init(&root);
        assert_eq!(inspect(&root)["upstream"], false);
        let remote = tmp.path().join("remote.git");
        git(tmp.path(), &["init", "--bare", remote.to_str().unwrap()]).unwrap();
        git(
            &root,
            &["remote", "add", "origin", remote.to_str().unwrap()],
        )
        .unwrap();
        git(&root, &["push", "-u", "origin", "HEAD"]).unwrap();
        let clean = inspect(&root);
        assert_eq!(clean["upstream"], true);
        assert_eq!(clean["ahead"], 0);
        fs::write(root.join("a.txt"), "second").unwrap();
        git(&root, &["add", "."]).unwrap();
        git(&root, &["commit", "-m", "second"]).unwrap();
        assert_eq!(inspect(&root)["ahead"], 1);
        git(&root, &["push"]).unwrap();
        git(&root, &["reset", "--hard", "HEAD~1"]).unwrap();
        let behind = inspect(&root);
        assert_eq!(behind["ahead"], 0);
        assert_eq!(behind["behind"], 1);
        fs::write(root.join("a.txt"), "divergent").unwrap();
        git(&root, &["add", "."]).unwrap();
        git(&root, &["commit", "-m", "divergent"]).unwrap();
        let divergent = inspect(&root);
        assert_eq!(divergent["ahead"], 1);
        assert_eq!(divergent["behind"], 1);
        fs::write(root.join("new"), "untracked").unwrap();
        assert_eq!(inspect(&root)["changes"][0]["status"], "??");
        git(&root, &["checkout", "--detach"]).unwrap();
        let detached = inspect(&root);
        assert_eq!(detached["branch"], "(detached)");
        assert_eq!(detached["upstream"], false);
    }
}
