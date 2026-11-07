# =============================================================================
# Fluxwire Makefile
# =============================================================================
#
# A thin, documented wrapper around cargo. Every target is a single, readable
# cargo invocation (or a short shell snippet) so the Makefile never hides what
# is actually being run. Nothing here is required: `cargo build` etc. still
# work. The Makefile exists to (a) remember the right flags, (b) mirror CI
# locally, and (c) paper over platform quirks (see "Platform detection").
#
# Run `make` or `make help` for the full, generated target list.
#
# How this file is organised
#   1. Settings        shell, make behaviour, defaults
#   2. Variables       everything you can override on the command line
#   3. Platform        macOS Metal / runtime-shaders detection
#   4. Helpers         small macros reused by the targets
#   5. Targets         grouped by purpose; each has a `## description`
#
# Conventions
#   * `## text` after a target name is its one-line help; `##@ Title` starts a
#     help section. `make help` is generated from these, so keep them current.
#   * Debug is the default profile. Release variants carry a `-release` suffix.
#   * Targets that need an optional tool check for it and print an install hint.
#   * Unix only (macOS, Linux). On Windows use cargo directly or WSL.
# =============================================================================


# -----------------------------------------------------------------------------
# 1. Settings
# -----------------------------------------------------------------------------

# bash for `pipefail` and `$'..'`; -e/-u make recipes fail fast on errors and typos.
SHELL       := /usr/bin/env bash
.SHELLFLAGS := -eu -o pipefail -c

# Running bare `make` shows help instead of silently starting a long build.
.DEFAULT_GOAL := help

# Delete a target's output if its recipe fails; skip make's built-in C rules.
.DELETE_ON_ERROR:
MAKEFLAGS += --no-builtin-rules


# -----------------------------------------------------------------------------
# 2. Variables (override on the command line, e.g. `make test CRATE=fluxwire-format`)
# -----------------------------------------------------------------------------

CARGO  ?= cargo

# Package that `test-crate`, `clippy-crate`, `doc-crate` and `bench-crate` operate on.
CRATE  ?=

# Free-form arguments. For `run*` targets they go to the program (after `--`);
# for `test*` targets they go to the test binary (a name filter, `--nocapture`, ...).
ARGS   ?=

# Cross-compilation target triple, e.g. aarch64-apple-darwin. Empty means the host.
TARGET ?=

# Extra cargo features appended to every workspace build (comma separated).
FEATURES_EXTRA ?=

# Set to 1 to pass --locked (fail if Cargo.lock would change; what CI should use).
LOCKED ?=

# Parallel jobs for cargo (empty = cargo's default, one per CPU).
JOBS   ?=

# Anything else you want on every cargo command line, e.g. CARGO_FLAGS=--offline.
CARGO_FLAGS ?=

# Force (1) or disable (0) GPUI runtime Metal shader compilation. Empty = auto-detect.
RUNTIME_SHADERS ?=

# Where `make dist` writes archives.
DIST_DIR ?= dist

# Binaries that `make dist`, `make size` and `make install` care about.
BINARIES := fluxwire fluxwire-cli

# Rust package that produces the main binary.
BIN_PACKAGE := fluxwire

# Workspace version, read from [workspace.package] in Cargo.toml (first `version =` line).
VERSION := $(shell sed -n 's/^version *= *"\(.*\)"/\1/p' Cargo.toml | head -n 1)

# Host triple, evaluated lazily because it spawns rustc and most targets do not need it.
HOST_TRIPLE = $(shell rustc -vV | sed -n 's/^host: //p')

# Flags shared by every cargo invocation (assembled from the variables above).
CARGO_OPTS = $(if $(LOCKED),--locked) $(if $(JOBS),-j $(JOBS)) $(if $(TARGET),--target $(TARGET)) $(CARGO_FLAGS)

# Profile selectors. Written out as variables so recipes read naturally.
RELEASE := --release

# Directory holding built binaries for a profile: target/<profile> or target/<triple>/<profile>.
out_dir = target/$(if $(TARGET),$(TARGET)/)$(1)


