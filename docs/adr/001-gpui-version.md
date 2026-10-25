# ADR-001: GPUI version

Status: accepted (M1)

## Decision
Pin the crates.io release `gpui = "=0.2.2"` with an exact (`=`) requirement, in the workspace `Cargo.toml`. Only `fluxwire-ui` and `fluxwire` may depend on it. We do **not** use `gpui-component` (see ADR-002).

## Why
- A published crate gives reproducible, cacheable builds on the macOS, Linux, and Windows CI runners. A Zed git revision pulls in a much larger dependency tree and slower builds.
- The sibling project `microterm` ships on the same pin, so patterns (assets, actions, `TestAppContext` tests) carry over.
- GPUI's API churns (SPEC §27), so upgrades happen deliberately in their own PR.

## Consequences
- `gpui-component 0.8.0` depends on a different package, `gpui-pre =0.3.9`, so its types cannot be used with `gpui 0.2.2`. The two cannot be mixed.
- GPUI compiles Metal shaders at build time, which needs Xcode's Metal toolchain. `fluxwire-ui` exposes a `runtime-shaders` feature (`gpui/runtime_shaders`), and the Makefile enables it automatically when `xcrun --find metal` fails, so builds also work with only the Command Line Tools.
- Linux CI installs the Wayland/X11/xkbcommon/Vulkan development packages (see `.github/workflows/ci.yml`).
- `gpui` is a dev-dependency with `test-support` for `TestAppContext` tests.
