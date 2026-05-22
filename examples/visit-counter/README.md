# Visit Counter GumGum Example

This directory is a user-facing copy of the visit-counter app. The maintained E2E fixture lives at `tests/fixtures/visit-counter`; keep changes in sync there when changing app behavior.

The app is intentionally small, but it exercises the platform shape:

- API worker receives visits.
- KV stores per-visitor counters.
- Bucket stores raw request JSON objects.
- Queue receives events that point at bucket objects.
- Background worker consumes queue events and writes normalized rows into DB.

The Python implementation uses `uv` with a small FastAPI API worker and a queue worker. The API worker uses realistic clients when GumGum projects provider env (`redis`, `boto3` for S3/MinIO, and `confluent-kafka` for Redpanda/Kafka) and falls back to local files for tests/dev. The queue worker uses realistic clients for Postgres (`psycopg`), S3/MinIO (`boto3`), and Redpanda/Kafka (`confluent-kafka`) when bindings are present, and falls back to SQLite/filesystem adapters for tests/dev.

## Current starbase2 status

The full product path has been proven against starbase2. The repeat deployment is intentionally left running for sparse observation.

Current important facts:

- starbase2 is configured as `192.168.0.3`
- direct checks during CLI cleanup showed the remote daemon as older `dc996d4`; intentionally upgrade/install the current local daemon before relying on newer event/bucket/revision-delete APIs
- the full product path was proven earlier with core providers running: Postgres, Redis, MinIO, Redpanda
- visit-counter objects were bound to API/worker; older unrelated desired objects/secrets were visible as unbound and should not be pruned without explicit approval
- stale failed-deploy API rollback revisions were pruned metadata-only; do not apply rollback unless clean historical revisions are intentionally created
- publish dry-run preserves `api.visit-counter.leostera.test`, plans `api.visit-counter.leostera.dev`, and changes no public route

Safe observation:

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

Do not apply rollback, prune objects, clean up, deploy, run server add, or publish publicly unless explicitly intended. The `curl` command only mutates example app data by recording a visit.

## GumGum path

```bash
gumgum server add starbase2 --name starbase2 --root-domain leostera.dev
cd examples/visit-counter

gumgum db create visits
gumgum kv create user-counters
gumgum bucket create visit-requests
gumgum queue create visit-events

gumgum db bind visits --to worker --as DATABASE_URL
gumgum kv bind user-counters --to api --as USER_COUNTERS
gumgum bucket bind visit-requests --to api --as VISIT_REQUESTS_BUCKET
gumgum bucket bind visit-requests --to worker --as VISIT_REQUESTS_BUCKET
gumgum queue bind visit-events --to api --as VISIT_EVENTS_QUEUE
gumgum queue bind visit-events --to worker --as VISIT_EVENTS_QUEUE

gumgum deploy
curl api.visit-counter.leostera.test
gumgum events
gumgum logs api
gumgum logs worker

# public publishing is explicit and dry-run first
gumgum --dry-run publish api
```

## Safe starbase2 smoke

Use the repository smoke harness when intentionally exercising the real host. It snapshots remote containers before/after and fails if any pre-existing container disappears.

```bash
# show staged smoke modes and the recommended intentional sequence
scripts/smoke-visit-counter-starbase2.sh --help
scripts/smoke-visit-counter-starbase2.sh --plan
# --plan includes an ARTIFACT_ROOT export, summary.tsv review, and checksum verification steps

# safest default: print object/binding commands and run only deploy dry-run
scripts/smoke-visit-counter-starbase2.sh

# inspect daemon capabilities and verify compatibility before any mutation
gumgum server capabilities list --host starbase2
gumgum server capabilities list --host starbase2 --require gumgum:events,gumgum:rollback:revision_id,gumgum:objects:create_preview,gumgum:bindings:create_preview,gumgum:bindings:delete,gumgum:objects:delete,gumgum:deployments:delete,gumgum:buckets:objects
REQUIRE_CURRENT_DAEMON=1 scripts/smoke-visit-counter-starbase2.sh

# create/bind objects intentionally, optionally stopping before deploy dry-run
APPLY_OBJECTS=1 scripts/smoke-visit-counter-starbase2.sh
APPLY_OBJECTS=1 OBJECTS_ONLY=1 scripts/smoke-visit-counter-starbase2.sh

# apply intentionally; setup is still opt-in and object/deploy mutation must be explicit
APPLY_OBJECTS=1 APPLY=1 scripts/smoke-visit-counter-starbase2.sh
DEPLOY_ONLY=1 APPLY=1 scripts/smoke-visit-counter-starbase2.sh
OBSERVE_ONLY=1 scripts/smoke-visit-counter-starbase2.sh
OBSERVE_ONLY=1 ARTIFACT_DIR=/tmp/visit-counter-smoke scripts/smoke-visit-counter-starbase2.sh
RUN_SETUP=1 VERIFY_SETUP_IDEMPOTENCY=1 APPLY_OBJECTS=1 APPLY=1 scripts/smoke-visit-counter-starbase2.sh
RUN_SETUP=1 VERIFY_SETUP_IDEMPOTENCY=1 SETUP_ONLY=1 scripts/smoke-visit-counter-starbase2.sh
VERIFY_UPGRADE_IDEMPOTENCY=1 scripts/smoke-visit-counter-starbase2.sh
VERIFY_UPGRADE_IDEMPOTENCY=1 UPGRADE_ONLY=1 scripts/smoke-visit-counter-starbase2.sh
VERIFY_UPGRADE_IDEMPOTENCY=1 APPLY_UPGRADE=1 UPGRADE_ONLY=1 scripts/smoke-visit-counter-starbase2.sh
VERIFY_UPGRADE_IDEMPOTENCY=1 APPLY_UPGRADE=1 scripts/smoke-visit-counter-starbase2.sh

# cleanup/rollback checks: preview is non-destructive; apply cleanup is explicit
# stale rollback revision pruning is metadata-only but requires gumgum:rollback:revision_delete
# gumgum server capabilities list --host starbase2 --require gumgum:rollback:revision_delete
# gumgum rollback api/gumgum.toml --host starbase2 --worker visit-counter-api --revisions --limit 10
# gumgum rollback api/gumgum.toml --host starbase2 --worker visit-counter-api --delete-revision-id <stale-id>
VERIFY_CLEANUP_PREVIEW=1 CLEANUP_ONLY=1 scripts/smoke-visit-counter-starbase2.sh
APPLY_CLEANUP=1 CLEANUP_ONLY=1 scripts/smoke-visit-counter-starbase2.sh
APPLY_OBJECTS=1 VERIFY_CLEANUP_PREVIEW=1 scripts/smoke-visit-counter-starbase2.sh
APPLY_OBJECTS=1 APPLY_CLEANUP=1 scripts/smoke-visit-counter-starbase2.sh
APPLY_OBJECTS=1 APPLY=1 VERIFY_ROLLBACK_PREVIEW=1 scripts/smoke-visit-counter-starbase2.sh
```