# -----------------------------------------------------------------------------
# 3. Platform detection
# -----------------------------------------------------------------------------
# GPUI compiles Metal shaders at build time, which needs the Xcode Metal
# toolchain. With only the Command Line Tools installed, fall back to GPUI's
# runtime shader compilation (`runtime-shaders` feature). Override with
# RUNTIME_SHADERS=1 or RUNTIME_SHADERS=0.

ifeq ($(RUNTIME_SHADERS),)
ifeq ($(shell uname -s),Darwin)
ifeq ($(shell xcrun --find metal >/dev/null 2>&1 && echo yes),)
RUNTIME_SHADERS := 1
endif
endif
endif
ifeq ($(RUNTIME_SHADERS),0)
RUNTIME_SHADERS :=
endif

comma := ,
# Make has no predefined space; build one for the substitution below.
empty :=
space := $(empty) $(empty)

# Comma-joined feature list for the workspace build: `fluxwire/runtime-shaders` plus FEATURES_EXTRA.
_ws_features := $(strip $(if $(RUNTIME_SHADERS),fluxwire/runtime-shaders) $(FEATURES_EXTRA))
_ws_features := $(subst $(space),$(comma),$(_ws_features))
# Flag for commands that use `--workspace`. Empty when no features are needed.
FEATURES := $(if $(_ws_features),--features $(_ws_features))

# `-p <crate>` commands cannot use the `fluxwire/...` path unless that crate is the package,
# so the shader feature is added per package instead (only the two GUI-facing crates have it).
# Usage: $(call pkg_features,<package>)
pkg_features = $(if $(RUNTIME_SHADERS),$(if $(filter fluxwire fluxwire-ui,$(1)),--features runtime-shaders))

# Feature list for the in-process MCP dev host (see docs/agent-testing.md).
DEV_MCP_FEATURES := dev-mcp$(if $(RUNTIME_SHADERS),$(comma)runtime-shaders)


# -----------------------------------------------------------------------------
# 4. Helpers
# -----------------------------------------------------------------------------

# Fail early with a clear message when a required variable is empty.
# Usage: $(call require_var,CRATE)
require_var = @if [ -z "$($(1))" ]; then echo "error: $(1) is required, e.g. make $@ $(1)=<value>" >&2; exit 2; fi

# Fail early with an install hint when a cargo subcommand / tool is missing.
# Usage: $(call require_tool,<binary>,<install command>)
require_tool = @command -v $(1) >/dev/null 2>&1 || { echo "error: '$(1)' not found. Install it with: $(2)" >&2; exit 127; }


# =============================================================================
# 5. Targets
# =============================================================================

##@ Help

.PHONY: help
help: ## Show this help (targets, variables, examples)
	@printf '%s\n' "$$HELP_HEADER"
	@if [ -t 1 ]; then b=$$'\033[1m'; c=$$'\033[36m'; r=$$'\033[0m'; else b=; c=; r=; fi; \
	awk -v b="$$b" -v c="$$c" -v r="$$r" 'BEGIN { FS = ":.*## " } \
		/^##@ / { printf "\n%s%s%s\n", b, substr($$0, 5), r; next } \
		/^[a-zA-Z0-9_.-]+:.*## / { printf "  %s%-26s%s %s\n", c, $$1, r, $$2 }' $(MAKEFILE_LIST)
	@printf '%s\n' "$$HELP_FOOTER"

define HELP_HEADER
Fluxwire build tasks.  Usage: make <target> [VARIABLE=value ...]

Profiles: targets build the debug profile unless named *-release.
Debug output lives in target/debug, release output in target/release
(or target/<triple>/<profile> when TARGET is set).
endef
export HELP_HEADER

define HELP_FOOTER

Variables (override as `make <target> VAR=value`)
  CRATE=<name>             Package for test-crate, clippy-crate, doc-crate, bench-crate.
  ARGS="..."               Program args for run* targets; test filter / flags for test* targets.
  TARGET=<triple>          Cross-compile, e.g. x86_64-unknown-linux-musl.
  FEATURES_EXTRA=a,b       Extra cargo features added to workspace builds.
  RUNTIME_SHADERS=1|0      Force/disable GPUI runtime Metal shaders (default: auto-detect on macOS).
  LOCKED=1                 Pass --locked to cargo (reproducible; recommended in CI).
  JOBS=<n>                 Limit cargo parallelism.
  CARGO_FLAGS="..."        Any other flags for every cargo call (e.g. --offline).
  DIST_DIR=<dir>           Output directory for `make dist` (default: dist).

