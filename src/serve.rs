use crate::watch::State;
use anyhow::{Context, Result, bail};
use std::{
    fs,
    path::{Component, Path},
    sync::{Arc, RwLock},
};

/// Serve a local preview and inject reload polling only into preview responses.
pub(crate) fn serve(config_path: &Path, address: &str) -> Result<()> {
    let initial = crate::config::load(config_path)?;
    let baseline = crate::watch::snapshot(config_path, &initial);
    let built = crate::build::build(config_path)?;
    let base = built.base;
    let server =
        tiny_http::Server::http(address).map_err(|e| anyhow::anyhow!("starting server: {e}"))?;
    let session = format!(
        "{}-{:?}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_nanos()
    );
    let state = Arc::new(RwLock::new(State {
        root: built.root,
        base: base.clone(),
        revision: 0,
    }));
    crate::watch::spawn(
        config_path.to_path_buf(),
        Arc::clone(&state),
        initial,
        baseline,
    );
    println!("Serving http://{address}{base} (watching for changes)");
    for request in server.incoming_requests() {
        let path = request.url().split('?').next().unwrap_or("");
        let state = state
            .read()
            .map_err(|_| anyhow::anyhow!("preview state unavailable"))?;
        let response = (|| -> Result<(Vec<u8>, &'static str)> {
            if request
                .headers()
                .iter()
                .any(|h| h.field.equiv("X-Mdcontent-Preview") && h.value.as_str() == "revision")
            {
                return Ok((
                    serde_json::to_vec(
                        &serde_json::json!({"revision": state.revision, "base": state.base, "session": session}),
                    )?,
                    "application/json; charset=utf-8",
                ));
            }
            let path = path
                .strip_prefix(&state.base)
                .context("outside base path")?;
            let relative = Path::new(path);
            if relative
                .components()
                .any(|c| !matches!(c, Component::Normal(_)))
            {
                bail!("invalid path");
            }
            let mut file = state.root.join(relative);
            if file.is_dir() {
                file.push("index.html");
            }
            let file = fs::canonicalize(file)?;
            if !file.starts_with(&state.root) {
                bail!("outside output directory");
            }
            let mime = match file.extension().and_then(|s| s.to_str()) {
                Some("html") => "text/html; charset=utf-8",
                Some("css") => "text/css; charset=utf-8",
                Some("js") => "text/javascript; charset=utf-8",
                Some("json") => "application/json; charset=utf-8",
                Some("svg") => "image/svg+xml",
                Some("png") => "image/png",
                Some("ico") => "image/x-icon",
                Some("jpg" | "jpeg") => "image/jpeg",
                Some("webp") => "image/webp",
                _ => "application/octet-stream",
            };
            let mut body = fs::read(file)?;
            if mime.starts_with("text/html") {
                inject_reload(&mut body, &state, &session)?;
            }
            Ok((body, mime))
        })();
        let response = match response {
            Ok((body, mime)) => tiny_http::Response::from_data(body)
                .with_header(tiny_http::Header::from_bytes("Content-Type", mime).unwrap())
                .with_header(tiny_http::Header::from_bytes("Cache-Control", "no-store").unwrap()),
            Err(_) => {
                let mut body =
                    b"<!doctype html><title>Not found</title><body>Not found</body>".to_vec();
                inject_reload(&mut body, &state, &session)?;
                tiny_http::Response::from_data(body)
                    .with_status_code(404)
                    .with_header(
                        tiny_http::Header::from_bytes("Content-Type", "text/html; charset=utf-8")
                            .unwrap(),
                    )
                    .with_header(
                        tiny_http::Header::from_bytes("Cache-Control", "no-store").unwrap(),
                    )
            }
        };
        drop(state);
        let _ = request.respond(response);
    }
    Ok(())
}

fn inject_reload(body: &mut Vec<u8>, state: &State, session: &str) -> Result<()> {
    let base = serde_json::to_string(&state.base)?.replace('<', "\\u003c");
    let session = serde_json::to_string(session)?.replace('<', "\\u003c");
    let script = format!(
        "<script>(()=>{{let v={},base={},session={};setInterval(async()=>{{try{{let r=await fetch(location.pathname,{{cache:'no-store',headers:{{'X-Mdcontent-Preview':'revision'}}}});if(!r.ok)return;let x=await r.json();if(x.revision!==v||x.session!==session){{if(x.base===base)location.reload();else{{let p=location.pathname.startsWith(base)?location.pathname.slice(base.length):'';location.replace(x.base+p+location.search+location.hash)}}}}}}catch(e){{}}}},1000)}})()</script>",
        state.revision, base, session
    );
    if let Some(pos) = body.windows(7).rposition(|part| part == b"</body>") {
        body.splice(pos..pos, script.bytes());
    } else {
        body.extend_from_slice(script.as_bytes());
    }
    Ok(())
}
