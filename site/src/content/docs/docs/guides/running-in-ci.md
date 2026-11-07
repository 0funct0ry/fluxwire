---
title: Running in CI
description: Install the headless CLI, run a Collection, and publish JUnit or TAP results.
---

`fluxwire-cli` is a static binary with no GUI dependencies. It runs the same engine as the app.

```sh
curl -fsSL https://0funct0ry.github.io/fluxwire/install.sh | sh -s -- --cli-only
fluxwire run api/payments --env ci --var token="$API_TOKEN" -r pretty -r junit -o report.xml
```

## GitHub Actions

```yaml
jobs:
  api-tests:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - run: curl -fsSL https://0funct0ry.github.io/fluxwire/install.sh | sh -s -- --cli-only
      - run: ~/.local/bin/fluxwire run api --env ci -r pretty -r junit -o report.xml --bail
        env:
          API_TOKEN: ${{ secrets.API_TOKEN }}
      - uses: actions/upload-artifact@v4
        if: always()
        with: { name: api-report, path: report.xml }
```

## Useful options

| Option | Purpose |
|---|---|
| `--filter tag:smoke` | Run a subset. |
| `--data cases.csv` | Repeat the run per row. |
| `--concurrency 8` / `--sequential` | Control parallelism. |
| `--reporter pretty\|json\|junit\|tap` | Choose reporters; repeat for several. |
| `--bail` | Stop after the first failure. |

## Other checks

```sh
fluxwire fmt --check .       # exits 1 if any file is not canonical
fluxwire validate .          # parse diagnostics, rustc-style
```

See [Exit codes](../../reference/exit-codes/) for how to react to each outcome.
