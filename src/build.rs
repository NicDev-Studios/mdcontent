use anyhow::{Context, Result, bail};
use pulldown_cmark::{Event, Options, Parser as Markdown, Tag, html};
use serde::Serialize;
use std::{
    fs,
    path::{Component, Path},
};
use walkdir::WalkDir;

#[derive(Serialize)]
struct Page {
    title: String,
    url: String,
    text: String,
    html: String,
    source: String,
}
fn escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}
pub(crate) struct BuiltSite {
    pub root: std::path::PathBuf,
    pub base: String,
}
/// Render and publish a site while holding the output-specific build lock.
pub(crate) fn build(config_path: &Path) -> Result<BuiltSite> {
    let c = crate::config::load(config_path)?;
    let content = fs::canonicalize(&c.content_dir).context("content directory does not exist")?;
    fs::create_dir_all(&c.output_dir)?;
    let output = fs::canonicalize(&c.output_dir)?;
    if output == content || output.starts_with(&content) || content.starts_with(&output) {
        bail!("content and output directories must not overlap");
    }
    if let Some(public) = &c.public_dir {
        let public = fs::canonicalize(public).context("public directory does not exist")?;
        if output == public || output.starts_with(&public) || public.starts_with(&output) {
            bail!("public and output directories must not overlap");
        }
    }
    reject_output_symlinks(&output)?;
    let _lock = crate::output::lock(&output)?;
    crate::output::recover(&output)?;
    let staging = tempfile::tempdir_in(&output)?;
    render(&c, &content, staging.path())?;
    crate::output::publish(staging.path(), &output)?;
    println!("Built site in {}", output.display());
    Ok(BuiltSite {
        root: output,
        base: c.base_path,
    })
}

/// Generate a complete site in an isolated staging directory.
fn render(c: &crate::config::Config, content: &Path, output: &Path) -> Result<()> {
    let pages = load_pages(content, c)?;
    fs::write(
        output.join("style.css"),
        include_str!("../assets/style.css"),
    )?;
    fs::write(
        output.join("search.js"),
        include_str!("../assets/search.js"),
    )?;
    if let Some(css) = &c.custom_css {
        fs::copy(css, output.join("custom.css")).context("copying custom CSS")?;
    }
    if c.search {
        fs::write(
            output.join("search.json"),
            serde_json::to_string(
                &pages
                    .iter()
                    .map(|p| serde_json::json!({"title":p.title,"url":p.url,"text":p.text}))
                    .collect::<Vec<_>>(),
            )?,
        )?;
    }
    let logo_url = copy_icon(c.logo.as_deref(), output, "logo", &c.base_path)?;
    let favicon_url = match c.favicon.as_deref() {
        Some(path) => copy_icon(Some(path), output, "favicon", &c.base_path)?,
        None => logo_url.clone(),
    };
    let logo_markup = match logo_url {
        Some(url) => format!(
            "<img class=\"brand-logo\" src=\"{}\" alt=\"\">",
            escape(&url)
        ),
        None => format!("<span class=\"mark\">{}</span>", escape(&c.logo_text)),
    };
    let favicon_markup = favicon_url
        .map(|url| format!("<link rel=\"icon\" href=\"{}\">", escape(&url)))
        .unwrap_or_default();
    if let Some(public) = &c.public_dir {
        copy_public(public, output)?;
    }
    let navigation: Vec<&Page> = if c.navigation.is_empty() {
        pages.iter().collect()
    } else {
        let mut selected = Vec::new();
        for name in &c.navigation {
            let page = pages
                .iter()
                .find(|p| p.source == *name)
                .with_context(|| format!("navigation page not found: {name}"))?;
            if selected.iter().any(|p: &&Page| p.source == page.source) {
                bail!("duplicate navigation page: {name}");
            }
            selected.push(page);
        }
        selected
    };
    let header_links = c
        .header_links
        .iter()
        .map(|link| {
            format!(
                "<a href=\"{}\">{}</a>",
                escape(&link.url),
                escape(&link.label)
            )
        })
        .collect::<String>();
    let search_markup = if c.search {
        include_str!("../assets/search.html")
            .replace("{{search_placeholder}}", &escape(&c.search_placeholder))
    } else {
        String::new()
    };
    for page in &pages {
        let nav = navigation
            .iter()
            .map(|p| {
                format!(
                    "<a{} href=\"{}\">{}</a>",
                    if p.url == page.url {
                        " aria-current=\"page\""
                    } else {
                        ""
                    },
                    escape(&p.url),
                    escape(&p.title)
                )
            })
            .collect::<String>();
        let custom = if c.custom_css.is_some() {
            format!(
                "<link rel=\"stylesheet\" href=\"{}custom.css\">",
                escape(&c.base_path)
            )
        } else {
            String::new()
        };
        let doc = include_str!("../assets/page.html")
            .replace("{{title}}", &escape(&page.title))
            .replace("{{language}}", &escape(&c.language))
            .replace("{{navigation_label}}", &escape(&c.navigation_label))
            .replace("{{header_links}}", &header_links)
            .replace("{{search_markup}}", &search_markup)
            .replace("{{site_title}}", &escape(&c.title))
            .replace("{{description}}", &escape(&c.description))
            .replace("{{base}}", &escape(&c.base_path))
            .replace("{{accent}}", &c.accent)
            .replace("{{theme}}", c.theme.as_str())
            .replace(
                "{{switcher_enabled}}",
                if c.theme_switcher { "true" } else { "false" },
            )
            .replace(
                "{{switcher}}",
                if c.theme_switcher {
                    include_str!("../assets/theme-switcher.html")
                } else {
                    ""
                },
            )
            .replace("{{logo_markup}}", &logo_markup)
            .replace("{{favicon}}", &favicon_markup)
            .replace("{{footer}}", &escape(&c.footer))
            .replace("{{custom}}", &custom)
            .replace("{{nav}}", &nav)
            .replace("{{content}}", &page.html);
        let relative = page
            .url
            .strip_prefix(&c.base_path)
            .unwrap_or("")
            .trim_matches('/');
        let dir = output.join(relative);
        fs::create_dir_all(&dir)?;
        fs::write(dir.join("index.html"), doc)?;
    }
    Ok(())
}
/// Reject links that could redirect publication outside the output directory.
fn reject_output_symlinks(output: &Path) -> Result<()> {
    for entry in WalkDir::new(output).follow_links(false) {
        let entry = entry?;
        if entry.file_type().is_symlink() {
            bail!(
                "output directory contains a symlink: {}",
                entry.path().display()
            );
        }
    }
    Ok(())
}

