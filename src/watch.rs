use std::{
    fs,
    path::{Path, PathBuf},
    sync::{Arc, RwLock},
    thread,
    time::{Duration, UNIX_EPOCH},
};
use walkdir::WalkDir;

pub(crate) struct State {
    pub root: PathBuf,
    pub base: String,
    pub revision: u64,
}
type Stamp = Vec<(PathBuf, u64, u128, bool)>;
fn add_file(path: &Path, entries: &mut Stamp) {
    let m = fs::metadata(path);
    let (len, time, exists) = match m {
        Ok(m) => (
            m.len(),
            m.modified()
                .ok()
                .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                .map(|t| t.as_nanos())
                .unwrap_or(0),
            true,
        ),
        Err(_) => (0, 0, false),
    };
    entries.push((path.to_path_buf(), len, time, exists));
}
fn snapshot(config: &Path, c: &crate::config::Config) -> Stamp {
    let mut entries = Vec::new();
    add_file(config, &mut entries);
    for root in [Some(&c.content_dir), c.public_dir.as_ref()]
        .into_iter()
        .flatten()
    {
        add_file(root, &mut entries);
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
pub(crate) fn spawn(config: PathBuf, shared: Arc<RwLock<State>>) {
    thread::spawn(move || {
        let mut last_config = match crate::config::load(&config) {
            Ok(c) => c,
            Err(e) => {
                eprintln!("preview watch: {e:#}");
                return;
            }
        };
        let mut previous = snapshot(&config, &last_config);
        loop {
            thread::sleep(Duration::from_millis(700));
            if let Ok(c) = crate::config::load(&config) {
                last_config = c;
            }
            let next = snapshot(&config, &last_config);
            if next == previous {
                continue;
            }
            previous = next;
            let mut state = match shared.write() {
                Ok(state) => state,
                Err(_) => return,
            };
            match crate::build::build(&config).and_then(|_| {
                let c = crate::config::load(&config)?;
                state.root = fs::canonicalize(&c.output_dir)?;
                state.base = c.base_path;
                state.revision += 1;
                Ok(())
            }) {
                Ok(()) => {
                    println!("Preview updated");
                }
                Err(e) => eprintln!("Preview rebuild failed: {e:#}"),
            }
        }
    });
}
