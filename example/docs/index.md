# Getting started

mdcontent turns Markdown files into a static documentation site. Configure your project, add pages, and deploy the generated files to a static host.

## Create a project

Run `mdcontent init` in an empty directory. This creates a configuration file and your first page in `docs/index.md`.

```sh
mdcontent init
mdcontent serve
```

Open the local address printed in the terminal. The preview rebuilds and reloads when your content changes.

## Add pages

Add Markdown files to the `docs` directory. Each file becomes a page and appears in the navigation. The first level-one heading is used as its title.

## Configure the site

Edit `mdcontent.toml` to set your site name, accent color, theme and footer. You can also supply a custom CSS file.

## Build for deployment

Run `mdcontent build` and upload the contents of `site` to your static host. Set `base_path` if the site is hosted under a subdirectory.
