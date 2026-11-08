use sha2::{Digest, Sha256};
use std::{
    fs,
    io::Read,
    path::{Path, PathBuf},
    sync::{Arc, RwLock},
    thread,
    time::Duration,
};
use walkdir::WalkDir;

pub(crate) struct State {
    pub root: PathBuf,
    pub base: String,
    pub revision: u64,
}
pub(crate) type Stamp = Vec<(PathBuf, Option<Vec<u8>>)>;
/// Record a file’s content digest, including a marker for missing files.
fn add_file(path: &Path, entries: &mut Stamp) {
    let hash = fs::File::open(path).ok().and_then(|mut file| {
        let mut h = Sha256::new();
        let mut buf = [0_u8; 65536];
        loop {
            let n = file.read(&mut buf).ok()?;
            if n == 0 {
                break;
            }
            h.update(&buf[..n]);
        }
        Some(h.finalize().to_vec())
    });
    entries.push((path.to_path_buf(), hash));
}
/// Snapshot actual bytes so edits with unchanged timestamps and lengths still trigger a build.
pub(crate) fn snapshot(config: &Path, c: &crate::config::Config) -> Stamp {
    let mut entries = Vec::new();
    add_file(config, &mut entries);
    for root in [Some(&c.content_dir), c.public_dir.as_ref()]
        .into_iter()
        .flatten()
    {
        for entry in WalkDir::new(root)
            .follow_links(false)
            .into_iter()
            .filter_map(Result::ok)
        {
            if entry.file_type().is_file() {
                add_file(entry.path(), &mut entries);
            }
        }
    }
    for file in [&c.custom_css, &c.logo, &c.favicon].into_iter().flatten() {
        add_file(file, &mut entries);
    }
    entries.sort();
    entries
}
/// Retry failed builds with a delay; output conflicts may be fixed without changing input files.
pub(crate) fn spawn(
    config: PathBuf,
    shared: Arc<RwLock<State>>,
    mut last_config: crate::config::Config,
    mut previous: Stamp,
) {
    thread::spawn(move || {
        let mut retry = false;
        loop {
            thread::sleep(Duration::from_millis(if retry { 2500 } else { 700 }));
            if let Ok(c) = crate::config::load(&config) {
                last_config = c;
            }
            let next = snapshot(&config, &last_config);
            if !retry && next == previous {
                continue;
            }
            let mut state = match shared.write() {
                Ok(state) => state,
                Err(_) => return,
            };
            match crate::build::build(&config) {
                Ok(built) => {
                    state.root = built.root;
                    state.base = built.base;
                    state.revision += 1;
                    previous = next;
                    retry = false;
                    println!("Preview updated");
                }
                Err(e) => {
                    if !retry {
                        eprintln!("Preview rebuild failed; retrying: {e:#}");
                    }
                    retry = true;
                }
            }
        }
    });
}
