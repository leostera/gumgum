ALTER TABLE deployment_revisions RENAME TO deployment_revisions_old;

CREATE TABLE deployment_revisions (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    worker TEXT NOT NULL,
    image TEXT NOT NULL,
    container TEXT NOT NULL,
    route TEXT,
    port INTEGER NOT NULL,
    health TEXT NOT NULL,
    created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
);

INSERT INTO deployment_revisions (id, worker, image, container, route, port, health, created_at)
SELECT id, worker, image, container, route, port, health, created_at
FROM deployment_revisions_old;

DROP TABLE deployment_revisions_old;
