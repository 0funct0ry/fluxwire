---
title: Exit codes
description: The process exit codes of the Fluxwire command line.
---

When several conditions apply, the highest-precedence code wins: a script that threw (3) over a network failure
(2) over a test or request failure (1).

| Code | Name | Meaning |
|---|---|---|
| `0` | `OK` | Everything passed. |
| `1` | `TESTS_FAILED` | A test, expectation or request failed. |
| `2` | `NETWORK` | A network failure prevented a request from completing. |
| `3` | `SCRIPT` | A script threw. |
| `4` | `CONFLICTS` | OpenAPI conflicts block a pull or push. |
| `64` | `USAGE` | Invalid command-line usage. |
| `65` | `PARSE` | A file could not be parsed. |
| `66` | `UNSUPPORTED` | A file requires a feature this version does not support. |

## Using exit codes in CI

```sh
fluxwire run api/ --env ci --reporter junit -o report.xml
case $? in
  0) echo "all passed" ;;
  1) echo "tests failed" ;;
  2) echo "network failure" ;;
  3) echo "a script threw" ;;
  *) echo "usage or file error" ;;
esac
```
