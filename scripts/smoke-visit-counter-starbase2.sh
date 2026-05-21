#!/usr/bin/env bash
set -euo pipefail

HOST=${HOST:-starbase2}
ROOT_DOMAIN=${ROOT_DOMAIN:-leostera.dev}
TEST_DOMAIN=${TEST_DOMAIN:-leostera.test}
GUMGUM=${GUMGUM:-cargo run -q -p gumgum-cli --bin gumgum --}
EXAMPLE_DIR=${EXAMPLE_DIR:-examples/visit-counter}
APPLY=${APPLY:-0}
RUN_SETUP=${RUN_SETUP:-0}

before_file=$(mktemp)
after_file=$(mktemp)
cleanup() {
  rm -f "$before_file" "$after_file"
}
trap cleanup EXIT

remote_containers() {
  ssh "$HOST" "docker ps --format '{{.Names}}' | sort" 2>/dev/null || true
}

container_delta_guard() {
  remote_containers >"$after_file"
  missing=$(comm -23 "$before_file" "$after_file" || true)
  if [ -n "$missing" ]; then
    echo "error: smoke removed unrelated pre-existing container(s):" >&2
    echo "$missing" >&2
    exit 1
  fi
}

run_gumgum() {
  echo "+ gumgum $*"
  # shellcheck disable=SC2086
  $GUMGUM "$@"
}

remote_containers >"$before_file"

if [ "$RUN_SETUP" = "1" ]; then
  run_gumgum setup "$HOST" --root-domain "$ROOT_DOMAIN" --test-domain "$TEST_DOMAIN"
fi

pushd "$EXAMPLE_DIR" >/dev/null

run_gumgum db create visits --host "$HOST" --root-domain "$ROOT_DOMAIN"
run_gumgum kv create user-counters --host "$HOST" --root-domain "$ROOT_DOMAIN"
run_gumgum bucket create visit-requests --host "$HOST" --root-domain "$ROOT_DOMAIN"
run_gumgum queue create visit-events --host "$HOST" --root-domain "$ROOT_DOMAIN"

run_gumgum db bind visits --host "$HOST" --to worker --as DATABASE_URL
run_gumgum kv bind user-counters --host "$HOST" --to api --as USER_COUNTERS
run_gumgum bucket bind visit-requests --host "$HOST" --to api --as VISIT_REQUESTS_BUCKET
run_gumgum bucket bind visit-requests --host "$HOST" --to worker --as VISIT_REQUESTS_BUCKET
run_gumgum queue bind visit-events --host "$HOST" --to api --as VISIT_EVENTS_QUEUE
run_gumgum queue bind visit-events --host "$HOST" --to worker --as VISIT_EVENTS_QUEUE

if [ "$APPLY" = "1" ]; then
  run_gumgum deploy --host "$HOST"
  curl -fsS -H "Host: api.visit-counter.${TEST_DOMAIN}" "http://${HOST}/" >/tmp/gumgum-visit-counter-response.txt
  grep -q "Hello visitor" /tmp/gumgum-visit-counter-response.txt
  run_gumgum events --host "$HOST" --limit 20
  run_gumgum logs --host "$HOST" api --tail 20 || true
  run_gumgum logs --host "$HOST" worker --tail 20 || true
else
  run_gumgum --dry-run deploy --host "$HOST"
fi

popd >/dev/null

container_delta_guard

echo "visit-counter smoke completed; APPLY=$APPLY; pre-existing containers preserved"
