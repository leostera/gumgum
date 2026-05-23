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
CLOUDFLARE_EXPECT_HOSTS=${GUMGUM_CLOUDFLARE_EXPECT_HOSTS:-grafana.${ROOT_DOMAIN}}
ENV_EXPECT_CONTAINERS=${GUMGUM_ENV_EXPECT_CONTAINERS:-gumgum-preview-dev-leostera-visit-counter-api,gumgum-prod-dev-leostera-visit-counter-api,gumgum-preview-provider-redis,gumgum-prod-provider-redis}
ARTIFACT_DIR=${ARTIFACT_DIR:-}

if [[ -z "$HOST" ]]; then
  cat <<'EOF'
skip: set GUMGUM_SMOKE_HOST=<host> to run platform observation smoke checks
optional env:
  MODE=all|status|grafana|prometheus|cloudflare|backends|env|idempotency
  GUMGUM_ROOT_DOMAIN=leostera.dev
  GUMGUM_GRAFANA_URL=https://grafana.<root-domain>
  GUMGUM_GRAFANA_USER=gumgum
  GUMGUM_GRAFANA_PASSWORD=...
  GUMGUM_GRAFANA_DASHBOARD_QUERY='API Overview'
  GUMGUM_PROMETHEUS_EXPECT_JOBS='gumgum-preview-api,gumgum-prod-api'
  GUMGUM_PROMETHEUS_QUERY='visit_counter_info'
  GUMGUM_CLOUDFLARE_EXPECT_HOSTS='grafana.<root-domain>,visit-counter.<root-domain>'
  GUMGUM_ENV_EXPECT_CONTAINERS='gumgum-preview-dev-leostera-visit-counter-api,gumgum-prod-dev-leostera-visit-counter-api,gumgum-preview-provider-redis,gumgum-prod-provider-redis'
  GUMGUM_ALLOW_MUTATION=1  # required only for MODE=idempotency
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

