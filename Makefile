# GPUI compiles Metal shaders at build time, which needs the Xcode Metal toolchain. Without it
# (Command Line Tools only), fall back to GPUI's runtime shader compilation.
comma := ,
FEATURES :=
ifeq ($(shell uname -s),Darwin)
ifeq ($(shell xcrun --find metal >/dev/null 2>&1 && echo yes),)
FEATURES := --features fluxwire/runtime-shaders
endif
endif

.PHONY: build run test lint fmt smoke smoke-ui cli cli-static

build:
	cargo build --workspace $(FEATURES)

run:
	cargo run -p fluxwire $(FEATURES)

test:
	cargo test --workspace $(FEATURES)

lint:
	cargo fmt --all --check
	cargo clippy --workspace --all-targets $(FEATURES) -- -D warnings

fmt:
	cargo fmt --all

# Headless MCP server smoke: rmcp client over stdio and the unix socket (no display needed).
smoke:
	cargo test -p fluxwire-agent --test transports --test scenarios $(FEATURES)

# Drives the dev-mcp app with the shipped scenarios. Needs a display (xvfb on Linux).
smoke-ui:
	SMOKE_FEATURES="dev-mcp$(if $(FEATURES),$(comma)runtime-shaders)" scripts/smoke-ui.sh $(SMOKE_ARGS) \
		fixtures/scenarios/m02-collections.yaml fixtures/scenarios/m04-environments.yaml \
		fixtures/scenarios/m07-scripts-trust.yaml fixtures/scenarios/m08-websocket.yaml

# Headless CLI (no GPUI).
cli:
	cargo build --release -p fluxwire-cli

# Static Linux binary (needs `rustup target add x86_64-unknown-linux-musl` and musl-tools).
cli-static:
	cargo build --release -p fluxwire-cli --target x86_64-unknown-linux-musl