/// Read Markdown pages and render their safe HTML and search text.
fn load_pages(content: &Path, c: &crate::config::Config) -> Result<Vec<Page>> {
    let mut files: Vec<_> = WalkDir::new(content)
        .follow_links(false)
        .into_iter()
        .collect::<std::result::Result<Vec<_>, _>>()?
        .into_iter()
        .filter(|e| e.file_type().is_file() && e.path().extension().is_some_and(|x| x == "md"))
        .map(|e| e.into_path())
        .collect();
    files.sort();
    if files.is_empty() {
        bail!("no Markdown files found in {}", content.display());
    }
    let mut pages = Vec::new();
    for file in files {
        let rel = file.strip_prefix(content)?;
        let mut parts: Vec<String> = rel
            .components()
            .map(|x| x.as_os_str().to_string_lossy().into_owned())
            .collect();
        let name = parts.pop().unwrap();
        let stem = name.strip_suffix(".md").unwrap();
        if stem != "index" {
            parts.push(stem.to_string());
        }
        if parts.iter().any(|p| {
            p.is_empty()
                || !p
                    .chars()
                    .all(|x| x.is_ascii_alphanumeric() || x == '-' || x == '_')
        }) {
            bail!(
                "invalid page path {}: use ASCII letters, digits, - and _",
                rel.display()
            );
        }
        let url = format!("{}{}/", c.base_path, parts.join("/")).replace("//", "/");
        if pages.iter().any(|p: &Page| p.url == url) {
            bail!("duplicate page URL {url}");
        }
        let source = fs::read_to_string(&file)?;
        let title = source
            .lines()
            .find_map(|l| l.strip_prefix("# "))
            .unwrap_or(if stem == "index" { &c.title } else { stem })
            .trim()
            .to_string();
        let mut text = String::new();
        let mut events = Vec::new();
        for event in Markdown::new_ext(
            &source,
            Options::ENABLE_TABLES
                | Options::ENABLE_STRIKETHROUGH
                | Options::ENABLE_TASKLISTS
                | Options::ENABLE_FOOTNOTES,
        ) {
            match event {
                Event::Html(_) | Event::InlineHtml(_) => (),
                Event::Text(ref t) | Event::Code(ref t) => {
                    text.push_str(t);
                    text.push(' ');
                    events.push(event);
                }
                Event::Start(Tag::Link {
                    link_type,
                    dest_url,
                    title,
                    id,
                }) => {
                    let dest = dest_url.to_string();
                    let safe = !dest
                        .trim_start()
                        .to_ascii_lowercase()
                        .starts_with("javascript:")
                        && !dest.trim_start().to_ascii_lowercase().starts_with("data:");
                    let dest = if safe {
                        rewrite_link(&dest, rel, &c.base_path)
                    } else {
                        "#".into()
                    };
                    events.push(Event::Start(Tag::Link {
                        link_type,
                        dest_url: dest.into(),
                        title,
                        id,
                    }));
                }
                _ => events.push(event),
            }
        }
        let mut rendered = String::new();
        html::push_html(&mut rendered, events.into_iter());
        pages.push(Page {
            title,
            url,
            text,
            html: rendered,
            source: rel.to_string_lossy().replace('\\', "/"),
        });
    }
    Ok(pages)
}

