#!/usr/bin/env bash
set -euo pipefail

MODE=${MODE:-all}
HOST=${GUMGUM_SMOKE_HOST:-${HOST:-}}
ROOT_DOMAIN=${GUMGUM_ROOT_DOMAIN:-${ROOT_DOMAIN:-leostera.dev}}
GUMGUM=${GUMGUM:-gumgum}
GRAFANA_URL=${GUMGUM_GRAFANA_URL:-https://grafana.${ROOT_DOMAIN}}
GRAFANA_USER=${GUMGUM_GRAFANA_USER:-gumgum}
GRAFANA_PASSWORD=${GUMGUM_GRAFANA_PASSWORD:-gumgum-local-dev}
DASHBOARD_QUERY=${GUMGUM_GRAFANA_DASHBOARD_QUERY:-API Overview}
PROMETHEUS_EXPECT_JOBS=${GUMGUM_PROMETHEUS_EXPECT_JOBS:-gumgum-preview-api,gumgum-prod-api}
PROMETHEUS_QUERY=${GUMGUM_PROMETHEUS_QUERY:-visit_counter_info}
ARTIFACT_DIR=${ARTIFACT_DIR:-}

if [[ -z "$HOST" ]]; then
  cat <<'EOF'
skip: set GUMGUM_SMOKE_HOST=<host> to run platform observation smoke checks
optional env:
  MODE=all|status|grafana|prometheus
  GUMGUM_ROOT_DOMAIN=leostera.dev
  GUMGUM_GRAFANA_URL=https://grafana.<root-domain>
  GUMGUM_GRAFANA_USER=gumgum
  GUMGUM_GRAFANA_PASSWORD=...
  GUMGUM_GRAFANA_DASHBOARD_QUERY='API Overview'
  GUMGUM_PROMETHEUS_EXPECT_JOBS='gumgum-preview-api,gumgum-prod-api'
  GUMGUM_PROMETHEUS_QUERY='visit_counter_info'
  ARTIFACT_DIR=/tmp/gumgum-platform-smoke
EOF
  exit 0
fi

if [[ -n "$ARTIFACT_DIR" ]]; then
  mkdir -p "$ARTIFACT_DIR"
fi

capture() {
  local name=$1
  shift
  if [[ -n "$ARTIFACT_DIR" ]]; then
    "$@" | tee "$ARTIFACT_DIR/$name"
  else
    "$@"
  fi
}

fail() {
  echo "error: $*" >&2
  exit 1
}

require_contains() {
  local haystack=$1
  local needle=$2
  if ! grep -Fq -- "$needle" "$haystack"; then
    fail "expected $haystack to contain: $needle"
  fi
}

status_smoke() {
  echo "== platform status smoke: host=$HOST root_domain=$ROOT_DOMAIN =="
  local status_file
  status_file=$(mktemp)
  (cd examples/visit-counter && "$GUMGUM" status --host "$HOST") | tee "$status_file"
  if [[ -n "$ARTIFACT_DIR" ]]; then cp "$status_file" "$ARTIFACT_DIR/status.txt"; fi

  require_contains "$status_file" "gumgumd: healthy"
  require_contains "$status_file" "Providers: 9/9 running"
  if grep -Fq "provider warning" "$status_file"; then
    fail "status emitted provider warning"
  fi

  local docker_file
  docker_file=$(mktemp)
  ssh "$HOST" 'docker ps --format "{{.Names}}	{{.Status}}	{{.Ports}}"' | tee "$docker_file"
  if [[ -n "$ARTIFACT_DIR" ]]; then cp "$docker_file" "$ARTIFACT_DIR/docker-ps.txt"; fi

  for container in \
    gumgum-vaultwarden gumgum-otel gumgum-prometheus gumgum-grafana \
    gumgum-loki gumgum-tempo gumgum-caddy gumgum-cloudflared; do
    require_contains "$docker_file" "$container"
  done
  require_contains "$docker_file" "0.0.0.0:80->80/tcp"
  require_contains "$docker_file" "0.0.0.0:443->443/tcp"

  local labels_file
  labels_file=$(mktemp)
  ssh "$HOST" 'docker inspect -f "{{json .Config.Labels}}" gumgum-grafana' | tee "$labels_file"
  if [[ -n "$ARTIFACT_DIR" ]]; then cp "$labels_file" "$ARTIFACT_DIR/grafana-labels.json"; fi
  require_contains "$labels_file" "\"caddy\":\"grafana.${ROOT_DOMAIN}\""
  require_contains "$labels_file" "\"gumgum.platform.service\":\"grafana\""

  echo "ok: platform status smoke passed"
}

prometheus_smoke() {
  echo "== prometheus API smoke: host=$HOST jobs=$PROMETHEUS_EXPECT_JOBS query=$PROMETHEUS_QUERY =="
  local targets_file
  targets_file=$(mktemp)
  ssh "$HOST" 'ip=$(docker inspect -f "{{range .NetworkSettings.Networks}}{{.IPAddress}}{{end}}" gumgum-prometheus); curl -fsS "http://$ip:9090/api/v1/targets?state=active"' \
    | python3 -m json.tool | tee "$targets_file"
  if [[ -n "$ARTIFACT_DIR" ]]; then cp "$targets_file" "$ARTIFACT_DIR/prometheus-targets.json"; fi

  python3 - "$targets_file" "$PROMETHEUS_EXPECT_JOBS" <<'PY'
import json, sys
path, jobs = sys.argv[1], [job for job in sys.argv[2].split(',') if job]
data = json.load(open(path))
active = data.get('data', {}).get('activeTargets', [])
by_job = {}
for target in active:
    job = target.get('labels', {}).get('job') or target.get('discoveredLabels', {}).get('job')
    if job:
        by_job.setdefault(job, []).append(target)
missing = []
unhealthy = []
for job in jobs:
    targets = by_job.get(job, [])
    if not targets:
        missing.append(job)
    elif not any(target.get('health') == 'up' for target in targets):
        unhealthy.append(job)
if missing or unhealthy:
    raise SystemExit(f"missing_jobs={missing} unhealthy_jobs={unhealthy}")
PY

  local query_file
  query_file=$(mktemp)
  local encoded_query
  encoded_query=$(python3 - "$PROMETHEUS_QUERY" <<'PY'
import sys, urllib.parse
print(urllib.parse.quote(sys.argv[1]))
PY
)
  ssh "$HOST" "ip=\$(docker inspect -f '{{range .NetworkSettings.Networks}}{{.IPAddress}}{{end}}' gumgum-prometheus); curl -fsS \"http://\$ip:9090/api/v1/query?query=$encoded_query\"" \
    | python3 -m json.tool | tee "$query_file"
  if [[ -n "$ARTIFACT_DIR" ]]; then cp "$query_file" "$ARTIFACT_DIR/prometheus-query.json"; fi
  python3 - "$query_file" <<'PY'
import json, sys
data=json.load(open(sys.argv[1]))
if data.get('status') != 'success':
    raise SystemExit('query did not succeed')
if not data.get('data', {}).get('result'):
    raise SystemExit('query returned no samples')
PY
  echo "ok: prometheus API smoke passed"
}

grafana_smoke() {
  echo "== grafana public/API smoke: url=$GRAFANA_URL dashboard=$DASHBOARD_QUERY =="
  local login_file
  login_file=$(mktemp)
  curl -k -fsSI "$GRAFANA_URL/login" | tee "$login_file"
  if [[ -n "$ARTIFACT_DIR" ]]; then cp "$login_file" "$ARTIFACT_DIR/grafana-login-headers.txt"; fi
  require_contains "$login_file" "HTTP/"

  local datasources_file
  datasources_file=$(mktemp)
  curl -k -fsS -u "$GRAFANA_USER:$GRAFANA_PASSWORD" "$GRAFANA_URL/api/datasources" \
    | python3 -m json.tool | tee "$datasources_file"
  if [[ -n "$ARTIFACT_DIR" ]]; then cp "$datasources_file" "$ARTIFACT_DIR/grafana-datasources.json"; fi
  for datasource in Prometheus Loki Tempo; do
    require_contains "$datasources_file" "\"name\": \"$datasource\""
  done

  local encoded_query
  encoded_query=$(python3 - "$DASHBOARD_QUERY" <<'PY'
import sys, urllib.parse
print(urllib.parse.quote(sys.argv[1]))
PY
)
  local search_file
  search_file=$(mktemp)
  curl -k -fsS -u "$GRAFANA_USER:$GRAFANA_PASSWORD" \
    "$GRAFANA_URL/api/search?query=$encoded_query" | python3 -m json.tool | tee "$search_file"
  if [[ -n "$ARTIFACT_DIR" ]]; then cp "$search_file" "$ARTIFACT_DIR/grafana-dashboard-search.json"; fi
  require_contains "$search_file" "\"title\": \"$DASHBOARD_QUERY\""

  local uid
  uid=$(python3 - "$search_file" "$DASHBOARD_QUERY" <<'PY'
import json, sys
items=json.load(open(sys.argv[1]))
query=sys.argv[2]
for item in items:
    if item.get('title') == query:
        print(item.get('uid',''))
        break
PY
)
  [[ -n "$uid" ]] || fail "dashboard $DASHBOARD_QUERY did not include a uid"

  local dashboard_file
  dashboard_file=$(mktemp)
  curl -k -fsS -u "$GRAFANA_USER:$GRAFANA_PASSWORD" \
    "$GRAFANA_URL/api/dashboards/uid/$uid" | python3 -m json.tool | tee "$dashboard_file"
  if [[ -n "$ARTIFACT_DIR" ]]; then cp "$dashboard_file" "$ARTIFACT_DIR/grafana-dashboard.json"; fi
  require_contains "$dashboard_file" "\"title\": \"$DASHBOARD_QUERY\""
  require_contains "$dashboard_file" "visit_counter_requests_total"

  echo "ok: grafana public/API smoke passed"
}

case "$MODE" in
  all)
    status_smoke
    grafana_smoke
    prometheus_smoke
    ;;
  status) status_smoke ;;
  grafana) grafana_smoke ;;
  prometheus) prometheus_smoke ;;
  *) fail "unknown MODE=$MODE (expected all|status|grafana|prometheus)" ;;
esac

if [[ -n "$ARTIFACT_DIR" ]]; then
  (
    cd "$ARTIFACT_DIR"
    find . -maxdepth 1 -type f -print | sed 's#^./##' | sort > index.txt
    shasum -a 256 * > checksums.sha256
  )
  echo "artifacts: $ARTIFACT_DIR"
fi
