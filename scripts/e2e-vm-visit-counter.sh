#!/usr/bin/env bash
set -euo pipefail

usage() {
  cat <<'USAGE'
Usage: scripts/e2e-vm-visit-counter.sh --host HOST --root-domain DOMAIN --artifact-dir DIR [--name NAME] [--test-domain DOMAIN] [--apply]

Runs the regular ignored Rust visit-counter E2E test against an explicit isolated host.
This wrapper intentionally has no default host and refuses starbase2/shared domains.
Without --apply, the Rust test performs its plan-only smoke path.

Required:
  --host HOST             isolated VM/server host or configured GumGum server name
  --root-domain DOMAIN    root domain owned by the isolated test VM
  --artifact-dir DIR      directory for transcript, graph/log/container snapshots, checksums

Optional:
  --name NAME             GumGum server name used by the test (default: e2e)
  --test-domain DOMAIN    test domain (default: test.ROOT_DOMAIN)
  --apply                 allow the ignored Rust E2E to mutate the isolated host
USAGE
}

HOST=""
ROOT_DOMAIN=""
ARTIFACT_DIR=""
SERVER_NAME="e2e"
TEST_DOMAIN=""
APPLY=0

while [[ $# -gt 0 ]]; do
  case "$1" in
    --host) HOST="${2:-}"; shift 2 ;;
    --root-domain) ROOT_DOMAIN="${2:-}"; shift 2 ;;
    --artifact-dir) ARTIFACT_DIR="${2:-}"; shift 2 ;;
    --name) SERVER_NAME="${2:-}"; shift 2 ;;
    --test-domain) TEST_DOMAIN="${2:-}"; shift 2 ;;
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

export GUMGUM_E2E_HOST="$HOST"
export GUMGUM_E2E_ROOT_DOMAIN="$ROOT_DOMAIN"
export GUMGUM_E2E_ARTIFACT_DIR="$ARTIFACT_DIR"
export GUMGUM_E2E_SERVER_NAME="$SERVER_NAME"
if [[ -n "$TEST_DOMAIN" ]]; then
  export GUMGUM_E2E_TEST_DOMAIN="$TEST_DOMAIN"
fi
if [[ "$APPLY" -eq 1 ]]; then
  export GUMGUM_E2E_APPLY=1
else
  unset GUMGUM_E2E_APPLY || true
fi

cargo test -p gumgum-cli --test visit_counter_e2e -- --ignored --exact visit_counter_deploys_from_fixture_manifest --nocapture
