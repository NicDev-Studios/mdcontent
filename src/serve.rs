use crate::watch::State;
use anyhow::{Context, Result, bail};
use std::{
    fs,
    path::{Component, Path},
    sync::{Arc, RwLock},
};

pub(crate) fn serve(config_path: &Path, address: &str) -> Result<()> {
    crate::build::build(config_path)?;
    let c = crate::config::load(config_path)?;
    let root = fs::canonicalize(&c.output_dir)?;
    let base = c.base_path;
    let server =
        tiny_http::Server::http(address).map_err(|e| anyhow::anyhow!("starting server: {e}"))?;
    let state = Arc::new(RwLock::new(State {
        root,
        base: base.clone(),
        revision: 0,
    }));
    crate::watch::spawn(fs::canonicalize(config_path)?, Arc::clone(&state));
    println!("Serving http://{address}{base} (watching for changes)");
    for request in server.incoming_requests() {
        let path = request.url().split('?').next().unwrap_or("");
        let state = state
            .read()
            .map_err(|_| anyhow::anyhow!("preview state unavailable"))?;
        let response = (|| -> Result<(Vec<u8>, &'static str)> {
            let path = path
                .strip_prefix(&state.base)
                .context("outside base path")?;
            if path == "__mdcontent_revision" {
                return Ok((
                    state.revision.to_string().into_bytes(),
                    "text/plain; charset=utf-8",
                ));
            }
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
                let endpoint =
                    serde_json::to_string(&format!("{}__mdcontent_revision", state.base))?;
                let script = format!(
                    "<script>(()=>{{let v={};setInterval(async()=>{{try{{let r=await fetch({},{{cache:'no-store'}});if(r.ok&&Number(await r.text())!==v)location.reload()}}catch(e){{}}}},1000)}})()</script>",
                    state.revision, endpoint
                );
                if let Some(pos) = body.windows(7).rposition(|part| part == b"</body>") {
                    body.splice(pos..pos, script.bytes());
                } else {
                    body.extend_from_slice(script.as_bytes());
                }
            }
            Ok((body, mime))
        })();
        let response = match response {
            Ok((body, mime)) => tiny_http::Response::from_data(body)
                .with_header(tiny_http::Header::from_bytes("Content-Type", mime).unwrap())
                .with_header(tiny_http::Header::from_bytes("Cache-Control", "no-store").unwrap()),
            Err(_) => tiny_http::Response::from_string("Not found").with_status_code(404),
        };
        let _ = request.respond(response);
    }
    Ok(())
}
