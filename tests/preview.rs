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
#[test]
fn preview_rebuilds_and_signals_browser_after_edit_and_delete() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    fs::create_dir_all(root.join("docs")).unwrap();
    fs::write(root.join("docs/index.md"), "# Home\nFirst text").unwrap();
    fs::write(root.join("docs/extra.md"), "# Extra").unwrap();
    fs::write(root.join("mdcontent.toml"), "").unwrap();
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
            .is_some_and(|r| r.contains("First text") && r.contains("__mdcontent_revision"))
    });
    thread::sleep(Duration::from_secs(1));
    fs::write(root.join("docs/index.md"), "# Home\nUpdated text").unwrap();
    wait_for(|| get(&address, "/").is_some_and(|r| r.contains("Updated text")));
    assert!(
        get(&address, "/__mdcontent_revision")
            .unwrap()
            .ends_with('1')
    );
    fs::remove_file(root.join("docs/extra.md")).unwrap();
    wait_for(|| get(&address, "/extra/").is_some_and(|r| r.starts_with("HTTP/1.1 404")));
    assert!(!root.join("site/extra/index.html").exists());
    let built = fs::read_to_string(root.join("site/index.html")).unwrap();
    assert!(!built.contains("__mdcontent_revision"));
}