cloudflare_smoke() {
  echo "== Cloudflare/tunnel smoke: host=$HOST hosts=$CLOUDFLARE_EXPECT_HOSTS =="
  local cf_file
  cf_file=$(mktemp)
  ssh "$HOST" "GUMGUM_CLOUDFLARE_EXPECT_HOSTS='$CLOUDFLARE_EXPECT_HOSTS' python3 -" <<'PY' | python3 -m json.tool | tee "$cf_file"
import json, os, pathlib, urllib.request, urllib.parse
root = pathlib.Path.home() / '.gumgum'
grant_path = root / 'cloudflare' / 'grant.json'
domains_path = root / 'domains.json'
if not grant_path.exists():
    raise SystemExit('missing Cloudflare grant on remote host')
if not domains_path.exists():
    raise SystemExit('missing domains.json on remote host')
grant = json.load(open(grant_path))
headers = {'Authorization': 'Bearer ' + grant['access_token']}
hosts = [host for host in os.environ['GUMGUM_CLOUDFLARE_EXPECT_HOSTS'].split(',') if host]
zone_name = grant.get('zone_name') or max(hosts, key=len).split('.', 1)[1]
zone_resp = json.load(urllib.request.urlopen(urllib.request.Request(
    'https://api.cloudflare.com/client/v4/zones?name=' + urllib.parse.quote(zone_name),
    headers=headers,
), timeout=30))
if not zone_resp.get('success') or not zone_resp.get('result'):
    raise SystemExit('Cloudflare zone lookup failed')
zone = zone_resp['result'][0]
account_id = zone['account']['id']
tunnels = json.load(urllib.request.urlopen(urllib.request.Request(
    f'https://api.cloudflare.com/client/v4/accounts/{account_id}/cfd_tunnel?name=gumgum',
    headers=headers,
), timeout=30))['result']
tunnel = next((t for t in tunnels if not t.get('deleted') and t.get('deleted_at') is None), None)
if not tunnel:
    raise SystemExit('gumgum Cloudflare tunnel missing')
config = json.load(urllib.request.urlopen(urllib.request.Request(
    f'https://api.cloudflare.com/client/v4/accounts/{account_id}/cfd_tunnel/{tunnel["id"]}/configurations',
    headers=headers,
), timeout=30)).get('result', {}).get('config', {})
ingress_hosts = {entry.get('hostname') for entry in config.get('ingress', []) if entry.get('hostname')}
records = {}
for host in hosts:
    rec_resp = json.load(urllib.request.urlopen(urllib.request.Request(
        f'https://api.cloudflare.com/client/v4/zones/{zone["id"]}/dns_records?name=' + urllib.parse.quote(host),
        headers=headers,
    ), timeout=30))
    matches = rec_resp.get('result', [])
    if not matches:
        raise SystemExit(f'missing Cloudflare DNS record for {host}')
    if host not in ingress_hosts:
        raise SystemExit(f'missing tunnel ingress for {host}')
    records[host] = [{k: rec.get(k) for k in ('type', 'name', 'content', 'proxied', 'comment')} for rec in matches]
print(json.dumps({'zone': zone_name, 'tunnel': tunnel['id'], 'tunnel_status': tunnel.get('status'), 'hosts': hosts, 'records': records}, indent=2))
PY
  if [[ -n "$ARTIFACT_DIR" ]]; then cp "$cf_file" "$ARTIFACT_DIR/cloudflare.json"; fi
  for host in ${CLOUDFLARE_EXPECT_HOSTS//,/ }; do
    local dns_file
    dns_file=$(mktemp)
    { dig +short "$host" A || true; dig +short "$host" CNAME || true; } | tee "$dns_file"
    if [[ -n "$ARTIFACT_DIR" ]]; then cp "$dns_file" "$ARTIFACT_DIR/dns-$host.txt"; fi
    if [[ ! -s "$dns_file" ]]; then
      fail "public DNS did not resolve $host"
    fi
  done
  echo "ok: Cloudflare/tunnel smoke passed"
}

backends_smoke() {
  echo "== observability backend smoke: host=$HOST =="
  local backends_file
  backends_file=$(mktemp)
  ssh "$HOST" 'python3 - <<"PY"
import json, socket, subprocess, urllib.request

def ip(name):
    out = subprocess.check_output(["docker", "inspect", "-f", "{{range .NetworkSettings.Networks}}{{.IPAddress}}{{end}}", name], text=True).strip()
    if not out:
        raise SystemExit(f"missing ip for {name}")
    return out

def http_get(name, port, path):
    import time
    addr = ip(name)
    last = None
    for _ in range(6):
        try:
            with urllib.request.urlopen(f"http://{addr}:{port}{path}", timeout=10) as resp:
                return resp.status, resp.read(200).decode(errors="replace")
        except Exception as error:
            last = error
            time.sleep(5)
    raise last

def tcp_open(name, port):
    addr = ip(name)
    with socket.create_connection((addr, port), timeout=5):
        return True

results = {}
for name, port, path in [("gumgum-loki", 3100, "/ready"), ("gumgum-tempo", 3200, "/ready")]:
    status, body = http_get(name, port, path)
    if status >= 400:
        raise SystemExit(f"{name}{path} returned {status}")
    results[name] = {"status": status, "body": body[:120]}
for port in (4317, 4318):
    if not tcp_open("gumgum-otel", port):
        raise SystemExit(f"gumgum-otel:{port} not reachable")
results["gumgum-otel"] = {"ports": [4317, 4318]}
inspect = subprocess.check_output(["docker", "inspect", "gumgum-vaultwarden"], text=True)
value = json.loads(inspect)[0]
env = value.get("Config", {}).get("Env", [])
mounts = value.get("Mounts", [])
if "SIGNUPS_ALLOWED=false" not in env:
    raise SystemExit("Vaultwarden signups are not disabled")
if not any(mount.get("Destination") == "/data" for mount in mounts):
    raise SystemExit("Vaultwarden /data mount missing")
results["gumgum-vaultwarden"] = {"signups_disabled": True, "data_mount": True}
print(json.dumps(results, indent=2))
PY' | python3 -m json.tool | tee "$backends_file"
  if [[ -n "$ARTIFACT_DIR" ]]; then cp "$backends_file" "$ARTIFACT_DIR/backends.json"; fi
  echo "ok: observability backend smoke passed"
}

env_smoke() {
  echo "== environment isolation smoke: host=$HOST containers=$ENV_EXPECT_CONTAINERS =="
  local env_file
  env_file=$(mktemp)
  ssh "$HOST" "GUMGUM_ENV_EXPECT_CONTAINERS='$ENV_EXPECT_CONTAINERS' python3 -" <<'PY' | python3 -m json.tool | tee "$env_file"
import json, os, subprocess
containers = [value for value in os.environ['GUMGUM_ENV_EXPECT_CONTAINERS'].split(',') if value]
results = {}
for name in containers:
    raw = subprocess.check_output(['docker', 'inspect', name], text=True)
    value = json.loads(raw)[0]
    labels = value.get('Config', {}).get('Labels') or {}
    networks = sorted((value.get('NetworkSettings', {}).get('Networks') or {}).keys())
    status = value.get('State', {}).get('Status')
    if status != 'running':
        raise SystemExit(f'{name} is not running: {status}')
    expected_env = None
    if 'gumgum-preview-' in name or '-preview-' in name:
        expected_env = 'preview'
    elif 'gumgum-prod-' in name or '-prod-' in name:
        expected_env = 'prod'
    if expected_env and labels.get('gumgum.environment') != expected_env:
        raise SystemExit(f'{name} missing gumgum.environment={expected_env}: {labels}')
    if expected_env and '-provider-' not in name and not any(expected_env in network for network in networks):
        raise SystemExit(f'{name} is not attached to an {expected_env} network: {networks}')
    results[name] = {'environment': labels.get('gumgum.environment'), 'networks': networks, 'managed': labels.get('gumgum.managed')}
print(json.dumps(results, indent=2))
PY
  if [[ -n "$ARTIFACT_DIR" ]]; then cp "$env_file" "$ARTIFACT_DIR/env-isolation.json"; fi
  echo "ok: environment isolation smoke passed"
}

idempotency_smoke() {
  echo "== platform boot idempotency smoke: host=$HOST =="
  if [[ "${GUMGUM_ALLOW_MUTATION:-0}" != "1" ]]; then
    echo "skip: MODE=idempotency requires GUMGUM_ALLOW_MUTATION=1 because it POSTs /v0/providers/defaults/boot"
    return 0
  fi
  local before_file boot1_file boot2_file after_file
  before_file=$(mktemp)
  boot1_file=$(mktemp)
  boot2_file=$(mktemp)
  after_file=$(mktemp)
  ssh "$HOST" 'docker inspect -f "{{.Name}} {{.Id}}" gumgum-vaultwarden gumgum-otel gumgum-prometheus gumgum-grafana gumgum-loki gumgum-tempo gumgum-caddy gumgum-cloudflared 2>/dev/null | sort' | tee "$before_file"
  curl -fsS -X POST "http://$HOST:7777/v0/providers/defaults/boot" | python3 -m json.tool | tee "$boot1_file"
  curl -fsS -X POST "http://$HOST:7777/v0/providers/defaults/boot" | python3 -m json.tool | tee "$boot2_file"
  ssh "$HOST" 'docker inspect -f "{{.Name}} {{.Id}}" gumgum-vaultwarden gumgum-otel gumgum-prometheus gumgum-grafana gumgum-loki gumgum-tempo gumgum-caddy gumgum-cloudflared 2>/dev/null | sort' | tee "$after_file"
  if [[ -n "$ARTIFACT_DIR" ]]; then
    cp "$before_file" "$ARTIFACT_DIR/idempotency-before.txt"
    cp "$boot1_file" "$ARTIFACT_DIR/idempotency-boot-1.json"
    cp "$boot2_file" "$ARTIFACT_DIR/idempotency-boot-2.json"
    cp "$after_file" "$ARTIFACT_DIR/idempotency-after.txt"
  fi
  if ! diff -u "$before_file" "$after_file"; then
    fail "platform boot changed stable platform container ids"
  fi
  echo "ok: platform boot idempotency smoke passed"
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
    cloudflare_smoke
    backends_smoke
    env_smoke
    ;;
  status) status_smoke ;;
  grafana) grafana_smoke ;;
  prometheus) prometheus_smoke ;;
  cloudflare) cloudflare_smoke ;;
  backends) backends_smoke ;;
  env) env_smoke ;;
  idempotency) idempotency_smoke ;;
  *) fail "unknown MODE=$MODE (expected all|status|grafana|prometheus|cloudflare|backends|env|idempotency)" ;;
esac

if [[ -n "$ARTIFACT_DIR" ]]; then
  (
    cd "$ARTIFACT_DIR"
    find . -maxdepth 1 -type f -print | sed 's#^./##' | sort > index.txt
    shasum -a 256 * > checksums.sha256
  )
  echo "artifacts: $ARTIFACT_DIR"
fi