/// Copy configured static files while rejecting links and reserved names.
fn copy_public(public: &Path, output: &Path) -> Result<()> {
    let public = fs::canonicalize(public).context("public directory does not exist")?;
    if output == public || output.starts_with(&public) || public.starts_with(output) {
        bail!("public and output directories must not overlap");
    }
    for entry in WalkDir::new(&public).follow_links(false) {
        let entry = entry?;
        if entry.file_type().is_symlink() {
            bail!(
                "public directory must not contain symlinks: {}",
                entry.path().display()
            );
        }
        if !entry.file_type().is_file() {
            continue;
        }
        let relative = entry.path().strip_prefix(&public)?;
        let destination = output.join(relative);
        let first = relative
            .components()
            .next()
            .and_then(|c| c.as_os_str().to_str())
            .unwrap_or("");
        if matches!(
            first,
            "style.css" | "search.js" | "search.json" | "custom.css" | "index.html"
        ) || first.starts_with("logo.")
            || first.starts_with("favicon.")
            || relative.extension().is_some_and(|e| e == "html")
        {
            bail!(
                "public asset uses a reserved output path: {}",
                relative.display()
            );
        }
        if destination.is_dir() {
            bail!(
                "public asset conflicts with output directory: {}",
                relative.display()
            );
        }
        if let Some(parent) = destination.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::copy(entry.path(), destination)?;
    }
    Ok(())
}
/// Copy a configured icon using a predictable output filename.
fn copy_icon(path: Option<&Path>, output: &Path, name: &str, base: &str) -> Result<Option<String>> {
    let Some(path) = path else { return Ok(None) };
    let extension = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    if !matches!(
        extension.as_str(),
        "svg" | "png" | "ico" | "jpg" | "jpeg" | "webp"
    ) {
        bail!("{name} must be an SVG, PNG, ICO, JPEG, or WebP file");
    }
    let filename = format!("{name}.{extension}");
    fs::copy(path, output.join(&filename))
        .with_context(|| format!("copying {name} from {}", path.display()))?;
    Ok(Some(format!("{base}{filename}")))
}
/// Resolve relative Markdown links against their source page.
fn rewrite_link(dest: &str, source: &Path, base: &str) -> String {
    let split = dest.find(['#', '?']).unwrap_or(dest.len());
    let (path, suffix) = dest.split_at(split);
    let Some(stem) = path.strip_suffix(".md") else {
        return dest.to_string();
    };
    if stem.contains(':') || stem.starts_with('/') {
        return dest.to_string();
    }
    let mut parts: Vec<String> = source
        .parent()
        .unwrap_or(Path::new(""))
        .components()
        .filter_map(|p| match p {
            Component::Normal(s) => Some(s.to_string_lossy().into_owned()),
            _ => None,
        })
        .collect();
    for part in stem.split('/') {
        match part {
            "" | "." => (),
            ".." => {
                if parts.pop().is_none() {
                    return "#".into();
                }
            }
            value => parts.push(value.to_string()),
        }
    }
    if parts.last().is_some_and(|p| p == "index") {
        parts.pop();
    }
    format!("{}{}/{}", base, parts.join("/"), suffix).replace("//", "/")
}
pub(crate) fn init() -> Result<()> {
    for p in ["mdcontent.toml", "docs/index.md"] {
        if Path::new(p).exists() {
            bail!("{p} already exists; refusing to overwrite");
        }
    }
    fs::create_dir_all("docs")?;
    fs::write("mdcontent.toml", include_str!("../example/mdcontent.toml"))?;
    fs::write("docs/index.md", include_str!("../example/docs/index.md"))?;
    println!("Created mdcontent.toml and docs/index.md. Run mdcontent serve.");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn markdown_links_follow_source_file_location() {
        assert_eq!(
            rewrite_link("guide.md#intro", Path::new("index.md"), "/docs/"),
            "/docs/guide/#intro"
        );
        assert_eq!(
            rewrite_link("../index.md", Path::new("guide/setup.md"), "/"),
            "/"
        );
        assert_eq!(
            rewrite_link("../../secret.md", Path::new("guide/setup.md"), "/"),
            "#"
        );
        assert_eq!(
            rewrite_link("https://example.com/a.md", Path::new("index.md"), "/"),
            "https://example.com/a.md"
        );
    }
}
