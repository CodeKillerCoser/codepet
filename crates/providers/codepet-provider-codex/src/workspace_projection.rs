use std::ffi::OsStr;
use std::fs;
use std::path::{Component, Path, PathBuf};

pub fn project_workspace_root(value: Option<&str>) -> Option<String> {
    let (workspace, fallback) = normalized_workspace(value)?;
    let project_root = discover_repository_root(&workspace).unwrap_or(workspace);
    path_string(&project_root).or(Some(fallback))
}

fn normalized_workspace(value: Option<&str>) -> Option<(PathBuf, String)> {
    let value = value?.trim();
    if value.is_empty() {
        return None;
    }
    let path = Path::new(value);
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir().ok()?.join(path)
    };
    let lexical = lexical_normalize(&absolute);
    let fallback = path_string(&lexical).unwrap_or_else(|| value.to_string());
    Some((lexical, fallback))
}

fn discover_repository_root(workspace: &Path) -> Option<PathBuf> {
    if !workspace.exists() {
        return None;
    }
    let repository_root = workspace
        .ancestors()
        .find(|candidate| candidate.join(".git").exists())?;
    let git_marker = repository_root.join(".git");
    let git_dir = if git_marker.is_dir() {
        fs::canonicalize(&git_marker).ok()?
    } else {
        resolve_git_file(&git_marker)?
    };
    let common_dir = resolve_common_dir(&git_dir)?;
    if common_dir.file_name() == Some(OsStr::new(".git")) {
        return common_dir.parent().map(Path::to_path_buf);
    }
    configured_worktree(&common_dir)
        .or_else(|| fs::canonicalize(repository_root).ok())
}

fn resolve_git_file(marker: &Path) -> Option<PathBuf> {
    let contents = fs::read_to_string(marker).ok()?;
    let raw_path = contents.lines().find_map(|line| {
        line.trim()
            .strip_prefix("gitdir:")
            .map(str::trim)
            .filter(|value| !value.is_empty())
    })?;
    let path = Path::new(raw_path);
    let resolved = if path.is_absolute() {
        path.to_path_buf()
    } else {
        marker.parent()?.join(path)
    };
    fs::canonicalize(resolved).ok()
}

fn resolve_common_dir(git_dir: &Path) -> Option<PathBuf> {
    let marker = git_dir.join("commondir");
    if !marker.is_file() {
        return Some(git_dir.to_path_buf());
    }
    let raw_path = fs::read_to_string(marker).ok()?;
    let raw_path = raw_path.trim();
    if raw_path.is_empty() {
        return None;
    }
    let path = Path::new(raw_path);
    let resolved = if path.is_absolute() {
        path.to_path_buf()
    } else {
        git_dir.join(path)
    };
    fs::canonicalize(resolved).ok()
}

fn configured_worktree(common_dir: &Path) -> Option<PathBuf> {
    let contents = fs::read_to_string(common_dir.join("config")).ok()?;
    let mut in_core = false;
    for line in contents.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            in_core = line.eq_ignore_ascii_case("[core]");
            continue;
        }
        if !in_core || line.starts_with('#') || line.starts_with(';') {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        if !key.trim().eq_ignore_ascii_case("worktree") {
            continue;
        }
        let value = unquote(value.trim());
        if value.is_empty() {
            return None;
        }
        let path = Path::new(value);
        let resolved = if path.is_absolute() {
            path.to_path_buf()
        } else {
            common_dir.join(path)
        };
        return fs::canonicalize(resolved).ok();
    }
    None
}

fn unquote(value: &str) -> &str {
    value
        .strip_prefix('"')
        .and_then(|value| value.strip_suffix('"'))
        .unwrap_or(value)
}

fn lexical_normalize(path: &Path) -> PathBuf {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                normalized.pop();
            }
            Component::Prefix(_) | Component::RootDir | Component::Normal(_) => {
                normalized.push(component.as_os_str());
            }
        }
    }
    normalized
}

#[cfg(not(windows))]
fn path_string(path: &Path) -> Option<String> {
    path.to_str().map(str::to_string)
}

