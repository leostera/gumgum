CREATE TABLE IF NOT EXISTS desired_providers (
    name TEXT PRIMARY KEY,
    capability TEXT NOT NULL,
    updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE TABLE IF NOT EXISTS desired_deployments (
    worker TEXT PRIMARY KEY,
    image TEXT NOT NULL,
    container TEXT NOT NULL,
    route TEXT,
    port INTEGER NOT NULL,
    health TEXT NOT NULL,
    updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE TABLE IF NOT EXISTS global_objects (
    kind TEXT NOT NULL,
    name TEXT NOT NULL,
    namespace TEXT NOT NULL,
    root_domain TEXT NOT NULL,
    dns TEXT NOT NULL,
    provider TEXT NOT NULL,
    status TEXT NOT NULL,
    updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    PRIMARY KEY(kind, name)
);

CREATE TABLE IF NOT EXISTS bindings (
    worker TEXT NOT NULL,
    binding TEXT NOT NULL,
    object_kind TEXT NOT NULL,
    object_name TEXT NOT NULL,
    access TEXT NOT NULL,
    updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    PRIMARY KEY(worker, binding)
);

CREATE TABLE IF NOT EXISTS deployment_revisions (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    worker TEXT NOT NULL,
    image TEXT NOT NULL,
    container TEXT NOT NULL,
    route TEXT,
    port INTEGER NOT NULL,
    health TEXT NOT NULL,
    created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE TABLE IF NOT EXISTS object_secrets (
    object_kind TEXT NOT NULL,
    object_name TEXT NOT NULL,
    key TEXT NOT NULL,
    value TEXT NOT NULL,
    created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    PRIMARY KEY(object_kind, object_name, key)
);

CREATE TABLE IF NOT EXISTS reconciliation_events (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    kind TEXT NOT NULL DEFAULT 'reconciliation',
    operation_id TEXT,
    status TEXT NOT NULL,
    target TEXT NOT NULL,
    action TEXT NOT NULL,
    message TEXT NOT NULL,
    created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
);
