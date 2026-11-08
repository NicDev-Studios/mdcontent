use anyhow::{Context, Result, bail};
use serde::Deserialize;
use std::{
    fs,
    path::{Path, PathBuf},
};

#[derive(Deserialize)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct Config {
    pub(crate) title: String,
    pub(crate) description: String,
    pub(crate) content_dir: PathBuf,
    pub(crate) output_dir: PathBuf,
    pub(crate) base_path: String,
    pub(crate) accent: String,
    pub(crate) logo_text: String,
    pub(crate) footer: String,
    pub(crate) custom_css: Option<PathBuf>,
    pub(crate) favicon: Option<PathBuf>,
    pub(crate) logo: Option<PathBuf>,
    pub(crate) theme: Theme,
    pub(crate) theme_switcher: bool,
    pub(crate) language: String,
    pub(crate) navigation_label: String,
    pub(crate) search: bool,
    pub(crate) search_placeholder: String,
    pub(crate) navigation: Vec<String>,
    pub(crate) header_links: Vec<HeaderLink>,
    pub(crate) public_dir: Option<PathBuf>,
}
impl Default for Config {
    /// Provide default site configuration.
    fn default() -> Self {
        Self {
            title: "Documentation".into(),
            description: "Everything you need to get started.".into(),
            content_dir: "docs".into(),
            output_dir: "site".into(),
            base_path: "/".into(),
            accent: "#527060".into(),
            logo_text: "M".into(),
            footer: "Built with mdcontent".into(),
            custom_css: None,
            favicon: None,
            logo: None,
            theme: Theme::System,
            theme_switcher: true,
            language: "en".into(),
            navigation_label: "Documentation".into(),
            search: true,
            search_placeholder: "Search documentation".into(),
            navigation: Vec::new(),
            header_links: Vec::new(),
            public_dir: None,
        }
    }
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct HeaderLink {
    pub(crate) label: String,
    pub(crate) url: String,
}
#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum Theme {
    Light,
    Dark,
    System,
}
impl Theme {
    /// Return the configuration value for this theme.
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Light => "light",
            Self::Dark => "dark",
            Self::System => "system",
        }
    }
}
/// Load and validate configuration relative to its file.
pub(crate) fn load(path: &Path) -> Result<Config> {
    let mut c: Config = toml::from_str(
        &fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?,
    )?;
    let root = path.parent().unwrap_or(Path::new("."));
    c.content_dir = root.join(c.content_dir);
    c.output_dir = root.join(c.output_dir);
    c.custom_css = c.custom_css.map(|p| root.join(p));
    c.favicon = c.favicon.map(|p| root.join(p));
    c.logo = c.logo.map(|p| root.join(p));
    c.public_dir = c.public_dir.map(|p| root.join(p));
    if c.language.is_empty()
        || !c
            .language
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-')
    {
        bail!("language must be a language tag such as en or de-DE");
    }
    for link in &c.header_links {
        if !(link.url.starts_with("https://")
            || link.url.starts_with("http://")
            || (link.url.starts_with('/') && !link.url.starts_with("//")))
            || link.url.chars().any(char::is_control)
        {
            bail!("header link URLs must be http(s) URLs or absolute site paths");
        }
    }
    if !c.base_path.starts_with('/')
        || c.base_path.contains(['?', '#', '\\', '"', '\'', '<', '>'])
        || c.base_path.split('/').any(|x| x == ".." || x == ".")
    {
        bail!("base_path must be an absolute URL path without traversal, query, or fragment");
    }
    c.base_path = format!("/{}/", c.base_path.trim_matches('/')).replace("//", "/");
    if !(c.accent.len() == 7
        && c.accent.starts_with('#')
        && c.accent[1..].bytes().all(|b| b.is_ascii_hexdigit()))
    {
        bail!("accent must be a hex color such as #527060");
    }
    if !c.theme_switcher && matches!(c.theme, Theme::System) {
        bail!("when theme_switcher is disabled, theme must be light or dark");
    }
    Ok(c)
}
