# ADR-025: GraphQL protocol and the shared HTTP core

Status: accepted (M16)

## Context

GraphQL is an `http` request plus a `graphql query` block and an optional `json variables` block (SPEC §7.9). It must
follow the protocol checklist: its own crate depending only on `fluxwire-protocol`, a feature name in `requires`,
no protocol-specific code in the runner, history and CLI. A GraphQL send is an HTTP exchange, so it needs the same
transport, auth, redirects and `http expect` checks as HTTP, and the hard rule forbids protocol crates depending on
each other.

## Decisions

- **`fluxwire-httpcore` holds everything HTTP.** The hyper and rustls engine, auth (basic, bearer, API key, digest,
  OAuth2), the `http` block planner (`plan`, `HttpPlan`), expect checking, history snapshots, script views and cURL moved
  out of `fluxwire-http` unchanged. `fluxwire-http` is now only the `Protocol` implementation; `fluxwire-graphql` is a
  second one over the same core. No behaviour of HTTP changed and its tests pass as before.
- **Claiming a document.** The `http` request language belongs to the HTTP protocol, so a request language lookup would
  run a GraphQL file as plain HTTP. `Protocol::claims(&FluxDoc)` (default `false`) lets a protocol take a document
  whose blocks it recognises; the runner asks protocols to claim before it looks at the language. GraphQL claims a
  file with a top-level `graphql query` block.
- **Request shape.** POST sends `{query, variables, operationName}` as JSON. A `GET` request puts the same three
  values in the query string (persisted-style queries). `operation: Name` in front matter picks one operation of a
  multi-operation document; a single named operation is sent by name automatically. `http expect` bodies match
  partially against the whole response, and an expected JSON object that names neither `data` nor `errors` matches
  against `data`.
- **`requires` is maintained on save.** `fluxwire-format::requires::sync_requires` adds `graphql` to `requires` when a
  file has a `graphql query` block and removes it when the block goes, leaving every other entry untouched. It runs
  inside the canonical formatter, so `fluxwire fmt`, app save and MCP writes agree. A registry without the feature
  does not manage the entry at all.
- **The feature is a cargo feature.** `fluxwire-protocol/graphql` registers the protocol, its roles (`graphql query`,
  `json variables`) and the `graphql` feature in `ProtocolRegistry::builtin()`. `fluxwire-runner`, `-agent`, `-cli`,
  `-ui` and `fluxwire` forward it and enable it by default. A build without it parses `requires: [graphql]` as
  unsupported (F401): the file opens, edits and saves, and `fluxwire validate` and `run` exit 66. `cargo build -p
  fluxwire --no-default-features` is that build, and CI covers it.
- **Schema handling is pure.** `fluxwire-graphql` holds the parser with byte spans, the introspection model and its
  per-URL-and-environment cache, validation, schema-aware completion and the query builder model, all tested without
  GPUI. The UI draws them. The cache lives under the workspace's `.fluxwire/graphql/`, keyed by a stable hash of the
  environment name and URL; the raw introspection response is stored and the schema rebuilt on load.
- **Importers write real GraphQL files.** Postman `graphql` bodies, Insomnia `application/graphql` bodies and Hurl
  ```` ```graphql ```` bodies (with `variables { ... }`) become `graphql query` and `json variables` blocks with
  `requires: [graphql]`.

## Notes

- PROMPTS.md numbers this milestone M16; SPEC §27 lists it as M17 (and M16 as the integrated terminal). Commits and
  docs use M16, following the prompt.
- Subscriptions, persisted-query registries, SDL import and `fw.graphql.*` scripting are deferred.
