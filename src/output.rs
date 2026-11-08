use anyhow::{Context, Result, bail};
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io::Read,
    path::{Component, Path, PathBuf},
};
use walkdir::WalkDir;

const MANIFEST: &str = ".mdcontent-manifest.json";
const LOCK: &str = ".mdcontent.lock";
const JOURNAL: &str = ".mdcontent-journal";
#[derive(Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Manifest {
    version: u8,
    files: BTreeMap<String, String>,
}
#[derive(Serialize, Deserialize)]
struct Change {
    name: String,
    old: Option<String>,
    new: Option<String>,
    backup: Option<usize>,
}
#[derive(Serialize, Deserialize)]
struct Journal {
    version: u8,
    changes: Vec<Change>,
    new_manifest: Vec<u8>,
}

/// A single writer owns the output from recovery through manifest commit.
pub(crate) fn lock(output: &Path) -> Result<fs::File> {
    let path = output.join(LOCK);
    if path
        .symlink_metadata()
        .is_ok_and(|m| m.file_type().is_symlink())
    {
        bail!("output lock cannot be a symlink");
    }
    let file = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(path)?;
    file.lock_exclusive()?;
    Ok(file)
}
/// Encode path components without conflating literal backslashes with separators.
fn encode(path: &Path) -> Result<String> {
    let mut result = Vec::new();
    for component in path.components() {
        let Component::Normal(name) = component else {
            bail!("invalid output path");
        };
        result.push(
            name.to_str()
                .context("output filename is not UTF-8")?
                .replace('%', "%25")
                .replace('\\', "%5C"),
        );
    }
    if result.is_empty() || matches!(result[0].as_str(), MANIFEST | LOCK | JOURNAL) {
        bail!("reserved output path");
    }
    Ok(result.join("/"))
}
/// Decode and validate a manifest path before using it below the output root.
fn safe_path(name: &str) -> Result<PathBuf> {
    let mut path = PathBuf::new();
    if name.is_empty() {
        bail!("empty output path");
    }
    for part in name.split('/') {
        let decoded = part.replace("%5C", "\\").replace("%25", "%");
        let p = Path::new(&decoded);
        if decoded.is_empty()
            || p.components().count() != 1
            || !matches!(p.components().next(), Some(Component::Normal(_)))
        {
            bail!("invalid output path: {name}");
        }
        path.push(decoded);
    }
    if matches!(path.components().next(), Some(Component::Normal(p)) if p == MANIFEST || p == LOCK || p == JOURNAL)
    {
        bail!("reserved output path");
    }
    Ok(path)
}
/// Hash a file incrementally without loading large assets into memory.
fn digest(path: &Path) -> Result<String> {
    let mut file = fs::File::open(path)?;
    let mut h = Sha256::new();
    let mut buffer = [0_u8; 65536];
    loop {
        let n = file.read(&mut buffer)?;
        if n == 0 {
            break;
        }
        h.update(&buffer[..n]);
    }
    Ok(h.finalize().iter().map(|b| format!("{b:02x}")).collect())
}
/// Index staged files by their portable manifest path and content digest.
fn collect(stage: &Path) -> Result<BTreeMap<String, String>> {
    let mut files = BTreeMap::new();
    for entry in WalkDir::new(stage) {
        let entry = entry?;
        if entry.file_type().is_symlink() {
            bail!("generated output contains a symlink");
        }
        if entry.file_type().is_file() {
            files.insert(
                encode(entry.path().strip_prefix(stage)?)?,
                digest(entry.path())?,
            );
        }
    }
    Ok(files)
}
/// Read the current manifest and migrate paths from the original format.
fn read_manifest(output: &Path) -> Result<Manifest> {
    let path = output.join(MANIFEST);
    if !path.exists() {
        return Ok(Manifest {
            version: 2,
            files: BTreeMap::new(),
        });
    }
    let mut manifest: Manifest =
        serde_json::from_slice(&fs::read(path)?).context("invalid mdcontent output manifest")?;
    if manifest.version == 1 {
        let mut migrated = BTreeMap::new();
        for (key, value) in manifest.files {
            if key.contains('\\') {
                bail!("invalid legacy manifest path");
            }
            migrated.insert(encode(Path::new(&key))?, value);
        }
        manifest = Manifest {
            version: 2,
            files: migrated,
        };
    }
    if manifest.version != 2 {
        bail!("unsupported mdcontent output manifest version");
    }
    for key in manifest.files.keys() {
        safe_path(key)?;
    }
    Ok(manifest)
}
fn current(path: &Path) -> Result<Option<String>> {
    if path.is_file() {
        Ok(Some(digest(path)?))
    } else {
        Ok(None)
    }
}
/// Remove only the internal transaction journal after completion or recovery.
fn remove_journal(dir: &Path) -> Result<()> {
    for entry in WalkDir::new(dir)
        .min_depth(1)
        .contents_first(true)
        .follow_links(false)
    {
        let entry = entry?;
        if entry.file_type().is_symlink() {
            bail!("journal contains a symlink");
        }
        if entry.file_type().is_dir() {
            fs::remove_dir(entry.path())?;
        } else {
            fs::remove_file(entry.path())?;
        }
    }
    fs::remove_dir(dir)?;
    Ok(())
}
/// Restore an interrupted publication, without overwriting files changed afterward.
pub(crate) fn recover(output: &Path) -> Result<()> {
    let dir = output.join(JOURNAL);
    if !dir.exists() {
        return Ok(());
    }
    let journal: Journal = serde_json::from_slice(&fs::read(dir.join("journal.json"))?)
        .context("invalid output recovery journal")?;
    if journal.version != 1 {
        bail!("unsupported output recovery journal");
    }
    if fs::read(output.join(MANIFEST)).ok().as_deref() == Some(&journal.new_manifest) {
        return remove_journal(&dir);
    }
    for change in &journal.changes {
        let path = output.join(safe_path(&change.name)?);
        let actual = current(&path)?;
        if actual.is_some() && actual != change.old && actual != change.new {
            bail!(
                "recovery would overwrite a modified file: {}",
                path.display()
            );
        }
        if change.old.is_some() != change.backup.is_some() {
            bail!("invalid recovery journal");
        }
        if let (Some(expected), Some(index)) = (&change.old, change.backup)
            && digest(&dir.join(index.to_string()))? != *expected
        {
            bail!("invalid recovery backup");
        }
    }
    for change in &journal.changes {
        let path = output.join(safe_path(&change.name)?);
        if path.is_file() && current(&path)? != change.old {
            fs::remove_file(&path)?;
        }
    }
    for change in &journal.changes {
        if let Some(index) = change.backup {
            let path = output.join(safe_path(&change.name)?);
            if path.is_dir() {
                fs::remove_dir(&path)
                    .with_context(|| format!("recovery directory not empty: {}", path.display()))?;
            }
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::copy(dir.join(index.to_string()), path)?;
        }
    }
    remove_journal(&dir)
}
/// Stage a reversible journal before touching any previously generated file.
pub(crate) fn publish(stage: &Path, output: &Path) -> Result<()> {
    let old = read_manifest(output)?;
    let next = collect(stage)?;
    let mut removals = BTreeSet::new();
    for (name, hash) in &old.files {
        if !next.contains_key(name)
            && current(&output.join(safe_path(name)?))?.as_ref() == Some(hash)
        {
            removals.insert(name.clone());
        }
    }
    let mut dirs = BTreeSet::new();
    for (name, expected) in &next {
        let rel = safe_path(name)?;
        let dest = output.join(&rel);
        let mut parent = dest.parent();
        while let Some(p) = parent {
            if p == output {
                break;
            }
            if p.is_file() {
                let key = encode(p.strip_prefix(output)?)?;
                if !removals.contains(&key) {
                    bail!(
                        "output parent is an untracked or modified file: {}",
                        p.display()
                    );
                }
            }
            parent = p.parent();
        }
        if dest.is_dir() {
            let prefix = format!("{name}/");
            if !old.files.keys().any(|file| file.starts_with(&prefix)) {
                bail!("output path is an untracked directory: {}", dest.display());
            }
            for entry in WalkDir::new(&dest).min_depth(1).contents_first(true) {
                let e = entry?;
                let key = encode(e.path().strip_prefix(output)?)?;
                if e.file_type().is_file() && !removals.contains(&key) {
                    bail!(
                        "output directory contains an untracked or modified file: {}",
                        e.path().display()
                    );
                }
                if e.file_type().is_dir() {
                    let prefix = format!("{key}/");
                    if !old.files.keys().any(|name| name.starts_with(&prefix)) {
                        bail!(
                            "output directory contains an untracked directory: {}",
                            e.path().display()
                        );
                    }
                    dirs.insert(e.path().to_path_buf());
                }
            }
            dirs.insert(dest.clone());
        } else if dest.is_file() {
            let actual = digest(&dest)?;
            if actual != *expected && old.files.get(name) != Some(&actual) {
                bail!(
                    "refusing to overwrite an untracked or modified output file: {}",
                    dest.display()
                );
            }
        }
    }
    let mut names: BTreeSet<String> = next.keys().cloned().collect();
    names.extend(removals.iter().cloned());
    let tmp = tempfile::tempdir_in(output)?;
    let mut changes = Vec::new();
    for name in names {
        let path = output.join(safe_path(&name)?);
        let previous = current(&path)?;
        if let Some(actual) = &previous {
            let allowed = next.get(&name) == Some(actual) || old.files.get(&name) == Some(actual);
            if !allowed {
                bail!("output changed during build: {}", path.display());
            }
        }
        if removals.contains(&name) && previous.as_ref() != old.files.get(&name) {
            bail!("output changed during build: {}", path.display());
        }
        let backup = if previous.is_some() {
            let index = changes.len();
            fs::copy(&path, tmp.path().join(index.to_string()))?;
            Some(index)
        } else {
            None
        };
        changes.push(Change {
            new: next.get(&name).cloned(),
            name,
            old: previous,
            backup,
        });
    }
    for change in &changes {
        let path = output.join(safe_path(&change.name)?);
        if current(&path)? != change.old {
            bail!("output changed during build: {}", path.display());
        }
        if let (Some(expected), Some(index)) = (&change.old, change.backup)
            && digest(&tmp.path().join(index.to_string()))? != *expected
        {
            bail!("output changed during backup: {}", path.display());
        }
    }
    let bytes = serde_json::to_vec(&Manifest {
        version: 2,
        files: next.clone(),
    })?;
    fs::write(
        tmp.path().join("journal.json"),
        serde_json::to_vec(&Journal {
            version: 1,
            changes,
            new_manifest: bytes.clone(),
        })?,
    )?;
    let tmp_path = tmp.keep();
    fs::rename(&tmp_path, output.join(JOURNAL))?;
    let result = (|| -> Result<()> {
        for name in &removals {
            let path = output.join(safe_path(name)?);
            if path.is_file() {
                fs::remove_file(&path)?;
            }
        }
        let mut dirs: Vec<_> = dirs.into_iter().collect();
        dirs.sort_by_key(|d| std::cmp::Reverse(d.components().count()));
        for dir in dirs {
            fs::remove_dir(dir)?;
        }
        for name in next.keys() {
            let rel = safe_path(name)?;
            let dest = output.join(&rel);
            if let Some(parent) = dest.parent() {
                fs::create_dir_all(parent)?;
            }
            let mut temp = tempfile::NamedTempFile::new_in(
                dest.parent().context("output path has no parent")?,
            )?;
            std::io::copy(&mut fs::File::open(stage.join(rel))?, &mut temp)?;
            temp.persist(dest).map_err(|e| e.error)?;
        }
        let mut temp = tempfile::NamedTempFile::new_in(output)?;
        std::io::Write::write_all(&mut temp, &bytes)?;
        temp.persist(output.join(MANIFEST)).map_err(|e| e.error)?;
        Ok(())
    })();
    match result {
        Ok(()) => remove_journal(&output.join(JOURNAL)),
        Err(e) => {
            recover(output).context("failed to recover interrupted publication")?;
            Err(e)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn interrupted_publication_recovers_and_preserves_user_changes() {
        let root = tempfile::tempdir().unwrap();
        let output = root.path();
        let journal = output.join(JOURNAL);
        fs::create_dir(&journal).unwrap();
        fs::write(journal.join("0"), "before").unwrap();
        let old = digest(&journal.join("0")).unwrap();
        fs::write(output.join("page"), "after").unwrap();
        let new = digest(&output.join("page")).unwrap();
        let old_manifest = Manifest {
            version: 2,
            files: BTreeMap::from([("page".to_string(), old.clone())]),
        };
        fs::write(
            output.join(MANIFEST),
            serde_json::to_vec(&old_manifest).unwrap(),
        )
        .unwrap();
        let state = Journal {
            version: 1,
            changes: vec![Change {
                name: "page".into(),
                old: Some(old),
                new: Some(new),
                backup: Some(0),
            }],
            new_manifest: b"new manifest".to_vec(),
        };
        fs::write(
            journal.join("journal.json"),
            serde_json::to_vec(&state).unwrap(),
        )
        .unwrap();
        fs::write(output.join("page"), "user edit").unwrap();
        assert!(recover(output).is_err());
        fs::write(output.join("page"), "after").unwrap();
        recover(output).unwrap();
        assert_eq!(fs::read_to_string(output.join("page")).unwrap(), "before");
        assert!(!journal.exists());
        let stage = tempfile::tempdir().unwrap();
        fs::write(stage.path().join("page"), "after").unwrap();
        publish(stage.path(), output).unwrap();
        assert_eq!(fs::read_to_string(output.join("page")).unwrap(), "after");
    }
}