Examples
  make build                       Debug build of the whole workspace.
  make run                         Start the desktop app (debug).
  make run-cli ARGS="run ./fixtures/payments-api"       Run the headless CLI on a workspace.
  make test-crate CRATE=fluxwire-format ARGS=span       Tests of one crate, filtered by name.
  make ci                          Everything CI runs, locally.
  make clean-intermediates         Free disk space but keep the built binaries.
  make dist TARGET=aarch64-apple-darwin   Package a release archive for another target.

Tip: `make -n <target>` prints the commands without running them.
endef
export HELP_FOOTER


##@ Build

.PHONY: build build-release build-headless build-headless-release build-dev-mcp cli cli-static check check-release

build: ## Build the whole workspace (debug)
	$(CARGO) build --workspace $(FEATURES) $(CARGO_OPTS)

build-release: ## Build the whole workspace (release: thin LTO, stripped)
	$(CARGO) build --workspace $(RELEASE) $(FEATURES) $(CARGO_OPTS)

build-headless: ## Build the binary without the GUI (no GPUI), debug
	$(CARGO) build -p $(BIN_PACKAGE) --no-default-features $(CARGO_OPTS)

build-headless-release: ## Build the binary without the GUI (no GPUI), release
	$(CARGO) build -p $(BIN_PACKAGE) --no-default-features $(RELEASE) $(CARGO_OPTS)

build-dev-mcp: ## Build the app with the in-process MCP host for agent/UI testing (debug)
	$(CARGO) build -p $(BIN_PACKAGE) --features $(DEV_MCP_FEATURES) $(CARGO_OPTS)

cli: ## Build only the headless CLI binary (release, no GPUI)
	$(CARGO) build $(RELEASE) -p fluxwire-cli $(CARGO_OPTS)

# Needs `rustup target add x86_64-unknown-linux-musl` and musl-tools. CI calls this target.
cli-static: ## Build a static Linux CLI (musl, release); TARGET=aarch64-unknown-linux-musl for arm64
	$(CARGO) build $(RELEASE) -p fluxwire-cli --target $(or $(TARGET),x86_64-unknown-linux-musl) $(if $(LOCKED),--locked) $(CARGO_FLAGS)

check: ## Type-check everything without producing binaries (fast, debug)
	$(CARGO) check --workspace --all-targets $(FEATURES) $(CARGO_OPTS)

check-release: ## Type-check everything with the release profile
	$(CARGO) check --workspace --all-targets $(RELEASE) $(FEATURES) $(CARGO_OPTS)


##@ Run

.PHONY: run run-release run-dev-mcp run-cli

run: ## Start the desktop app (debug). ARGS are passed to the program
	$(CARGO) run -p $(BIN_PACKAGE) $(call pkg_features,fluxwire) $(CARGO_OPTS) $(if $(ARGS),-- $(ARGS))

run-release: ## Start the desktop app (release). ARGS are passed to the program
	$(CARGO) run -p $(BIN_PACKAGE) $(RELEASE) $(call pkg_features,fluxwire) $(CARGO_OPTS) $(if $(ARGS),-- $(ARGS))

run-dev-mcp: ## Start the app with the in-process MCP host (ARGS e.g. "--mcp-socket /tmp/fw.sock")
	$(CARGO) run -p $(BIN_PACKAGE) --features $(DEV_MCP_FEATURES) $(CARGO_OPTS) $(if $(ARGS),-- $(ARGS))

run-cli: ## Run the headless CLI (debug), e.g. make run-cli ARGS="run ./fixtures/payments-api"
	$(CARGO) run -p fluxwire-cli $(CARGO_OPTS) $(if $(ARGS),-- $(ARGS))


##@ Test