The deploy path builds locally, opens an SSH tunnel to the GumGum registry on starbase2, pushes the stable revision tag, asks `gumgumd` to reconcile the container, verifies DNS/Caddy with a `Host: api.visit-counter.leostera.test` request, and can optionally preview rollback. Mutating modes require a current daemon that advertises safe delete/rollback capabilities, and real upgrade smoke verifies those capabilities after the upgrade completes. If stale failed-deploy rollback entries are discovered, `--delete-revision-id` prunes only revision metadata; check `--revisions` first and require `gumgum:rollback:revision_delete` before using it. `SETUP_ONLY=1` exits after setup/idempotency and the container preservation guard, before object or deploy smoke. `UPGRADE_ONLY=1` exits after the upgrade/idempotency path and container preservation guard, which is the safest way to intentionally upgrade before object/apply smoke. `OBJECTS_ONLY=1` exits after object creation/binding and the container preservation guard, before any deploy planning. `DEPLOY_ONLY=1` skips object creation/binding and deploys or dry-runs from existing desired state, which is useful after object apply. `OBSERVE_ONLY=1` collects status, events, grouped events, and worker logs from an existing deployment without object/deploy/cleanup mutation. `ARTIFACT_DIR=/path` preserves response, graph/container snapshots, planned/executed GumGum commands, deploy/observe output, failure messages, plus `README.txt`, mode/status/exit-code/timestamp/duration-rich `summary.txt`, `index.txt`, and `checksums.sha256` for review. `ARTIFACT_ROOT=/path` derives a per-mode artifact directory for each staged smoke run (including distinct upgrade dry-run/apply and current-daemon check directories) so a full sequence does not overwrite earlier evidence, and writes root `README.txt`, `summary.tsv`, and `index.txt` across stages. `CLEANUP_ONLY=1` skips object creation/binding and exits after cleanup preview/apply plus the container preservation guard. Cleanup preview mode snapshots the desired graph before/after and fails if a preview mutates state. Explicit cleanup apply mode also snapshots the graph and verifies visit-counter object/binding desired state is gone without removing pre-existing containers. The default mode does not mutate starbase2 objects.

## Local fallback smoke test

```bash
cd api && VISIT_COUNTER_STATE_DIR=/tmp/visit-counter-example uv run visit-counter-api
cd worker && VISIT_COUNTER_STATE_DIR=/tmp/visit-counter-example uv run visit-counter-worker
curl -i http://127.0.0.1:3000/
sqlite3 /tmp/visit-counter-example/visits.sqlite 'select visitor_id, path, bucket_key from visits;'
```

## Expected object graph

```text
provider/postgres.main backs object/db/visits
provider/redis.main backs object/kv/user-counters
provider/minio.main backs object/bucket/visit-requests
provider/redpanda.main backs object/queue/visit-events
worker/visit-counter-api binds kv/user-counters as USER_COUNTERS
worker/visit-counter-api binds bucket/visit-requests as VISIT_REQUESTS_BUCKET
worker/visit-counter-api binds queue/visit-events as VISIT_EVENTS_QUEUE
worker/visit-counter-worker binds db/visits as DATABASE_URL
worker/visit-counter-worker binds bucket/visit-requests as VISIT_REQUESTS_BUCKET
worker/visit-counter-worker binds queue/visit-events as VISIT_EVENTS_QUEUE
route/api.visit-counter.leostera.test routes_to worker/visit-counter-api
```
