use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs,
    path::{Component, Path, PathBuf},
};
use walkdir::WalkDir;

const MANIFEST: &str = ".mdcontent-manifest.json";
#[derive(Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Manifest {
    version: u8,
    files: BTreeMap<String, String>,
}

fn safe_path(name: &str) -> Result<PathBuf> {
    let path = Path::new(name);
    if name.is_empty()
        || name.contains('\\')
        || !path.components().all(|p| matches!(p, Component::Normal(_)))
        || name == MANIFEST
    {
        bail!("invalid path in output manifest: {name}");
    }
    Ok(path.to_path_buf())
}
fn digest(path: &Path) -> Result<String> {
    let mut file = fs::File::open(path)?;
    let mut hasher = Sha256::new();
    use std::io::Read;
    let mut buffer = [0_u8; 65536];
    loop {
        let n = file.read(&mut buffer)?;
        if n == 0 {
            break;
        }
        hasher.update(&buffer[..n]);
    }
    Ok(hasher
        .finalize()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect())
}
fn collect(stage: &Path) -> Result<BTreeMap<String, String>> {
    let mut files = BTreeMap::new();
    for entry in WalkDir::new(stage) {
        let entry = entry?;
        if entry.file_type().is_symlink() {
            bail!("generated output contains a symlink");
        }
        if entry.file_type().is_file() {
            let relative = entry.path().strip_prefix(stage)?;
            let name = relative
                .to_str()
                .context("output filename is not UTF-8")?
                .replace('\\', "/");
            safe_path(&name)?;
            files.insert(name, digest(entry.path())?);
        }
    }
    Ok(files)
}
pub(crate) fn publish(stage: &Path, output: &Path) -> Result<()> {
    let manifest_path = output.join(MANIFEST);
    let old: Manifest = if manifest_path.exists() {
        let manifest: Manifest = serde_json::from_slice(&fs::read(&manifest_path)?)
            .context("invalid mdcontent output manifest")?;
        if manifest.version != 1 {
            bail!("unsupported mdcontent output manifest version");
        }
        manifest
    } else {
        Manifest {
            version: 1,
            files: BTreeMap::new(),
        }
    };
    for name in old.files.keys() {
        safe_path(name)?;
    }
    let next = collect(stage)?;
    // Never overwrite an untracked or locally changed file. Identical legacy output can be adopted.
    for (name, expected) in &next {
        let dest = output.join(safe_path(name)?);
        if dest.exists() {
            if !dest.is_file() {
                bail!("output path is not a file: {}", dest.display());
            }
            let actual = digest(&dest)?;
            if actual != *expected && old.files.get(name) != Some(&actual) {
                bail!(
                    "refusing to overwrite an untracked or modified output file: {}",
                    dest.display()
                );
            }
        }
    }
    for name in next.keys() {
        let rel = safe_path(name)?;
        let dest = output.join(&rel);
        if let Some(parent) = dest.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::copy(stage.join(rel), dest)?;
    }
    for (name, previous_digest) in &old.files {
        if next.contains_key(name) {
            continue;
        }
        let dest = output.join(safe_path(name)?);
        if dest.is_file() && digest(&dest)? == *previous_digest {
            fs::remove_file(dest)?;
        }
    }
    let manifest = Manifest {
        version: 1,
        files: next,
    };
    // Rename a temporary manifest into place after all generated files are written.
    let mut temp = tempfile::NamedTempFile::new_in(output)?;
    serde_json::to_writer(&mut temp, &manifest)?;
    temp.persist(&manifest_path).map_err(|e| e.error)?;
    Ok(())
}