.PHONY: test test-release test-crate test-ui test-headless nextest smoke smoke-ui bench bench-crate

test: ## Run every test in the workspace (debug)
	$(CARGO) test --workspace $(FEATURES) $(CARGO_OPTS) $(if $(ARGS),-- $(ARGS))

test-release: ## Run every test with optimizations (catches release-only bugs, slower to compile)
	$(CARGO) test --workspace $(RELEASE) $(FEATURES) $(CARGO_OPTS) $(if $(ARGS),-- $(ARGS))

test-crate: ## Test one package: CRATE=<name> [ARGS=<filter>]
	$(call require_var,CRATE)
	$(CARGO) test -p $(CRATE) $(call pkg_features,$(CRATE)) $(CARGO_OPTS) $(if $(ARGS),-- $(ARGS))

test-ui: ## Test only the GPUI crate (fluxwire-ui) [ARGS=<filter>]
	$(CARGO) test -p fluxwire-ui $(call pkg_features,fluxwire-ui) $(CARGO_OPTS) $(if $(ARGS),-- $(ARGS))

test-headless: ## Test the agent transport and scenario suites without a display (same as CI's headless job)
	$(CARGO) test -p fluxwire-agent --test transports --test scenarios --test ui_tools $(CARGO_OPTS)

nextest: ## Run the workspace tests with cargo-nextest (faster, nicer output)
	$(call require_tool,cargo-nextest,cargo install cargo-nextest --locked)
	$(CARGO) nextest run --workspace $(FEATURES) $(CARGO_OPTS) $(ARGS)

# Headless MCP server smoke: rmcp client over stdio and the unix socket (no display needed).
smoke: ## Headless MCP smoke test (stdio + unix socket transports)
	$(CARGO) test -p fluxwire-agent --test transports --test scenarios $(FEATURES) $(CARGO_OPTS)

# Drives the dev-mcp app with the shipped scenarios. Needs a display (xvfb on Linux).
# Extra scenario flags go in SMOKE_ARGS.
smoke-ui: ## Drive the running app with the shipped UI scenarios (needs a display)
	SMOKE_FEATURES="$(DEV_MCP_FEATURES)" scripts/smoke-ui.sh $(SMOKE_ARGS) \
		fixtures/scenarios/m02-collections.yaml fixtures/scenarios/m04-environments.yaml \
		fixtures/scenarios/m07-scripts-trust.yaml fixtures/scenarios/m08-websocket.yaml

bench: ## Run all benchmarks (criterion; always release)
	$(CARGO) bench --workspace $(FEATURES) $(CARGO_OPTS) $(if $(ARGS),-- $(ARGS))

bench-crate: ## Benchmark one package: CRATE=<name> [ARGS=<filter>]
	$(call require_var,CRATE)
	$(CARGO) bench -p $(CRATE) $(CARGO_OPTS) $(if $(ARGS),-- $(ARGS))


##@ Code quality

.PHONY: fmt fmt-check clippy clippy-release clippy-crate lint lint-release fix doc doc-open doc-crate coverage

fmt: ## Format all code in place
	$(CARGO) fmt --all

fmt-check: ## Verify formatting without changing files (what CI runs)
	$(CARGO) fmt --all --check

clippy: ## Lint with clippy, warnings are errors (debug)
	$(CARGO) clippy --workspace --all-targets $(FEATURES) $(CARGO_OPTS) -- -D warnings

clippy-release: ## Lint with clippy using the release profile (catches cfg/optimization-dependent lints)
	$(CARGO) clippy --workspace --all-targets $(RELEASE) $(FEATURES) $(CARGO_OPTS) -- -D warnings

clippy-crate: ## Lint one package: CRATE=<name>
	$(call require_var,CRATE)
	$(CARGO) clippy -p $(CRATE) --all-targets $(call pkg_features,$(CRATE)) $(CARGO_OPTS) -- -D warnings

lint: fmt-check clippy ## Formatting check + clippy (debug); the pre-commit gate

lint-release: fmt-check clippy-release ## Formatting check + clippy (release profile)

