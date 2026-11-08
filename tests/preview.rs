use std::{
    fs,
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    process::{Child, Command, Stdio},
    thread,
    time::{Duration, Instant},
};

struct Server(Child);
impl Drop for Server {
    /// Stop and reap the test preview server.
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
/// Request a preview page with a bounded response read.
fn get(address: &str, path: &str) -> Option<String> {
    let mut stream = TcpStream::connect(address).ok()?;
    stream.set_read_timeout(Some(Duration::from_secs(2))).ok()?;
    write!(
        stream,
        "GET {path} HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n"
    )
    .ok()?;
    let mut response = String::new();
    stream.read_to_string(&mut response).ok()?;
    Some(response)
}
/// Request the preview revision with a bounded response read.
fn get_revision(address: &str, path: &str) -> Option<String> {
    let mut stream = TcpStream::connect(address).ok()?;
    stream.set_read_timeout(Some(Duration::from_secs(2))).ok()?;
    write!(stream, "GET {path} HTTP/1.1\r\nHost: localhost\r\nX-Mdcontent-Preview: revision\r\nConnection: close\r\n\r\n").ok()?;
    let mut response = String::new();
    stream.read_to_string(&mut response).ok()?;
    Some(response)
}
/// Wait for a preview condition until the test deadline.
fn wait_for(mut f: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(12);
    while Instant::now() < deadline {
        if f() {
            return;
        }
        thread::sleep(Duration::from_millis(100));
    }
    panic!("preview did not update in time");
}
// Execute the actual injected script with mocked browser primitives when Node is available.
/// Exercise preview polling against mocked browser primitives.
fn verify_reload_script(html: &str) {
    let Some(start) = html.rfind("<script>(()=>") else {
        panic!("reload script missing")
    };
    let code = &html[start + 8..];
    let code = &code[..code.find("</script>").unwrap()];
    let session = code
        .split("session=")
        .nth(1)
        .unwrap()
        .split(";setInterval")
        .next()
        .unwrap();
    let js = format!(
        r#"let tick, moved='', reloaded=false, documentText='old', serverText='new', data={{revision:0,base:'/',session:'initial'}};
let location={{pathname:'/',search:'?x=1',hash:'#h',replace:p=>moved=p,reload:()=>{{reloaded=true;documentText=serverText}}}};
let setInterval=f=>tick=f; let fetch=async()=>({{ok:true,json:async()=>data}});
data.session={};eval({});
(async()=>{{await tick();if(moved||reloaded)process.exit(1);data.session+='-changed';await tick();if(!reloaded||moved)process.exit(5);data.session={};reloaded=false;documentText='old';data.revision=1;await tick();if(!reloaded||documentText!=='new')process.exit(4);data={{session:data.session,revision:1,base:'/manual/'}};await tick();if(moved!='/manual/?x=1#h')process.exit(2)}})().catch(()=>process.exit(3));"#,
        session,
        serde_json::to_string(code).unwrap(),
        session
    );
    match Command::new("node").args(["-e", &js]).status() {
        Ok(status) => assert!(status.success(), "reload script did not respond correctly"),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            eprintln!("Node unavailable; script execution test skipped")
        }
        Err(e) => panic!("failed to run Node: {e}"),
    }
}
/// Rebuild preview pages and signal changes to connected clients.
#[test]
fn preview_rebuilds_and_signals_browser_after_edit_and_delete() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    fs::create_dir_all(root.join("docs")).unwrap();
    fs::create_dir_all(root.join("public")).unwrap();
    fs::write(root.join("public/__mdcontent_revision"), "asset contents").unwrap();
    fs::write(root.join("docs/index.md"), "# Home\nFirst text").unwrap();
    fs::write(root.join("docs/extra.md"), "# Extra").unwrap();
    fs::write(root.join("mdcontent.toml"), "public_dir='public'\n").unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap().to_string();
    drop(listener);
    let _server = Server(
        Command::new(env!("CARGO_BIN_EXE_mdcontent"))
            .arg("serve")
            .arg("--config")
            .arg(root.join("mdcontent.toml"))
            .arg("--address")
            .arg(&address)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap(),
    );
    wait_for(|| {
        get(&address, "/")
            .is_some_and(|r| r.contains("First text") && r.contains("X-Mdcontent-Preview"))
    });
    assert!(
        get(&address, "/__mdcontent_revision")
            .unwrap()
            .contains("asset contents")
    );
    let html = get(&address, "/").unwrap();
    verify_reload_script(&html);
    thread::sleep(Duration::from_secs(1));
    let modified = filetime::FileTime::from_last_modification_time(
        &fs::metadata(root.join("docs/index.md")).unwrap(),
    );
    fs::write(root.join("docs/index.md"), "# Home\nOther text").unwrap();
    filetime::set_file_mtime(root.join("docs/index.md"), modified).unwrap();
    wait_for(|| get(&address, "/").is_some_and(|r| r.contains("Other text")));
    assert!(get_revision(&address, "/").is_some_and(|r| r.contains("\"revision\":1")));
    fs::remove_file(root.join("docs/extra.md")).unwrap();
    wait_for(|| get(&address, "/extra/").is_some_and(|r| r.starts_with("HTTP/1.1 404")));
    assert!(!root.join("site/extra/index.html").exists());
    assert!(
        get(&address, "/extra/")
            .unwrap()
            .contains("X-Mdcontent-Preview")
    );
    fs::write(root.join("docs/extra.md"), "# Restored").unwrap();
    wait_for(|| get(&address, "/extra/").is_some_and(|r| r.contains("Restored")));
    let built = fs::read_to_string(root.join("site/index.html")).unwrap();
    assert!(!built.contains("X-Mdcontent-Preview"));
    let old_page = fs::read(root.join("site/index.html")).unwrap();
    fs::write(root.join("site/index.html"), "user change").unwrap();
    fs::write(root.join("docs/index.md"), "# Home\nRetry works").unwrap();
    thread::sleep(Duration::from_secs(2));
    assert!(get(&address, "/").unwrap().contains("user change"));
    fs::write(root.join("site/index.html"), old_page).unwrap();
    wait_for(|| get(&address, "/").is_some_and(|r| r.contains("Retry works")));
    fs::write(
        root.join("mdcontent.toml"),
        "public_dir='public'\nbase_path='/manual/'\n",
    )
    .unwrap();
    wait_for(|| get_revision(&address, "/").is_some_and(|r| r.contains("\"base\":\"/manual/\"")));
    assert!(get(&address, "/manual/").unwrap().contains("Retry works"));
}

/// Resolve project paths relative to the supplied configuration path.
#[cfg(unix)]
#[test]
fn symlinked_config_keeps_project_relative_paths() {
    use std::os::unix::fs::symlink;
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    fs::create_dir_all(root.join("project/docs")).unwrap();
    fs::create_dir_all(root.join("real")).unwrap();
    fs::write(root.join("project/docs/index.md"), "# From project").unwrap();
    fs::write(root.join("real/config.toml"), "").unwrap();
    symlink(
        root.join("real/config.toml"),
        root.join("project/mdcontent.toml"),
    )
    .unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap().to_string();
    drop(listener);
    let _server = Server(
        Command::new(env!("CARGO_BIN_EXE_mdcontent"))
            .arg("serve")
            .arg("--config")
            .arg(root.join("project/mdcontent.toml"))
            .arg("--address")
            .arg(&address)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap(),
    );
    wait_for(|| get(&address, "/").is_some_and(|r| r.contains("From project")));
    fs::write(root.join("project/docs/index.md"), "# Updated project").unwrap();
    wait_for(|| get(&address, "/").is_some_and(|r| r.contains("Updated project")));
}
