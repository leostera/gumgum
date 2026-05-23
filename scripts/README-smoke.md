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
MODE=all        scripts/smoke-platform-observe.sh
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

This script is observation-only. It does not deploy, mutate desired state, or stop
containers. It may be run against starbase2 explicitly:

```bash
GUMGUM_SMOKE_HOST=starbase2 \
GUMGUM_ROOT_DOMAIN=leostera.dev \
ARTIFACT_DIR=/tmp/gumgum-platform-smoke-starbase2 \
scripts/smoke-platform-observe.sh
```

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
