//! Immutable sidecar installation, independent of the App's location.
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::{self, Read},
    path::{Component, Path, PathBuf},
};

fn relative(value: &str) -> io::Result<&Path> {
    let path = Path::new(value);
    if path.as_os_str().is_empty()
        || path
            .components()
            .any(|c| !matches!(c, Component::Normal(_)))
    {
        return Err(io::Error::other("召回包文件路径无效"));
    }
    Ok(path)
}
pub fn install(source: &Path, workspace: &Path) -> io::Result<Option<PathBuf>> {
    if source.join("bundle.json").is_file() {
        return install_archive(source, workspace).map(Some);
    }
    let manifest = source.join("manifest.json");
    if !manifest.is_file() {
        return Ok(None);
    }
    let bytes = fs::read(&manifest)?;
    let info: Value = serde_json::from_slice(&bytes)?;
    if info["protocol"] != 1 {
        return Err(io::Error::other("召回包协议不兼容"));
    }
    let hash = format!("{:x}", Sha256::digest(&bytes));
    let parent = workspace.join("tools/task-recall/versions");
    fs::create_dir_all(&parent)?;
    let destination = parent.join(hash);
    if destination.join("manifest.json").is_file() {
        return Ok(Some(destination));
    }
    let temp = tempfile::TempDir::new_in(&parent)?;
    let files = info["files"]
        .as_array()
        .ok_or_else(|| io::Error::other("召回包缺少文件表"))?;
    for file in files {
        let name = relative(
            file["path"]
                .as_str()
                .ok_or_else(|| io::Error::other("召回包路径缺失"))?,
        )?;
        let input = source.join(name);
        let output = temp.path().join(name);
        let content = fs::read(&input)?;
        if file["sha256"].as_str() != Some(&format!("{:x}", Sha256::digest(&content))) {
            return Err(io::Error::other("召回包文件校验失败"));
        }
        fs::create_dir_all(output.parent().unwrap())?;
        fs::write(&output, &content)?;
        fs::set_permissions(output, fs::metadata(input)?.permissions())?;
    }
    fs::write(temp.path().join("manifest.json"), bytes)?;
    fs::rename(temp.path(), &destination)?;
    Ok(Some(destination))
}

fn file_hash(path: &Path) -> io::Result<String> {
    let mut file = fs::File::open(path)?;
    let mut hash = Sha256::new();
    let mut buffer = [0u8; 65536];
    loop {
        let size = file.read(&mut buffer)?;
        if size == 0 {
            break;
        }
        hash.update(&buffer[..size]);
    }
    Ok(format!("{:x}", hash.finalize()))
}
fn install_archive(source: &Path, workspace: &Path) -> io::Result<PathBuf> {
    let descriptor: Value = serde_json::from_slice(&fs::read(source.join("bundle.json"))?)?;
    let hash = descriptor["sha256"]
        .as_str()
        .filter(|s| s.len() == 64 && s.bytes().all(|c| c.is_ascii_hexdigit()))
        .ok_or_else(|| io::Error::other("召回包摘要无效"))?;
    if descriptor["protocol"] != 1 {
        return Err(io::Error::other("召回包协议不兼容"));
    }
    let parent = workspace.join("tools/task-recall/versions");
    fs::create_dir_all(&parent)?;
    let destination = parent.join(hash);
    if destination.join("manifest.json").is_file() {
        return Ok(destination);
    }
    let archive = source.join("bundle.tar.gz");
    if file_hash(&archive)? != hash {
        return Err(io::Error::other("召回压缩包校验失败"));
    }
    let temp = tempfile::TempDir::new_in(&parent)?;
    let mut tar = tar::Archive::new(flate2::read::GzDecoder::new(fs::File::open(archive)?));
    let mut total = 0u64;
    for entry in tar.entries()? {
        let mut entry = entry?;
        if !entry.header().entry_type().is_file() {
            return Err(io::Error::other("召回包不允许链接和特殊文件"));
        }
        total = total
            .checked_add(entry.size())
            .ok_or_else(|| io::Error::other("召回包大小无效"))?;
        if total > 4 * 1024 * 1024 * 1024 || !entry.unpack_in(temp.path())? {
            return Err(io::Error::other("召回包大小或路径无效"));
        }
    }
    let manifest: Value = serde_json::from_slice(&fs::read(temp.path().join("manifest.json"))?)?;
    for file in manifest["files"]
        .as_array()
        .ok_or_else(|| io::Error::other("召回包清单缺失"))?
    {
        let path = relative(
            file["path"]
                .as_str()
                .ok_or_else(|| io::Error::other("召回包路径缺失"))?,
        )?;
        if file["sha256"].as_str() != Some(&file_hash(&temp.path().join(path))?) {
            return Err(io::Error::other("召回解压文件校验失败"));
        }
    }
    fs::rename(temp.path(), &destination)?;
    Ok(destination)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    fn archive(root: &Path, link: bool) {
        fs::create_dir_all(root).unwrap();
        let file = fs::File::create(root.join("bundle.tar.gz")).unwrap();
        let mut tar = tar::Builder::new(flate2::write::GzEncoder::new(
            file,
            flate2::Compression::fast(),
        ));
        let contents = b"print('fixture')";
        let manifest=serde_json::to_vec(&json!({"protocol":1,"files":[{"path":"main.py","sha256":format!("{:x}",Sha256::digest(contents))}]})).unwrap();
        for (name, bytes) in [
            ("manifest.json", manifest.as_slice()),
            ("main.py", contents.as_slice()),
        ] {
            let mut header = tar::Header::new_gnu();
            header.set_size(bytes.len() as u64);
            header.set_mode(0o600);
            header.set_cksum();
            tar.append_data(&mut header, name, bytes).unwrap();
        }
        if link {
            let mut header = tar::Header::new_gnu();
            header.set_size(0);
            header.set_mode(0o600);
            header.set_entry_type(tar::EntryType::Symlink);
            header.set_link_name("../outside").unwrap();
            header.set_cksum();
            tar.append_data(&mut header, "escape", &[][..]).unwrap();
        }
        tar.into_inner().unwrap().finish().unwrap();
        fs::write(
            root.join("bundle.json"),
            serde_json::to_vec(
                &json!({"protocol":1,"sha256":file_hash(&root.join("bundle.tar.gz")).unwrap()}),
            )
            .unwrap(),
        )
        .unwrap();
    }
    #[test]
    fn immutable_archive_install_is_repeatable_and_corrupt_input_is_rejected() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("source");
        let workspace = temp.path().join("workspace");
        archive(&source, false);
        let first = install(&source, &workspace).unwrap().unwrap();
        assert_eq!(
            fs::read(first.join("main.py")).unwrap(),
            b"print('fixture')"
        );
        assert_eq!(install(&source, &workspace).unwrap(), Some(first));
        fs::write(source.join("bundle.tar.gz"), b"corrupt").unwrap();
        assert!(install(&source, &temp.path().join("fresh")).is_err());
    }
    #[test]
    fn archive_links_cannot_escape_the_installed_version() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("source");
        archive(&source, true);
        assert!(install(&source, &temp.path().join("workspace")).is_err());
        assert!(!temp.path().join("outside").exists());
    }
}
