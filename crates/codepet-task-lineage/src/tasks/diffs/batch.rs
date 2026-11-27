use super::*;

type Patches<'a> = std::collections::HashMap<&'a [u8], &'a [u8]>;

/// Git emits NUL-delimited raw records followed by a NUL and the corresponding
/// patches in the same order. Raw records let us handle quoted paths and renames
/// without guessing file names from the human-readable patch headers.
pub(super) fn patches(output: &[u8]) -> Result<Patches<'_>> {
    if output.is_empty() {
        return Ok(Patches::new());
    }
    let boundary = output
        .windows(2)
        .position(|pair| pair == b"\0\0")
        .ok_or("Git 没有返回可识别的批量差异")?;
    let mut raw = output[..boundary].split(|byte| *byte == 0);
    let mut names = vec![];
    while let Some(metadata) = raw.next() {
        if !metadata.starts_with(b":") {
            return Err("Git 差异元数据无效".into());
        }
        let status = metadata
            .rsplit(|byte| *byte == b' ')
            .next()
            .unwrap_or_default();
        let source = raw.next().ok_or("Git 差异缺少源路径")?;
        let path = if matches!(status.first(), Some(b'R' | b'C')) {
            raw.next().ok_or("Git 差异缺少目标路径")?
        } else {
            source
        };
        names.push(path);
    }
    let mut patch = &output[boundary + 2..];
    let mut patches = Patches::new();
    for name in names {
        if !patch.starts_with(b"diff --git ") {
            return Err("Git 差异文件与补丁数量不一致".into());
        }
        let end = patch
            .windows(b"\ndiff --git ".len())
            .position(|candidate| candidate == b"\ndiff --git ")
            .map_or(patch.len(), |next| next + 1);
        if patches.insert(name, &patch[..end]).is_some() {
            return Err("Git 对同一路径返回了多份差异".into());
        }
        patch = &patch[end..];
    }
    if !patch.is_empty() {
        return Err("Git 返回了无法匹配路径的差异".into());
    }
    Ok(patches)
}

pub(super) fn read(root: &Path, files: &[ChangedFile], commit: Option<&str>) -> Result<Vec<u8>> {
    let paths: Vec<_> = files
        .iter()
        .filter(|file| {
            commit.is_some() || !matches!(file.change_type.as_str(), "untracked" | "conflicted")
        })
        .flat_map(|file| std::iter::once(&file.path).chain(file.original_path.iter()))
        .map(|path| format!(":(literal){path}"))
        .collect();
    if paths.is_empty() {
        return Ok(vec![]);
    }
    let mut args = match commit {
        Some(sha) => vec![
            "show",
            "--format=",
            "--diff-merges=first-parent",
            "--root",
            sha,
        ],
        None => vec!["diff", "HEAD"],
    };
    args.extend([
        "--raw",
        "-z",
        "--patch",
        "--no-ext-diff",
        "--no-textconv",
        "--no-color",
        "--find-renames",
        "--src-prefix=a/",
        "--dst-prefix=b/",
        "--unified=3",
        "--",
    ]);
    args.extend(paths.iter().map(String::as_str));
    workbench::git(root, &args)
}
