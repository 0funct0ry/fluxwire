---
title: Install
description: Install the Fluxwire app and the headless command line on macOS, Linux, and Windows.
---

Fluxwire ships as a desktop app (`fluxwire`) and a static, GUI-free command-line build (`fluxwire-cli`). Both are
built from the same engine and accept the same commands.

## macOS

```sh
brew install --cask 0funct0ry/tap/fluxwire
```

Alternatively, download the signed `.dmg` for Apple silicon or Intel from the
[releases page](https://github.com/0funct0ry/fluxwire/releases/latest).

## Linux

```sh
curl -fsSL https://0funct0ry.github.io/fluxwire/install.sh | sh
```

The script installs to `~/.local` and verifies the SHA-256 checksum of the archive before unpacking it. Archives for
x86_64 and arm64 are also on the releases page. Linux needs a Vulkan-capable GPU driver and a Wayland or X11 session
for the app; the command line has no such requirement.

## Windows

Download `fluxwire-x86_64.msi` from the releases page and run it. The installer adds `fluxwire` to your `PATH` and
registers `.flux.md` files with the app.

## Command line only (CI and containers)

```sh
curl -fsSL https://0funct0ry.github.io/fluxwire/install.sh | sh -s -- --cli-only
```

Add `--version 1.0.0` to pin a release. See [Running in CI](../../guides/running-in-ci/) for a complete pipeline.

## Verify the install

```sh
fluxwire --version
```

## Updates

Fluxwire never checks for updates on its own. Turn on **Settings → Updates → Check for updates** or press
**Check now** to query GitHub releases; nothing else is contacted. There is no account, cloud service, or telemetry.
