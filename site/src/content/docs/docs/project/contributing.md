---
title: Contributing
description: Build Fluxwire from source, run the checks, and follow the repository conventions.
---

Fluxwire is MIT licensed and developed at [github.com/0funct0ry/fluxwire](https://github.com/0funct0ry/fluxwire).

## Build and test

```sh
make build
make run
make test
make lint
```

The Makefile wraps cargo and enables the `runtime-shaders` feature automatically on macOS without Xcode's Metal
toolchain. Linux needs the GPUI system libraries listed in `.github/workflows/ci.yml`. The toolchain is stable Rust,
edition 2024, with a minimum supported version of 1.89.

```sh
cargo build -p fluxwire --no-default-features   # headless build; must contain no GPUI
```

## Repository rules

- Only `fluxwire-ui` and `fluxwire` may depend on GPUI. Every other crate must compile without it.
- Protocol crates depend on `fluxwire-protocol`, never on each other. Nothing outside a protocol crate uses HTTP types.
- `.flux.md` files are written only through the span editor and canonical formatter.
- Use the project vocabulary: Workspace, Collection, Folder, Request, Protocol, Block, Role, Feature, Run.
- All UI colours come from theme tokens.
- Decisions that are not covered by the specification go in `docs/adr/NNN-title.md`.
- Each milestone adds a `CHANGELOG.md` entry, documentation under `site/`, and a demo script in `docs/demos/`.

## Working on this site

```sh
cd site
npm install
npm run dev        # regenerates the reference pages, then serves the site
npm run build
```

The `fw` API, command-line, exit-code, role, keybinding, and front matter pages are generated from source by
`npm run gen`. Commit the regenerated files; CI fails when they are stale.

## Reporting issues

Open an issue at [github.com/0funct0ry/fluxwire/issues](https://github.com/0funct0ry/fluxwire/issues). Include the
Fluxwire version (`fluxwire --version`) and, for a format problem, the smallest `.flux.md` file that reproduces it.
