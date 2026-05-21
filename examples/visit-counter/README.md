# Visit Counter GumGum Example

This is the canonical end-to-end GumGum fixture. It is intentionally small, but it exercises the platform shape:

- API worker receives visits.
- KV stores per-visitor counters.
- Bucket stores raw request JSON objects.
- Queue receives events that point at bucket objects.
- Background worker consumes queue events and writes normalized rows into DB.

The Python implementation uses `uv` with a small FastAPI API worker and a stdlib queue worker. It currently includes filesystem/SQLite fallbacks so the example can run before every provider-backed binding is complete. GumGum should progressively replace those fallbacks with real provider env projections.

## GumGum path

```bash
gumgum setup starbase2 --root-domain leostera.dev
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
```

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
provider/minio.main backs object/blob/visit-requests
provider/redpanda.main backs object/queue/visit-events
worker/visit-counter-api binds kv/user-counters as USER_COUNTERS
worker/visit-counter-api binds blob/visit-requests as VISIT_REQUESTS_BUCKET
worker/visit-counter-api binds queue/visit-events as VISIT_EVENTS_QUEUE
worker/visit-counter-worker binds db/visits as DATABASE_URL
worker/visit-counter-worker binds blob/visit-requests as VISIT_REQUESTS_BUCKET
worker/visit-counter-worker binds queue/visit-events as VISIT_EVENTS_QUEUE
route/api.visit-counter.leostera.test routes_to worker/visit-counter-api
```
