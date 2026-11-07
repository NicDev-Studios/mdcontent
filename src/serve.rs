use anyhow::{Context, Result, bail};
use std::{
    fs,
    path::{Component, Path},
};

pub(crate) fn serve(config_path: &Path, address: &str) -> Result<()> {
    crate::build::build(config_path)?;
    let c = crate::config::load(config_path)?;
    let root = fs::canonicalize(&c.output_dir)?;
    let server =
        tiny_http::Server::http(address).map_err(|e| anyhow::anyhow!("starting server: {e}"))?;
    println!(
        "Serving http://{address}{} (re-run after editing)",
        c.base_path
    );
    for request in server.incoming_requests() {
        let path = request.url().split('?').next().unwrap_or("");
        let response = (|| -> Result<(Vec<u8>, &'static str)> {
            let path = path
                .strip_prefix(&c.base_path)
                .context("outside base path")?;
            let relative = Path::new(path);
            if relative
                .components()
                .any(|c| !matches!(c, Component::Normal(_)))
            {
                bail!("invalid path");
            }
            let mut file = root.join(relative);
            if file.is_dir() {
                file.push("index.html");
            }
            let file = fs::canonicalize(file)?;
            if !file.starts_with(&root) {
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
            Ok((fs::read(file)?, mime))
        })();
        let response = match response {
            Ok((body, mime)) => tiny_http::Response::from_data(body)
                .with_header(tiny_http::Header::from_bytes("Content-Type", mime).unwrap()),
            Err(_) => tiny_http::Response::from_string("Not found").with_status_code(404),
        };
        let _ = request.respond(response);
    }
    Ok(())
}