fix: ## Apply rustfmt and clippy's machine-applicable fixes (modifies files, needs a clean-ish tree)
	$(CARGO) clippy --workspace --all-targets $(FEATURES) $(CARGO_OPTS) --fix --allow-dirty --allow-staged
	$(CARGO) fmt --all

doc: ## Build API docs for workspace crates (no dependencies)
	$(CARGO) doc --workspace --no-deps $(FEATURES) $(CARGO_OPTS)

doc-open: ## Build API docs and open them in the browser
	$(CARGO) doc --workspace --no-deps $(FEATURES) $(CARGO_OPTS) --open

doc-crate: ## Build and open docs for one package: CRATE=<name>
	$(call require_var,CRATE)
	$(CARGO) doc -p $(CRATE) --no-deps $(CARGO_OPTS) --open

coverage: ## Line coverage report via cargo-llvm-cov (HTML in target/llvm-cov/html)
	$(call require_tool,cargo-llvm-cov,cargo install cargo-llvm-cov --locked)
	$(CARGO) llvm-cov --workspace $(FEATURES) $(CARGO_OPTS) --html


##@ Dependencies and policy

.PHONY: deny audit outdated tree verify-headless update schema

deny: ## Check licences, bans and sources with cargo-deny
	$(call require_tool,cargo-deny,cargo install cargo-deny --locked)
	$(CARGO) deny check

audit: ## Check Cargo.lock against the RustSec advisory database
	$(call require_tool,cargo-audit,cargo install cargo-audit --locked)
	$(CARGO) audit

outdated: ## List dependencies with newer versions available (cargo-outdated)
	$(call require_tool,cargo-outdated,cargo install cargo-outdated --locked)
	$(CARGO) outdated --workspace --root-deps-only

tree: ## Show the dependency tree of the main binary (duplicates highlighted: ARGS=-d)
	$(CARGO) tree -p $(BIN_PACKAGE) $(ARGS)

# Architecture rule (CLAUDE.md): only fluxwire-ui and fluxwire may depend on GPUI. The headless
# build of the binary must not pull it in, or the static CLI for CI would break.
verify-headless: ## Fail if the headless build depends on GPUI (architecture rule, also checked in CI)
	@if $(CARGO) tree -p $(BIN_PACKAGE) --no-default-features -e normal $(CARGO_OPTS) | grep -qE '(^|[^a-z_-])gpui '; then \
		echo "error: the headless build of $(BIN_PACKAGE) depends on gpui" >&2; exit 1; \
	else echo "ok: headless build has no gpui dependency"; fi

update: ## Update Cargo.lock to the newest compatible versions (review the diff before committing)
	$(CARGO) update

# Regenerates docs/schema/front-matter.schema.json from the Rust types.
schema: ## Regenerate docs/schema/front-matter.schema.json
	FLUXWIRE_UPDATE_SCHEMA=1 $(CARGO) test -p fluxwire-format schema $(CARGO_OPTS)


##@ Aggregates

.PHONY: ci ci-release verify

# Mirrors .github/workflows/ci.yml: fmt, clippy, tests, release build, headless build.
ci: fmt-check clippy test build-release build-headless verify-headless test-headless ## Run what CI runs (fmt, clippy, tests, release + headless builds)

ci-release: lint-release test-release build-release ## Like ci, but lints and tests with the release profile

verify: lint test ## Quick pre-commit check: lint + tests (debug)


##@ Install and package

.PHONY: install uninstall dist

install: ## Install the release `fluxwire` binary into ~/.cargo/bin
	$(CARGO) install --path crates/$(BIN_PACKAGE) --locked $(call pkg_features,fluxwire) --force

uninstall: ## Remove the installed `fluxwire` binary
	$(CARGO) uninstall $(BIN_PACKAGE)

