# Agent handoff: gumgum

Keep this file short. It replaces the need to replay old Ralph/autoresearch iteration logs.

## Current mission

gumgum is a local-first/self-hosted PaaS. The normal mutation path is typed graph convergence:

```rust
let plan = GraphActionPlanner::plan_transition(&old_graph, &new_graph);
let actions = GraphActionExecutor::execute_steps(&plan.steps, context).await?;
```

User-facing commands mutate desired state, the daemon plans graph transitions, executes provider/runtime changes, and records durable events. There is intentionally no normal user-facing `gumgum reconcile`; convergence is automatic.

## Development rules

- Do not run `./autoresearch.sh` unless the user explicitly asks.
- Use conventional commit messages.
- Prefer direct `gumgum` / `cargo run` commands for validation so raw output is visible.
- Human-readable CLI output/errors by default; JSON only under `--json`.
- Keep Docker direct. Do not add a runtime abstraction without concrete need.
- Prefer typed graph/domain values and `impl` methods over stringly helpers.
- Prefer provider-specific modules.
- Keep dry-run/preview paths non-mutating and capability-gated when talking to remote daemons.
- Visit-counter is an internal fixture; do not add product-facing visit-counter-specific CLI flags or messages.
- Do not commit `.ralph/**`, `autoresearch*`, `.env`, virtualenvs, node_modules, build output, or other local artifacts.

## Product/CLI direction

- `gumgum server add <host> ...` should set up/install/register a server and boot built-in providers.
- Providers are internal; do not expose a public `gumgum providers` or `gumgum server providers` UX unless the product direction changes.
- Built-in provider set is DB/KV/queue/bucket/secrets.
- `gumgum operations` is removed; grouped operation-like views belong under `gumgum events --grouped`.
- Bucket object commands belong only under `gumgum bucket`.
- Bucket object transfer direction:
  - local → remote: CLI reads local file and sends bytes to daemon.
  - remote → local: CLI fetches bytes from daemon and writes local file.
  - remote → remote: daemon performs S3 copy.
- Longer-term simplification: a project/workspace should be bound to one server so most commands do not need `--host`.
- A server effectively owns root domains. Do not silently overwrite a server root domain during setup; adding/removing domains should be explicit.

## Current implementation notes

- `crates/gumgum-core` — graph model, provider reconciliation, deployment state, container reconciliation, config store.
- `crates/gumgum-api` — daemon HTTP API and reports.
- `crates/gumgum-cli` — CLI, server setup/upgrade/client commands, deploy/log/status/publish/rollback/object commands.
- Built-in providers:
  - Postgres DBs
  - Redis KV namespaces
  - MinIO buckets/blob storage
  - Redpanda queues/topics
  - secret provider plumbing
- Deployment flow:
  - build stable revision-tagged images
  - push via local registry tunnel when needed
  - replace only gumgum-managed containers
  - inject binding env/secrets
  - configure `.test` Caddy routes
  - health check containers over attached Docker networks
- Observability/control-plane UX:
  - `gumgum status`
  - `gumgum events`
  - `gumgum events --grouped`
  - `gumgum logs`
  - rollback revision listing/preview/apply safety
  - metadata-only rollback revision pruning
  - publish dry-run with explicit public-route planning
  - bucket object commands (`ls`, `get`, `rm`, `cp`, `sync`) through embedded S3/MinIO APIs

## Starbase2 safety

- Host: `starbase2` / `192.168.0.3`.
- Do not stop or remove unrelated containers on starbase2.
- Do not prune unrelated desired objects/secrets without explicit owner approval and before/after evidence.
- Do not apply rollback unless clean historical revisions have intentionally been created and inspected.
- Do not run setup/upgrade/server add/object apply/deploy/cleanup/publish apply unless explicitly intended.
- Direct checks during CLI cleanup showed the remote daemon as older `dc996d4`; intentionally upgrade/install current local daemon before relying on newer event/bucket/revision-delete APIs.
- After upgrade, require:

```bash
gumgum server capabilities list --host starbase2 \
  --require gumgum:events,gumgum:rollback:revision_id,gumgum:rollback:revision_delete,gumgum:objects:create_preview,gumgum:bindings:create_preview,gumgum:bindings:delete,gumgum:objects:delete,gumgum:deployments:delete,gumgum:buckets:objects
```

## Safe starbase2 observation commands

```bash
cd examples/visit-counter

gumgum status --host starbase2
gumgum events --host starbase2 --limit 20
gumgum events --host starbase2 --grouped --limit 10
gumgum logs api --host starbase2 --tail 60
gumgum logs worker --host starbase2 --tail 60
gumgum rollback api/gumgum.toml --host starbase2 --worker visit-counter-api --preview
gumgum rollback worker/gumgum.toml --host starbase2 --worker visit-counter-worker --preview
gumgum --dry-run publish api/gumgum.toml --host starbase2
curl -k --resolve api.visit-counter.leostera.test:443:192.168.0.3 \
  https://api.visit-counter.leostera.test/
```

The `curl` mutates only app data by creating a visit; it does not mutate gumgum desired/control-plane state.

## Validation gates

Fast local gate:

```bash
scripts/test-fast.sh
```

Equivalent manual commands:

```bash
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
```

Fixture Python gates when touching Python test apps:

```bash
( cd examples/visit-counter/api && uv run pytest )
( cd examples/visit-counter/worker && uv run pytest )
find examples/visit-counter -type d -name __pycache__ -prune -exec rm -rf {} +
```

Testing tiers:

```bash
# fast unit/integration gate
scripts/test-fast.sh

# graph/property tests (expand this as property suites land)
cargo test -p gumgum-core graph

# fuzzing (requires cargo-fuzz; targets will be added under fuzz/)
cargo fuzz run manifest_parse

# VM E2E (must require an explicit disposable host; never default to starbase2)
scripts/e2e-vm.sh --host <vm-host> --root-domain <domain>
```

Run the Rust/Python gates after implementation changes. For documentation-only changes, `cargo fmt --check` is usually sufficient unless the user asks for more.

## Key files

- `README.md` — user-facing product/setup guide only.
- `AGENTS.md` — development rules, safety notes, and implementation context.
- `examples/visit-counter/README.md` — canonical fixture and starbase2 notes.
- `crates/gumgum-core/src/graph/README.md` — typed graph model guidance.
- `scripts/smoke-visit-counter-starbase2.sh` — staged safety harness retained for intentional evidence gathering.
