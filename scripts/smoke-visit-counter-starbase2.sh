#!/usr/bin/env bash
set -euo pipefail

HOST=${HOST:-starbase2}
ROOT_DOMAIN=${ROOT_DOMAIN:-leostera.dev}
TEST_DOMAIN=${TEST_DOMAIN:-leostera.test}
GUMGUM=${GUMGUM:-cargo run -q -p gumgum-cli --bin gumgum --}
EXAMPLE_DIR=${EXAMPLE_DIR:-examples/visit-counter}
APPLY=${APPLY:-0}
APPLY_OBJECTS=${APPLY_OBJECTS:-0}
OBJECTS_ONLY=${OBJECTS_ONLY:-0}
DEPLOY_ONLY=${DEPLOY_ONLY:-0}
OBSERVE_ONLY=${OBSERVE_ONLY:-0}
RUN_SETUP=${RUN_SETUP:-0}
SETUP_ONLY=${SETUP_ONLY:-0}
VERIFY_SETUP_IDEMPOTENCY=${VERIFY_SETUP_IDEMPOTENCY:-0}
VERIFY_UPGRADE_IDEMPOTENCY=${VERIFY_UPGRADE_IDEMPOTENCY:-0}
APPLY_UPGRADE=${APPLY_UPGRADE:-0}
UPGRADE_ONLY=${UPGRADE_ONLY:-0}
VERIFY_CLEANUP_PREVIEW=${VERIFY_CLEANUP_PREVIEW:-0}
APPLY_CLEANUP=${APPLY_CLEANUP:-0}
CLEANUP_ONLY=${CLEANUP_ONLY:-0}
VERIFY_ROLLBACK_PREVIEW=${VERIFY_ROLLBACK_PREVIEW:-0}
REQUIRE_CURRENT_DAEMON=${REQUIRE_CURRENT_DAEMON:-0}
HELP=${HELP:-0}
PLAN=${PLAN:-0}
ARTIFACT_DIR=${ARTIFACT_DIR:-}

print_plan() {
  cat <<'EOF'
recommended intentional starbase2 sequence:
  1. scripts/smoke-visit-counter-starbase2.sh
  2. REQUIRE_CURRENT_DAEMON=1 scripts/smoke-visit-counter-starbase2.sh
  3. RUN_SETUP=1 VERIFY_SETUP_IDEMPOTENCY=1 SETUP_ONLY=1 scripts/smoke-visit-counter-starbase2.sh
  4. VERIFY_UPGRADE_IDEMPOTENCY=1 UPGRADE_ONLY=1 scripts/smoke-visit-counter-starbase2.sh
  5. VERIFY_UPGRADE_IDEMPOTENCY=1 APPLY_UPGRADE=1 UPGRADE_ONLY=1 scripts/smoke-visit-counter-starbase2.sh
  6. CLEANUP_ONLY=1 VERIFY_CLEANUP_PREVIEW=1 scripts/smoke-visit-counter-starbase2.sh
  7. APPLY_OBJECTS=1 OBJECTS_ONLY=1 scripts/smoke-visit-counter-starbase2.sh
  8. DEPLOY_ONLY=1 APPLY=1 scripts/smoke-visit-counter-starbase2.sh
  9. OBSERVE_ONLY=1 scripts/smoke-visit-counter-starbase2.sh
 10. CLEANUP_ONLY=1 APPLY_CLEANUP=1 scripts/smoke-visit-counter-starbase2.sh
EOF
}

print_help() {
  cat <<'EOF'
visit-counter starbase2 smoke modes:
  default: non-mutating object command printout + deploy dry-run
  REQUIRE_CURRENT_DAEMON=1: fail unless gumgumd advertises visit-counter-safe capabilities
  RUN_SETUP=1 VERIFY_SETUP_IDEMPOTENCY=1 SETUP_ONLY=1: run setup twice, then stop
  VERIFY_UPGRADE_IDEMPOTENCY=1 UPGRADE_ONLY=1: dry-run upgrade twice, then stop
  VERIFY_UPGRADE_IDEMPOTENCY=1 APPLY_UPGRADE=1 UPGRADE_ONLY=1: apply upgrade twice, verify capabilities, then stop
  APPLY_OBJECTS=1 OBJECTS_ONLY=1: create/bind objects, then stop before deploy
  DEPLOY_ONLY=1 APPLY=1: deploy/curl using existing desired object/binding state
  OBSERVE_ONLY=1: show status/events/operations/logs for existing deployment
  ARTIFACT_DIR=<dir>: copy response/graph/container snapshots and observe output into a directory
  ARTIFACT_DIR also writes index.txt, summary.txt, and checksums.sha256 for review
  CLEANUP_ONLY=1 VERIFY_CLEANUP_PREVIEW=1: preview cleanup without creating objects
  CLEANUP_ONLY=1 APPLY_CLEANUP=1: apply cleanup without creating objects
  PLAN=1 or --plan: print the recommended intentional apply sequence
EOF
}

