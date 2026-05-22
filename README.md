# gumgum

gumgum is a self-hosted app platform for your VPS or local machine. It gives small projects a Cloudflare-like workflow without handing the runtime to a hosted control plane: deploy workers, attach databases and queues, inspect logs/events, and publish routes from one CLI.

## Why gumgum?

- **One server, one simple platform.** Turn a Linux host or your local computer into a small PaaS.
- **Batteries included.** gumgum brings the core backing services most apps need: Postgres, Redis, MinIO/S3 buckets, Redpanda queues, and secrets.
- **Project-first workflow.** Create resources, bind them to workers, deploy, view logs, and inspect events from the project directory.
- **Local-first control.** The daemon runs on your server and converges desired state there; your CLI sends intent, not manual Docker choreography.
- **Human CLI by default.** Commands print readable output unless you ask for `--json`.

## What gumgum manages

gumgum can manage:

- app/worker deployment to Docker
- `.test` local routes through the gumgum gateway
- Postgres databases
- Redis KV namespaces
- MinIO/S3 buckets and bucket objects
- Redpanda queues/topics
- worker environment bindings
- logs, events, grouped event summaries, rollback metadata, and publish previews

## Install

Build the CLI from this repository for now:

```bash
git clone https://github.com/leostera/gumgum.git
cd gumgum
cargo build --release
install -m 0755 target/release/gumgum ~/.local/bin/gumgum
```

Make sure `~/.local/bin` is on your `PATH`.

## Set up a server

A gumgum server is the host that owns your app runtime and root domain. `server add` installs/configures `gumgumd`, starts the built-in providers, saves the server locally, and configures local resolver entries for the server's test domain.

### Local machine

```bash
gumgum server add 0.0.0.0 --name local --root-domain example.dev
```

### Remote host

```bash
gumgum server add 203.0.113.10 --name prod --root-domain example.com
```

Use `--user` if SSH needs an explicit user:

```bash
gumgum server add 203.0.113.10 --user root --name prod --root-domain example.com
```

Inspect configured servers:

```bash
gumgum server list
gumgum server ping --host prod
gumgum server capabilities list --host prod
```

Remove a server from your local list:

```bash
gumgum server rm prod
```

## Create a project

Initialize a worker or workspace:

```bash
gumgum init --name api --kind worker
```

Create backing resources:

```bash
gumgum db create app-db
gumgum kv create app-cache
gumgum bucket create uploads
gumgum queue create jobs
```

Bind resources to a worker:

```bash
gumgum db bind app-db --to api --as DATABASE_URL
gumgum kv bind app-cache --to api --as CACHE
gumgum bucket bind uploads --to api --as UPLOADS_BUCKET
gumgum queue bind jobs --to api --as JOBS_QUEUE
```

Deploy:

```bash
gumgum deploy
```

## Day-to-day commands

```bash
gumgum status
gumgum logs
gumgum env
gumgum events
gumgum events --grouped
gumgum graph
gumgum graph affected worker/api
```

List resources:

```bash
gumgum db list
gumgum kv list
gumgum bucket list
gumgum queue list
gumgum secret list
```

Work with bucket objects:

```bash
gumgum bucket ls uploads
gumgum bucket get uploads path/to/file.json
gumgum bucket cp ./local.json uploads/local.json
gumgum bucket cp uploads/local.json ./local-copy.json
gumgum bucket cp uploads/local.json uploads/archive/local.json
gumgum bucket rm uploads archive/local.json
```

Preview public publishing before changing public route state:

```bash
gumgum --dry-run publish
```

## Configuration

View local gumgum config:

```bash
gumgum config list
```

Config keys are schema-backed, not arbitrary key/value storage:

```bash
gumgum config get ui.color
gumgum config set ui.color true
```

## JSON output

Use `--json` when scripting:

```bash
gumgum --json status
gumgum --json events --limit 10
```

## Example

See `examples/visit-counter` for an end-to-end app that uses DB, KV, bucket, queue, worker bindings, logs, events, rollback previews, and publish dry-runs.
