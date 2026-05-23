#!/usr/bin/env bash
set -euo pipefail

MODE=${MODE:-all}
HOST=${GUMGUM_E2E_HOST:-${HOST:-}}
ROOT_DOMAIN=${GUMGUM_E2E_ROOT_DOMAIN:-${GUMGUM_ROOT_DOMAIN:-}}
GUMGUM=${GUMGUM:-gumgum}
ARTIFACT_DIR=${ARTIFACT_DIR:-}
WORKSPACE=${GUMGUM_E2E_WORKSPACE:-examples/visit-counter}
API_MANIFEST=${GUMGUM_E2E_API_MANIFEST:-api/gumgum.toml}
WORKER_MANIFEST=${GUMGUM_E2E_WORKER_MANIFEST:-worker/gumgum.toml}
API_WORKER=${GUMGUM_E2E_API_WORKER:-visit-counter-api}
GRAFANA_URL=${GUMGUM_GRAFANA_URL:-https://grafana.${ROOT_DOMAIN}}
REQUIRED_CAPABILITIES=${GUMGUM_E2E_REQUIRED_CAPABILITIES:-gumgum:events,gumgum:deployments:stream,gumgum:rollback:revisions,gumgum:rollback:revision_id,gumgum:rollback:revision_delete,gumgum:buckets:objects,gumgum:grafana:artifacts,gumgum:observability:prometheus_scrapes}

if [[ -z "$HOST" || -z "$ROOT_DOMAIN" || "${GUMGUM_ALLOW_MUTATION:-0}" != "1" ]]; then
  cat <<'EOF'
skip: set GUMGUM_E2E_HOST=<disposable-host>, GUMGUM_E2E_ROOT_DOMAIN=<domain>, and GUMGUM_ALLOW_MUTATION=1 to run mutating platform E2E
optional env:
  MODE=all|deploy-grafana|rollback-safety|capabilities
  GUMGUM_E2E_WORKSPACE=examples/visit-counter
  GUMGUM_E2E_API_MANIFEST=api/gumgum.toml
  GUMGUM_E2E_WORKER_MANIFEST=worker/gumgum.toml
  GUMGUM_E2E_API_WORKER=visit-counter-api
  GUMGUM_GRAFANA_URL=https://grafana.<domain>
  ARTIFACT_DIR=/tmp/gumgum-platform-e2e

This script mutates the target by deploying the fixture. It refuses starbase2
unless GUMGUM_ALLOW_STARBASE2_MUTATION=1 is also set.
EOF
  exit 0
fi

if [[ "$HOST" == "starbase2" && "${GUMGUM_ALLOW_STARBASE2_MUTATION:-0}" != "1" ]]; then
  echo "skip: refusing mutating E2E against starbase2 without GUMGUM_ALLOW_STARBASE2_MUTATION=1"
  exit 0
fi

if [[ -n "$ARTIFACT_DIR" ]]; then mkdir -p "$ARTIFACT_DIR"; fi

fail() { echo "error: $*" >&2; exit 1; }

capture() {
  local name=$1
  shift
  if [[ -n "$ARTIFACT_DIR" ]]; then
    "$@" | tee "$ARTIFACT_DIR/$name"
  else
    "$@"
  fi
}

require_contains() {
  local file=$1 needle=$2
  grep -Fq -- "$needle" "$file" || fail "expected $file to contain: $needle"
}

capabilities_smoke() {
  echo "== disposable E2E capabilities: host=$HOST =="
  capture capabilities.txt "$GUMGUM" server capabilities list --host "$HOST" --require "$REQUIRED_CAPABILITIES"
}

deploy_grafana_e2e() {
  echo "== disposable E2E deploy + Grafana artifacts: host=$HOST workspace=$WORKSPACE =="
  (
    cd "$WORKSPACE"
    capture deploy-api.txt "$GUMGUM" deploy "$API_MANIFEST" --host "$HOST"
    capture deploy-worker.txt "$GUMGUM" deploy "$WORKER_MANIFEST" --host "$HOST"
  )
  local search_file dashboard_file uid
  search_file=$(mktemp)
  curl -k -fsS -u "${GUMGUM_GRAFANA_USER:-gumgum}:${GUMGUM_GRAFANA_PASSWORD:-gumgum-local-dev}" \
    "$GRAFANA_URL/api/search?query=API%20Overview" | python3 -m json.tool | tee "$search_file"
  if [[ -n "$ARTIFACT_DIR" ]]; then cp "$search_file" "$ARTIFACT_DIR/grafana-search-after-deploy.json"; fi
  require_contains "$search_file" '"title": "API Overview"'
  uid=$(python3 - "$search_file" <<'PY'
import json, sys
for item in json.load(open(sys.argv[1])):
    if item.get('title') == 'API Overview':
        print(item.get('uid',''))
        break
PY
)
  [[ -n "$uid" ]] || fail "deployed dashboard did not include uid"
  dashboard_file=$(mktemp)
  curl -k -fsS -u "${GUMGUM_GRAFANA_USER:-gumgum}:${GUMGUM_GRAFANA_PASSWORD:-gumgum-local-dev}" \
    "$GRAFANA_URL/api/dashboards/uid/$uid" | python3 -m json.tool | tee "$dashboard_file"
  if [[ -n "$ARTIFACT_DIR" ]]; then cp "$dashboard_file" "$ARTIFACT_DIR/grafana-dashboard-after-deploy.json"; fi
  require_contains "$dashboard_file" "visit_counter_requests_total"
  echo "ok: deploy applied Grafana artifacts"
}

rollback_safety_smoke() {
  echo "== disposable E2E rollback safety preview: host=$HOST worker=$API_WORKER =="
  (
    cd "$WORKSPACE"
    capture rollback-revisions.txt "$GUMGUM" rollback "$API_MANIFEST" --host "$HOST" --worker "$API_WORKER" --revisions --limit 5
    capture rollback-preview.txt "$GUMGUM" rollback "$API_MANIFEST" --host "$HOST" --worker "$API_WORKER" --preview
  )
  if [[ "${GUMGUM_E2E_ROLLBACK_APPLY:-0}" == "1" ]]; then
    (
      cd "$WORKSPACE"
      capture rollback-apply.txt "$GUMGUM" rollback "$API_MANIFEST" --host "$HOST" --worker "$API_WORKER"
    )
  else
    echo "skip: rollback apply requires GUMGUM_E2E_ROLLBACK_APPLY=1"
  fi
}

case "$MODE" in
  all)
    capabilities_smoke
    deploy_grafana_e2e
    rollback_safety_smoke
    ;;
  capabilities) capabilities_smoke ;;
  deploy-grafana) capabilities_smoke; deploy_grafana_e2e ;;
  rollback-safety) capabilities_smoke; rollback_safety_smoke ;;
  *) fail "unknown MODE=$MODE (expected all|deploy-grafana|rollback-safety|capabilities)" ;;
esac

if [[ -n "$ARTIFACT_DIR" ]]; then
  (
    cd "$ARTIFACT_DIR"
    find . -maxdepth 1 -type f -print | sed 's#^./##' | sort > index.txt
    shasum -a 256 * > checksums.sha256
  )
  echo "artifacts: $ARTIFACT_DIR"
fi