# Produces dist/fluxwire-<version>-<triple>.tar.gz plus a .sha256 next to it. The archive holds
# the release binaries and any of LICENSE*, README*, CHANGELOG.md that exist. Use TARGET=<triple>
# to package a cross-compiled build (the target must be installed with rustup).
dist: build-release ## Package release binaries into dist/fluxwire-<version>-<triple>.tar.gz (+ sha256)
	@set -e; \
	triple="$(if $(TARGET),$(TARGET),$(HOST_TRIPLE))"; \
	name="fluxwire-$(VERSION)-$$triple"; \
	stage="$(DIST_DIR)/$$name"; \
	rm -rf "$$stage"; mkdir -p "$$stage"; \
	for bin in $(BINARIES); do \
		cp "$(call out_dir,release)/$$bin" "$$stage/"; \
	done; \
	for doc in LICENSE LICENSE-MIT LICENSE-APACHE README.md CHANGELOG.md; do \
		[ -f "$$doc" ] && cp "$$doc" "$$stage/" || true; \
	done; \
	tar -czf "$(DIST_DIR)/$$name.tar.gz" -C "$(DIST_DIR)" "$$name"; \
	rm -rf "$$stage"; \
	( cd "$(DIST_DIR)" && shasum -a 256 "$$name.tar.gz" > "$$name.tar.gz.sha256" ); \
	ls -lh "$(DIST_DIR)/$$name.tar.gz"


##@ Release packaging (SPEC §24)

.PHONY: bundle-macos bundle-linux bundle-windows cli-archive checksums release icons install-script

# Short commit hash baked into About (read by `option_env!("FLUXWIRE_COMMIT")`).
export FLUXWIRE_COMMIT ?= $(shell git rev-parse --short HEAD 2>/dev/null)

# Signing and notarization run only when the secrets are present:
#   APPLE_SIGNING_IDENTITY ("Developer ID Application: ..."), and for notarization
#   APPLE_ID, APPLE_TEAM_ID, APPLE_APP_PASSWORD. Without them the .app and .dmg are unsigned.
bundle-macos: build-release ## Build dist/fluxwire-<version>-<triple>.dmg (signed + notarized when APPLE_* are set)
	@set -e; \
	triple="$(if $(TARGET),$(TARGET),$(HOST_TRIPLE))"; \
	app="$(DIST_DIR)/Fluxwire.app"; \
	rm -rf "$$app"; mkdir -p "$$app/Contents/MacOS" "$$app/Contents/Resources"; \
	cp "$(call out_dir,release)/fluxwire" "$$app/Contents/MacOS/fluxwire"; \
	sed "s/@VERSION@/$(VERSION)/g" packaging/macos/Info.plist.in > "$$app/Contents/Info.plist"; \
	cp packaging/icons/fluxwire.icns "$$app/Contents/Resources/fluxwire.icns"; \
	if [ -n "$${APPLE_SIGNING_IDENTITY:-}" ]; then \
		codesign --force --options runtime --timestamp --entitlements packaging/macos/entitlements.plist \
			--sign "$$APPLE_SIGNING_IDENTITY" "$$app"; \
	else echo "bundle-macos: APPLE_SIGNING_IDENTITY not set, leaving the app unsigned"; fi; \
	dmg="$(DIST_DIR)/fluxwire-$(VERSION)-$$triple.dmg"; \
	stage="$(DIST_DIR)/dmg-stage"; rm -rf "$$stage" "$$dmg"; mkdir -p "$$stage"; \
	cp -R "$$app" "$$stage/"; ln -s /Applications "$$stage/Applications"; \
	hdiutil create -volname Fluxwire -srcfolder "$$stage" -ov -format UDZO "$$dmg" >/dev/null; \
	rm -rf "$$stage"; \
	if [ -n "$${APPLE_SIGNING_IDENTITY:-}" ]; then codesign --force --timestamp --sign "$$APPLE_SIGNING_IDENTITY" "$$dmg"; fi; \
	if [ -n "$${APPLE_ID:-}" ] && [ -n "$${APPLE_TEAM_ID:-}" ] && [ -n "$${APPLE_APP_PASSWORD:-}" ]; then \
		xcrun notarytool submit "$$dmg" --apple-id "$$APPLE_ID" --team-id "$$APPLE_TEAM_ID" \
			--password "$$APPLE_APP_PASSWORD" --wait; \
		xcrun stapler staple "$$dmg"; \
	else echo "bundle-macos: notarization credentials not set, skipping"; fi; \
	ls -lh "$$dmg"

