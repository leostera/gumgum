ALTER TABLE desired_deployments RENAME TO desired_deployments_old;

CREATE TABLE desired_deployments (
    worker TEXT PRIMARY KEY,
    image TEXT NOT NULL,
    container TEXT NOT NULL,
    route TEXT,
    port INTEGER NOT NULL,
    health TEXT NOT NULL,
    updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
);

INSERT INTO desired_deployments (worker, image, container, route, port, health, updated_at)
SELECT worker, image, container, route, port, health, updated_at
FROM desired_deployments_old;

DROP TABLE desired_deployments_old;
