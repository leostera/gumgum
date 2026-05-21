#!/usr/bin/env bash
set -euo pipefail

HOST=${HOST:-starbase2}
ROOT_DOMAIN=${ROOT_DOMAIN:-leostera.dev}
TEST_DOMAIN=${TEST_DOMAIN:-leostera.test}
GUMGUM=${GUMGUM:-cargo run -q -p gumgum-cli --bin gumgum --}
EXAMPLE_DIR=${EXAMPLE_DIR:-examples/visit-counter}
APPLY=${APPLY:-0}
APPLY_OBJECTS=${APPLY_OBJECTS:-0}
RUN_SETUP=${RUN_SETUP:-0}
VERIFY_SETUP_IDEMPOTENCY=${VERIFY_SETUP_IDEMPOTENCY:-0}
VERIFY_UPGRADE_IDEMPOTENCY=${VERIFY_UPGRADE_IDEMPOTENCY:-0}
APPLY_UPGRADE=${APPLY_UPGRADE:-0}
VERIFY_CLEANUP_PREVIEW=${VERIFY_CLEANUP_PREVIEW:-0}
APPLY_CLEANUP=${APPLY_CLEANUP:-0}
VERIFY_ROLLBACK_PREVIEW=${VERIFY_ROLLBACK_PREVIEW:-0}
REQUIRE_CURRENT_DAEMON=${REQUIRE_CURRENT_DAEMON:-0}

before_file=$(mktemp)
after_file=$(mktemp)
before_graph_file=$(mktemp)
after_graph_file=$(mktemp)
cleanup() {
  rm -f "$before_file" "$after_file" "$before_graph_file" "$after_graph_file"
}
trap cleanup EXIT

remote_containers() {
  ssh "$HOST" "docker ps --format '{{.Names}}' | sort" 2>/dev/null || true
}

