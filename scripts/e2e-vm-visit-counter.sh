#!/usr/bin/env bash
set -euo pipefail

usage() {
  cat <<'USAGE'
Usage: scripts/e2e-vm-visit-counter.sh --host HOST --root-domain DOMAIN --artifact-dir DIR [--name NAME] [--ssh-user USER] [--apply]

Runs the visit-counter VM E2E harness against an explicit isolated host.
This script intentionally has no default host and refuses starbase2. Without
--apply it records the planned GumGum commands but does not mutate the host.

Required:
  --host HOST             isolated VM/server host or configured GumGum server name
  --root-domain DOMAIN    root domain owned by the isolated test VM
  --artifact-dir DIR      directory for transcript, graph/log/container snapshots, checksums

Optional:
  --name NAME             GumGum server name (default: e2e-HOST with unsafe chars replaced)
  --ssh-user USER         SSH user passed to server add/status when needed
  --apply                 actually run setup/resource/deploy/logs/events/bucket/rollback checks
USAGE
}

HOST=""
ROOT_DOMAIN=""
ARTIFACT_DIR=""
SERVER_NAME=""
SSH_USER=""
APPLY=0

while [[ $# -gt 0 ]]; do
  case "$1" in
    --host) HOST="${2:-}"; shift 2 ;;
    --root-domain) ROOT_DOMAIN="${2:-}"; shift 2 ;;
    --artifact-dir) ARTIFACT_DIR="${2:-}"; shift 2 ;;
    --name) SERVER_NAME="${2:-}"; shift 2 ;;
    --ssh-user|--user) SSH_USER="${2:-}"; shift 2 ;;
    --apply) APPLY=1; shift ;;
    -h|--help) usage; exit 0 ;;
    *) echo "unknown argument: $1" >&2; usage >&2; exit 64 ;;
  esac
done

if [[ -z "$HOST" || -z "$ROOT_DOMAIN" || -z "$ARTIFACT_DIR" ]]; then
  echo "--host, --root-domain, and --artifact-dir are required" >&2
  usage >&2
  exit 64
fi
if [[ "$HOST" == "starbase2" || "$HOST" == "192.168.0.3" || "$HOST" == *"starbase2"* ]]; then
  echo "refusing to run VM E2E against starbase2; provide an isolated host" >&2
  exit 64
fi
if [[ "$ROOT_DOMAIN" == "leostera.dev" || "$ROOT_DOMAIN" == "leostera.test" ]]; then
  echo "refusing shared/root developer domains; provide an isolated E2E domain" >&2
  exit 64
fi

SERVER_NAME="${SERVER_NAME:-e2e-${HOST//[^A-Za-z0-9_.-]/-}}"
REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
FIXTURE_DIR="$REPO_ROOT/tests/fixtures/visit-counter"
mkdir -p "$ARTIFACT_DIR"
TRANSCRIPT="$ARTIFACT_DIR/transcript.log"
COMMANDS="$ARTIFACT_DIR/commands.txt"
: >"$TRANSCRIPT"
: >"$COMMANDS"

log() { printf '%s %s\n' "$(date -u +%Y-%m-%dT%H:%M:%SZ)" "$*" | tee -a "$TRANSCRIPT"; }
record_cmd() { printf '%q ' "$@" >>"$COMMANDS"; printf '\n' >>"$COMMANDS"; }
run_cmd() {
  record_cmd "$@"
  log "+ $*"
  if [[ "$APPLY" -eq 1 ]]; then
    "$@" 2>&1 | tee -a "$TRANSCRIPT"
  fi
}
run_in_fixture() {
  record_cmd "$@"
  log "+ (cd tests/fixtures/visit-counter && $*)"
  if [[ "$APPLY" -eq 1 ]]; then
    (cd "$FIXTURE_DIR" && "$@") 2>&1 | tee -a "$TRANSCRIPT"
  fi
}
run_in_fixture_allow_timeout() {
  record_cmd "$@"
  log "+ (cd tests/fixtures/visit-counter && $*)"
  if [[ "$APPLY" -eq 1 ]]; then
    set +e
    (cd "$FIXTURE_DIR" && "$@") 2>&1 | tee -a "$TRANSCRIPT"
    local status=${PIPESTATUS[0]}
    set -e
    if [[ "$status" -ne 0 && "$status" -ne 124 ]]; then
      return "$status"
    fi
  fi
}

GUMGUM=(cargo run -p gumgum-cli --bin gumgum --)
HOST_ARGS=(--host "$SERVER_NAME")
USER_ARGS=()
if [[ -n "$SSH_USER" ]]; then
  USER_ARGS=(--user "$SSH_USER")
fi

