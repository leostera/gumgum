# Agent handoff: GumGum.dev

Keep this file short. It replaces the need to replay the full Ralph iteration log.

## Current mission

Harden GumGum as a local-first PaaS/control plane whose normal mutation path is typed graph convergence:

```rust
let plan = GraphActionPlanner::plan_transition(&old_graph, &new_graph);
let actions = GraphActionExecutor::execute_steps(&plan.steps, context).await?;
```

The canonical validation fixture is `examples/visit-counter`.

## Non-negotiables

- Do not run `./autoresearch.sh` unless the user explicitly asks.
- Do not stop or remove unrelated containers on starbase2 (`192.168.0.3`).
- Do not prune unrelated desired objects/secrets without explicit owner approval and before/after evidence.
- Do not apply rollback unless clean historical revisions have intentionally been created and inspected.
- Do not run setup/upgrade/server add/object apply/deploy/cleanup/publish apply unless explicitly intended.
- Prefer direct `gumgum`/`cargo run` commands for starbase2 validation so raw output is visible.
- Visit-counter is an internal fixture; do not add product-facing visit-counter-specific CLI flags or messages.
- Keep Docker direct. Do not add a runtime abstraction without concrete need.
- Prefer typed graph/domain values and `impl` methods over stringly helpers.
- Prefer provider-specific modules.
- There is no normal user-facing `gumgum reconcile`; convergence is automatic.
- Use conventional commit messages.

## Starbase2 current state

- host: `starbase2` / `192.168.0.3`
- configured server record points at `192.168.0.3`
- direct checks during CLI cleanup showed the remote daemon as older `dc996d4`; intentionally upgrade/install current local daemon before relying on newer event/bucket/revision-delete APIs
- after upgrade, require:

```bash
gumgum server capabilities list --host starbase2 \
  --require gumgum:events,gumgum:rollback:revision_id,gumgum:rollback:revision_delete,gumgum:objects:create_preview,gumgum:bindings:create_preview,gumgum:bindings:delete,gumgum:objects:delete,gumgum:deployments:delete,gumgum:buckets:objects
```

- repeat visit-counter API/worker deployment was intentionally left running during hardening
- before further starbase2 mutation, re-check `gumgum status --host starbase2` and capabilities
- active visit-counter objects were bound; unrelated older objects/secrets were visible as unbound
- stale API rollback revisions were metadata-pruned; do not apply rollback unless clean history is intentionally created

## Safe observation commands

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

The `curl` mutates only app data by creating a visit; it does not mutate GumGum desired/control-plane state.

## Validation gates

```bash
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
( cd examples/visit-counter/api && uv run pytest )
( cd examples/visit-counter/worker && uv run pytest )
find examples/visit-counter -type d -name __pycache__ -prune -exec rm -rf {} +
```

Run the Rust/Python gates after implementation changes. For documentation-only changes, `cargo fmt --check` is usually sufficient unless the user asks for more.

## Key files

- `README.md` — compact project status and command map.
- `examples/visit-counter/README.md` — canonical fixture and starbase2 notes.
- `crates/gumgum-core/src/graph/README.md` — typed graph model guidance.
- `.ralph/visit-counter-platform-hardening.md` — trimmed Ralph task/status file; do not let it grow into a full iteration transcript again.