verify_test_dns() {
  local route="api.visit-counter.${TEST_DOMAIN}"
  if command -v dig >/dev/null 2>&1; then
    dig +short "@$HOST" "$route" | grep -q .
  elif command -v nslookup >/dev/null 2>&1; then
    nslookup "$route" "$HOST" >/dev/null
  else
    echo "warning: dig/nslookup unavailable; skipping explicit .test DNS verification" >&2
  fi
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

capture_graph() {
  local output_file="$1"
  # shellcheck disable=SC2086
  if ! $GUMGUM --json graph --host "$HOST" >"$output_file"; then
    echo "warning: could not capture desired graph snapshot" >&2
    : >"$output_file"
  fi
}

assert_graph_unchanged() {
  if ! cmp -s "$before_graph_file" "$after_graph_file"; then
    echo "error: preview operation mutated desired graph" >&2
    diff -u "$before_graph_file" "$after_graph_file" >&2 || true
    exit 1
  fi
}

assert_visit_resources_absent() {
  local graph_file="$1"
  local unexpected
  unexpected=$(grep -E 'visit-requests|visit-events|user-counters|object/db/visits|DATABASE_URL|USER_COUNTERS|VISIT_REQUESTS_BUCKET|VISIT_EVENTS_QUEUE' "$graph_file" || true)
  if [ -n "$unexpected" ]; then
    echo "error: cleanup left visit-counter object/binding desired state:" >&2
    echo "$unexpected" >&2
    exit 1
  fi
}

has_daemon_capability() {
  local capability="$1"
  if command -v curl >/dev/null 2>&1; then
    curl -fsS "http://${HOST}:7777/v0/version" 2>/dev/null | grep -q "\"${capability}\""
  else
    return 1
  fi
}

require_daemon_capabilities() {
  echo "+ gumgum server $HOST capabilities --require-visit-counter"
  # shellcheck disable=SC2086
  if $GUMGUM server "$HOST" capabilities --require-visit-counter; then
    return
  fi
  echo "error: gumgumd on $HOST is not ready for mutating visit-counter smoke modes" >&2
  exit 1
}

plan_gumgum() {
  echo "+ gumgum $*"
}

run_object_step() {
  if [ "$APPLY_OBJECTS" = "1" ]; then
    run_gumgum "$@"
  else
    plan_gumgum "$@"
  fi
}

if [ "$APPLY" = "1" ] && [ "$APPLY_OBJECTS" != "1" ]; then
  echo "error: APPLY=1 requires APPLY_OBJECTS=1 so deploy bindings exist intentionally" >&2
  exit 1
fi
if [ "$APPLY_CLEANUP" = "1" ] && [ "$APPLY_OBJECTS" != "1" ]; then
  echo "error: APPLY_CLEANUP=1 requires APPLY_OBJECTS=1 to make destructive cleanup explicit" >&2
  exit 1
fi
if [ "$REQUIRE_CURRENT_DAEMON" = "1" ] || [ "$APPLY_OBJECTS" = "1" ] || [ "$APPLY" = "1" ] || [ "$APPLY_CLEANUP" = "1" ]; then
  require_daemon_capabilities events rollback_revision_id binding_delete object_delete deployment_delete
fi

remote_containers >"$before_file"

if [ "$RUN_SETUP" = "1" ]; then
  run_gumgum setup "$HOST" --root-domain "$ROOT_DOMAIN" --test-domain "$TEST_DOMAIN"
  if [ "$VERIFY_SETUP_IDEMPOTENCY" = "1" ]; then
    run_gumgum setup "$HOST" --root-domain "$ROOT_DOMAIN" --test-domain "$TEST_DOMAIN"
  fi
fi

if [ "$VERIFY_UPGRADE_IDEMPOTENCY" = "1" ]; then
  if [ "$APPLY_UPGRADE" = "1" ]; then
    run_gumgum server "$HOST" upgrade
    run_gumgum server "$HOST" upgrade
    require_daemon_capabilities events rollback_revision_id binding_delete object_delete deployment_delete
  else
    run_gumgum --dry-run server "$HOST" upgrade
    run_gumgum --dry-run server "$HOST" upgrade
  fi
fi

pushd "$EXAMPLE_DIR" >/dev/null

run_object_step db create visits --host "$HOST" --root-domain "$ROOT_DOMAIN"
run_object_step kv create user-counters --host "$HOST" --root-domain "$ROOT_DOMAIN"
run_object_step bucket create visit-requests --host "$HOST" --root-domain "$ROOT_DOMAIN"
run_object_step queue create visit-events --host "$HOST" --root-domain "$ROOT_DOMAIN"

run_object_step db bind visits --host "$HOST" --to worker --as DATABASE_URL
run_object_step kv bind user-counters --host "$HOST" --to api --as USER_COUNTERS
run_object_step bucket bind visit-requests --host "$HOST" --to api --as VISIT_REQUESTS_BUCKET
run_object_step bucket bind visit-requests --host "$HOST" --to worker --as VISIT_REQUESTS_BUCKET
run_object_step queue bind visit-events --host "$HOST" --to api --as VISIT_EVENTS_QUEUE
run_object_step queue bind visit-events --host "$HOST" --to worker --as VISIT_EVENTS_QUEUE

if [ "$VERIFY_CLEANUP_PREVIEW" = "1" ] || [ "$APPLY_CLEANUP" = "1" ]; then
  if ! has_daemon_capability "binding_delete" || ! has_daemon_capability "object_delete"; then
    echo "warning: gumgumd on $HOST does not advertise safe delete APIs; run gumgum setup/upgrade before cleanup verification" >&2
  else
    capture_graph "$before_graph_file"
    preview_flag="--preview"
  if [ "$APPLY_CLEANUP" = "1" ]; then
    preview_flag=""
  fi
  # shellcheck disable=SC2086
  run_gumgum db unbind visits --host "$HOST" --to worker --as DATABASE_URL $preview_flag
  # shellcheck disable=SC2086
  run_gumgum kv unbind user-counters --host "$HOST" --to api --as USER_COUNTERS $preview_flag
  # shellcheck disable=SC2086
  run_gumgum bucket unbind visit-requests --host "$HOST" --to api --as VISIT_REQUESTS_BUCKET $preview_flag
  # shellcheck disable=SC2086
  run_gumgum bucket unbind visit-requests --host "$HOST" --to worker --as VISIT_REQUESTS_BUCKET $preview_flag
  # shellcheck disable=SC2086
  run_gumgum queue unbind visit-events --host "$HOST" --to api --as VISIT_EVENTS_QUEUE $preview_flag
  # shellcheck disable=SC2086
  run_gumgum queue unbind visit-events --host "$HOST" --to worker --as VISIT_EVENTS_QUEUE $preview_flag
  # shellcheck disable=SC2086
  run_gumgum db delete visits --host "$HOST" --root-domain "$ROOT_DOMAIN" $preview_flag
  # shellcheck disable=SC2086
  run_gumgum kv delete user-counters --host "$HOST" --root-domain "$ROOT_DOMAIN" $preview_flag
  # shellcheck disable=SC2086
  run_gumgum bucket delete visit-requests --host "$HOST" --root-domain "$ROOT_DOMAIN" $preview_flag
  # shellcheck disable=SC2086
  run_gumgum queue delete visit-events --host "$HOST" --root-domain "$ROOT_DOMAIN" $preview_flag
  capture_graph "$after_graph_file"
    if [ "$APPLY_CLEANUP" = "1" ]; then
      assert_visit_resources_absent "$after_graph_file"
    else
      assert_graph_unchanged
    fi
  fi
fi

if [ "$APPLY" = "1" ]; then
  run_gumgum deploy --host "$HOST"
  verify_test_dns
  curl -fsS -H "Host: api.visit-counter.${TEST_DOMAIN}" "http://${HOST}/" >/tmp/gumgum-visit-counter-response.txt
  grep -q "Hello visitor" /tmp/gumgum-visit-counter-response.txt
  run_gumgum events --host "$HOST" --limit 20
  if [ "$VERIFY_ROLLBACK_PREVIEW" = "1" ]; then
    run_gumgum rollback --host "$HOST" --worker api --preview || true
  fi
  run_gumgum logs --host "$HOST" api --tail 20 || true
  run_gumgum logs --host "$HOST" worker --tail 20 || true
else
  run_gumgum --dry-run deploy --host "$HOST"
fi

popd >/dev/null

container_delta_guard

echo "visit-counter smoke completed; APPLY_OBJECTS=$APPLY_OBJECTS APPLY=$APPLY APPLY_CLEANUP=$APPLY_CLEANUP; pre-existing containers preserved"