bundle-linux: build-release ## Build dist/fluxwire-<version>-<triple>.tar.gz with .desktop file, icons and MIME type
	@set -e; \
	triple="$(if $(TARGET),$(TARGET),$(HOST_TRIPLE))"; \
	name="fluxwire-$(VERSION)-$$triple"; stage="$(DIST_DIR)/$$name"; \
	rm -rf "$$stage"; mkdir -p "$$stage/bin" "$$stage/share/applications" "$$stage/share/mime/packages" \
		"$$stage/share/icons/hicolor/256x256/apps" "$$stage/share/icons/hicolor/512x512/apps"; \
	cp "$(call out_dir,release)/fluxwire" "$$stage/bin/"; \
	cp packaging/linux/fluxwire.desktop "$$stage/share/applications/"; \
	cp packaging/linux/fluxwire-mime.xml "$$stage/share/mime/packages/fluxwire.xml"; \
	cp packaging/icons/fluxwire-256.png "$$stage/share/icons/hicolor/256x256/apps/fluxwire.png"; \
	cp packaging/icons/fluxwire-512.png "$$stage/share/icons/hicolor/512x512/apps/fluxwire.png"; \
	for doc in LICENSE README.md CHANGELOG.md; do [ -f "$$doc" ] && cp "$$doc" "$$stage/" || true; done; \
	tar -czf "$(DIST_DIR)/$$name.tar.gz" -C "$(DIST_DIR)" "$$name"; rm -rf "$$stage"; \
	ls -lh "$(DIST_DIR)/$$name.tar.gz"

# Needs the WiX v4 CLI (`dotnet tool install --global wix`) and 7-Zip. Run on Windows.
bundle-windows: build-release ## Build dist/fluxwire-<version>-<triple>.msi and .zip (Windows, WiX v4)
	@set -e; \
	triple="$(if $(TARGET),$(TARGET),$(HOST_TRIPLE))"; mkdir -p "$(DIST_DIR)"; \
	wix build -d Version=$(VERSION) -d BinDir="$(call out_dir,release)" -d IconFile=packaging/icons/fluxwire.ico \
		-o "$(DIST_DIR)/fluxwire-$(VERSION)-$$triple.msi" packaging/windows/fluxwire.wxs; \
	( cd "$(call out_dir,release)" && 7z a -tzip "$(CURDIR)/$(DIST_DIR)/fluxwire-$(VERSION)-$$triple.zip" fluxwire.exe ); \
	ls -lh "$(DIST_DIR)"/fluxwire-$(VERSION)-$$triple.*

# fluxwire-cli-<version>-<triple>.tar.gz (zip on Windows). Linux builds use musl: TARGET=x86_64-unknown-linux-musl.
cli-archive: ## Archive the headless CLI: dist/fluxwire-cli-<version>-<triple>.tar.gz (TARGET=<triple> to cross-build)
	$(CARGO) build --release -p fluxwire-cli $(if $(TARGET),--target $(TARGET)) $(if $(LOCKED),--locked)
	@set -e; \
	triple="$(if $(TARGET),$(TARGET),$(HOST_TRIPLE))"; \
	name="fluxwire-cli-$(VERSION)-$$triple"; stage="$(DIST_DIR)/$$name"; \
	rm -rf "$$stage"; mkdir -p "$$stage"; \
	cp "$(call out_dir,release)/fluxwire-cli"* "$$stage/"; \
	for doc in LICENSE README.md; do [ -f "$$doc" ] && cp "$$doc" "$$stage/" || true; done; \
	case "$$triple" in \
		*windows*) ( cd "$(DIST_DIR)" && 7z a -tzip "$$name.zip" "$$name" >/dev/null ) ;; \
		*) tar -czf "$(DIST_DIR)/$$name.tar.gz" -C "$(DIST_DIR)" "$$name" ;; \
	esac; \
	rm -rf "$$stage"; ls -lh "$(DIST_DIR)"/$$name.*