log "GumGum visit-counter VM E2E harness"
log "host=$HOST server_name=$SERVER_NAME root_domain=$ROOT_DOMAIN fixture=$FIXTURE_DIR apply=$APPLY"
if [[ "$APPLY" -ne 1 ]]; then
  log "planning only; rerun with --apply to mutate the isolated host"
fi

run_cmd "${GUMGUM[@]}" server add "$HOST" --name "$SERVER_NAME" --root-domain "$ROOT_DOMAIN" "${USER_ARGS[@]}"
run_cmd "${GUMGUM[@]}" status "${HOST_ARGS[@]}"
run_cmd "${GUMGUM[@]}" server capabilities list "${HOST_ARGS[@]}" --require gumgum:events,gumgum:objects:create_preview,gumgum:bindings:create_preview,gumgum:bindings:delete,gumgum:objects:delete,gumgum:deployments:delete,gumgum:buckets:objects

run_in_fixture "${GUMGUM[@]}" db create visits "${HOST_ARGS[@]}"
run_in_fixture "${GUMGUM[@]}" kv create user-counters "${HOST_ARGS[@]}"
run_in_fixture "${GUMGUM[@]}" bucket create visit-requests "${HOST_ARGS[@]}"
run_in_fixture "${GUMGUM[@]}" queue create visit-events "${HOST_ARGS[@]}"
run_in_fixture "${GUMGUM[@]}" db bind visits --to worker --as DATABASE_URL "${HOST_ARGS[@]}"
run_in_fixture "${GUMGUM[@]}" kv bind user-counters --to api --as USER_COUNTERS "${HOST_ARGS[@]}"
run_in_fixture "${GUMGUM[@]}" bucket bind visit-requests --to api --as VISIT_REQUESTS_BUCKET "${HOST_ARGS[@]}"
run_in_fixture "${GUMGUM[@]}" bucket bind visit-requests --to worker --as VISIT_REQUESTS_BUCKET "${HOST_ARGS[@]}"
run_in_fixture "${GUMGUM[@]}" queue bind visit-events --to api --as VISIT_EVENTS_QUEUE "${HOST_ARGS[@]}"
run_in_fixture "${GUMGUM[@]}" queue bind visit-events --to worker --as VISIT_EVENTS_QUEUE "${HOST_ARGS[@]}"

run_in_fixture "${GUMGUM[@]}" deploy "${HOST_ARGS[@]}"
run_in_fixture "${GUMGUM[@]}" env "${HOST_ARGS[@]}" --qualified >"$ARTIFACT_DIR/env.txt"
run_in_fixture "${GUMGUM[@]}" events "${HOST_ARGS[@]}" --limit 50 >"$ARTIFACT_DIR/events.txt"
run_in_fixture "${GUMGUM[@]}" events "${HOST_ARGS[@]}" --grouped --limit 20 >"$ARTIFACT_DIR/events-grouped.txt"
run_in_fixture_allow_timeout timeout 20s "${GUMGUM[@]}" logs -f "${HOST_ARGS[@]}" --tail 20 >"$ARTIFACT_DIR/logs-follow.txt"
run_in_fixture "${GUMGUM[@]}" bucket cp ./README.md visit-requests/e2e/README.md "${HOST_ARGS[@]}"
run_in_fixture "${GUMGUM[@]}" bucket cp visit-requests/e2e/README.md visit-requests/e2e/README.copy.md "${HOST_ARGS[@]}"
run_in_fixture "${GUMGUM[@]}" bucket ls visit-requests e2e/ "${HOST_ARGS[@]}" >"$ARTIFACT_DIR/bucket-ls.txt"
run_in_fixture "${GUMGUM[@]}" rollback api/gumgum.toml "${HOST_ARGS[@]}" --worker visit-counter-api --preview >"$ARTIFACT_DIR/rollback-api-preview.txt"
run_in_fixture "${GUMGUM[@]}" --dry-run publish api/gumgum.toml "${HOST_ARGS[@]}" >"$ARTIFACT_DIR/publish-api-dry-run.txt"
run_in_fixture "${GUMGUM[@]}" graph "${HOST_ARGS[@]}" >"$ARTIFACT_DIR/graph.txt"

if command -v sha256sum >/dev/null 2>&1; then
  (cd "$ARTIFACT_DIR" && find . -type f ! -name checksums.sha256 -print0 | sort -z | xargs -0 sha256sum >checksums.sha256)
elif command -v shasum >/dev/null 2>&1; then
  (cd "$ARTIFACT_DIR" && find . -type f ! -name checksums.sha256 -print0 | sort -z | xargs -0 shasum -a 256 >checksums.sha256)
fi

log "done; artifacts in $ARTIFACT_DIR"