if [ "$PLAN" = "1" ] || [ "${1:-}" = "--plan" ]; then
  print_plan
  exit 0
fi

if [ "$HELP" = "1" ] || [ "${1:-}" = "--help" ] || [ "${1:-}" = "-h" ]; then
  print_help
  exit 0
fi

before_file=$(mktemp)
after_file=$(mktemp)
before_graph_file=$(mktemp)
after_graph_file=$(mktemp)
if [ -n "$ARTIFACT_DIR" ]; then
  mkdir -p "$ARTIFACT_DIR"
fi

write_artifact_summary() {
  if [ -z "$ARTIFACT_DIR" ]; then
    return
  fi
  cat >"$ARTIFACT_DIR/summary.txt" <<EOF
created_at=$(date -u +%Y-%m-%dT%H:%M:%SZ)
host=$HOST
root_domain=$ROOT_DOMAIN
test_domain=$TEST_DOMAIN
artifact_dir=$ARTIFACT_DIR
apply_objects=$APPLY_OBJECTS
apply=$APPLY
objects_only=$OBJECTS_ONLY
deploy_only=$DEPLOY_ONLY
observe_only=$OBSERVE_ONLY
cleanup_only=$CLEANUP_ONLY
apply_cleanup=$APPLY_CLEANUP
apply_upgrade=$APPLY_UPGRADE
EOF
}

write_artifact_readme() {
  if [ -z "$ARTIFACT_DIR" ]; then
    return
  fi
  cat >"$ARTIFACT_DIR/README.txt" <<'EOF'
Visit-counter smoke artifacts

Key files:
  summary.txt             run mode, host, domains, and timestamp
  index.txt               sorted list of captured files
  checksums.sha256        integrity checksums for captured files
  containers-before.txt   remote containers before the smoke stage
  containers-after.txt    remote containers after the smoke stage
  deploy-dry-run.txt      dry-run deploy plan output, when captured
  deploy.txt              apply deploy output, when captured
  gumgum-visit-counter-response.txt  route curl response, when deploy applies
  status/events/operations/logs-*.txt observe-only output, when captured
  graph-before.json / graph-after.json cleanup preview/apply graph snapshots

Safe review:
  diff -u containers-before.txt containers-after.txt
  shasum -a 256 -c checksums.sha256
EOF
}

write_artifact_index() {
  if [ -z "$ARTIFACT_DIR" ]; then
    return
  fi
  (
    cd "$ARTIFACT_DIR"
    find . -maxdepth 1 -type f -print | sed 's#^./##' | sort >index.txt
  )
}

write_artifact_checksums() {
  if [ -z "$ARTIFACT_DIR" ]; then
    return
  fi
  if ! command -v shasum >/dev/null 2>&1; then
    return
  fi
  (
    cd "$ARTIFACT_DIR"
    find . -maxdepth 1 -type f ! -name checksums.sha256 ! -name index.txt -print \
      | sed 's#^./##' \
      | sort \
      | xargs shasum -a 256 >checksums.sha256
  )
}

write_artifacts() {
  write_artifact_summary
  write_artifact_readme
  write_artifact_index
  write_artifact_checksums
  write_artifact_index
}
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
  if [ -n "$ARTIFACT_DIR" ]; then
    cp "$before_file" "$ARTIFACT_DIR/containers-before.txt"
    cp "$after_file" "$ARTIFACT_DIR/containers-after.txt"
  fi
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

run_gumgum_artifact() {
  local artifact_name="$1"
  shift
  if [ -n "$ARTIFACT_DIR" ]; then
    run_gumgum "$@" | tee "$ARTIFACT_DIR/$artifact_name.txt"
  else
    run_gumgum "$@"
  fi
}

