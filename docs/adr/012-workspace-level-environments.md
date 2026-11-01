# ADR-012: Workspace-level environments for v1.0

Status: accepted (M9.02). Supersedes the "environments belong to a Collection" and "active environment per Collection" decisions of ADR-011.

## Context

M9.02 first modelled environments per Collection, with a per-collection active choice, a "current collection" resolver, collection sections in the panel, a collection step in the switcher, and `env/<collection>/<name>` tab keys. For the MVP that is more machinery than the product needs: most workspaces have one collection, and a user who works across collections expects one environment switch to apply to everything they send.

## Decisions

- **Environments are workspace-level.** They live in `<workspace root>/environments/<name>.env` and `<name>.local.env`. When the Workspace root is itself a collection (the usual single-collection case) this is exactly the collection's own `environments/` directory, so existing fixtures and repositories keep working unchanged.
- **One active environment per Workspace** (`EnvStore.active`). The session stores `active_env` plus `no_environment` (an explicit "No environment", as opposed to the first-environment default). This is the M9.01 session shape again; the short-lived `active_envs` map is removed.
- **Every send, run, hover, and Problems check uses it,** whatever collection the request is in. `ScopeInputs.env_root` (and `ScriptOptions.env_root`) point scope building at the workspace root; `None` keeps the old behaviour (the collection root) for callers such as the future CLI that have no workspace.
- **Secret names stay per collection** (`[secrets]` in `fluxwire.toml`). For a workspace environment a name is secret if any collection lists it.
- **`.local.env` ignore rule (E106)** is checked against the Workspace's `.gitignore`. The app does not write to a workspace that has no manifest, so it only warns.
- **No collection UI for environments.** The pill is the environment name; the switcher has no collection step; the panel is a flat list; the New environment dialog has no Collection field; the tab key is `env/<name>` (a key of the form `env/<collection>/<name>` maps to its last segment). Environment actions are Workspace-level.
- **Collection-level environments are deferred.** A later release can add a per-collection layer above or below the workspace one; the in-memory document model (`EnvDoc`) and the validator are already independent of where the files live.

## Consequences

Simpler UI and session. Two collections cannot have different `staging` environments in v1.0; their differences belong in collection `vars` or folder `vars`, which already layer over the environment.
