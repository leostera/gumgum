# Visit Counter E2E Fixture

This is the maintained visit-counter fixture for GumGum integration and VM E2E tests.

It exercises:

- API worker HTTPS ingress.
- Background worker deployment without invented public ingress.
- DB, KV, bucket, and queue objects.
- Worker bindings and provider-projected environment variables.
- Bucket object local-to-remote and remote-to-remote commands.
- Logs, events, grouped events, rollback preview, publish dry-run, and delete guards.

Use the explicit VM harness from the repository root:

```bash
scripts/e2e-vm-visit-counter.sh \
  --host <isolated-host> \
  --root-domain <isolated-domain> \
  --artifact-dir /tmp/gumgum-e2e-visit-counter

# Mutating run, only against an isolated VM/host:
scripts/e2e-vm-visit-counter.sh \
  --host <isolated-host> \
  --root-domain <isolated-domain> \
  --artifact-dir /tmp/gumgum-e2e-visit-counter \
  --apply
```

The harness has no default host and refuses known shared hosts/domains such as starbase2.
