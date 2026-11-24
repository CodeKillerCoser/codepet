//! Stable stdio entry point, independent of the application's installation path.
//! The bootstrap stays in bin; immutable runners and the active ledger are switched atomically.
mod platform;

use fs2::FileExt;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::{self, Write},
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

pub fn binary_name() -> String {
    format!("codepet-task-mcp{}", std::env::consts::EXE_SUFFIX)
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Current {
    schema: u32,
    digest: String,
    bootstrap_digest: String,
    data_directory: PathBuf,
}

fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn read_current(root: &Path) -> io::Result<Current> {
    let current: Current =
        serde_json::from_slice(&fs::read(root.join("current.json"))?).map_err(io::Error::other)?;
    if current.schema != 1
        || !current.data_directory.is_absolute()
        || ![&current.digest, &current.bootstrap_digest]
            .iter()
            .all(|s| s.len() == 64 && s.bytes().all(|b| b.is_ascii_hexdigit()))
    {
        return Err(io::Error::other(
            "任务 MCP 安装记录无效，请重新打开 Code Pet 修复接入",
        ));
    }
    Ok(current)
}

fn write_file(destination: &Path, bytes: &[u8], executable: bool, replace: bool) -> io::Result<()> {
    let parent = destination
        .parent()
        .ok_or_else(|| io::Error::other("missing parent"))?;
    fs::create_dir_all(parent)?;
    let mut temp = tempfile::NamedTempFile::new_in(parent)?;
    temp.write_all(bytes)?;
    if executable {
        platform::make_executable(temp.as_file())?;
    }
    temp.as_file().sync_all()?;
    if replace {
        temp.persist(destination).map_err(|e| e.error)?;
    } else {
        temp.persist_noclobber(destination).map_err(|e| e.error)?;
    }
    Ok(())
}

/// Install only a standalone runner from the trusted package resources, never the GUI executable.
/// Existing bootstrap processes are left intact, including when Windows keeps them locked.
pub fn install(source: &Path, workspace: &Path, data: &Path) -> io::Result<Value> {
    if !workspace.is_absolute() || !data.is_absolute() {
        return Err(io::Error::other("任务 MCP 工作区和数据目录必须是绝对路径"));
    }
    let bytes = fs::read(source).map_err(|e| {
        io::Error::new(
            e.kind(),
            format!(
                "独立任务 MCP 程序不可用：{}；请安装完整版本的 Code Pet（{e}）",
                source.display()
            ),
        )
    })?;
    if bytes.is_empty() {
        return Err(io::Error::other("独立任务 MCP 程序为空"));
    }
    let root = workspace.join("tools/task-mcp");
    fs::create_dir_all(&root)?;
    let lock = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(root.join("install.lock"))?;
    lock.lock_exclusive()?;
    let launcher = workspace.join("bin").join(binary_name());
    let runner_digest = digest(&bytes);
    let bootstrap_digest = if launcher.exists() {
        let current = read_current(&root)?;
        if digest(&fs::read(&launcher)?) != current.bootstrap_digest {
            return Err(io::Error::other(
                "任务 MCP 固定入口已被修改，请保留现场后重新安装",
            ));
        }
        current.bootstrap_digest
    } else {
        runner_digest.clone()
    };
    let runner = root
        .join("versions")
        .join(&runner_digest)
        .join(binary_name());
    if runner.exists() {
        if digest(&fs::read(&runner)?) != runner_digest {
            return Err(io::Error::other("任务 MCP 版本文件校验失败，请重新安装"));
        }
    } else {
        write_file(&runner, &bytes, true, false)?;
    }
    // Prepare the manifest before first exposing the bootstrap. Later updates replace
    // only this small record, after a complete new runner has been installed.
    let current = Current {
        schema: 1,
        digest: runner_digest,
        bootstrap_digest,
        data_directory: data.to_owned(),
    };
    write_file(
        &root.join("current.json"),
        &serde_json::to_vec(&current)?,
        false,
        true,
    )?;
    if !launcher.exists() {
        write_file(&launcher, &bytes, true, false)?;
    }
    Ok(configuration(&launcher))
}

pub fn configuration(launcher: &Path) -> Value {
    let config = format!(
        "[mcp_servers.codepet_tasks]\ncommand = {}\nargs = [\"--managed-stdio\"]\nenv = {{ CODEPET_AGENT = \"codex\" }}\n",
        json!(launcher.to_string_lossy())
    );
    json!({"transport":"stdio", "command":launcher, "config":config})
}

/// Reads an atomic manifest at each connection. App moves and old App removal do not
/// affect this command; a new App launch installs its runner and pins its active ledger.
pub fn launch() -> io::Result<i32> {
    let executable = std::env::current_exe()?;
    let workspace = executable
        .parent()
        .and_then(Path::parent)
        .ok_or_else(|| io::Error::other("任务 MCP 固定入口路径无效"))?;
    let root = workspace.join("tools/task-mcp");
    let current = read_current(&root)?;
    let runner = root
        .join("versions")
        .join(&current.digest)
        .join(binary_name());
    if digest(&fs::read(&runner)?) != current.digest {
        return Err(io::Error::other(
            "任务 MCP 版本校验失败，请重新打开 Code Pet",
        ));
    }
    let mut command = Command::new(runner);
    command
        .arg("--data-dir")
        .arg(current.data_directory)
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit());
    platform::launch(command)
}
