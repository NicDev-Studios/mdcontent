use std::{
    fs,
    process::Command,
    time::{SystemTime, UNIX_EPOCH},
};

#[test]
fn build_respects_navigation_search_and_public_assets() {
    let root = std::env::temp_dir().join(format!(
        "mdcontent-test-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir_all(root.join("docs")).unwrap();
    fs::create_dir_all(root.join("public/assets")).unwrap();
    fs::write(root.join("docs/index.md"), "# Home\n").unwrap();
    fs::write(root.join("docs/guide.md"), "# Guide\n").unwrap();
    fs::write(root.join("public/assets/test.txt"), "asset").unwrap();
    fs::write(root.join("mdcontent.toml"), "language = 'de'\nsearch = false\nnavigation = ['guide.md']\npublic_dir = 'public'\n[[header_links]]\nlabel = 'Source'\nurl = 'https://example.com'\n").unwrap();
    for _ in 0..2 {
        let result = Command::new(env!("CARGO_BIN_EXE_mdcontent"))
            .args(["build", "--config"])
            .arg(root.join("mdcontent.toml"))
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
    }
    let page = fs::read_to_string(root.join("site/index.html")).unwrap();
    assert!(
        page.contains("lang=\"de\"") && page.contains(">Guide</a>") && page.contains(">Source</a>")
    );
    assert!(!page.contains("id=\"search\"") && !page.contains(">Home</a>"));
    assert!(!root.join("site/search.json").exists());
    assert_eq!(
        fs::read_to_string(root.join("site/assets/test.txt")).unwrap(),
        "asset"
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn rebuild_removes_only_unchanged_generated_files() {
    let root = std::env::temp_dir().join(format!(
        "mdcontent-clean-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir_all(root.join("docs")).unwrap();
    fs::create_dir_all(root.join("public")).unwrap();
    fs::write(root.join("docs/index.md"), "# Home").unwrap();
    fs::write(root.join("docs/guide.md"), "# Guide").unwrap();
    fs::write(root.join("public/remove.txt"), "old").unwrap();
    fs::write(root.join("public/keep.txt"), "original").unwrap();
    let config = root.join("mdcontent.toml");
    fs::write(&config, "public_dir = 'public'\n").unwrap();
    let build = || {
        Command::new(env!("CARGO_BIN_EXE_mdcontent"))
            .args(["build", "--config"])
            .arg(&config)
            .output()
            .unwrap()
    };
    assert!(build().status.success());
    fs::write(root.join("site/untracked.txt"), "untouched").unwrap();
    fs::write(root.join("site/keep.txt"), "user changed").unwrap();
    fs::remove_file(root.join("docs/guide.md")).unwrap();
    fs::remove_file(root.join("public/remove.txt")).unwrap();
    fs::remove_file(root.join("public/keep.txt")).unwrap();
    fs::write(&config, "public_dir = 'public'\nsearch = false\n").unwrap();
    assert!(build().status.success());
    assert!(!root.join("site/guide/index.html").exists());
    assert!(!root.join("site/remove.txt").exists());
    assert!(!root.join("site/search.json").exists());
    assert_eq!(
        fs::read_to_string(root.join("site/keep.txt")).unwrap(),
        "user changed"
    );
    assert_eq!(
        fs::read_to_string(root.join("site/untracked.txt")).unwrap(),
        "untouched"
    );
    fs::write(root.join("public/untracked.txt"), "conflict").unwrap();
    let result = build();
    assert!(!result.status.success());
    assert_eq!(
        fs::read_to_string(root.join("site/untracked.txt")).unwrap(),
        "untouched"
    );
    fs::remove_dir_all(root).unwrap();
}