#[cfg(windows)]
fn path_string(path: &Path) -> Option<String> {
    let value = path.to_str()?;
    if let Some(value) = value.strip_prefix(r"\\?\UNC\") {
        return Some(format!(r"\\{value}"));
    }
    Some(value.strip_prefix(r"\\?\").unwrap_or(value).to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn main_checkout_and_linked_worktree_share_project_root() {
        let fixture = RepositoryFixture::new("project");
        let worktree = fixture.linked_worktree("linked");

        let main_root = project_workspace_root(Some(fixture.main.to_str().unwrap()));
        let worktree_root = project_workspace_root(Some(worktree.to_str().unwrap()));

        assert_eq!(main_root, worktree_root);
        assert_eq!(
            main_root,
            path_string(&fs::canonicalize(&fixture.main).unwrap())
        );
    }

    #[test]
    fn non_git_workspace_uses_normalized_absolute_root() {
        let temp = TempDir::new().unwrap();
        let workspace = temp.path().join("plain");
        fs::create_dir_all(workspace.join("nested")).unwrap();
        let input = workspace.join("nested").join("..");

        assert_eq!(
            project_workspace_root(Some(input.to_str().unwrap())),
            path_string(&workspace)
        );
    }

    #[test]
    fn same_named_repositories_keep_distinct_project_roots() {
        let first = RepositoryFixture::new("shared");
        let second = RepositoryFixture::new("shared");

        assert_ne!(
            project_workspace_root(Some(first.main.to_str().unwrap())),
            project_workspace_root(Some(second.main.to_str().unwrap()))
        );
    }

    #[test]
    fn missing_workspace_falls_back_to_its_normalized_path() {
        let temp = TempDir::new().unwrap();
        let missing = temp.path().join(".codex/worktrees/deleted/project");

        assert_eq!(
            project_workspace_root(Some(missing.to_str().unwrap())),
            path_string(&lexical_normalize(&missing))
        );
    }

    #[test]
    fn separate_git_dir_uses_configured_primary_worktree() {
        let temp = TempDir::new().unwrap();
        let main = temp.path().join("main/project");
        let common_dir = temp.path().join("metadata/project.git");
        let linked = temp.path().join("linked/project");
        let linked_git_dir = common_dir.join("worktrees/linked");
        fs::create_dir_all(&main).unwrap();
        fs::create_dir_all(&linked).unwrap();
        fs::create_dir_all(&linked_git_dir).unwrap();
        fs::write(
            main.join(".git"),
            format!("gitdir: {}\n", common_dir.display()),
        )
        .unwrap();
        fs::write(
            common_dir.join("config"),
            format!("[core]\n\tworktree = {}\n", main.display()),
        )
        .unwrap();
        fs::write(linked_git_dir.join("commondir"), "../..\n").unwrap();
        fs::write(
            linked.join(".git"),
            format!("gitdir: {}\n", linked_git_dir.display()),
        )
        .unwrap();

        assert_eq!(
            project_workspace_root(Some(main.to_str().unwrap())),
            project_workspace_root(Some(linked.to_str().unwrap()))
        );
    }

    struct RepositoryFixture {
        temp: TempDir,
        main: PathBuf,
    }

    impl RepositoryFixture {
        fn new(name: &str) -> Self {
            let temp = TempDir::new().unwrap();
            let main = temp.path().join("checkout").join(name);
            fs::create_dir_all(main.join(".git/worktrees")).unwrap();
            Self { temp, main }
        }

        fn linked_worktree(&self, id: &str) -> PathBuf {
            let git_dir = self.main.join(".git/worktrees").join(id);
            fs::create_dir_all(&git_dir).unwrap();
            fs::write(git_dir.join("commondir"), "../..\n").unwrap();
            let worktree = self
                .temp
                .path()
                .join("worktrees")
                .join(id)
                .join(self.main.file_name().unwrap());
            fs::create_dir_all(&worktree).unwrap();
            fs::write(
                worktree.join(".git"),
                format!("gitdir: {}\n", git_dir.display()),
            )
            .unwrap();
            worktree
        }
    }
}
