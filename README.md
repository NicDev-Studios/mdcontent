# mdcontent

A small Rust-powered Markdown documentation generator. The generated site is static HTML and CSS with a small, dependency-free search script; it does not require a browser framework or WebAssembly.

## Getting started

Install Rust, then run:

```sh
cargo install --path .
mkdir my-docs && cd my-docs
mdcontent init
mdcontent serve
```

Open `http://127.0.0.1:3000/`. Run `mdcontent build` to generate `site/` for any static host. The preview server builds once on startup; restart it after editing. Use `--config path/to/mdcontent.toml` to select a configuration file.

## Configuration

Edit `mdcontent.toml` to configure the site title, description, content and output directories, deployment `base_path` (for example `/docs/`), accent color, logo text and footer. Set `theme = "system"`, `"light"`, or `"dark"` to choose the default appearance. With `theme_switcher = true`, visitors can choose System, Light, or Dark and their choice is saved locally. To use a single fixed theme, set `theme_switcher = false` and choose `"light"` or `"dark"`. Set `custom_css` to a stylesheet path relative to the configuration file to override the theme. Set `logo` and optionally `favicon` to image paths relative to the configuration file; the logo is also used as the favicon when `favicon` is omitted. Without a logo image, `logo_text` is displayed. Paths are relative to the configuration file. `base_path` must match your deployment URL.

Additional options in `mdcontent.toml`:

- `language`: HTML language tag, such as `en` or `de-DE`.
- `navigation_label`: sidebar and breadcrumb label.
- `navigation`: ordered list of Markdown paths relative to the content directory. Omitted pages remain accessible; leave unset for automatic navigation.
- `search`: enable or disable the search interface and index; `search_placeholder` sets its label.
- `[[header_links]]`: repeat with `label` and an HTTP(S) URL or absolute site path.
- `public_dir`: directory whose files are copied to the output root, for example images or downloads. Use `base_path` in URLs when hosting in a subdirectory. Generated filenames and HTML are reserved; symlinks are rejected.

Markdown files in `docs/` become pages. An `index.md` maps to its directory; other files map to a directory of the same name. Filenames and directories use ASCII letters, digits, hyphens and underscores. The first level-one heading supplies the navigation title. Raw HTML in Markdown is omitted. Content assets can be supplied through `public_dir`. The preview does not watch files or remove stale output files: use a dedicated empty output directory for a fresh build. Rebuild into an empty output directory after disabling search to remove any old search index. Only publish content you intend to make public: the search index contains page text.

## Development

```sh
cargo test
cargo run -- build --config example/mdcontent.toml
cargo run -- serve --config example/mdcontent.toml
```
