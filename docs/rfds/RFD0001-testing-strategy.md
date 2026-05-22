# RFD0001 - Testing Strategy

- Feature Name: `testing-strategy`
- Start Date: `2026-05-21`
- Status: `presented`
- RFD PR: [leostera/gumgum#0000](https://github.com/leostera/gumgum/pull/0000)
- Gumgum Issue: [leostera/gumgum#0000](https://github.com/leostera/gumgum/issues/0000)

## Summary
[summary]: #summary

gumgum needs a layered testing strategy that proves CLI workflows, manifest/config parsing, typed graph convergence, provider planning, and real server behavior independently and together.

- Unit tests should cover all pure or mostly-pure behavior: CLI parsing, manifest validation, config schema, graph/domain newtypes, presentation, provider plans, and safe-delete guards.
- Property tests should stress graph reconciliation: arbitrary valid mutation sequences should either converge to the expected desired graph or return a structured error without panics or partial corruption.
- Fuzz tests should target parser and boundary surfaces: TOML manifests/config, CLI arg parsing, graph/domain identifiers, daemon request DTOs, and route/env/bucket path parsing.
- E2E tests should run slower but realistic scenarios against an isolated VM and verify containers, routes, providers, bindings, logs, events, bucket objects, rollback, cleanup, and publish dry-runs.
- Out of scope for this RFD: replacing Docker, testing every third-party provider implementation exhaustively, or requiring VM E2E tests for every local edit.

## Motivation
[motivation]: #motivation

gumgum is a control plane. Bugs are expensive because they can create, delete, expose, or misconfigure real runtime state. The current tests cover important pieces, but the coverage is organic rather than systematic.

Today gumgum pays several testing costs:

- CLI UX regressions are easy because many commands share argument structs, output conventions, and compatibility aliases.
- Graph convergence is the product spine, but most tests are example-based; they do not yet explore many update orderings or deletion/recreation sequences.
- Manifest/config/CLI parsers accept user-controlled input and should never panic, even on garbage.
- Real provider behavior can only be proven against Docker/VM state, but these tests need isolation and strong safety checks so they do not mutate a developer's unrelated containers.
- Example apps such as visit-counter currently double as product fixtures; they should move under `tests/` and become explicit E2E fixtures with stable expectations.

The goal is confidence at the right layer: fast unit tests for local edits, property/fuzz tests for broad input coverage, and slower VM tests before releases or risky control-plane changes.

## Guide-level explanation
[guide-level-explanation]: #guide-level-explanation

A contributor should be able to choose the right test command by asking what changed.

For a pure CLI or manifest change:

```bash
cargo test
```

For graph planner/executor changes:

```bash
cargo test -p gumgum-core graph
cargo test -p gumgum-core --test graph_properties
```

For parser hardening:

```bash
cargo fuzz run manifest_parse
cargo fuzz run cli_args
cargo fuzz run graph_identifiers
```

For a full host check:

```bash
scripts/e2e-vm.sh --vm gumgum-test --fixture tests/visit-counter
```

The tests should be organized by layer:

1. **Unit tests** live beside code in `#[cfg(test)]` modules when they test one module or function.
2. **Integration tests** live under crate-level `tests/` directories when they test public crate behavior across modules.
3. **Property tests** live under `crates/*/tests/*_properties.rs` and use reproducible seeds on failure.
4. **Fuzz targets** live under `fuzz/fuzz_targets/` and are safe to run indefinitely.
5. **E2E fixtures** live under repository `tests/fixtures/` or `tests/apps/`, not under `examples/`, and are intentionally deployable to an isolated VM.

A failure should clearly say which layer failed. Unit/property/fuzz failures should not require a server. E2E failures should preserve artifacts: command output, graph snapshots, container lists, logs, response bodies, and checksums.

## Reference-level explanation
[reference-level-explanation]: #reference-level-explanation

### Unit test coverage map

Every command/parser/model area should have direct tests for success and failure paths.

CLI grammar and output:

- bare commands show help, not JSON `INVALID_ARGS`
- human output by default, JSON only with `--json`
- `gumgum server` grammar and `--host` forms
- `gumgum init` workspace-only grammar
- `gumgum worker create/list/delete`
- resource grammars for `db`, `kv`, `bucket`, `queue`, `secret`
- bucket-only object commands (`ls`, `get`, `rm`, `cp`, `sync`)
- `gumgum env` default vs `--qualified`
- workspace `logs`, `env`, `publish`, and `logs -f`

Manifest/config/domain parsing:

- workspace manifest templates round-trip
- worker manifest templates round-trip
- invalid manifests produce structured errors
- config schema accepts only known keys
- route hosts, health paths, worker IDs, object names, provider names, binding names, and object refs reject bad values

Graph and provider planning:

- object lifecycle creates typed graph nodes
- binding lifecycle creates typed binding nodes
- deploy lifecycle creates runtime/gateway graph steps
- rollback routes through typed deployment steps
- delete guards reject deleting bound objects/workers
- provider plans never leak secret values

Presentation and safety:

- dry-run/preview paths do not mutate graph state
- old daemon errors include upgrade/capability hints
- logs/events/status/publish/rollback output is stable enough to test

### Property tests for graph reconciliation

Property tests should generate valid desired-state mutations and assert invariants after planning and execution.

Suggested model:

```rust
enum ModelOp {
    AddProvider(Capability),
    AddObject { capability, name },
    Bind { worker, binding, object },
    Unbind { worker, binding },
    Deploy { worker, route, image },
    Undeploy { worker },
    DeleteObject { capability, name },
}
```

For each generated sequence:

1. Apply the operation to an in-memory reference model.
2. Apply the same operation to gumgum's desired graph/store path.
3. Run `GraphActionPlanner::plan_transition(old, new)`.
4. Execute using a fake/no-op execution context where possible.
5. Assert either:
   - convergence succeeds and materialized graph matches the reference model, or
   - a structured `GumgumError` is returned with subsystem/code/message, no panic, and no partial invalid graph.

Core invariants:

- every binding points to an existing object
- deleting a bound object is rejected
- deleting a bound worker is rejected
- graph planning is deterministic for the same old/new graph
- repeated planning after convergence is empty or idempotent
- execution steps are deduplicated where intended
- route/runtime/provider targets stay distinct
- no panic for any generated valid operation sequence

Negative property tests should generate invalid identifiers, invalid routes, missing objects, conflicting bindings, and out-of-order deletes; these must fail cleanly.

### Fuzz targets

Use `cargo-fuzz` with small harnesses. Initial fuzz targets:

- `manifest_parse`: calls `validate_str(data, "fuzz.toml")` and asserts no panic.
- `config_parse`: parses config values/keys and asserts structured rejection for unknown/invalid values.
- `cli_args`: tokenizes bytes into argv-like strings and calls Clap `try_parse_from`.
- `graph_identifiers`: feeds arbitrary strings into `WorkerId`, `GraphNodeId`, `ProviderName`, `ObjectName`, `BindingName`, `ObjectRef`, `RouteHost`, and `HealthPath` constructors.
- `api_requests`: deserializes daemon request DTOs from arbitrary JSON and validates handlers/helpers do not panic.
- `bucket_paths`: fuzzes local/remote bucket path classification and split logic.

Fuzz targets should never touch Docker, the network, `$HOME`, or real config.

### E2E VM tests

E2E tests should run against an isolated VM or disposable host. They must never target a developer's normal server by default.

Required VM properties:

- Docker installed
- SSH access
- no unrelated containers, or an explicit baseline snapshot
- disposable root/test domains routed to the VM
- artifact directory for all logs and snapshots

E2E phases:

1. **Setup**
   - `gumgum server add <vm> --root-domain <domain>`
   - verify daemon health and capabilities
   - verify built-in providers are running
2. **Workspace/worker creation**
   - `gumgum init`
   - `gumgum worker create api`
   - `gumgum worker create worker`
   - validate manifests and workspace members
3. **Resource lifecycle**
   - create/list db, kv, bucket, queue, secret
   - bind to workers
   - verify `gumgum env` and `gumgum env --qualified`
   - verify deleting bound objects fails
4. **Deploy**
   - deploy workspace
   - verify API ingress over HTTPS
   - verify background worker uses container health only, no invented route
   - verify container env uses Docker-network provider addresses
5. **Runtime behavior**
   - call API
   - verify queue/bucket/DB/KV path if fixture supports it
   - verify `gumgum logs` and workspace `gumgum logs -f` prefix lines
   - verify `gumgum events --grouped`
6. **Bucket objects**
   - local→remote copy
   - remote→local copy
   - remote→remote copy
   - list/get/rm/sync
7. **Rollback/publish/delete safety**
   - rollback preview
   - revision list
   - publish dry-run
   - unbind then delete resources
   - cleanup verifies no pre-existing containers disappeared

Artifacts per run:

- command transcript
- stdout/stderr per command
- graph before/after each phase
- container list before/after each phase
- daemon logs
- app logs
- HTTP responses
- checksums for copied bucket objects

### CI shape

Recommended tiers:

- **PR fast path**: `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test`.
- **Nightly or pre-release**: property tests with more cases, fuzz smoke corpus runs, and one VM E2E fixture.
- **Manual release gate**: full VM E2E matrix and long fuzz run.

## Drawbacks
[drawbacks]: #drawbacks

- More test infrastructure increases maintenance work.
- Property tests require careful generators; bad generators can create false confidence or too many invalid cases.
- Fuzz targets can be noisy if they exercise code that assumes filesystem/network state.
- VM E2E tests are slower and require environment management.

## Rationale and alternatives
[rationale-and-alternatives]: #rationale-and-alternatives

A layered strategy is better than trying to make one giant smoke test prove everything. Unit tests give fast feedback, property tests explore state-space, fuzz tests harden input boundaries, and VM E2E tests prove actual integration.

Alternatives considered:

- **Only unit tests**: too weak for convergence and provider/runtime integration.
- **Only E2E tests**: too slow and brittle for normal development.
- **Only fuzzing parsers**: useful, but it does not prove graph or runtime semantics.
- **Mock Docker everywhere**: good for planner logic, but insufficient for server setup/provider behavior.

## Prior art
[prior-art]: #prior-art

Relevant practices:

- Kubernetes controller tests split reconciliation unit tests from envtest/e2e clusters.
- Terraform providers use acceptance tests for real cloud behavior plus unit tests for plan logic.
- cargo-fuzz/libFuzzer is common for Rust parser and boundary hardening.
- proptest/quickcheck-style state machines are useful for graph/store invariants.

## Unresolved questions
[unresolved-questions]: #unresolved-questions

- Should E2E fixtures live under `tests/fixtures`, `tests/apps`, or a dedicated `e2e/` directory?
- Should visit-counter move directly from `examples/` to `tests/fixtures/visit-counter`, or be copied first while README/docs catch up?
- What VM provisioning tool should be the default: plain SSH, Lima, Multipass, cloud-init, or something else?
- How much old-daemon compatibility should property tests model?
- Which daemon APIs need fake execution contexts for deterministic property tests?

## Future possibilities
[future-possibilities]: #future-possibilities

- Add a `gumgum doctor` command whose checks mirror E2E setup expectations.
- Generate CLI reference snapshots from Clap definitions and test README examples against them.
- Keep minimized fuzz corpora in-tree for regressions.
- Add graph model checking for larger operation sequences.
- Add E2E matrix tests for local server, remote VM, and upgrade-from-previous-release paths.