checksums: ## Write dist/SHA256SUMS for every archive, dmg and msi in dist/
	@cd $(DIST_DIR) && files=$$(ls *.tar.gz *.zip *.dmg *.msi 2>/dev/null); \
	[ -n "$$files" ] || { echo "nothing to checksum in $(DIST_DIR)"; exit 1; }; \
	(command -v sha256sum >/dev/null && sha256sum $$files || shasum -a 256 $$files) > SHA256SUMS; cat SHA256SUMS

release: ## Package this host's artifacts (app bundle + CLI archive) and SHA256SUMS
	@case "$$(uname -s)" in \
		Darwin) $(MAKE) bundle-macos ;; \
		Linux) $(MAKE) bundle-linux ;; \
		*) $(MAKE) bundle-windows ;; \
	esac
	$(MAKE) cli-archive
	$(MAKE) checksums

icons: ## Regenerate packaging/icons from fluxwire.svg
	sh scripts/make-icons.sh

install-script: ## Stage scripts/install.sh for the docs site (site/public/install.sh)
	mkdir -p site/public && cp scripts/install.sh site/public/install.sh


##@ Clean

.PHONY: clean clean-debug clean-release clean-incremental clean-intermediates clean-dist clean-docs

# Remove the whole target directory, binaries included.
clean: ## Remove the entire build (target/), binaries included
	$(CARGO) clean

clean-debug: ## Remove only debug build output (target/debug)
	rm -rf target/debug target/*/debug

clean-release: ## Remove only release build output (target/release)
	rm -rf target/release target/*/release

# Incremental caches are the cheapest thing to throw away: they rebuild quickly and are
# often tens of GB for a GPUI project.
clean-incremental: ## Remove incremental-compilation caches only (cheap, frees a lot of space)
	rm -rf target/debug/incremental target/release/incremental target/*/debug/incremental target/*/release/incremental

# Keeps the final binaries in target/<profile>/ and target/<triple>/<profile>/ and deletes the
# rest. On macOS and Linux the binaries are hardlinks into deps/, so removing deps/ leaves
# them intact. The next build recompiles all dependencies from scratch.
clean-intermediates: ## Remove everything except the final binaries (deps, build scripts, caches)
	@for dir in target/debug target/release target/*/debug target/*/release; do \
		[ -d "$$dir" ] || continue; \
		rm -rf "$$dir/deps" "$$dir/incremental" "$$dir/build" "$$dir/.fingerprint" "$$dir/examples"; \
	done
	@rm -rf target/criterion target/tmp
	@du -sh target

clean-dist: ## Remove packaged archives (dist/)
	rm -rf $(DIST_DIR)

clean-docs: ## Remove generated API docs (target/doc)
	rm -rf target/doc $(foreach t,$(wildcard target/*/doc),$(t))


##@ Information

.PHONY: size doctor version

size: ## Show built binary sizes and how much disk each part of target/ uses
	@echo "Binaries:"; \
	for dir in target/debug target/release target/*/debug target/*/release; do \
		for bin in $(BINARIES); do \
			[ -f "$$dir/$$bin" ] && ls -lh "$$dir/$$bin" | awk '{printf "  %-8s %s\n", $$5, $$9}'; \
		done; \
	done; true
	@echo "Disk usage:"; du -sh target 2>/dev/null | sed 's/^/  /'; \
	du -sh target/*/ 2>/dev/null | sed 's/^/  /'; true

doctor: ## Print toolchain, platform detection and optional-tool availability
	@echo "rustc:            $$(rustc --version)"
	@echo "cargo:            $$($(CARGO) --version)"
	@echo "host triple:      $(HOST_TRIPLE)"
	@echo "workspace:        $(VERSION)"
	@echo "runtime shaders:  $(if $(RUNTIME_SHADERS),on (Metal toolchain missing or forced),off)"
	@echo "feature flags:    $(if $(FEATURES),$(FEATURES),(none))"
	@echo "optional tools:"
	@for t in cargo-nextest cargo-deny cargo-audit cargo-llvm-cov cargo-outdated cargo-watch; do \
		if command -v $$t >/dev/null 2>&1; then printf '  %-18s yes\n' $$t; else printf '  %-18s no\n' $$t; fi; \
	done

version: ## Print the workspace version
	@echo $(VERSION)
