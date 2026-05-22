# gumgum

gumgum is a self-hosted app platform for your VPS or local machine. It gives small projects a Cloudflare-like workflow without handing the runtime to a hosted control plane: set up one server, deploy workers, attach databases/KV/buckets/queues/secrets, inspect logs and events, and publish routes from one CLI.

## Table of contents

- [Why gumgum?](#why-gumgum)
- [Core concepts](#core-concepts)
- [What gumgum manages](#what-gumgum-manages)
- [Install the CLI](#install-the-cli)
- [Set up a server](#set-up-a-server)
  - [Local server](#local-server)
  - [Remote server](#remote-server)
  - [Inspect and remove servers](#inspect-and-remove-servers)
- [Create a project or workspace](#create-a-project-or-workspace)
  - [Create a workspace](#create-a-workspace)
  - [Create workers](#create-workers)
- [Manage resources](#manage-resources)
  - [Databases](#databases)
  - [KV namespaces](#kv-namespaces)
  - [Buckets](#buckets)
  - [Queues](#queues)
  - [Secrets](#secrets)
- [Bind resources to workers](#bind-resources-to-workers)
- [Deploy](#deploy)
- [Logs, environment, status, and events](#logs-environment-status-and-events)
- [Graph inspection](#graph-inspection)
- [Bucket object operations](#bucket-object-operations)
- [Publish](#publish)
- [Rollback and revisions](#rollback-and-revisions)
- [Configuration](#configuration)
- [JSON output](#json-output)
- [End-to-end example](#end-to-end-example)
- [Command reference](#command-reference)

## Why gumgum?

- **One server, one simple platform.** Turn a Linux host or your local computer into a small PaaS.
- **Batteries included.** gumgum starts the backing services most apps need: Postgres, Redis, MinIO/S3 buckets, Redpanda queues, and secrets.
- **Project-first workflow.** Create resources, bind them to workers, deploy, view logs, and inspect events from the project directory.
- **Local-first control.** The daemon runs on your server. Your CLI sends intent; gumgum handles runtime/provider convergence there.
- **Human CLI by default.** Commands print readable output unless you ask for `--json`.

## Core concepts

- **Server**: the machine that runs `gumgumd`, Docker workloads, the gateway, and built-in providers. A server owns the root/test domains used by apps hosted there.
- **Project**: one app/worker with a `gumgum.toml` manifest.
- **Workspace**: a directory containing multiple worker projects.
- **Resource**: a backing service object such as a database, KV namespace, bucket, queue, or secret.
- **Binding**: an environment projection from a resource into a worker, for example `DATABASE_URL` or `UPLOADS_BUCKET`.
- **Desired state**: the resources, bindings, routes, and deployments gumgum should maintain on the server.

## What gumgum manages

gumgum can manage:

- app/worker deployment to Docker
- local `.test` routes through the gumgum gateway
- Postgres databases
- Redis KV namespaces
- MinIO/S3 buckets and bucket objects
- Redpanda queues/topics
- worker environment bindings
- logs, events, grouped event summaries, rollback metadata, and publish previews

## Install the CLI

Build the CLI from this repository:

```bash
git clone https://github.com/leostera/gumgum.git
cd gumgum
cargo build --release
install -m 0755 target/release/gumgum ~/.local/bin/gumgum
```

Make sure `~/.local/bin` is on your `PATH`:

```bash
export PATH="$HOME/.local/bin:$PATH"
```

Check the CLI:

```bash
gumgum --help
gumgum version
```

## Set up a server

`gumgum server add` is the main setup command. It installs/configures `gumgumd`, starts built-in providers, saves the server locally, and configures local resolver entries for the server's test domain.

### Local server

Use this when you want to run apps on your own machine:

```bash
gumgum server add 0.0.0.0 --name local --root-domain example.dev
```

This configures a local gumgum server named `local`. gumgum derives a test domain from the root domain, for example `example.test` from `example.dev`.

You can choose an explicit test domain:

```bash
gumgum server add 0.0.0.0 \
  --name local \
  --root-domain example.dev \
  --test-domain example.test
```

### Remote server

Use this when you want to run apps on another machine over SSH:

```bash
gumgum server add 203.0.113.10 --name prod --root-domain example.com
```

If SSH needs a user:

```bash
gumgum server add 203.0.113.10 \
  --user root \
  --name prod \
  --root-domain example.com
```

Preview setup without changing anything:

```bash
gumgum --dry-run server add 203.0.113.10 --name prod --root-domain example.com
```

### Inspect and remove servers

List configured servers:

```bash
gumgum server list
```

Ping the daemon:

```bash
gumgum server ping --host prod
```

Inspect daemon capabilities:

```bash
gumgum server capabilities list --host prod
```

Require specific daemon capabilities:

```bash
gumgum server capabilities list --host prod \
  --require gumgum:events,gumgum:buckets:objects
```

Remove a server from your local config:

```bash
gumgum server rm prod
```

Removing a server record does not delete remote containers or resources.

## Create a project or workspace

### Create a workspace

`gumgum init` creates a workspace manifest in the current directory:

```bash
gumgum init --name my-app
```

Useful options:

```bash
gumgum init --name my-app --root-domain example.com
gumgum init --name my-app --namespace production
gumgum init --name my-app --force
```

### Create workers

Workers are managed with `gumgum worker`, separate from workspace initialization.

Create workers inside the current workspace:

```bash
gumgum worker create api --port 3000
gumgum worker create jobs --port 3001
```

This creates worker folders with their own `gumgum.toml` files and adds them to the workspace members list.

Choose an explicit directory or zones:

```bash
gumgum worker create api --dir apps/api --port 3000
gumgum worker create api --zone example.com --zone example.net
```

List workers:

```bash
gumgum worker list
```

Remove a worker from the workspace:

```bash
gumgum worker delete api
```

`worker delete` only removes the worker from the workspace manifest. It never deletes source files.

Workspace-aware commands such as `logs`, `env`, and `publish --dry-run` can operate across all workers from the workspace root.

## Manage resources

Resource commands share a common shape:

```bash
gumgum <kind> list
gumgum <kind> create <name>
gumgum <kind> delete <name> --preview
gumgum <kind> delete <name>
```

`<kind>` is one of:

- `db`
- `kv`
- `bucket`
- `queue`
- `secret`

Pass `--host <server>` when you want to target a specific server instead of the configured/default one.

### Databases

Create and list Postgres databases:

```bash
gumgum db create app-db
gumgum db list
```

Delete safely with a preview first:

```bash
gumgum db delete app-db --preview
gumgum db delete app-db
```

Create with an explicit namespace or root domain when needed:

```bash
gumgum db create app-db --namespace prod
gumgum db create app-db --root-domain example.com
```

### KV namespaces

Create and list Redis-backed KV namespaces:

```bash
gumgum kv create app-cache
gumgum kv list
```

Delete:

```bash
gumgum kv delete app-cache --preview
gumgum kv delete app-cache
```

### Buckets

Create and list MinIO/S3-backed buckets:

```bash
gumgum bucket create uploads
gumgum bucket list
```

Delete:

```bash
gumgum bucket delete uploads --preview
gumgum bucket delete uploads
```

Bucket object commands are covered in [Bucket object operations](#bucket-object-operations).

### Queues

Create and list Redpanda-backed queues/topics:

```bash
gumgum queue create jobs
gumgum queue list
```

Delete:

```bash
gumgum queue delete jobs --preview
gumgum queue delete jobs
```

### Secrets

Create and list secrets:

```bash
gumgum secret create api-token
gumgum secret list
```

Delete:

```bash
gumgum secret delete api-token --preview
gumgum secret delete api-token
```

## Bind resources to workers

Bindings project resource connection information into a worker's environment.

General shape:

```bash
gumgum <kind> bind <resource-name> --to <worker> --as <ENV_NAME>
gumgum <kind> unbind <resource-name> --to <worker> --as <ENV_NAME> --preview
gumgum <kind> unbind <resource-name> --to <worker> --as <ENV_NAME>
```

Examples:

```bash
gumgum db bind app-db --to api --as DATABASE_URL
gumgum kv bind app-cache --to api --as CACHE
gumgum bucket bind uploads --to api --as UPLOADS_BUCKET
gumgum queue bind jobs --to api --as JOBS_QUEUE
gumgum secret bind api-token --to api --as API_TOKEN
```

Bindings default to read-write access. You can request another access mode:

```bash
gumgum bucket bind uploads --to api --as UPLOADS_BUCKET --access read-only
```

## Deploy

Deploy the current project:

```bash
gumgum deploy
```

Deploy a specific manifest:

```bash
gumgum deploy api/gumgum.toml
```

Target a specific server:

```bash
gumgum deploy --host prod
```

Deploy in production mode:

```bash
gumgum deploy --prod
```

Delete desired deployment state for a worker:

```bash
gumgum deploy api/gumgum.toml --delete
```

Preview deploy behavior without mutating state:

```bash
gumgum --dry-run deploy
```

## Logs, environment, status, and events

Check overall server status:

```bash
gumgum status
gumgum status --host prod
```

Show logs for the current project or workspace:

```bash
gumgum logs
gumgum logs --tail 200
```

Show logs for one worker:

```bash
gumgum logs api
gumgum logs api --tail 100
```

Follow logs for a single worker:

```bash
gumgum logs api --follow
```

Print environment for the current project or workspace:

```bash
gumgum env
```

Narrow environment output:

```bash
gumgum env --project my-app
gumgum env --worker api
gumgum env --project my-app --worker api
```

Workspace environment keys are namespaced by project and worker:

```dotenv
MY_APP_API_DATABASE_URL=...
MY_APP_API_CACHE_URL=...
```

Show control-plane events:

```bash
gumgum events
gumgum events --limit 20
gumgum events --grouped
gumgum events --kind mutation
gumgum events --kind reconciliation
```

## Graph inspection

Render the desired graph:

```bash
gumgum graph
gumgum graph show
```

Show the subgraph affected by a target:

```bash
gumgum graph affected worker/api
gumgum graph affected api
```

The target must exist in the workspace/desired graph. Invalid targets fail instead of returning an empty affected list.

## Bucket object operations

Bucket object commands only exist under `gumgum bucket`.

List objects in a bucket:

```bash
gumgum bucket ls uploads
gumgum bucket ls uploads raw/
```

Read an object to stdout:

```bash
gumgum bucket get uploads path/to/file.json
```

Copy local to remote:

```bash
gumgum bucket cp ./local.json uploads/local.json
```

Copy remote to local:

```bash
gumgum bucket cp uploads/local.json ./local-copy.json
```

Copy remote to remote:

```bash
gumgum bucket cp uploads/local.json uploads/archive/local.json
```

Sync a remote prefix to another remote prefix:

```bash
gumgum bucket sync uploads/raw uploads/archive/raw
```

Remove an object:

```bash
gumgum bucket rm uploads archive/local.json
```

## Publish

Preview public publishing before changing public route state:

```bash
gumgum --dry-run publish
```

Preview a specific target:

```bash
gumgum --dry-run publish api/gumgum.toml
```

Plan an explicit public domain:

```bash
gumgum --dry-run publish api/gumgum.toml --public-domain api.example.com
```

Apply publishing only after reviewing the dry-run:

```bash
gumgum publish api/gumgum.toml --public-domain api.example.com
```

Workspace dry-runs plan each member. `--public-domain` is only valid when publishing one worker.

## Rollback and revisions

Preview rollback for the current project:

```bash
gumgum rollback --preview
```

Preview rollback for a worker:

```bash
gumgum rollback api/gumgum.toml --worker api --preview
```

List deployment revisions:

```bash
gumgum rollback api/gumgum.toml --worker api --revisions
gumgum rollback api/gumgum.toml --worker api --revisions --limit 20
```

Preview or apply a specific revision:

```bash
gumgum rollback api/gumgum.toml --worker api --revision-id 12 --preview
gumgum rollback api/gumgum.toml --worker api --revision-id 12
```

Delete stale revision metadata without changing containers:

```bash
gumgum rollback api/gumgum.toml --worker api --delete-revision-id 12
```

## Configuration

View local gumgum config:

```bash
gumgum config list
```

Read and write known config keys:

```bash
gumgum config get ui.color
gumgum config set ui.color true
gumgum config get format
gumgum config set format human
gumgum config get registry_port
gumgum config set registry_port 5000
```

Config is schema-backed, not arbitrary key/value storage. Unknown keys fail with a human-readable error.

Server-scoped config uses the same schema:

```bash
gumgum server config --host prod list
gumgum server config --host prod get ui.color
gumgum server config --host prod set ui.color true
```

## JSON output

Use `--json` when scripting:

```bash
gumgum --json status
gumgum --json events --limit 10
gumgum --json db list
gumgum --json bucket list
```

Errors are human-readable by default and structured as JSON only with `--json`.

## End-to-end example

The `examples/visit-counter` workspace demonstrates:

- API and background worker projects
- DB/KV/bucket/queue resources
- resource bindings
- deploy
- logs and environment inspection
- events and grouped events
- rollback previews
- publish dry-runs

Try it after configuring a server:

```bash
cd examples/visit-counter
gumgum db create visits
gumgum kv create user-counters
gumgum bucket create visit-requests
gumgum queue create visit-events

gumgum db bind visits --to worker --as DATABASE_URL
gumgum kv bind user-counters --to api --as USER_COUNTERS
gumgum bucket bind visit-requests --to api --as VISIT_REQUESTS_BUCKET
gumgum bucket bind visit-requests --to worker --as VISIT_REQUESTS_BUCKET
gumgum queue bind visit-events --to api --as VISIT_EVENTS_QUEUE
gumgum queue bind visit-events --to worker --as VISIT_EVENTS_QUEUE

gumgum deploy
gumgum logs
gumgum events --grouped
gumgum --dry-run publish
```

## Command reference

Global flags:

```bash
gumgum --json <command>
gumgum --dry-run <command>
```

Server commands:

```bash
gumgum server
gumgum server list
gumgum server add <host> [--name <name>] [--user <user>] [--root-domain <domain>] [--test-domain <domain>]
gumgum server rm <host-or-name>
gumgum server ping --host <host-or-name>
gumgum server capabilities list --host <host-or-name> [--require <capability,...>]
gumgum server config --host <host-or-name> list|get|set
gumgum server upgrade --host <host-or-name>
```

Project/runtime commands:

```bash
gumgum init [--name <name>] [--root-domain <domain>] [--namespace <namespace>] [--force]
gumgum worker create <name> [--port <port>] [--namespace <namespace>] [--dir <path>] [--zone <domain>] [--force]
gumgum worker list [workspace]
gumgum worker delete <name> [workspace]
gumgum deploy [path] [--host <server>] [--prod] [--delete]
gumgum publish [target] [--host <server>] [--public-domain <domain>] [--tunnel <kind>]
gumgum logs [path-or-worker] [--host <server>] [--tail <n>] [--follow]
gumgum env [path] [--host <server>] [--project <name>] [--worker <name>]
gumgum status [--host <server>]
gumgum events [--host <server>] [--limit <n>] [--kind mutation|reconciliation] [--grouped]
gumgum graph [--host <server>] [show|affected <target>]
gumgum rollback [path] [--host <server>] [--worker <name>] [--preview] [--revisions] [--revision-id <id>] [--delete-revision-id <id>]
```

Resource commands:

```bash
gumgum db list|create|delete|bind|unbind
gumgum kv list|create|delete|bind|unbind
gumgum bucket list|create|delete|bind|unbind|ls|get|rm|cp|sync
gumgum queue list|create|delete|bind|unbind
gumgum secret list|create|delete|bind|unbind
```
