# GumGum core graph model

The graph module owns the canonical desired-state reconciliation model. API and CLI DTOs may use edge-friendly strings, but values should be converted into these graph/domain types before planning or execution.

## Canonical typed graph values

Typed and validated in `types.rs`:

- `WorkerId` — deployment worker identity.
- `ContainerName` — runtime container identity.
- `ImageName` — container image reference.
- `RouteHost` — routed hostname.
- `Port` — non-zero TCP port.
- `HealthPath` — absolute health-check path.
- `ProviderName` — provider identity such as `postgres.main`.
- `ObjectName` — provider-backed object identity.
- `BindingName` — environment binding name.
- `ObjectRef` — object reference in `capability/name` form.
- `GraphNodeId` — desired graph node id, currently used for removal.

Typed reconciliation actions now cover provider, worker, container, deploy, route, binding, object, and removal targets. `GraphActionPlanner` and `GraphActionExecutor` should stay the only normal execution path for graph transitions.

## Intentionally string-based values

These remain raw strings for now because they are edge/persistence/presentation values rather than canonical graph-domain internals:

- `DesiredDeploy`, `GlobalObject`, `WorkerBinding`, and `DesiredProvider` fields in `graph_store.rs`: persisted/API-adjacent DTOs. They expose methods such as `graph_node`, `execution_step`, and `*_reconciliation_steps` to convert into typed graph values.
- `ReconcileEvent::{kind, operation_id, target, action, message, created_at}` and `NewReconcileEvent`: control-plane event-log records. `kind` distinguishes user desired-state mutations from reconciliation/execution steps. `operation_id` groups planned/executed/failed records from the same execution run, `target`/`action` are stable display/filter keys, `message` is user-facing text, and `created_at` is SQLite text output.
- `GraphExecutionStep::description`: user-facing execution text.
- `GraphExecutionTarget::{WorkerRuntime,ContainerRuntime,Gateway,GraphStore}` fields: compatibility/presentation targets used while normalizing legacy worker/container/route actions into typed `DeployRuntime` where possible.
- `DesiredGraphNode::{Daemon,Worker,Container,Route}` fields: legacy/presentation-style desired nodes retained for compatibility with older graph projections. New deployment execution should prefer `DesiredGraphNode::Deployment`.
- Provider/user config values such as namespaces, root domains, access labels, DNS strings, and secret field names: these are still DTO/config/persistence values and should be typed only when they become graph-domain invariants.

## Persistence and migrations

The daemon persists desired graph state in GumGum's own SQLite database (`graph.sqlite`). Schema changes belong in embedded sqlx migrations under `crates/gumgum-core/migrations`; daemon startup runs those migrations before opening `GraphStore`. Avoid one-off Rust migration patches for GumGum-owned internal tables.

## Direction

When adding new reconciliation behavior:

1. Model desired graph internals with validating newtypes.
2. Keep API/CLI DTO strings at the boundary.
3. Add methods on domain structs for conversion/planning instead of loose helper functions.
4. Execute through `GraphActionPlanner` + `GraphActionExecutor`.
5. Record reconciliation events through graph execution context.
