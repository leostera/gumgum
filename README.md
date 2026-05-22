# GumGum.dev

GumGum is a local-first PaaS/control-plane experiment for running small apps on a single host with typed desired-state graph convergence.

The current implementation centers on this flow:

```rust
let plan = GraphActionPlanner::plan_transition(&old_graph, &new_graph);
let actions = GraphActionExecutor::execute_steps(&plan.steps, context).await?;
```

User-facing commands mutate desired state, the daemon plans the graph transition, executes provider/runtime changes, and records durable events with grouped event views. There is intentionally no normal user-facing `gumgum reconcile`; convergence is automatic.

## What exists now

- Rust workspace:
  - `crates/gumgum-core` — graph model, provider reconciliation, deployment state, container reconciliation, config store.
  - `crates/gumgum-api` — daemon HTTP API and reports.
  - `crates/gumgum-cli` — CLI, server setup/upgrade/client commands, deploy/log/status/publish/rollback/object commands.
- Provider-backed object lifecycle for:
  - Postgres DBs
  - Redis KV namespaces
  - MinIO buckets/blob storage
  - Redpanda queues/topics
- Deployment flow for local/remote Docker hosts:
  - build stable revision-tagged images
  - push via local registry tunnel when needed
  - replace only GumGum-managed containers
  - inject binding env/secrets
  - configure `.test` Caddy routes
  - health check containers over attached Docker networks
- Durable observability/control-plane UX:
  - `gumgum status`
  - `gumgum events`
  - `gumgum events --grouped`
  - `gumgum logs`
  - rollback revision listing/preview/apply safety
  - metadata-only rollback revision pruning
  - publish dry-run with explicit public-route planning
  - bucket object commands (`ls`, `get`, `rm`, `cp`, `sync`) through MinIO-backed daemon APIs

## Canonical fixture

`examples/visit-counter` is the canonical end-to-end fixture. It exercises DB/KV/blob/queue objects, bindings, image build/push/deploy, local DNS/gateway, logs, events/grouped events, health, rollback safety, cleanup safety, and publish dry-run.

Current known starbase2 state is summarized in `examples/visit-counter/README.md`.

## Current starbase2 snapshot

As of the CLI UX cleanup pass:

- host: `starbase2` / `192.168.0.3`
- configured server record points at `192.168.0.3`
- direct checks currently show the remote daemon as older `dc996d4`; intentionally upgrade/install the current local daemon before relying on newer event/bucket/revision-delete APIs
- after upgrade, this is the required capability gate:

```bash
gumgum server capabilities list --host starbase2 \
  --require gumgum:events,gumgum:rollback:revision_id,gumgum:rollback:revision_delete,gumgum:objects:create_preview,gumgum:bindings:create_preview,gumgum:bindings:delete,gumgum:objects:delete,gumgum:deployments:delete,gumgum:buckets:objects
```

- repeat visit-counter deployment was intentionally left running for observation during hardening
- before any further starbase2 mutation, re-check daemon version/capabilities and status
- do not prune unrelated desired objects/secrets without explicit owner approval
- do not apply rollback unless clean historical revisions have intentionally been created

## Common commands

```bash
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test

cd examples/visit-counter
uv run --directory api pytest
uv run --directory worker pytest

gumgum status --host starbase2
gumgum events --host starbase2 --limit 20
gumgum events --host starbase2 --grouped --limit 10
gumgum logs api --host starbase2 --tail 60
gumgum logs worker --host starbase2 --tail 60
curl -k --resolve api.visit-counter.leostera.test:443:192.168.0.3 \
  https://api.visit-counter.leostera.test/
```

Do not run setup/upgrade/object apply/deploy/cleanup/rollback apply/publish apply/server add against starbase2 unless that mutation is explicitly intended.

## Documentation map

- `docs/README.md` — older planning/research pack and RFD map.
- `crates/gumgum-core/src/graph/README.md` — canonical graph/domain model direction.
- `examples/visit-counter/README.md` — fixture usage, starbase2 status, safe smoke modes, and validation notes.
- `scripts/smoke-visit-counter-starbase2.sh` — staged safety harness retained for intentional evidence gathering; direct `gumgum` commands are currently preferred for day-to-day validation.

## Development rules

- Prefer typed graph/domain values and `impl` methods over stringly helpers.
- Keep provider details in provider-specific modules.
- Keep Docker direct; do not add a runtime abstraction without a concrete need.
- Use graph planning/execution for desired-state transitions.
- Keep dry-run/preview paths non-mutating and capability-gated when talking to remote daemons.
- Use conventional commit messages.
- Do not run `./autoresearch.sh` unless explicitly requested.
