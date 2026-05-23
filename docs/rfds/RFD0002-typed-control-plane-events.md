# RFD0002 - Typed Control Plane Events and Graph Mutations

- Feature Name: `typed_control_plane_events`
- Start Date: `2026-05-23`
- Status: `presented`
- RFD PR: N/A
- Gumgum Issue: N/A

## Summary

GumGum's control plane should converge through typed events, graph mutations, desired graph transitions, action planning, and execution. This RFD records the Track 2 direction and the parts already implemented so future slices keep tightening the same spine instead of adding ad-hoc report or stringly mutation paths.

- Normal mutations flow toward `Event -> GraphMutation -> DesiredGraph -> GumgumAction -> Executor`.
- CLI JSON event-like outputs should be newline-delimited typed events where the output is a stream of events.
- Human CLI output remains a presenter over typed data, not the source of truth.
- Grouped/aggregate views may remain report objects when explicitly requested.
- Local image build/push via Docker CLI is intentionally out of scope for this RFD.

## Motivation

GumGum already converges desired state through a typed graph, but not every input and output path is equally typed. Some daemon APIs still expose report shapes or string action summaries, and some CLI presentation paths historically knew too much about those report details. That makes it harder to provide live progress, deterministic tests, and safe automation because callers must infer lifecycle state from strings.

The intended control-plane shape solves these problems by making each step explicit:

1. a user/API event is decoded into a typed graph mutation,
2. mutations derive the next desired graph,
3. graph differences derive typed actions,
4. action execution emits typed events for planned/executed/failed lifecycle states,
5. the CLI presents those events as human lines or NDJSON.

This lets GumGum test convergence symbolically before side effects and gives users a stable automation contract without sacrificing friendly human output.

## Guide-level explanation

A deployment starts as a user intent: deploy worker `api@preview` with image `registry/api:rev`, route `visit-counter.leostera.dev`, port `3000`, and health path `/_/ready`.

Today this intent is partially typed already:

```rust
let old_graph = store.load_desired_graph()?;
let mutation = desired_deploy.upsert_mutation()?;
let new_graph = GraphMutation::apply_all(&old_graph, [&mutation]);
let plan = GraphActionPlanner::plan_transition(&old_graph, &new_graph);
let actions = GraphActionExecutor::execute_steps(&plan.steps, context).await?;
```

The CLI should not reconstruct meaning from `actions`. Instead, execution steps project into typed events:

```rust
step.planned_event(operation_id.clone());
step.executed_event(operation_id.clone(), "container already matches desired state");
step.failed_event(operation_id.clone(), error_message);
```

A human presenter can render:

```text
#42 op=reconcile-123 2026-05-23 08:10:23 reconciliation executed deployment/api@preview - container already matches desired state
```

The JSON presenter emits one typed event per line:

```json
{"type":"reconcile_step_executed","operation_id":"reconcile-123","target":"deployment/api@preview","action":"ensure_container","message":"container already matches desired state"}
```

Grouped views are intentionally different: `gumgum events --grouped --json` returns an aggregate report object because the user requested a summary, not an event stream.

## Current implementation snapshot

Implemented in Track 1 and early Track 2:

- `GumgumEvent` exists in `crates/gumgum-core/src/events.rs`.
- Stored `ReconcileEvent` rows project to `GumgumEvent`.
- `GraphMutation` exists in `crates/gumgum-core/src/graph/mutation.rs`.
- `DesiredDeploy`, `GlobalObject`, and `WorkerBinding` expose mutation helpers.
- `GumgumAction` currently aliases `GraphExecutionStep` as the action graph surface.
- `GraphExecutionStep` can project planned/executed/failed `GumgumEvent`s.
- Daemon deploy/delete reports include `typed_events`.
- `gumgum events --json` emits NDJSON typed events.
- `gumgum --json deploy` emits NDJSON typed events for worker/workspace deploy and delete paths when typed events are available.
- `crates/gumgum-cli/src/event_presenter.rs` centralizes human and NDJSON event rendering.

## Deferred work

The following work is explicitly deferred to later slices:

- Live server-sent/event-streaming progress while long deploy operations run. Current deploy flow still returns typed events after daemon-side reconciliation returns.
- BuildKit or Docker daemon image build/push support. Local `docker build` and `docker push` remain shell-based until a dedicated build/push-auth slice.
- A fully distinct `GumgumAction` enum if `GraphExecutionStep` stops being sufficient as the action graph surface.
- Wider property tests for arbitrary `Event -> GraphMutation -> DesiredGraph -> ActionGraph` determinism.
- A compatibility policy for older daemons once typed events become mandatory instead of additive.

## Invariants

- Human-readable output is default.
- JSON event streams are newline-delimited records, one event per line.
- Aggregate JSON reports are allowed only for aggregate commands or explicit aggregate flags.
- Dry-run/preview paths must not mutate desired graph or remote resources.
- Normal mutation path remains automatic convergence; users should not need a routine `gumgum reconcile` command.
- GumGum only mutates GumGum-managed resources.

## Rollout

The rollout is incremental and backwards-tolerant:

1. Add typed structures alongside existing reports.
2. Populate typed event arrays in daemon responses.
3. Prefer typed events in CLI presenters, falling back to legacy report rows for older daemons.
4. Move more mutation paths to `GraphMutation` helpers.
5. Add live streaming only once event shape and presentation are stable.
