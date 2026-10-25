# GPUI compiles Metal shaders at build time, which needs the Xcode Metal toolchain. Without it
# (Command Line Tools only), fall back to GPUI's runtime shader compilation.
FEATURES :=
ifeq ($(shell uname -s),Darwin)
ifeq ($(shell xcrun --find metal >/dev/null 2>&1 && echo yes),)
FEATURES := --features fluxwire/runtime-shaders
endif
endif

.PHONY: build run test lint fmt

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
