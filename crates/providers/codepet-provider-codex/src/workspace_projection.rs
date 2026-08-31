use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::ffi::OsStr;
use std::fs;
use std::path::{Component, Path, PathBuf};

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
enum RepositoryIdentity {
    Remote(String),
    CommonDir(PathBuf),
}

#[derive(Clone, Debug)]
struct RepositoryEvidence {
    identity: RepositoryIdentity,
    project_root: PathBuf,
}

#[derive(Clone, Debug)]
struct ManagedWorkspace {
    managed_root: PathBuf,
    project_name: String,
}

pub(crate) struct WorkspaceProjector {
    managed_roots: Vec<PathBuf>,
    repositories_by_name: BTreeMap<String, BTreeMap<RepositoryIdentity, PathBuf>>,
    repositories_by_managed_project:
        BTreeMap<(PathBuf, String), BTreeMap<RepositoryIdentity, PathBuf>>,
    input_repositories: HashMap<PathBuf, RepositoryEvidence>,
}

impl WorkspaceProjector {
    pub(crate) fn prepare<'a>(values: impl IntoIterator<Item = &'a str>) -> Self {
        let workspaces = values
            .into_iter()
            .filter_map(|value| normalized_workspace(Some(value)).map(|(path, _)| path))
            .collect::<Vec<_>>();
        let mut managed_roots = known_managed_roots();
        managed_roots.extend(
            workspaces
                .iter()
                .filter_map(|workspace| inferred_codex_managed_root(workspace)),
        );
        managed_roots.sort();
        managed_roots.dedup();
        managed_roots.sort_by(|left, right| {
            right
                .components()
                .count()
                .cmp(&left.components().count())
                .then_with(|| left.cmp(right))
        });

        let mut projector = Self {
            managed_roots,
            repositories_by_name: BTreeMap::new(),
            repositories_by_managed_project: BTreeMap::new(),
            input_repositories: HashMap::new(),
        };
        let mut managed_names = BTreeMap::<PathBuf, BTreeSet<String>>::new();
        for workspace in &workspaces {
            let managed = projector.managed_workspace(workspace);
            if let Some(managed) = &managed {
                managed_names
                    .entry(managed.managed_root.clone())
                    .or_default()
                    .insert(managed.project_name.clone());
            }
            let Some(evidence) = discover_repository(workspace) else {
                continue;
            };
            let project_name = managed
                .as_ref()
                .map(|managed| managed.project_name.clone())
                .or_else(|| project_name(&evidence.project_root));
            if let Some(project_name) = project_name {
                projector.insert_repository(project_name.clone(), &evidence);
                if let Some(managed) = managed.as_ref() {
                    projector.insert_managed_repository(
                        managed.managed_root.clone(),
                        project_name,
                        &evidence,
                    );
                }
            }
            projector
                .input_repositories
                .insert(workspace.clone(), evidence);
        }
        for (managed_root, project_names) in managed_names {
            projector.scan_managed_repositories(&managed_root, &project_names);
        }
        projector
    }

    pub(crate) fn project(&self, value: Option<&str>) -> Option<String> {
        let (workspace, fallback) = normalized_workspace(value)?;
        let managed = self.managed_workspace(&workspace);
        if let Some(evidence) = self
            .input_repositories
            .get(&workspace)
            .cloned()
            .or_else(|| discover_repository(&workspace))
        {
            let name = managed
                .as_ref()
                .map(|managed| managed.project_name.clone())
                .or_else(|| project_name(&evidence.project_root));
            let project_root = name
                .as_ref()
                .and_then(|name| self.repositories_by_name.get(name))
                .and_then(|repositories| repositories.get(&evidence.identity))
                .unwrap_or(&evidence.project_root);
            return path_string(project_root).or(Some(fallback));
        }
        if let Some(managed) = managed {
            let repositories = self.repositories_by_managed_project.get(&(
                managed.managed_root.clone(),
                managed.project_name.clone(),
            ));
            let global_repository_count = self
                .repositories_by_name
                .get(&managed.project_name)
                .map(BTreeMap::len)
                .unwrap_or(0);
            let project_root = match repositories.map(BTreeMap::len).unwrap_or(0) {
                // Out-of-root repositories cannot prove attribution, but multiple identities still
                // prove that collapsing every deleted worktree by basename would be ambiguous.
                0 if global_repository_count > 1 => workspace,
                // Remove the ephemeral worktree id when no surviving metadata can identify it.
                0 => managed.managed_root.join(&managed.project_name),
                1 => repositories
                    .and_then(|repositories| repositories.iter().next())
                    .and_then(|(identity, local_root)| {
                        self.repositories_by_name
                            .get(&managed.project_name)
                            .and_then(|repositories| repositories.get(identity))
                            .or(Some(local_root))
                    })
                    .cloned()
                    .unwrap_or(workspace),
                // Multiple proven identities make a deleted worktree ambiguous.
                _ => workspace,
            };
            return path_string(&project_root).or(Some(fallback));
        }
        path_string(&workspace).or(Some(fallback))
    }

    fn managed_workspace(&self, workspace: &Path) -> Option<ManagedWorkspace> {
        self.managed_roots.iter().find_map(|managed_root| {
            let relative = workspace.strip_prefix(managed_root).ok()?;
            let mut components = relative.components();
            let Component::Normal(_) = components.next()? else {
                return None;
            };
            let Component::Normal(project_name) = components.next()? else {
                return None;
            };
            Some(ManagedWorkspace {
                managed_root: managed_root.clone(),
                project_name: project_name.to_str()?.to_string(),
            })
        })
    }

    fn insert_repository(&mut self, project_name: String, evidence: &RepositoryEvidence) {
        let representative = self
            .repositories_by_name
            .entry(project_name)
            .or_default()
            .entry(evidence.identity.clone())
            .or_insert_with(|| evidence.project_root.clone());
        if evidence.project_root < *representative {
            *representative = evidence.project_root.clone();
        }
    }

    fn insert_managed_repository(
        &mut self,
        managed_root: PathBuf,
        project_name: String,
        evidence: &RepositoryEvidence,
    ) {
        let representative = self
            .repositories_by_managed_project
            .entry((managed_root, project_name))
            .or_default()
            .entry(evidence.identity.clone())
            .or_insert_with(|| evidence.project_root.clone());
        if evidence.project_root < *representative {
            *representative = evidence.project_root.clone();
        }
    }

    fn scan_managed_repositories(
        &mut self,
        managed_root: &Path,
        project_names: &BTreeSet<String>,
    ) {
        let Ok(entries) = fs::read_dir(managed_root) else {
            return;
        };
        for entry in entries.flatten() {
            for project_name in project_names {
                let candidate = entry.path().join(project_name);
                let Some(evidence) = discover_repository(&candidate) else {
                    continue;
                };
                self.insert_repository(project_name.clone(), &evidence);
                self.insert_managed_repository(
                    managed_root.to_path_buf(),
                    project_name.clone(),
                    &evidence,
                );
            }
        }
    }
}

