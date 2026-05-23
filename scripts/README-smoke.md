# GumGum smoke test tiers

These scripts are repeatable smoke checks for platform and fixture E2E behavior.
They are intentionally opt-in: without required environment variables they print skip
instructions and exit successfully.

## Fast local gate

```bash
scripts/test-fast.sh
```

Runs formatting, clippy, Rust tests, and graph property tests. This is the default
pre-commit validation gate.

## Platform observation smoke

```bash
GUMGUM_SMOKE_HOST=<host> \
GUMGUM_ROOT_DOMAIN=<root-domain> \
ARTIFACT_DIR=/tmp/gumgum-platform-smoke \
scripts/smoke-platform-observe.sh
```

Modes:

```bash
MODE=status     scripts/smoke-platform-observe.sh
MODE=grafana    scripts/smoke-platform-observe.sh
MODE=prometheus scripts/smoke-platform-observe.sh
MODE=cloudflare scripts/smoke-platform-observe.sh
MODE=backends   scripts/smoke-platform-observe.sh
MODE=env        scripts/smoke-platform-observe.sh
MODE=bucket     scripts/smoke-platform-observe.sh
MODE=all        scripts/smoke-platform-observe.sh

# Mutating/idempotent boot check; must be explicitly enabled.
GUMGUM_ALLOW_MUTATION=1 MODE=idempotency scripts/smoke-platform-observe.sh
```

Checks performed:

- `gumgum status` reports a healthy daemon and all reported providers running.
- Platform containers are running: Vaultwarden, OTEL, Prometheus, Grafana, Loki,
  Tempo, Caddy, Cloudflared.
- Caddy publishes host ports 80/443 when direct ingress is expected.
- Grafana container labels publish `grafana.<root-domain>` through Caddy.
- Grafana public `/login` responds.
- Grafana API exposes Prometheus, Loki, and Tempo datasources.
- Grafana API can find and fetch the configured dashboard, defaulting to
  `API Overview`, and verifies the fixture metric panel.
- Prometheus active target API reports expected app scrape jobs as `up`.
- Prometheus query API returns samples for the configured fixture metric,
  defaulting to `visit_counter_info`.
- Cloudflare API contains DNS records and tunnel ingress for expected hostnames,
  and public DNS resolves them.
- Loki and Tempo readiness APIs respond, OTEL ports are reachable, and
  Vaultwarden has signups disabled with a persistent `/data` mount.
- Preview/prod fixture app and provider containers carry environment labels and
  are attached to environment-specific Docker networks.
- Bucket object listing works through the daemon for the configured fixture
  bucket, defaulting to `visit-requests`.
- Optional idempotency mode POSTs provider boot twice and verifies stable
  platform container IDs do not change.

This script is observation-only. It does not deploy, mutate desired state, or stop
containers. It may be run against starbase2 explicitly:

```bash
GUMGUM_SMOKE_HOST=starbase2 \
GUMGUM_ROOT_DOMAIN=leostera.dev \
ARTIFACT_DIR=/tmp/gumgum-platform-smoke-starbase2 \
scripts/smoke-platform-observe.sh
```

## Grafana browser smoke

```bash
GUMGUM_BROWSER_SMOKE=1 \
GUMGUM_GRAFANA_URL=https://grafana.<root-domain> \
GUMGUM_GRAFANA_USER=gumgum \
GUMGUM_GRAFANA_PASSWORD=... \
node scripts/grafana-browser-smoke.mjs
```

This optional Playwright-based check skips when `GUMGUM_BROWSER_SMOKE=1` is not
set or when Playwright is unavailable. It logs in, resolves the configured
dashboard through the Grafana API, opens the dashboard URL, and checks that the
fixture panel renders.

## Disposable-host mutating platform E2E

```bash
GUMGUM_E2E_HOST=<disposable-host> \
GUMGUM_E2E_ROOT_DOMAIN=<domain> \
GUMGUM_ALLOW_MUTATION=1 \
ARTIFACT_DIR=/tmp/gumgum-platform-e2e \
scripts/e2e-platform-disposable.sh
```

This tier deploys the visit-counter fixture, verifies Grafana artifacts through
Grafana APIs, and runs rollback revision/preview safety checks. It skips without
explicit host/domain/mutation env vars and refuses `starbase2` unless
`GUMGUM_ALLOW_STARBASE2_MUTATION=1` is also set. Use `MODE=capabilities`,
`MODE=deploy-grafana`, or `MODE=rollback-safety` for narrower checks.

## Visit-counter staged smoke

```bash
scripts/smoke-visit-counter-starbase2.sh --help
scripts/smoke-visit-counter-starbase2.sh --plan
```

This older staged harness covers visit-counter object/deploy/observe/cleanup
flows. Mutation modes are explicit through environment flags.

## Disposable-host E2E

Use the Rust E2E fixture only with explicit disposable host variables:

```bash
GUMGUM_E2E_HOST=<vm-host> \
GUMGUM_E2E_ROOT_DOMAIN=<domain> \
GUMGUM_E2E_ARTIFACT_DIR=/tmp/gumgum-e2e \
cargo test -p gumgum-cli --test visit_counter_e2e -- --ignored
```

Never point destructive disposable-host tests at starbase2 unless the owner has
explicitly authorized that run.
