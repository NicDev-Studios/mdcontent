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
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
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
fn get_revision(address: &str, path: &str) -> String {
    let mut stream = TcpStream::connect(address).unwrap();
    write!(stream, "GET {path} HTTP/1.1\r\nHost: localhost\r\nX-Mdcontent-Preview: revision\r\nConnection: close\r\n\r\n").unwrap();
    let mut response = String::new();
    stream.read_to_string(&mut response).unwrap();
    response
}
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
fn verify_reload_script(html: &str) {
    let Some(start) = html.rfind("<script>(()=>") else {
        panic!("reload script missing")
    };
    let code = &html[start + 8..];
    let code = &code[..code.find("</script>").unwrap()];
    let js = format!(
        r#"let tick, moved='', data={{revision:0,base:'/'}};
let location={{pathname:'/',search:'?x=1',hash:'#h',replace:p=>moved=p}};
let setInterval=f=>tick=f; let fetch=async()=>({{ok:true,json:async()=>data}});
eval({});
(async()=>{{await tick();if(moved)process.exit(1);data={{revision:1,base:'/manual/'}};await tick();if(moved!='/manual/?x=1#h')process.exit(2)}})().catch(()=>process.exit(3));"#,
        serde_json::to_string(code).unwrap()
    );
    match Command::new("node").args(["-e", &js]).status() {
        Ok(status) => assert!(status.success(), "reload script did not respond correctly"),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            eprintln!("Node unavailable; script execution test skipped")
        }
        Err(e) => panic!("failed to run Node: {e}"),
    }
}
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
    assert!(get_revision(&address, "/").contains("\"revision\":1"));
    fs::remove_file(root.join("docs/extra.md")).unwrap();
    wait_for(|| get(&address, "/extra/").is_some_and(|r| r.starts_with("HTTP/1.1 404")));
    assert!(!root.join("site/extra/index.html").exists());
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
    wait_for(|| get_revision(&address, "/").contains("\"base\":\"/manual/\""));
    assert!(get(&address, "/manual/").unwrap().contains("Retry works"));
}

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