pub fn project_workspace_root(value: Option<&str>) -> Option<String> {
    let value = value?;
    WorkspaceProjector::prepare([value]).project(Some(value))
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

fn discover_repository(workspace: &Path) -> Option<RepositoryEvidence> {
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
    let project_root = if common_dir.file_name() == Some(OsStr::new(".git")) {
        common_dir.parent().map(Path::to_path_buf)?
    } else {
        configured_worktree(&common_dir)
            .or_else(|| fs::canonicalize(repository_root).ok())?
    };
    let identity = repository_remote_identity(&common_dir)
        .map(RepositoryIdentity::Remote)
        .unwrap_or_else(|| RepositoryIdentity::CommonDir(common_dir));
    Some(RepositoryEvidence {
        identity,
        project_root,
    })
}

fn project_name(project_root: &Path) -> Option<String> {
    project_root.file_name()?.to_str().map(str::to_string)
}

fn known_managed_roots() -> Vec<PathBuf> {
    let mut roots = Vec::new();
    if let Some(codex_home) = std::env::var_os("CODEX_HOME") {
        roots.push(absolute_lexical(Path::new(&codex_home)).join("worktrees"));
    }
    if let Some(home) = std::env::var_os("HOME") {
        roots.push(
            absolute_lexical(Path::new(&home))
                .join(".codex")
                .join("worktrees"),
        );
    }
    #[cfg(windows)]
    if let Some(profile) = std::env::var_os("USERPROFILE") {
        roots.push(
            absolute_lexical(Path::new(&profile))
                .join(".codex")
                .join("worktrees"),
        );
    }
    roots
}

fn inferred_codex_managed_root(path: &Path) -> Option<PathBuf> {
    let components = path.components().collect::<Vec<_>>();
    let mut prefix = PathBuf::new();
    for (index, component) in components.iter().enumerate() {
        prefix.push(component.as_os_str());
        if !matches!(component, Component::Normal(value) if *value == OsStr::new(".codex")) {
            continue;
        }
        let Some(Component::Normal(next)) = components.get(index + 1) else {
            continue;
        };
        if *next == OsStr::new("worktrees") {
            prefix.push(next);
            return Some(prefix);
        }
    }
    None
}

fn absolute_lexical(path: &Path) -> PathBuf {
    if path.is_absolute() {
        lexical_normalize(path)
    } else if let Ok(current_dir) = std::env::current_dir() {
        lexical_normalize(&current_dir.join(path))
    } else {
        path.to_path_buf()
    }
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

fn repository_remote_identity(common_dir: &Path) -> Option<String> {
    let contents = fs::read_to_string(common_dir.join("config")).ok()?;
    let mut current_remote = None;
    let mut remotes = BTreeMap::<String, String>::new();
    for line in contents.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            current_remote = remote_section_name(line);
            continue;
        }
        let Some(remote) = current_remote.as_ref() else {
            continue;
        };
        if line.is_empty() || line.starts_with('#') || line.starts_with(';') {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        if key.trim().eq_ignore_ascii_case("url") {
            remotes.insert(remote.clone(), unquote(value.trim()).to_string());
        }
    }
    let value = if let Some(origin) = remotes.get("origin") {
        origin
    } else if remotes.len() == 1 {
        remotes.values().next()?
    } else {
        return None;
    };
    normalize_remote_identity(value)
}

fn remote_section_name(line: &str) -> Option<String> {
    let section = line.strip_prefix('[')?.strip_suffix(']')?.trim();
    let mut fields = section.splitn(2, char::is_whitespace);
    if !fields.next()?.eq_ignore_ascii_case("remote") {
        return None;
    }
    let name = unquote(fields.next()?.trim());
    (!name.is_empty()).then(|| name.to_ascii_lowercase())
}

fn normalize_remote_identity(value: &str) -> Option<String> {
    let value = value.trim().trim_end_matches('/');
    if value.is_empty() {
        return None;
    }
    if let Some((scheme, remainder)) = value.split_once("://") {
        if !scheme.eq_ignore_ascii_case("file") {
            if let Some((authority, path)) = remainder.split_once('/') {
                let host = authority.rsplit('@').next()?.to_ascii_lowercase();
                let path = trim_git_suffix(path.trim_matches('/'));
                if !host.is_empty() && !path.is_empty() {
                    return Some(format!("{host}/{path}"));
                }
            }
        }
    } else if let Some((authority, path)) = value.split_once(':') {
        let is_windows_drive = authority.len() == 1
            && authority
                .as_bytes()
                .first()
                .is_some_and(u8::is_ascii_alphabetic);
        if !is_windows_drive && !authority.contains('/') && !authority.contains('\\') {
            let host = authority.rsplit('@').next()?.to_ascii_lowercase();
            let path = trim_git_suffix(path.trim_matches('/'));
            if !host.is_empty() && !path.is_empty() {
                return Some(format!("{host}/{path}"));
            }
        }
    }
    let value = trim_git_suffix(value).trim_end_matches('/');
    if value.starts_with("file://") || Path::new(value).is_absolute() {
        return Some(value.to_string());
    }
    None
}

fn trim_git_suffix(value: &str) -> &str {
    let Some(index) = value.len().checked_sub(4) else {
        return value;
    };
    match value.get(index..) {
        Some(suffix) if suffix.eq_ignore_ascii_case(".git") => &value[..index],
        _ => value,
    }
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
    fn same_named_git_and_non_git_workspaces_stay_distinct() {
        let repository = RepositoryFixture::new("shared");
        let plain_temp = TempDir::new().unwrap();
        let plain = plain_temp.path().join("shared");
        fs::create_dir_all(&plain).unwrap();
        let repository_path = repository.main.to_str().unwrap();
        let plain_path = plain.to_str().unwrap();
        let projector = WorkspaceProjector::prepare([repository_path, plain_path]);

        assert_ne!(
            projector.project(Some(repository_path)),
            projector.project(Some(plain_path))
        );
    }

    #[test]
    fn same_named_repositories_keep_distinct_project_roots() {
        let first = RepositoryFixture::new("shared");
        let second = RepositoryFixture::new("shared");
        let managed = TempDir::new().unwrap();
        let missing = managed
            .path()
            .join(".codex/worktrees/deleted/shared");
        let first_path = first.main.to_str().unwrap();
        let second_path = second.main.to_str().unwrap();
        let missing_path = missing.to_str().unwrap();
        let projector = WorkspaceProjector::prepare([first_path, second_path, missing_path]);

        assert_ne!(
            projector.project(Some(first_path)),
            projector.project(Some(second_path))
        );
        assert_eq!(
            projector.project(Some(missing_path)),
            path_string(&missing)
        );
    }

    #[test]
    fn deleted_managed_worktrees_share_a_stable_name_fallback() {
        let temp = TempDir::new().unwrap();
        let managed_root = temp.path().join(".codex/worktrees");
        let first = managed_root.join("deleted-one/project");
        let second = managed_root.join("deleted-two/project");
        let first_path = first.to_str().unwrap();
        let second_path = second.to_str().unwrap();
        let projector = WorkspaceProjector::prepare([first_path, second_path]);

        assert_eq!(
            projector.project(Some(first_path)),
            projector.project(Some(second_path))
        );
        assert_eq!(
            projector.project(Some(first_path)),
            path_string(&managed_root.join("project"))
        );
    }

    #[test]
    fn deleted_managed_worktree_uses_the_only_proven_repository() {
        let fixture = RepositoryFixture::new("project");
        let managed_root = fixture.temp.path().join(".codex/worktrees");
        let live = fixture.linked_worktree_at(&managed_root, "live");
        let missing = managed_root.join("deleted/project");
        let live_path = live.to_str().unwrap();
        let missing_path = missing.to_str().unwrap();
        let projector = WorkspaceProjector::prepare([live_path, missing_path]);

        assert_eq!(
            projector.project(Some(live_path)),
            projector.project(Some(missing_path))
        );
        assert_eq!(
            projector.project(Some(missing_path)),
            path_string(&fs::canonicalize(&fixture.main).unwrap())
        );
    }

    #[test]
    fn deleted_managed_worktree_does_not_borrow_another_managed_roots_repository() {
        let fixture = RepositoryFixture::new("project");
        let first_managed_root = fixture.temp.path().join("first/.codex/worktrees");
        let second_managed_root = fixture.temp.path().join("second/.codex/worktrees");
        let live = fixture.linked_worktree_at(&first_managed_root, "live");
        let missing = second_managed_root.join("deleted/project");
        let live_path = live.to_str().unwrap();
        let missing_path = missing.to_str().unwrap();
        let projector = WorkspaceProjector::prepare([live_path, missing_path]);

        assert_ne!(
            projector.project(Some(live_path)),
            projector.project(Some(missing_path))
        );
        assert_eq!(
            projector.project(Some(missing_path)),
            path_string(&second_managed_root.join("project"))
        );
    }

    #[test]
    fn deleted_managed_fallback_is_stable_between_batch_and_single_projection() {
        let fixture = RepositoryFixture::new("project");
        let managed_root = fixture.temp.path().join("empty/.codex/worktrees");
        let missing = managed_root.join("deleted/project");
        let main_path = fixture.main.to_str().unwrap();
        let missing_path = missing.to_str().unwrap();
        let batch = WorkspaceProjector::prepare([main_path, missing_path]);

        assert_eq!(
            batch.project(Some(missing_path)),
            project_workspace_root(Some(missing_path))
        );
        assert_eq!(
            batch.project(Some(missing_path)),
            path_string(&managed_root.join("project"))
        );
    }

    #[test]
    fn clones_with_the_same_remote_share_a_representative_root() {
        let first = RepositoryFixture::new("shared");
        let second = RepositoryFixture::new("shared");
        first.set_remote("https://example.invalid/team/shared.git");
        second.set_remote("git@example.invalid:team/shared.git");
        let first_path = first.main.to_str().unwrap();
        let second_path = second.main.to_str().unwrap();
        let projector = WorkspaceProjector::prepare([first_path, second_path]);

        assert_eq!(
            projector.project(Some(first_path)),
            projector.project(Some(second_path))
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
            self.linked_worktree_at(&self.temp.path().join("worktrees"), id)
        }

        fn linked_worktree_at(&self, managed_root: &Path, id: &str) -> PathBuf {
            let git_dir = self.main.join(".git/worktrees").join(id);
            fs::create_dir_all(&git_dir).unwrap();
            fs::write(git_dir.join("commondir"), "../..\n").unwrap();
            let worktree = managed_root.join(id).join(self.main.file_name().unwrap());
            fs::create_dir_all(&worktree).unwrap();
            fs::write(
                worktree.join(".git"),
                format!("gitdir: {}\n", git_dir.display()),
            )
            .unwrap();
            worktree
        }

        fn set_remote(&self, remote: &str) {
            fs::write(
                self.main.join(".git/config"),
                format!("[remote \"origin\"]\n\turl = {remote}\n"),
            )
            .unwrap();
        }
    }
}