capture_graph() {
  local output_file="$1"
  local artifact_name="${2:-$(basename "$output_file")}.json"
  # shellcheck disable=SC2086
  if ! $GUMGUM --json graph --host "$HOST" >"$output_file"; then
    echo "warning: could not capture desired graph snapshot" >&2
    : >"$output_file"
  fi
  if [ -n "$ARTIFACT_DIR" ]; then
    cp "$output_file" "$ARTIFACT_DIR/$artifact_name"
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

if [ "$APPLY" = "1" ] && [ "$APPLY_OBJECTS" != "1" ] && [ "$DEPLOY_ONLY" != "1" ]; then
  echo "error: APPLY=1 requires APPLY_OBJECTS=1 or DEPLOY_ONLY=1 so deploy bindings exist intentionally" >&2
  exit 1
fi
if [ "$OBJECTS_ONLY" = "1" ] && [ "$APPLY_OBJECTS" != "1" ]; then
  echo "error: OBJECTS_ONLY=1 requires APPLY_OBJECTS=1 so object mutations are explicit" >&2
  exit 1
fi
if [ "$OBJECTS_ONLY" = "1" ] && [ "$DEPLOY_ONLY" = "1" ]; then
  echo "error: OBJECTS_ONLY=1 and DEPLOY_ONLY=1 cannot be combined" >&2
  exit 1
fi
if [ "$OBSERVE_ONLY" = "1" ] && { [ "$OBJECTS_ONLY" = "1" ] || [ "$DEPLOY_ONLY" = "1" ] || [ "$CLEANUP_ONLY" = "1" ] || [ "$SETUP_ONLY" = "1" ] || [ "$UPGRADE_ONLY" = "1" ]; }; then
  echo "error: OBSERVE_ONLY=1 cannot be combined with setup/upgrade/object/deploy/cleanup-only modes" >&2
  exit 1
fi
if [ "$OBJECTS_ONLY" = "1" ] && [ "$CLEANUP_ONLY" = "1" ]; then
  echo "error: OBJECTS_ONLY=1 and CLEANUP_ONLY=1 cannot be combined" >&2
  exit 1
fi
if [ "$DEPLOY_ONLY" = "1" ] && [ "$CLEANUP_ONLY" = "1" ]; then
  echo "error: DEPLOY_ONLY=1 and CLEANUP_ONLY=1 cannot be combined" >&2
  exit 1
fi
if [ "$APPLY_CLEANUP" = "1" ] && [ "$APPLY_OBJECTS" != "1" ] && [ "$CLEANUP_ONLY" != "1" ]; then
  echo "error: APPLY_CLEANUP=1 requires APPLY_OBJECTS=1 or CLEANUP_ONLY=1 to make destructive cleanup explicit" >&2
  exit 1
fi
if [ "$CLEANUP_ONLY" = "1" ] && [ "$VERIFY_CLEANUP_PREVIEW" != "1" ] && [ "$APPLY_CLEANUP" != "1" ]; then
  echo "error: CLEANUP_ONLY=1 requires VERIFY_CLEANUP_PREVIEW=1 or APPLY_CLEANUP=1" >&2
  exit 1
fi
if [ "$VERIFY_SETUP_IDEMPOTENCY" = "1" ] && [ "$RUN_SETUP" != "1" ]; then
  echo "error: VERIFY_SETUP_IDEMPOTENCY=1 requires RUN_SETUP=1" >&2
  exit 1
fi
if [ "$SETUP_ONLY" = "1" ] && [ "$RUN_SETUP" != "1" ]; then
  echo "error: SETUP_ONLY=1 requires RUN_SETUP=1" >&2
  exit 1
fi
if [ "$SETUP_ONLY" = "1" ] && [ "$UPGRADE_ONLY" = "1" ]; then
  echo "error: SETUP_ONLY=1 and UPGRADE_ONLY=1 cannot be combined" >&2
  exit 1
fi
if [ "$APPLY_UPGRADE" = "1" ] && [ "$VERIFY_UPGRADE_IDEMPOTENCY" != "1" ]; then
  echo "error: APPLY_UPGRADE=1 requires VERIFY_UPGRADE_IDEMPOTENCY=1 so real upgrades run the explicit idempotency path" >&2
  exit 1
fi
if [ "$UPGRADE_ONLY" = "1" ] && [ "$VERIFY_UPGRADE_IDEMPOTENCY" != "1" ]; then
  echo "error: UPGRADE_ONLY=1 requires VERIFY_UPGRADE_IDEMPOTENCY=1" >&2
  exit 1
fi
if [ "$REQUIRE_CURRENT_DAEMON" = "1" ] || [ "$APPLY_OBJECTS" = "1" ] || [ "$APPLY" = "1" ] || [ "$APPLY_CLEANUP" = "1" ] || [ "$OBSERVE_ONLY" = "1" ]; then
  require_daemon_capabilities events rollback_revision_id binding_delete object_delete deployment_delete
fi

remote_containers >"$before_file"

if [ "$RUN_SETUP" = "1" ]; then
  run_gumgum setup "$HOST" --root-domain "$ROOT_DOMAIN" --test-domain "$TEST_DOMAIN"
  if [ "$VERIFY_SETUP_IDEMPOTENCY" = "1" ]; then
    run_gumgum setup "$HOST" --root-domain "$ROOT_DOMAIN" --test-domain "$TEST_DOMAIN"
  fi
fi

if [ "$SETUP_ONLY" = "1" ]; then
  container_delta_guard
  write_artifacts
  echo "visit-counter smoke setup-only completed; RUN_SETUP=$RUN_SETUP VERIFY_SETUP_IDEMPOTENCY=$VERIFY_SETUP_IDEMPOTENCY; pre-existing containers preserved"
  exit 0
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

if [ "$UPGRADE_ONLY" = "1" ]; then
  container_delta_guard
  write_artifacts
  echo "visit-counter smoke upgrade-only completed; APPLY_UPGRADE=$APPLY_UPGRADE; pre-existing containers preserved"
  exit 0
fi

if [ "$OBSERVE_ONLY" = "1" ]; then
  run_gumgum_artifact status status --host "$HOST"
  run_gumgum_artifact events events --host "$HOST" --limit 20
  run_gumgum_artifact operations operations --host "$HOST" --limit 20
  run_gumgum_artifact logs-api logs --host "$HOST" api --tail 20 || true
  run_gumgum_artifact logs-worker logs --host "$HOST" worker --tail 20 || true
  container_delta_guard
  write_artifacts
  echo "visit-counter smoke observe-only completed; pre-existing containers preserved"
  exit 0
fi

pushd "$EXAMPLE_DIR" >/dev/null

if [ "$CLEANUP_ONLY" != "1" ] && [ "$DEPLOY_ONLY" != "1" ]; then
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
fi

if [ "$OBJECTS_ONLY" = "1" ]; then
  popd >/dev/null
  container_delta_guard
  write_artifacts
  echo "visit-counter smoke objects-only completed; APPLY_OBJECTS=$APPLY_OBJECTS; pre-existing containers preserved"
  exit 0
fi

if [ "$VERIFY_CLEANUP_PREVIEW" = "1" ] || [ "$APPLY_CLEANUP" = "1" ]; then
  if ! has_daemon_capability "binding_delete" || ! has_daemon_capability "object_delete"; then
    echo "warning: gumgumd on $HOST does not advertise safe delete APIs; run gumgum setup/upgrade before cleanup verification" >&2
  else
    capture_graph "$before_graph_file" graph-before
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
    capture_graph "$after_graph_file" graph-after
    if [ "$APPLY_CLEANUP" = "1" ]; then
      assert_visit_resources_absent "$after_graph_file"
    else
      assert_graph_unchanged
    fi
  fi
fi

if [ "$CLEANUP_ONLY" = "1" ]; then
  popd >/dev/null
  container_delta_guard
  write_artifacts
  echo "visit-counter smoke cleanup-only completed; APPLY_CLEANUP=$APPLY_CLEANUP; pre-existing containers preserved"
  exit 0
fi

if [ "$APPLY" = "1" ]; then
  run_gumgum_artifact deploy deploy --host "$HOST"
  verify_test_dns
  response_file="${ARTIFACT_DIR:-/tmp}/gumgum-visit-counter-response.txt"
  curl -fsS -H "Host: api.visit-counter.${TEST_DOMAIN}" "http://${HOST}/" >"$response_file"
  grep -q "Hello visitor" "$response_file"
  run_gumgum events --host "$HOST" --limit 20
  if [ "$VERIFY_ROLLBACK_PREVIEW" = "1" ]; then
    run_gumgum rollback --host "$HOST" --worker api --preview || true
  fi
  run_gumgum logs --host "$HOST" api --tail 20 || true
  run_gumgum logs --host "$HOST" worker --tail 20 || true
else
  run_gumgum_artifact deploy-dry-run --dry-run deploy --host "$HOST"
fi

popd >/dev/null

container_delta_guard
write_artifacts

echo "visit-counter smoke completed; APPLY_OBJECTS=$APPLY_OBJECTS APPLY=$APPLY DEPLOY_ONLY=$DEPLOY_ONLY APPLY_CLEANUP=$APPLY_CLEANUP; pre-existing containers preserved"
